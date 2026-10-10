//! Reporting back: how Tacet's progress reaches the agent a session came from.
//!
//! THE ORDER OF PREFERENCE IS THE ADR'S (docs/adr/0001-agent-sessions.md,
//! "Writing back"), and this module keeps it:
//!
//! 1. The handoff record (`handoff.rs`) — Tacet's own file, always written.
//! 2. The agent's official hook, which READS that record and hands it to the
//!    agent's model when the user goes back: Claude Code and Codex
//!    `SessionStart` (`additionalContext`), Antigravity `PreInvocation`
//!    (`injectSteps`). `hook_response` below is what `tacet handoff hook`
//!    prints; nothing of the agent's is written. RG-21, RG-23, RG-25.
//! 3. A write into the agent's own storage, each fail-closed and each asked
//!    about every time (`Confirm`): a note appended to a Claude Code
//!    transcript or a fork of it (`claude.rs`, RG-22), a Codex app-server
//!    `thread/inject_items` or a rollout append (`codex.rs`, RG-23/24), and
//!    Antigravity's `task.md` checkboxes behind an experimental flag
//!    (`antigravity.rs`, RG-26).
//!
//! WHY EVERY FILE WRITE IS GUARDED THREE WAYS (RG-31). MEASURED on Claude Code
//! 2.1.285, 9 Oct 2026 (E4): one entry with the wrong `parentUuid` and
//! `--resume` silently loaded nothing before it — the user's whole history,
//! still in the file, gone from the agent's view. So a writer refuses a session
//! an agent process has open (it may be appending at the same moment), refuses
//! a version nobody measured a write on (`version::may_write`), and checks its
//! own work after writing, undoing it when the check fails.
//!
//! THE ONLY VOICE IS A NOTE. MEASURED (E2): an assistant-role entry made the
//! agent's model believe it had written the text itself. Every write here
//! carries `NOTE_PREFIX` in the user role, the shape E3 measured to be loaded
//! AND attributed to Tacet.

pub mod antigravity;
pub mod claude;
pub mod codex;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value, json};

use super::handoff::{self, HandoffError, HandoffRecord};
use super::model::{AgentKind, SessionError, iso_utc, validate_id};
use super::secrets::Refusal;
use super::version::WriteRefused;

/// How every note Tacet writes into another agent's history begins. MEASURED
/// (E3, Claude Code 2.1.285): with exactly this opening, the resumed model
/// reported "Tacet, a local assistant, reported that …" — credited to Tacet,
/// not to the user and not to itself.
pub const NOTE_PREFIX: &str = "[Note from Tacet, a local assistant, not from the user]";

/// The largest note written into a transcript. A handoff is a page; the full
/// record stays in Tacet's file and the note says where.
pub const MAX_NOTE_BYTES: usize = 16 * 1024;

/// The largest hook context handed to an agent. Claude Code documents a cap
/// on what a hook may add to the context (10,000 characters at the time of
/// writing, NOT MEASURED here); staying under it keeps the record from being
/// cut by the agent at a point Tacet did not choose.
pub const MAX_CONTEXT_CHARS: usize = 9_000;

/// The largest hook input read from stdin. The agents send a few hundred
/// bytes of JSON; anything far larger is not a hook event.
pub const MAX_HOOK_INPUT: u64 = 1 << 20;

/// What a writer asks before it touches another agent's storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub heading: String,
    /// Shown one per line. The text comes partly from files another program
    /// wrote, so the asker sanitises each line where it meets the terminal.
    pub lines: Vec<String>,
}

/// The approval gate for a write-back. NOTHING ANSWERS YES BY DEFAULT: the
/// terminal implementation answers no when nobody can be asked (a pipe), the
/// way `TerminalApproval` does for outbound tools.
pub trait Confirm {
    fn confirm(&self, question: &Question) -> bool;
}

