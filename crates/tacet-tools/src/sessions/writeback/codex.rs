//! Reporting back to a Codex CLI thread: through its app-server (RG-23), or,
//! as a fallback, by appending to its rollout (RG-24), guarded as RG-31 asks.
//!
//! NOT MEASURED ON A REAL INSTALL. Codex was not installed on the machine this
//! was written on (10 Oct 2026). The app-server protocol below follows the
//! Codex documentation and source (openai/codex @ a06545b: `app-server`'s
//! `initialize` → `thread/resume` → `thread/inject_items`), and is tested
//! against a stand-in server script that answers the same messages; the
//! rollout shapes follow `rollout/src/recorder.rs` and
//! `external-agent-migration/src/sessions/append.rs` and are tested on
//! fixtures. `version::KNOWN` has no Codex row, so the rollout append is
//! REFUSED by the version gate for every Codex version until someone runs the
//! round trip on a real install and records the range there.
//!
//! THE APP-SERVER IS ANOTHER PROGRAM, AND IT IS THE NETWORK ONE STEP AWAY. It
//! runs with the user's Codex login and can reach OpenAI; `thread/inject_items`
//! itself starts no model turn (it adds items to the thread's history, which
//! the model sees on the next turn the user starts), so no paid turn is spent
//! here — but the process is the vendor's, not Tacet's. So it is never found
//! on PATH: a bare name such as `codex` is refused (`Command::new` would search
//! PATH for it, and the question would name a binary nobody chose), the path
//! the user gives is resolved to the absolute file that will run, the question
//! shows THAT path, and nothing starts before a yes.
//!
//! THE ROLLOUT APPEND IS THE FRAGILE PATH. Codex rebuilds the model's context
//! from `response_item` lines and the transcript the user sees from
//! `event_msg` lines, so a note both see is a PAIR, written in one `write`.
//! In `paginated` history mode every line carries the next `ordinal`, and a
//! user message is persisted as an `item_completed` event instead of
//! `user_message`. Codex's own appender fails closed on an active, archived
//! or diverged thread; so does this one.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Confirm, Question, WriteError, io_error, new_uuid, now_iso_ms};
use crate::sessions::codex::{CodexSource, HistoryMode, READ_CAP, is_live};
use crate::sessions::model::{AgentKind, Role, validate_id};
use crate::sessions::{secrets, version};

/// How long the app-server gets to answer each request.
pub const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest line read from the app-server: a protocol message, not a
/// stream of the thread's history.
const MAX_MESSAGE_BYTES: u64 = 8 << 20;

/// How many messages may arrive while waiting for one response — the server's
/// notifications are skipped, and a server that only ever notifies must not
/// keep the writer waiting forever.
const MAX_MESSAGES_PER_REQUEST: usize = 10_000;

/// The items `thread/inject_items` carries: one user-role message, the same
/// voice as the Claude note (E3).
pub fn note_items(note: &str) -> Value {
    json!([{
        "type": "message",
        "role": "user",
        "content": [{ "type": "input_text", "text": note }],
    }])
}

/// A JSON-RPC client over a line-delimited stream: Codex's app-server protocol
/// (JSON-RPC 2.0 messages one per line, the `"jsonrpc"` member left out, as
/// the Codex documentation shows it).
pub struct AppServer<'a> {
    next_line: Box<dyn FnMut() -> Result<Option<String>, WriteError> + 'a>,
    out: Box<dyn Write + 'a>,
    next_id: u64,
}

impl<'a> AppServer<'a> {
    pub fn new(
        next_line: impl FnMut() -> Result<Option<String>, WriteError> + 'a,
        out: impl Write + 'a,
    ) -> Self {
        AppServer {
            next_line: Box::new(next_line),
            out: Box::new(out),
            next_id: 1,
        }
    }

    fn send(&mut self, message: &Value) -> Result<(), WriteError> {
        let mut line = message.to_string();
        line.push('\n');
        self.out
            .write_all(line.as_bytes())
            .and_then(|_| self.out.flush())
            .map_err(|e| WriteError::Protocol(format!("could not write to the server: {e}")))
    }

    pub fn notify(&mut self, method: &str, params: Value) -> Result<(), WriteError> {
        self.send(&json!({ "method": method, "params": params }))
    }