/// Why a write-back did not happen, or was undone.
#[derive(Debug)]
pub enum WriteError {
    /// A fail-closed guard said no; the text says which and why.
    Refused(String),
    /// The version gate: no write was ever measured on this version.
    Version(WriteRefused),
    /// An agent process has the session open, or might have.
    Live(String),
    /// The person said no.
    Declined,
    Session(SessionError),
    Gate(Refusal),
    Handoff(HandoffError),
    /// The write happened, its check failed, and it was undone; the file is
    /// as it was before.
    RolledBack(String),
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    /// The Codex app-server did not answer as the protocol says.
    Protocol(String),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::Refused(why) => write!(f, "not written: {why}"),
            WriteError::Version(w) => write!(f, "{w}"),
            WriteError::Live(why) => write!(f, "not written: {why}"),
            WriteError::Declined => write!(f, "not written: declined"),
            WriteError::Session(e) => write!(f, "not written: {e}"),
            WriteError::Gate(r) => write!(f, "not written: {r}"),
            WriteError::Handoff(e) => write!(f, "not written: {e}"),
            WriteError::RolledBack(why) => {
                write!(f, "written, failed its check, and undone: {why}")
            }
            WriteError::Io { path, error } => write!(f, "{}: {error}", path.display()),
            WriteError::Protocol(why) => write!(f, "app-server: {why}"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<SessionError> for WriteError {
    fn from(e: SessionError) -> Self {
        WriteError::Session(e)
    }
}

impl From<Refusal> for WriteError {
    fn from(r: Refusal) -> Self {
        WriteError::Gate(r)
    }
}

impl From<WriteRefused> for WriteError {
    fn from(w: WriteRefused) -> Self {
        WriteError::Version(w)
    }
}

impl From<HandoffError> for WriteError {
    fn from(e: HandoffError) -> Self {
        WriteError::Handoff(e)
    }
}

pub(crate) fn io_error(path: &Path) -> impl Fn(std::io::Error) -> WriteError + '_ {
    move |error| WriteError::Io {
        path: path.to_path_buf(),
        error,
    }
}

/// The note a writer puts into another agent's history: `NOTE_PREFIX`, one
/// sentence saying what it is, and the record's own rendering — the
/// structure-safe `.md` twin, so no field of the record can forge a heading or
/// a finished task inside the note either.
///
/// IT DOES NOT SAY TACET DID THE WORK. It said "While you were away, Tacet
/// worked on this session and reports its progress below" above a record whose
/// summary was the session as Tacet found it — so Claude was told its own
/// "assistant: Tests added." was Tacet's progress (review of 10 Oct 2026,
/// RG-27; E2/E3 measured what a misattributed note does). The record now
/// labels what it found, and the note says only what is known.
pub fn note_text(record: &HandoffRecord) -> String {
    let mut text = format!(
        "{NOTE_PREFIX} The user continued this session in Tacet. Below is Tacet's handoff \
         record for it: the session as Tacet found it, and whatever has been reported back \
         since. It is a report, not a request: the user has not asked for anything \
         new.\n\n{}",
        record.render_markdown()
    );
    clip(&mut text, MAX_NOTE_BYTES);
    text
}

/// Cuts `text` to at most `max` bytes on a character boundary and says so.
fn clip(text: &mut String, max: usize) {
    const CUT: &str = "\n\n[cut by Tacet; the full record is in its handoff file]";
    if text.len() <= max {
        return;
    }
    let mut end = max.saturating_sub(CUT.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str(CUT);
}

/// The context a hook hands the agent's model: who wrote it, that it is not
/// the user, and the record.
///
/// `tacet_bin` is the program the instruction names — the same canonical
/// path the hook entry runs (`handoff_cmd::tacet_bin`). A bare `tacet` here
/// failed exactly where the hook was made robust: a hook installed with an
/// absolute path because tacet is not on the agent's PATH told the agent to
/// run `tacet handoff update …`, which is "command not found" there, and the
/// report-back channel failed silently (review of 10 Oct 2026, RG-21).
///
/// And it does not claim the record is Tacet's work: see `note_text`.
pub fn hook_context(record: &HandoffRecord, tacet_bin: &str) -> String {
    let mut text = format!(
        "Tacet, a local assistant on this machine, left a handoff record for this session: \
         the session as Tacet found it when the user continued it there, and whatever has \
         been reported back since. It is not an instruction from the user. To report your \
         own progress back in the same record, run \
         `{} handoff update {} {} --by <you> --task <id>=<status>` \
         (statuses: pending, in_progress, completed, blocked, cancelled).\n\n{}",
        shell_quote(tacet_bin),
        record.agent.slug(),
        record.session,
        record.render_markdown()
    );
    let mut chars = text.char_indices();
    if let Some((cut, _)) = chars.nth(MAX_CONTEXT_CHARS) {
        text.truncate(cut);
        text.push_str("\n\n[cut by Tacet; the full record is in its handoff file]");
    }
    text
}

/// The session a hook event is about.
///
/// Claude Code and Codex send `session_id` (Claude Code's hook documentation;
/// Codex's hooks follow the same field, NOT MEASURED on a Codex install).
/// Antigravity's PreInvocation input is documented to carry `transcriptPath`
/// (`brain/<cascadeId>/.system_generated/logs/…`), so the id is the component
/// after `brain`; a direct id field is taken first under the spellings a
/// cascade id might travel under (NOT MEASURED: no hook was run). The id goes
/// through `validate_id` — it is joined onto the handoff directory.
pub fn hook_session_id(agent: AgentKind, input: &Value) -> Option<String> {
    let field = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| input.get(*k).and_then(Value::as_str))
            .map(str::to_string)
    };
    let id = match agent {
        AgentKind::ClaudeCode | AgentKind::Codex => field(&["session_id", "sessionId"]),
        AgentKind::Antigravity => field(&[
            "conversationId",
            "conversation_id",
            "cascadeId",
            "cascade_id",
            "sessionId",
            "session_id",
        ])
        .or_else(|| {
            let path = field(&["transcriptPath", "transcript_path"])?;
            let parts: Vec<&str> = path.split(['/', '\\']).collect();
            let at = parts.iter().rposition(|p| *p == "brain")?;
            parts.get(at + 1).map(|s| s.to_string())
        }),
    }?;
    validate_id(&id).ok().map(|_| id)
}

/// What `tacet handoff hook <agent>` prints for one hook event, or `None` to
/// print nothing.
///
/// SILENCE IS THE COMMON ANSWER. The hook runs at the start of EVERY session
/// of that agent, and almost none of them have a Tacet record; printing
/// anything there would put Tacet into sessions it never touched. An event it
/// cannot parse is silence too: a broken hook must never stop the user's agent
/// from starting. A record that exists and cannot be read is the one case the
/// user is told about, where the agent has a user-facing field for it.
///
/// `tacet_bin`: the program the agent is told to run to report back — the
/// one the hook entry itself runs.
pub fn hook_response(agent: AgentKind, dir: &Path, input: &str, tacet_bin: &str) -> Option<Value> {
    let event: Value = serde_json::from_str(input).ok()?;
    let id = hook_session_id(agent, &event)?;
    let record = match handoff::read(dir, agent, &id) {
        Ok(record) => record,
        Err(HandoffError::Refused(Refusal::Io { error, .. }))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            return None;
        }
        Err(e) => {
            return match agent {
                AgentKind::Antigravity => None,
                _ => Some(json!({
                    "systemMessage": format!("Tacet's handoff record for this session was not used: {e}")
                })),
            };
        }
    };
    let context = hook_context(&record, tacet_bin);
    Some(match agent {
        AgentKind::ClaudeCode | AgentKind::Codex => json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": context,
            },
            "systemMessage": format!(
                "Tacet added its handoff for this session to the context (updated {}, {} task(s)).",
                record.updated_at,
                record.tasks.len()
            ),
        }),
        AgentKind::Antigravity => json!({
            "injectSteps": [{ "ephemeralMessage": context }],
        }),
    })
}

/// The hook event each agent runs `tacet handoff hook` on.
pub fn hook_event(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::ClaudeCode | AgentKind::Codex => "SessionStart",
        AgentKind::Antigravity => "PreInvocation",
    }
}