    /// Sends a request and returns its `result`. Notifications that arrive
    /// first are skipped; a request FROM the server (an approval, a login
    /// prompt) is answered with an error, because nothing here can answer it
    /// for the user.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, WriteError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "id": id, "method": method, "params": params }))?;
        for _ in 0..MAX_MESSAGES_PER_REQUEST {
            let Some(line) = (self.next_line)()? else {
                return Err(WriteError::Protocol(format!(
                    "the server closed its output before answering {method}"
                )));
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(msg) = serde_json::from_str::<Value>(line) else {
                return Err(WriteError::Protocol(format!(
                    "answered {method} with a line that is not JSON"
                )));
            };
            let has_method = msg.get("method").is_some();
            match msg.get("id") {
                Some(other) if has_method => {
                    self.send(&json!({
                        "id": other,
                        "error": { "code": -32601, "message": "Tacet does not answer server requests" },
                    }))?;
                }
                Some(answer) if answer.as_u64() == Some(id) => {
                    if let Some(err) = msg.get("error") {
                        let text = err
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("an error without a message");
                        return Err(WriteError::Protocol(format!("{method} failed: {text}")));
                    }
                    return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
                }
                // A notification, or an answer to something else.
                _ => {}
            }
        }
        Err(WriteError::Protocol(format!(
            "no answer to {method} in {MAX_MESSAGES_PER_REQUEST} messages"
        )))
    }
}

/// The three steps, on an open connection: `initialize` (and the
/// `initialized` notification the protocol expects after it), `thread/resume`,
/// `thread/inject_items`.
pub fn inject(server: &mut AppServer<'_>, thread_id: &str, note: &str) -> Result<(), WriteError> {
    server.request(
        "initialize",
        json!({
            "clientInfo": {
                "name": "tacet",
                "title": "Tacet",
                "version": env!("CARGO_PKG_VERSION"),
            }
        }),
    )?;
    server.notify("initialized", json!({}))?;
    server.request("thread/resume", json!({ "threadId": thread_id }))?;
    server.request(
        "thread/inject_items",
        json!({ "threadId": thread_id, "items": note_items(note) }),
    )?;
    Ok(())
}

/// Starts `program app-server`, injects `note` into thread `thread_id` — after
/// asking — and shuts the server down.
///
/// `root`: the Codex home whose writer locks are checked first. A thread
/// another Codex process owns is refused (RG-31): the app-server would have to
/// take the thread over from it. With no home, or a home that is not a
/// folder, a held lock cannot be ruled out, and that is refused too — every
/// other writer here fails closed when it cannot see the other writer.
pub fn inject_via_app_server(
    program: &Path,
    root: Option<&Path>,
    thread_id: &str,
    note: &str,
    confirm: &dyn Confirm,
    timeout: Duration,
) -> Result<(), WriteError> {
    validate_id(thread_id)?;
    if !note.starts_with(super::NOTE_PREFIX) {
        return Err(WriteError::Refused(
            "a note must open with Tacet's prefix; nothing else is sent".into(),
        ));
    }
    let program = resolve_program(program)?;
    let Some(root) = root else {
        return Err(WriteError::Live(
            "the Codex folder could not be resolved, so a held thread-writer lock cannot be \
             ruled out; pass --root"
                .into(),
        ));
    };
    if !root.is_dir() {
        return Err(WriteError::Live(format!(
            "{} is not a folder, so a held thread-writer lock cannot be ruled out; pass --root",
            root.display()
        )));
    }
    if is_live(root, thread_id) {
        return Err(WriteError::Live(format!(
            "a Codex process holds thread {thread_id} (thread-writer-locks/{thread_id}.lock)"
        )));
    }
    let mut lines = vec![
        format!("program: {} app-server", program.display()),
        "it runs with YOUR Codex login and can reach the network; Tacet starts no model turn \
         — `thread/inject_items` adds the note to the thread's history"
            .into(),
        "not measured on a real Codex install (none was available when this was written)".into(),
        "the note, as a user-role item:".into(),
    ];
    lines.extend(note.lines().map(|l| format!("  {l}")));
    let q = Question {
        heading: format!("Start the Codex app-server and add a note to thread {thread_id}?"),
        lines,
    };
    if !confirm.confirm(&q) {
        return Err(WriteError::Declined);
    }
    run_app_server(&program, thread_id, note, timeout)
}