/// Where each agent reads its hooks from, for the install screen. The user
/// names the file; these are what the agents' documentation says.
pub fn hook_files(agent: AgentKind) -> &'static [&'static str] {
    match agent {
        AgentKind::ClaudeCode => &["~/.claude/settings.json", ".claude/settings.json"],
        AgentKind::Codex => &["~/.codex/hooks.json"],
        AgentKind::Antigravity => &[".agents/hooks.json", "~/.gemini/config/hooks.json"],
    }
}

/// `text`, quoted for a POSIX shell when it has to be. The hook command is
/// run by the agent through a shell, and the path to the binary may hold a
/// space (`/Users/a b/.cargo/bin/tacet`).
pub fn shell_quote(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | ':'));
    if plain {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// The command a hook entry runs.
pub fn hook_command(tacet_bin: &str, agent: AgentKind) -> String {
    format!("{} handoff hook {}", shell_quote(tacet_bin), agent.slug())
}

/// The hook entry Tacet adds, in the agents' shared shape: a group with an
/// optional matcher and a list of command hooks. Claude Code's shape is
/// documented and was read from its docs; Codex's `hooks.json` and
/// Antigravity's `hooks.json` are given the same shape, NOT MEASURED on either
/// (neither ran a Tacet hook on this machine) — docs/agent-handoff.md says so
/// next to the snippet.
pub fn hook_group(agent: AgentKind, command: &str) -> Value {
    let hooks = json!([{ "type": "command", "command": command }]);
    match agent {
        // `startup|resume`: a fresh session and a resumed one are where the
        // user comes back to work Tacet touched; `clear` and `compact` would
        // re-add the record into a context the user just emptied.
        AgentKind::ClaudeCode | AgentKind::Codex => {
            json!({ "matcher": "startup|resume", "hooks": hooks })
        }
        AgentKind::Antigravity => json!({ "hooks": hooks }),
    }
}

/// The whole snippet, as a settings file holding only Tacet's hook would read.
pub fn install_snippet(agent: AgentKind, command: &str) -> Value {
    json!({ "hooks": { hook_event(agent): [hook_group(agent, command)] } })
}

/// Does any command hook under `groups` run `command`?
fn already_installed(groups: &[Value], command: &str) -> bool {
    groups.iter().any(|g| {
        g.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
            hs.iter()
                .any(|h| h.get("command").and_then(Value::as_str) == Some(command))
        }) || g.get("command").and_then(Value::as_str) == Some(command)
    })
}

/// Merges Tacet's hook into an existing settings file's text.
///
/// Returns the new text and whether anything changed: an entry that already
/// runs the same command is not added twice (an install run again must not
/// fire the hook twice per session). Everything else in the file is kept.
/// KEY ORDER IS NOT: serde_json here has no `preserve_order` feature (and a
/// feature flag is a dependency decision), so the keys come back sorted — the
/// install screen says so before the user answers.
///
/// A file whose shape is not what the agent documents (`hooks` not an object,
/// the event not a list) is REFUSED, not repaired: Tacet does not know what
/// the user meant by it.
pub fn merge_hook(
    existing: Option<&str>,
    agent: AgentKind,
    command: &str,
) -> Result<(String, bool), String> {
    let mut root: Map<String, Value> = match existing.map(str::trim) {
        None | Some("") => Map::new(),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => map,
            Ok(_) => return Err("the file is JSON but not an object".into()),
            Err(e) => return Err(format!("the file is not JSON: {e}")),
        },
    };
    let event = hook_event(agent);
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(hooks) = hooks else {
        return Err("`hooks` is not an object".into());
    };
    let groups = hooks
        .entry(event)
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(groups) = groups else {
        return Err(format!("`hooks.{event}` is not a list"));
    };
    if already_installed(groups, command) {
        let text = serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())?;
        return Ok((text + "\n", false));
    }
    groups.push(hook_group(agent, command));
    let text = serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())?;
    Ok((text + "\n", true))
}