/// The absolute file `program` names, or a refusal.
///
/// A name with no directory in it is refused rather than looked up: given
/// `codex`, `Command::new` searches PATH, so the binary that ran would be
/// whichever `codex` came first there, and the question would have shown only
/// the word. A path is resolved (symlinks too — an npm-installed `codex` is a
/// link to a script) so the question names the file that will run, and that
/// same resolved path is what is started.
fn resolve_program(program: &Path) -> Result<PathBuf, WriteError> {
    if program.components().count() < 2 && !program.is_absolute() {
        return Err(WriteError::Refused(format!(
            "--program {} is a bare name, which would be looked up on PATH; give the path to \
             the binary you mean (for example the one `command -v {}` prints)",
            program.display(),
            program.display()
        )));
    }
    let resolved = std::fs::canonicalize(program).map_err(io_error(program))?;
    if !resolved.is_file() {
        return Err(WriteError::Refused(format!(
            "{} is not a file",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn run_app_server(
    program: &Path,
    thread_id: &str,
    note: &str,
    timeout: Duration,
) -> Result<(), WriteError> {
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    // `program` is the resolved absolute path the question above showed
    // (`resolve_program`), so no PATH search happens here.
    let mut child = Command::new(program)
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(io_error(program))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| WriteError::Protocol("the server's output could not be opened".into()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| WriteError::Protocol("the server's input could not be opened".into()))?;
    let (tx, rx) = mpsc::channel::<Result<Option<String>, String>>();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        loop {
            let mut buf = Vec::new();
            match std::io::Read::take(&mut reader, MAX_MESSAGE_BYTES + 1)
                .read_until(b'\n', &mut buf)
            {
                Ok(0) => {
                    let _ = tx.send(Ok(None));
                    return;
                }
                Ok(_) if buf.len() as u64 > MAX_MESSAGE_BYTES => {
                    let _ = tx.send(Err("a message over the size cap".into()));
                    return;
                }
                Ok(_) => {
                    if tx
                        .send(Ok(Some(String::from_utf8_lossy(&buf).into_owned())))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            }
        }
    });
    let result = {
        let next = || -> Result<Option<String>, WriteError> {
            match rx.recv_timeout(timeout) {
                Ok(Ok(line)) => Ok(line),
                Ok(Err(e)) => Err(WriteError::Protocol(e)),
                Err(_) => Err(WriteError::Protocol(format!(
                    "no answer within {} s",
                    timeout.as_secs()
                ))),
            }
        };
        let mut server = AppServer::new(next, stdin);
        inject(&mut server, thread_id, note)
        // `server` drops here, closing the server's stdin: its signal to exit.
    };
    // A clean exit is waited for; a server that ignores a closed stdin is
    // stopped, because Tacet must not leave the vendor's process running.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    result
}

/// What a rollout append wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct RolloutAppended {
    pub path: PathBuf,
    pub mode: HistoryMode,
    /// The ordinals the two lines took, in paginated mode.
    pub ordinals: Option<(u64, u64)>,
}

/// Appends `note` to thread `id`'s rollout as a `response_item` (what the
/// model sees on resume) and an `event_msg` (what the user sees) — after
/// asking.
///
/// Refused when: a Codex process holds the thread's writer lock, the rollout
/// is archived or compressed, a prefix it was forked from is missing, a line
/// of it is not a rollout line, a paginated rollout's last line has no
/// ordinal, or — today, for every version — the version gate has no measured
/// Codex write range. Undone when the check after the write fails.
pub fn append_note(
    root: &Path,
    id: &str,
    note: &str,
    confirm: &dyn Confirm,
) -> Result<RolloutAppended, WriteError> {
    append_impl(root, id, note, confirm, true, None)
}

/// `measured_only`: false only in the tests below, which drive the mechanism
/// on fixtures while the version table still refuses every Codex version.
/// `forced_first`: the first ordinal to write, overriding the computed one —
/// how the tests show a reused ordinal is caught and undone. No production
/// path passes anything but `true, None`.
fn append_impl(
    root: &Path,
    id: &str,
    note: &str,
    confirm: &dyn Confirm,
    measured_only: bool,
    forced_first: Option<u64>,
) -> Result<RolloutAppended, WriteError> {
    validate_id(id)?;
    if !note.starts_with(super::NOTE_PREFIX) {
        return Err(WriteError::Refused(
            "a note must open with Tacet's prefix; nothing else is written".into(),
        ));
    }
    let live = || is_live(root, id);
    if live() {
        return Err(WriteError::Live(format!(
            "a Codex process holds thread {id} (thread-writer-locks/{id}.lock)"
        )));
    }
    // A `.jsonl.zst` comes back from the gate as `Refusal::Compressed`.
    let file = CodexSource.read_rollout(root, id)?;
    let r = &file.rollout;
    if let Some(meta) = r.meta_id.as_deref()
        && meta != id
        && validate_id(meta).is_ok()
        && is_live(root, meta)
    {
        return Err(WriteError::Live(format!(
            "a Codex process holds thread {meta}, which this rollout belongs to"
        )));
    }
    if file.archived {
        return Err(WriteError::Refused(
            "the thread is archived, which Codex does not resume, so a note there is never read"
                .into(),
        ));
    }
    if measured_only {
        version::may_write(
            AgentKind::Codex,
            None,
            r.cli_version.as_deref(),
            &file.path,
            false,
        )?;
    } else {
        version::check_write_target(AgentKind::Codex, &file.path, false)?;
    }
    if let Some(why) = &r.missing_prefix {
        return Err(WriteError::Refused(format!(
            "the history this thread was forked from is not readable ({why})"
        )));
    }
    let text = secrets::read_text(&file.path, READ_CAP)?;
    if !text.ends_with('\n') {
        return Err(WriteError::Refused(
            "the rollout's last line is unfinished (no newline at its end)".into(),
        ));
    }
    let mut last_has_ordinal = false;
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).unwrap_or(Value::Null);
        if v.get("type").and_then(Value::as_str).is_none() || v.get("payload").is_none() {
            return Err(WriteError::Refused(format!(
                "line {} is not a rollout line; an append after it is not one Codex was seen to \
                 read",
                n + 1
            )));
        }
        last_has_ordinal = v.get("ordinal").and_then(Value::as_u64).is_some();
    }
    let ordinals = match r.history_mode {
        HistoryMode::Legacy => None,
        HistoryMode::Paginated => {
            // Codex's own rule: no append after a last line without one.
            if !last_has_ordinal {
                return Err(WriteError::Refused(
                    "the paginated rollout's last line has no ordinal".into(),
                ));
            }
            let last = r.last_ordinal.ok_or_else(|| {
                WriteError::Refused("the paginated rollout has no ordinal to continue".into())
            })?;
            let a = forced_first.or(last.checked_add(1));
            let b = a.and_then(|a| a.checked_add(1));
            match (a, b) {
                (Some(a), Some(b)) => Some((a, b)),
                _ => {
                    return Err(WriteError::Refused(
                        "the rollout's ordinals are at their maximum".into(),
                    ));
                }
            }
        }
    };
    let thread = r.meta_id.clone().unwrap_or_else(|| id.to_string());
    let lines = rollout_lines(note, &thread, ordinals);
    let mut shown = vec![
        format!("file: {}", file.path.display()),
        format!(
            "history mode: {}",
            match r.history_mode {
                HistoryMode::Legacy => "legacy",
                HistoryMode::Paginated => "paginated",
            }
        ),
        "two lines: the note for the model (response_item) and for you (event_msg)".into(),
        "the note, in the user role:".into(),
    ];
    shown.extend(note.lines().map(|l| format!("  {l}")));
    let q = Question {
        heading: format!("Append a note to Codex thread {id}'s rollout?"),
        lines: shown,
    };
    if !confirm.confirm(&q) {
        return Err(WriteError::Declined);
    }
    if live() {
        return Err(WriteError::Live(format!(
            "a Codex process took thread {id} while the question was on screen"
        )));
    }
    let pre_len = text.len() as u64;
    let handle = secrets::open_append(&file.path)?;
    let now_len = handle.metadata().map_err(io_error(&file.path))?.len();
    if now_len != pre_len {
        return Err(WriteError::Refused(format!(
            "the rollout changed since it was read ({pre_len} → {now_len} bytes), or is not \
             valid UTF-8; nothing written"
        )));
    }
    let undo = |why: String| -> WriteError {
        match handle.set_len(pre_len) {
            Ok(()) => WriteError::RolledBack(why),
            Err(e) => WriteError::Refused(format!("{why} — AND THE ROLLBACK FAILED: {e}")),
        }
    };
    match (&handle).write(lines.as_bytes()) {
        Ok(n) if n == lines.len() => {}
        Ok(n) => return Err(undo(format!("a short write ({n} of {})", lines.len()))),
        Err(e) => return Err(undo(format!("the write failed: {e}"))),
    }
    let check = || -> Result<(), String> {
        let back = CodexSource
            .read_rollout(root, id)
            .map_err(|e| format!("the rollout could not be read back: {e}"))?;
        let b = &back.rollout;
        if back.path != file.path {
            return Err("the thread now reads from another file".into());
        }
        if b.valid_lines != r.valid_lines + 2 {
            return Err(format!(
                "{} rollout lines after the write, expected {}",
                b.valid_lines,
                r.valid_lines + 2
            ));
        }
        if let Some((_, hi)) = ordinals
            && b.last_ordinal != Some(hi)
        {
            return Err(format!(
                "the last ordinal is {:?}, not {hi}",
                b.last_ordinal
            ));
        }
        // One entry more, not one replaced: a line that reused an ordinal
        // would stand in for the line it repeats (the reader keeps the last
        // line per ordinal, as Codex's pagination would page it).
        if b.entries.len() != r.entries.len() + 1 {
            return Err(format!(
                "the model's history went from {} to {} items, not one more",
                r.entries.len(),
                b.entries.len()
            ));
        }
        match b.entries.last() {
            Some(e) if e.role == Role::User && e.text == note => Ok(()),
            _ => Err("the note is not the last thing the model would see".into()),
        }
    };
    if let Err(why) = check() {
        return Err(undo(why));
    }
    Ok(RolloutAppended {
        path: file.path.clone(),
        mode: r.history_mode,
        ordinals,
    })
}

/// The pair: `response_item` first (the model's copy, as Codex records a user
/// turn), then the event the user's transcript is rebuilt from.
fn rollout_lines(note: &str, thread: &str, ordinals: Option<(u64, u64)>) -> String {
    let ts = now_iso_ms();
    let item = json!({
        "type": "message",
        "role": "user",
        "content": [{ "type": "input_text", "text": note }],
    });
    let event = match ordinals {
        None => json!({ "type": "user_message", "message": note }),
        Some(_) => json!({
            "type": "item_completed",
            "thread_id": thread,
            "item": {
                "type": "UserMessage",
                "id": new_uuid(),
                "content": [{ "type": "text", "text": note }],
            },
        }),
    };
    let line = |ordinal: Option<u64>, kind: &str, payload: Value| {
        let mut v = json!({ "timestamp": ts, "type": kind, "payload": payload });
        if let Some(o) = ordinal {
            v["ordinal"] = json!(o);
        }
        v.to_string() + "\n"
    };
    line(ordinals.map(|o| o.0), "response_item", item)
        + &line(ordinals.map(|o| o.1), "event_msg", event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::writeback::NOTE_PREFIX;
    use std::cell::RefCell;

    struct Answer(bool, RefCell<usize>);

    impl Confirm for Answer {
        fn confirm(&self, _: &Question) -> bool {
            *self.1.borrow_mut() += 1;
            self.0
        }
    }

    fn yes() -> Answer {
        Answer(true, RefCell::new(0))
    }

    fn note() -> String {
        format!("{NOTE_PREFIX} The parser is split. Codeword: KESTREL-3.")
    }

    const LEGACY: &str = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a01";
    const PAGED: &str = "0199e000-0047-7000-8000-000000000001";

    fn scratch() -> PathBuf {
        let root = std::env::temp_dir().join(format!("tacet-wb-codex-{}", new_uuid()));
        std::fs::create_dir_all(root.join("sessions/2026/10/01")).unwrap();
        root
    }

    fn write_rollout(root: &Path, id: &str, lines: &[Value]) -> PathBuf {
        let path = root.join(format!(
            "sessions/2026/10/01/rollout-2026-10-01T09-15-00-{id}.jsonl"
        ));
        let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn legacy(root: &Path) -> PathBuf {
        write_rollout(
            root,
            LEGACY,
            &[
                json!({"timestamp":"2026-10-01T09:15:00.000Z","type":"session_meta","payload":{"id":LEGACY,"cwd":"/w","cli_version":"0.46.0","history_mode":"legacy"}}),
                json!({"timestamp":"2026-10-01T09:15:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"split the parser"}]}}),
                json!({"timestamp":"2026-10-01T09:15:01.001Z","type":"event_msg","payload":{"type":"user_message","message":"split the parser"}}),
            ],
        )
    }

    fn paged(root: &Path) -> PathBuf {
        write_rollout(
            root,
            PAGED,
            &[
                json!({"timestamp":"2026-10-06T11:00:00.000Z","ordinal":0,"type":"session_meta","payload":{"id":PAGED,"cwd":"/w","cli_version":"0.47.0","history_mode":"paginated"}}),
                json!({"timestamp":"2026-10-06T11:00:01.000Z","ordinal":1,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Split the parser module"}]}}),
                json!({"timestamp":"2026-10-06T11:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":null}}),
                json!({"timestamp":"2026-10-06T11:00:04.000Z","ordinal":4,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Split."}]}}),
            ],
        )
    }

    /// RG-24, legacy: the pair is appended in one piece, and the thread now
    /// ends in the note for the model and for the user.
    #[test]
    fn a_legacy_rollout_gets_the_event_and_response_item_pair() {
        let root = scratch();
        let path = legacy(&root);
        let before = std::fs::read_to_string(&path).unwrap();
        let done = append_impl(&root, LEGACY, &note(), &yes(), false, None).unwrap();
        assert_eq!(done.ordinals, None);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with(&before));
        let added: Vec<Value> = after[before.len()..]
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(added.len(), 2);
        assert_eq!(added[0]["type"], "response_item");
        assert_eq!(added[0]["payload"]["role"], "user");
        assert_eq!(added[0]["payload"]["content"][0]["text"], note().as_str());
        assert_eq!(added[1]["type"], "event_msg");
        assert_eq!(added[1]["payload"]["type"], "user_message");
        assert_eq!(added[1]["payload"]["message"], note().as_str());
        assert!(added.iter().all(|l| l.get("ordinal").is_none()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RG-24, paginated: the ordinals continue after the HIGHEST one, the
    /// visible half is an `item_completed`, and a last line without an
    /// ordinal is refused as Codex's own appender refuses it.
    #[test]
    fn a_paginated_rollout_continues_its_ordinals() {
        let root = scratch();
        let path = paged(&root);
        let done = append_impl(&root, PAGED, &note(), &yes(), false, None).unwrap();
        assert_eq!(done.ordinals, Some((5, 6)));
        let text = std::fs::read_to_string(&path).unwrap();
        let last: Vec<Value> = text
            .lines()
            .rev()
            .take(2)
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(last[0]["ordinal"], 6);
        assert_eq!(last[0]["payload"]["type"], "item_completed");
        assert_eq!(last[0]["payload"]["item"]["type"], "UserMessage");
        assert_eq!(last[1]["ordinal"], 5);
        // The token_count line between them has no ordinal; one LAST would be
        // refused.
        let mut text = text;
        text.push_str(
            &(json!({"timestamp":"2026-10-06T11:09:00.000Z","type":"event_msg","payload":{"type":"token_count"}}).to_string() + "\n"),
        );
        std::fs::write(&path, &text).unwrap();
        let r = append_impl(&root, PAGED, &note(), &yes(), false, None);
        assert!(
            matches!(r, Err(WriteError::Refused(ref w)) if w.contains("no ordinal")),
            "{r:?}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The Codex side of E4: a pair whose first ordinal repeats one already
    /// in the file would stand in for that line (the assistant's reply here)
    /// in the model's history. The check after the write sees one item
    /// replaced instead of one added, and the file is cut back to its bytes.
    #[test]
    fn a_reused_ordinal_is_caught_and_rolled_back() {
        let root = scratch();
        let path = paged(&root);
        let before = std::fs::read(&path).unwrap();
        let r = append_impl(&root, PAGED, &note(), &yes(), false, Some(4));
        match r {
            Err(WriteError::RolledBack(why)) => assert!(why.contains("not one more"), "{why}"),
            other => panic!("a reused ordinal was kept: {other:?}"),
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RG-31 and the fail-closed list: a held writer lock, an archived thread,
    /// a damaged line, a compressed rollout — and, through the public
    /// function, every Codex version, because none was measured. Nothing is
    /// asked and nothing is written in any of them.
    #[test]
    fn every_unsafe_rollout_is_refused_untouched() {
        let root = scratch();
        let path = legacy(&root);
        let before = std::fs::read(&path).unwrap();
        let ask = yes();
        let r = append_note(&root, LEGACY, &note(), &ask);
        assert!(matches!(r, Err(WriteError::Version(_))), "{r:?}");

        std::fs::create_dir_all(root.join("thread-writer-locks")).unwrap();
        let lock = root.join(format!("thread-writer-locks/{LEGACY}.lock"));
        std::fs::write(&lock, "").unwrap();
        let r = append_impl(&root, LEGACY, &note(), &ask, false, None);
        assert!(matches!(r, Err(WriteError::Live(_))), "{r:?}");
        std::fs::remove_file(&lock).unwrap();

        std::fs::write(&path, [before.as_slice(), b"not json\n"].concat()).unwrap();
        let r = append_impl(&root, LEGACY, &note(), &ask, false, None);
        assert!(matches!(r, Err(WriteError::Refused(_))), "{r:?}");
        std::fs::write(&path, &before).unwrap();

        let r = append_impl(&root, LEGACY, "no prefix", &ask, false, None);
        assert!(matches!(r, Err(WriteError::Refused(_))), "{r:?}");
        assert_eq!(*ask.1.borrow(), 0, "a refused write was asked about");
        assert_eq!(std::fs::read(&path).unwrap(), before);

        let archived = root.join("archived_sessions");
        std::fs::create_dir_all(&archived).unwrap();
        std::fs::rename(&path, archived.join(path.file_name().unwrap())).unwrap();
        let r = append_impl(&root, LEGACY, &note(), &ask, false, None);
        assert!(
            matches!(r, Err(WriteError::Refused(ref w)) if w.contains("archived")),
            "{r:?}"
        );

        let zst_id = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a09";
        let zst = root.join(format!(
            "sessions/2026/10/01/rollout-2026-10-01T09-15-00-{zst_id}.jsonl.zst"
        ));
        std::fs::write(&zst, b"\x28\xb5\x2f\xfd").unwrap();
        let r = append_impl(&root, zst_id, &note(), &ask, false, None);
        assert!(
            matches!(r, Err(WriteError::Session(_)) | Err(WriteError::Gate(_))),
            "{r:?}"
        );
        assert_eq!(std::fs::read(&zst).unwrap(), b"\x28\xb5\x2f\xfd");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The protocol client, in memory: notifications are skipped, a request
    /// from the server is answered with an error rather than left hanging, and
    /// an error answer stops the sequence.
    #[test]
    fn the_client_skips_notifications_and_refuses_server_requests() {
        let replies = [
            r#"{"method":"thread/started","params":{}}"#,
            r#"{"id":"srv-1","method":"account/login","params":{}}"#,
            r#"{"id":1,"result":{"userAgent":"x"}}"#,
            r#"{"id":2,"result":{}}"#,
            r#"{"id":3,"error":{"code":-32600,"message":"no such thread"}}"#,
        ];
        let mut it = replies.iter().map(|s| s.to_string());
        let mut sent: Vec<u8> = Vec::new();
        let result = {
            let mut server = AppServer::new(|| Ok(it.next()), &mut sent);
            inject(&mut server, LEGACY, &note())
        };
        match result {
            Err(WriteError::Protocol(why)) => assert!(why.contains("no such thread"), "{why}"),
            other => panic!("{other:?}"),
        }
        let sent: Vec<Value> = String::from_utf8(sent)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let methods: Vec<&str> = sent.iter().filter_map(|m| m["method"].as_str()).collect();
        assert_eq!(
            methods,
            [
                "initialize",
                "initialized",
                "thread/resume",
                "thread/inject_items"
            ]
        );
        assert!(
            sent.iter()
                .any(|m| m["id"] == "srv-1" && m.get("error").is_some())
        );
        let inject = sent
            .iter()
            .find(|m| m["method"] == "thread/inject_items")
            .unwrap();
        assert_eq!(inject["params"]["threadId"], LEGACY);
        assert_eq!(inject["params"]["items"], note_items(&note()));
        // A server that closes without answering is an error, not a hang.
        let mut none = std::iter::empty::<String>();
        let mut sink = Vec::new();
        let mut server = AppServer::new(|| Ok(none.next()), &mut sink);
        assert!(matches!(
            server.request("initialize", json!({})),
            Err(WriteError::Protocol(_))
        ));
    }

    /// RG-23 end to end, against a stand-in app-server: a shell script that
    /// logs what it receives and answers each request. The real `codex` is
    /// not run — it is not installed here, and it would be the network.
    #[cfg(unix)]
    #[test]
    fn the_note_reaches_a_stand_in_app_server_only_after_a_yes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let log = dir.join("received.log");
        let script = dir.join("fake-app-server");
        let body = r#"#!/bin/sh
[ "$1" = "app-server" ] || exit 64
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$LOG"
  id=$(printf '%s' "$line" | sed -n 's/^{"id":\([0-9][0-9]*\),.*/\1/p')
  [ -n "$id" ] || continue
  printf '{"method":"thread/status","params":{}}\n'
  printf '{"id":%s,"result":{}}\n' "$id"
done
"#
        .replace("\"$LOG\"", &format!("'{}'", log.display()));
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let declined = inject_via_app_server(
            &script,
            Some(&dir),
            LEGACY,
            &note(),
            &Answer(false, RefCell::new(0)),
            Duration::from_secs(10),
        );
        assert!(matches!(declined, Err(WriteError::Declined)));
        assert!(!log.exists(), "the server was started before the yes");

        inject_via_app_server(
            &script,
            Some(&dir),
            LEGACY,
            &note(),
            &yes(),
            Duration::from_secs(10),
        )
        .unwrap();
        let got: Vec<Value> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let methods: Vec<&str> = got.iter().filter_map(|m| m["method"].as_str()).collect();
        assert_eq!(
            methods,
            [
                "initialize",
                "initialized",
                "thread/resume",
                "thread/inject_items"
            ]
        );
        assert_eq!(
            got[3]["params"]["items"][0]["content"][0]["text"],
            note().as_str()
        );

        // A held writer lock stops it before anything starts.
        std::fs::create_dir_all(dir.join("thread-writer-locks")).unwrap();
        std::fs::write(dir.join(format!("thread-writer-locks/{LEGACY}.lock")), "").unwrap();
        let r = inject_via_app_server(
            &script,
            Some(&dir),
            LEGACY,
            &note(),
            &yes(),
            Duration::from_secs(10),
        );
        assert!(matches!(r, Err(WriteError::Live(_))), "{r:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Records each question and answers no.
    struct Shown(RefCell<Vec<Question>>);

    impl Confirm for Shown {
        fn confirm(&self, q: &Question) -> bool {
            self.0.borrow_mut().push(q.clone());
            false
        }
    }

    /// RG-23: `--program codex` would make `Command::new` search PATH, and the
    /// question would name a word, not a binary. A bare name is refused before
    /// anything is asked. The refusal looks at the shape, not the word, so the
    /// test uses `sh` — on every PATH, so a lookup WOULD find something — and
    /// a name on none. It does not write the literal `codex`: network_monopoly's
    /// name scan would then need a NAMED_NOT_RUN row for that word in the one
    /// file that does start the app-server, and such a row would also let a
    /// real `Command::new("codex")` here go unseen.
    #[test]
    fn a_bare_program_name_is_refused_before_anything_is_asked() {
        let dir = scratch();
        for bare in ["sh", "tacet-no-such-program"] {
            let ask = Answer(false, RefCell::new(0));
            let r = inject_via_app_server(
                Path::new(bare),
                Some(&dir),
                LEGACY,
                &note(),
                &ask,
                Duration::from_secs(10),
            );
            assert!(
                matches!(r, Err(WriteError::Refused(ref w)) if w.contains("bare name")),
                "{bare}: {r:?}"
            );
            assert_eq!(*ask.1.borrow(), 0, "{bare}: asked about a PATH lookup");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RG-23: the question names the file that will run — the resolved
    /// absolute path, through a symlink — not the spelling that was typed.
    #[cfg(unix)]
    #[test]
    fn the_question_names_the_resolved_binary() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let real = dir.join("real-codex");
        std::fs::write(&real, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = dir.join("linked-app-server");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let ask = Shown(RefCell::new(Vec::new()));
        let r = inject_via_app_server(
            &link,
            Some(&dir),
            LEGACY,
            &note(),
            &ask,
            Duration::from_secs(10),
        );
        assert!(matches!(r, Err(WriteError::Declined)), "{r:?}");
        let shown = ask.0.borrow();
        assert_eq!(
            shown[0].lines[0],
            format!(
                "program: {} app-server",
                std::fs::canonicalize(&real).unwrap().display()
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RG-31: with no Codex folder (or one that is not a folder) the writer
    /// lock cannot be checked, so the write is refused as Live — nothing is
    /// asked and the server is not started.
    #[cfg(unix)]
    #[test]
    fn without_a_codex_folder_nothing_is_asked_or_started() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let log = dir.join("started.log");
        let script = dir.join("fake-app-server");
        std::fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\nexit 0\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let missing = dir.join("no-such-codex-home");
        for root in [None, Some(missing.as_path()), Some(script.as_path())] {
            let ask = yes();
            let r = inject_via_app_server(
                &script,
                root,
                LEGACY,
                &note(),
                &ask,
                Duration::from_secs(10),
            );
            assert!(
                matches!(r, Err(WriteError::Live(ref w)) if w.contains("cannot be ruled out")),
                "{root:?}: {r:?}"
            );
            assert_eq!(*ask.1.borrow(), 0, "{root:?}: asked without a lock check");
        }
        assert!(!log.exists(), "the server was started without a lock check");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