/// A fresh UUID v4 for an entry or a forked session.
///
/// NO RANDOM CRATE (CONTRIBUTING §1). The bytes are SHA-256 of a
/// `RandomState` hash — std seeds it from the operating system's randomness
/// once per thread — the clock in nanoseconds, the process id and a counter.
/// Uniqueness is what an entry id needs; nothing here is a secret.
pub fn new_uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(seq);
    hasher.write_u128(nanos);
    let mixed = hasher.finish();
    let seed = format!("{mixed}:{nanos}:{}:{seq}", std::process::id());
    let mut b = tacet_kernel::sha256(seed.as_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b[..16].iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Now, as `YYYY-MM-DDTHH:MM:SS.mmmZ` — the shape Claude Code and Codex
/// write their own timestamps in.
pub fn now_iso_ms() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = iso_utc(now.as_secs() as i64);
    format!("{}.{:03}Z", secs.trim_end_matches('Z'), now.subsec_millis())
}

/// Writes a file next to `target` and renames it over `target`, so a reader
/// sees the old file or the new one, never half of one.
///
/// `fresh`: the target must not exist, and an existing one is never replaced
/// — the rename is a `hard_link` (which fails on an existing name) followed
/// by removing the temp name. `mode_from`: copy that file's permission bits,
/// so a rewritten `task.md` keeps the mode its owner gave it.
pub(crate) fn write_beside(
    target: &Path,
    bytes: &[u8],
    fresh: bool,
    mode_from: Option<&Path>,
) -> Result<(), WriteError> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let temp = target.with_file_name(format!(
        ".{name}.tacet.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let io = io_error(target);
    let cleanup = |e: WriteError| {
        let _ = std::fs::remove_file(&temp);
        e
    };
    tacet_kernel::fs::write_private(&temp, bytes)
        .map_err(&io)
        .map_err(cleanup)?;
    if let Some(from) = mode_from
        && let Ok(meta) = std::fs::metadata(from)
    {
        std::fs::set_permissions(&temp, meta.permissions())
            .map_err(&io)
            .map_err(cleanup)?;
    }
    if fresh {
        std::fs::hard_link(&temp, target)
            .map_err(&io)
            .map_err(cleanup)?;
        let _ = std::fs::remove_file(&temp);
        Ok(())
    } else {
        std::fs::rename(&temp, target).map_err(&io).map_err(cleanup)
    }
}

/// Is any running process one of `names`' programs? From `/bin/ps -A -o
/// comm=` on Unix: the executable path of every process. `None` when the list
/// could not be taken — the caller treats that as "yes", because the only job
/// of the answer is to stop a write (RG-31).
pub fn process_names() -> Option<Vec<String>> {
    #[cfg(unix)]
    {
        use std::process::{Command, Stdio};
        // An absolute path, as `claude::pid_alive` uses: not whatever `ps`
        // comes first on a PATH a shell profile built.
        let out = Command::new("/bin/ps")
            .args(["-A", "-o", "comm="])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
        )
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::handoff::{HandoffTask, Status};

    /// The program a hook entry runs, with a space so it must be quoted.
    const BIN: &str = "/Users/a b/.cargo/bin/tacet";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tacet-writeback-{name}-{}-{}",
            std::process::id(),
            new_uuid()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(agent: AgentKind, id: &str) -> HandoffRecord {
        let mut r = HandoffRecord::new(agent, id);
        r.summary = "Fixed the month rollover.".into();
        r.tasks.push(HandoffTask {
            id: "1".into(),
            title: "set day to 1 before adding a month".into(),
            status: Status::Completed,
            note: String::new(),
            updated_at: None,
        });
        r.next = "Run the gates.".into();
        r
    }

    #[test]
    fn a_uuid_is_version_4_and_never_repeats() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..2000 {
            let u = new_uuid();
            assert_eq!(u.len(), 36, "{u}");
            assert_eq!(&u[14..15], "4", "{u}");
            assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"), "{u}");
            assert!(seen.insert(u));
        }
    }

    #[test]
    fn the_note_opens_with_the_measured_prefix_and_carries_the_record() {
        let text = note_text(&record(AgentKind::ClaudeCode, "s-1"));
        assert!(text.starts_with(NOTE_PREFIX), "{text}");
        // RG-27 review: the note does not tell the agent Tacet did the work.
        assert!(!text.contains("Tacet worked on this session"), "{text}");
        assert!(text.contains("the session as Tacet found it"), "{text}");
        assert!(text.contains("- [x] #1 set day to 1 before adding a month"));
        assert!(text.contains("> Run the gates."));
        let mut long = record(AgentKind::ClaudeCode, "s-1");
        long.summary = "é".repeat(8_000);
        let text = note_text(&long);
        assert!(text.len() <= MAX_NOTE_BYTES, "{}", text.len());
        assert!(text.ends_with("handoff file]"));
    }

    /// RG-21: the exact JSON Claude Code's SessionStart hook reads, built from
    /// the record of the session named on stdin — and silence for a session
    /// Tacet never touched.
    #[test]
    fn the_claude_hook_prints_the_session_start_json_for_the_session_on_stdin() {
        let dir = scratch("hook");
        let id = "a035ad6d-1c2b-4f6e-9d1a-0b8c7e6f5a43";
        handoff::write(&dir, &record(AgentKind::ClaudeCode, id)).unwrap();
        let stdin = format!(
            r#"{{"session_id":"{id}","transcript_path":"/x/{id}.jsonl","cwd":"/w","hook_event_name":"SessionStart","source":"resume"}}"#
        );
        let out = hook_response(AgentKind::ClaudeCode, &dir, &stdin, BIN).expect("a record exists");
        assert_eq!(
            out["hookSpecificOutput"]["hookEventName"], "SessionStart",
            "{out}"
        );
        let ctx = out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(ctx.contains("not an instruction from the user"), "{ctx}");
        // RG-21 review: the program the agent is told to run is the one the
        // hook entry runs, quoted the same way — never a bare `tacet`.
        assert!(
            ctx.contains(&format!("`'{BIN}' handoff update claude {id} --by <you>")),
            "{ctx}"
        );
        assert!(!ctx.contains("`tacet handoff update"), "{ctx}");
        // RG-27 review: the record is not presented as Tacet's own work.
        assert!(!ctx.contains("reports what Tacet did"), "{ctx}");
        assert!(ctx.contains("the session as Tacet found it"), "{ctx}");
        assert!(ctx.contains("- [x] #1 set day to 1 before adding a month"));
        assert!(out["systemMessage"].as_str().unwrap().contains("1 task"));
        // Another session: nothing at all.
        let other = r#"{"session_id":"b035ad6d-1c2b-4f6e-9d1a-0b8c7e6f5a43"}"#;
        assert_eq!(hook_response(AgentKind::ClaudeCode, &dir, other, BIN), None);
        // Not JSON, no id, a traversal id: nothing, and no panic.
        for bad in ["", "not json", "{}", r#"{"session_id":"../../.ssh/x"}"#] {
            assert_eq!(
                hook_response(AgentKind::ClaudeCode, &dir, bad, BIN),
                None,
                "{bad}"
            );
        }
        // A record that is there and broken is said, not hidden.
        std::fs::write(
            dir.join(format!("{}-{id}.json", AgentKind::ClaudeCode.slug())),
            "{ broken",
        )
        .unwrap();
        let out = hook_response(AgentKind::ClaudeCode, &dir, &stdin, BIN).unwrap();
        assert!(out.get("hookSpecificOutput").is_none());
        assert!(out["systemMessage"].as_str().unwrap().contains("not used"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RG-23 (hook half) and RG-25: Codex gets the SessionStart shape,
    /// Antigravity `injectSteps` with one ephemeral message, its id taken from
    /// the documented `transcriptPath`.
    #[test]
    fn codex_and_antigravity_hooks_print_their_own_shapes() {
        let dir = scratch("hook2");
        let cid = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a01";
        handoff::write(&dir, &record(AgentKind::Codex, cid)).unwrap();
        let out = hook_response(
            AgentKind::Codex,
            &dir,
            &format!(r#"{{"session_id":"{cid}","hook_event_name":"SessionStart"}}"#),
            BIN,
        )
        .unwrap();
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "SessionStart");
        let aid = "7f3c2a10-5b6e-4c1d-9a8f-0e1d2c3b4a01";
        handoff::write(&dir, &record(AgentKind::Antigravity, aid)).unwrap();
        let input = format!(
            r#"{{"transcriptPath":"/h/.gemini/antigravity-ide/brain/{aid}/.system_generated/logs/transcript_full.jsonl"}}"#
        );
        let out = hook_response(AgentKind::Antigravity, &dir, &input, BIN).unwrap();
        let steps = out["injectSteps"].as_array().unwrap();
        assert_eq!(steps.len(), 1, "{out}");
        assert!(
            steps[0]["ephemeralMessage"]
                .as_str()
                .unwrap()
                .contains("Fixed the month rollover.")
        );
        assert!(out.get("hookSpecificOutput").is_none());
        // A direct id field wins over the path.
        let direct = format!(r#"{{"conversationId":"{aid}","transcriptPath":"/x/brain/zz/y"}}"#);
        assert_eq!(
            hook_session_id(
                AgentKind::Antigravity,
                &serde_json::from_str(&direct).unwrap()
            )
            .as_deref(),
            Some(aid)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hook_context_is_capped_on_a_character_boundary() {
        let mut r = record(AgentKind::ClaudeCode, "s-2");
        r.summary = "ğ".repeat(7_999);
        r.next = "ş".repeat(1_999);
        let ctx = hook_context(&r, BIN);
        assert!(
            ctx.chars().count() <= MAX_CONTEXT_CHARS + 80,
            "{}",
            ctx.len()
        );
        assert!(ctx.ends_with("handoff file]"));
    }

    /// RG-21 install: merged into an existing file, everything else kept, and
    /// a second install adds nothing.
    #[test]
    fn the_hook_merges_into_settings_once_and_keeps_the_rest() {
        let command = hook_command("/Users/a b/.cargo/bin/tacet", AgentKind::ClaudeCode);
        assert_eq!(
            command,
            format!(
                "'/Users/a b/.cargo/bin/tacet' handoff hook {}",
                AgentKind::ClaudeCode.slug()
            )
        );
        let existing = r#"{"model":"x","hooks":{"SessionStart":[{"matcher":"clear","hooks":[{"type":"command","command":"echo hi"}]}],"Stop":[]}}"#;
        let (text, changed) = merge_hook(Some(existing), AgentKind::ClaudeCode, &command).unwrap();
        assert!(changed);
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["model"], "x");
        assert_eq!(v["hooks"]["Stop"], json!([]));
        let groups = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "{text}");
        assert_eq!(groups[0]["hooks"][0]["command"], "echo hi");
        assert_eq!(groups[1]["matcher"], "startup|resume");
        assert_eq!(groups[1]["hooks"][0]["command"], command.as_str());
        let (again, changed) = merge_hook(Some(&text), AgentKind::ClaudeCode, &command).unwrap();
        assert!(!changed);
        assert_eq!(again, text);
        // No file at all: the snippet itself.
        let (fresh, _) = merge_hook(None, AgentKind::Antigravity, "t").unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&fresh).unwrap(),
            install_snippet(AgentKind::Antigravity, "t")
        );
        // Shapes the agent does not document are refused, not repaired.
        for bad in [
            "[]",
            "nope",
            r#"{"hooks":[]}"#,
            r#"{"hooks":{"SessionStart":{}}}"#,
        ] {
            assert!(
                merge_hook(Some(bad), AgentKind::ClaudeCode, "t").is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_fresh_write_never_replaces_an_existing_file() {
        let dir = scratch("beside");
        let target = dir.join("x.jsonl");
        write_beside(&target, b"one\n", true, None).unwrap();
        assert!(matches!(
            write_beside(&target, b"two\n", true, None),
            Err(WriteError::Io { .. })
        ));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "one\n");
        write_beside(&target, b"three\n", false, None).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "three\n");
        // No temp file is left behind either way.
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "{left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
