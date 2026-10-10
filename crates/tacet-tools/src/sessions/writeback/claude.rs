//! A note written into a Claude Code transcript (RG-22), or a fork of the
//! transcript carrying one, guarded as RG-31 asks.
//!
//! THE OFFICIAL PATH IS PREFERRED. The SessionStart hook (`hook_response` in
//! `mod.rs`, RG-21) hands the handoff to Claude Code without touching a
//! transcript and survives format changes. This file is the opt-in
//! alternative, for a user who wants the note to stay in the session's own
//! history; Claude Code's docs call that history "internal … and changes
//! between versions", so every step below fails closed.
//!
//! WHAT WAS MEASURED, AND WHAT EACH GUARD ANSWERS (Claude Code 2.1.285,
//! 9 Oct 2026, the E1–E5 experiment, `.audit/cc-append-experiment.py`):
//!
//! * E3 — one user-role entry `[Note from Tacet, a local assistant, not from
//!   the user] …` chained to the exact leaf, then a `last-prompt` line naming
//!   it, was loaded by `--resume` and credited to Tacet. That is the only shape
//!   written here; the entry copies `isSidechain`, `userType`, `entrypoint`,
//!   `cwd`, `sessionId`, `version` and `gitBranch` from the leaf.
//! * E4 — the same entry with a WRONG `parentUuid` made `--resume` load nothing
//!   before it. So after the write the chain is rebuilt with the reader
//!   (`ClaudeSource::read_report`, the same walk Claude Code does) and with a
//!   second, independent walk here; if the new entry is not the leaf or the old
//!   leaf is not its ancestor, the file is truncated back to its length before
//!   the write.
//! * E5 — `--resume <id> --fork-session` left the original byte-identical. The
//!   fork mode here does the same by hand: a NEW file under a new session id,
//!   the note appended there, the original only read.
//!
//! THE WRITE IS ONE `write` CALL WITH `O_APPEND`, both lines in one buffer, so
//! a reader never sees the entry without its `last-prompt`, and Claude Code
//! appending at the same moment cannot interleave inside it. A live session is
//! refused anyway (its `sessions/<pid>.json` with a running pid): two writers
//! choosing a leaf at the same time is the E4 failure by another road.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::{Confirm, Question, WriteError, io_error, new_uuid, now_iso_ms, write_beside};
use crate::sessions::claude::{ClaudeSource, MAX_TRANSCRIPT_BYTES};
use crate::sessions::model::{AgentKind, validate_id};
use crate::sessions::{secrets, version};

/// The fields a note entry copies from the leaf it is chained to — the list
/// the E3 entry was built with.
pub const COPIED_FIELDS: &[&str] = &[
    "isSidechain",
    "userType",
    "entrypoint",
    "cwd",
    "sessionId",
    "version",
    "gitBranch",
];

/// What was appended.
#[derive(Debug, Clone, PartialEq)]
pub struct Appended {
    pub path: PathBuf,
    /// The note's uuid: the session's leaf now.
    pub leaf: String,
    /// The leaf before the note, now its parent.
    pub parent: String,
    /// Where the transcript was copied before the write, when a backup
    /// directory was given.
    pub backup: Option<PathBuf>,
}

/// What was forked.
#[derive(Debug, Clone, PartialEq)]
pub struct Forked {
    /// The new transcript.
    pub path: PathBuf,
    /// The new session id: `claude --resume <session>` opens it.
    pub session: String,
    pub leaf: String,
    /// SHA-256 of the original before and after — equal, or this is an error.
    pub original_sha256: String,
}

/// A session read and judged writable: what both modes start from.
struct Prepared {
    path: PathBuf,
    /// The transcript's bytes as read — the exact file, checked below.
    text: String,
    leaf: String,
    /// The leaf's own line, the fields the note copies.
    base: Map<String, Value>,
    version: String,
}

fn prepare(source: &ClaudeSource, root: &Path, id: &str) -> Result<Prepared, WriteError> {
    validate_id(id)?;
    let report = source.read_report(root, id)?;
    // Both gates, in the order `may_write` runs them: a `.jsonl` target, and a
    // version some write was measured on.
    version::may_write(
        AgentKind::ClaudeCode,
        None,
        report.session.version.as_deref(),
        &report.path,
        false,
    )?;
    let leaf = report.leaf.ok_or_else(|| {
        WriteError::Refused("the transcript has no chain to attach a note to".into())
    })?;
    // A rebuild that had to step over something is a chain this module cannot
    // vouch for: a loop, a parent outside the file, lines it did not
    // understand. The note would land on a history the agent may not load.
    if !report
        .warnings
        .iter()
        .all(|w| w.contains("has not been read"))
    {
        return Err(WriteError::Refused(format!(
            "the chain did not rebuild cleanly: {}",
            report.warnings.join("; ")
        )));
    }
    let text = secrets::read_text(&report.path, MAX_TRANSCRIPT_BYTES)?;
    if !text.ends_with('\n') {
        // A last line without its newline is a line still being written, or
        // a crash in the middle of one; a note after it would be glued on.
        return Err(WriteError::Refused(
            "the transcript's last line is unfinished (no newline at its end)".into(),
        ));
    }
    let base = leaf_fields(&text, &leaf).ok_or_else(|| {
        WriteError::Refused(format!("the leaf {leaf} has no line of its own to copy"))
    })?;
    if base.get("sessionId").and_then(Value::as_str) != Some(id) {
        return Err(WriteError::Refused(format!(
            "the leaf's sessionId is not {id}; this file holds another session's chain"
        )));
    }
    let version = report.session.version.unwrap_or_default();
    Ok(Prepared {
        path: report.path,
        text,
        leaf,
        base,
        version,
    })
}

/// The copied fields of the LAST line written for `leaf` (a uuid can be
/// re-appended; the reader keeps the last write, and so does this).
fn leaf_fields(text: &str, leaf: &str) -> Option<Map<String, Value>> {
    let mut found = None;
    for line in text.lines() {
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if obj.get("uuid").and_then(Value::as_str) == Some(leaf) {
            found = Some(obj);
        }
    }
    let obj = found?;
    Some(
        COPIED_FIELDS
            .iter()
            .filter_map(|k| obj.get(*k).map(|v| (k.to_string(), v.clone())))
            .collect(),
    )
}

/// The two lines E3 measured: the note entry and the `last-prompt` naming it.
fn note_lines(
    base: &Map<String, Value>,
    session: &str,
    parent: &str,
    uuid: &str,
    text: &str,
) -> String {
    let mut entry = base.clone();
    entry.insert("sessionId".into(), Value::String(session.into()));
    entry.insert("parentUuid".into(), Value::String(parent.into()));
    entry.insert("type".into(), Value::String("user".into()));
    entry.insert("uuid".into(), Value::String(uuid.into()));
    entry.insert("timestamp".into(), Value::String(now_iso_ms()));
    entry.insert("message".into(), json!({ "role": "user", "content": text }));
    let last = json!({
        "type": "last-prompt",
        "lastPrompt": text,
        "leafUuid": uuid,
        "sessionId": session,
    });
    format!("{}\n{}\n", Value::Object(entry), last)
}

/// `uuid -> parentUuid` of the main chain's entries, the last write of each
/// uuid winning — the rule the reader uses, kept apart from it so the check
/// after a write is not the reader agreeing with itself.
fn parents(text: &str) -> HashMap<String, Option<String>> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let chain = matches!(
            v.get("type").and_then(Value::as_str),
            Some("user" | "assistant" | "attachment" | "system")
        );
        let side = v.get("isSidechain").and_then(Value::as_bool) == Some(true);
        let Some(uuid) = v.get("uuid").and_then(Value::as_str) else {
            continue;
        };
        if chain && !side {
            let parent = v
                .get("parentUuid")
                .and_then(Value::as_str)
                .map(str::to_string);
            out.insert(uuid.to_string(), parent);
        }
    }
    out
}

/// Is `ancestor` on the chain from `from` back to its root?
fn reaches(text: &str, from: &str, ancestor: &str) -> bool {
    let map = parents(text);
    let mut seen = HashSet::new();
    let mut cur = Some(from.to_string());
    while let Some(uuid) = cur {
        if uuid == ancestor {
            return true;
        }
        if !seen.insert(uuid.clone()) {
            return false;
        }
        cur = map.get(&uuid).cloned().flatten();
    }
    false
}

/// The check after a write: the note is the leaf the agent will resume from,
/// and the history before it is still on its chain (E4).
fn verify(
    source: &ClaudeSource,
    root: &Path,
    session: &str,
    path: &Path,
    old_leaf: &str,
    new_leaf: &str,
) -> Result<(), String> {
    let report = source
        .read_report(root, session)
        .map_err(|e| format!("the transcript could not be read back: {e}"))?;
    if report.path != path {
        return Err(format!(
            "the session now reads from {}, not the file written",
            report.path.display()
        ));
    }
    if report.leaf.as_deref() != Some(new_leaf) {
        return Err(format!(
            "the leaf the agent would resume from is {:?}, not the note {new_leaf}",
            report.leaf
        ));
    }
    let text = secrets::read_text(path, MAX_TRANSCRIPT_BYTES).map_err(|e| e.to_string())?;
    if !reaches(&text, new_leaf, old_leaf) {
        return Err(format!(
            "the old leaf {old_leaf} is not an ancestor of the note: a resume would drop the \
             history before it"
        ));
    }
    Ok(())
}

fn question(heading: String, p: &Prepared, note: &str, extra: &[String]) -> Question {
    let mut lines = vec![
        format!("file:    {}", p.path.display()),
        format!("version: {} (inside the measured write range)", p.version),
        format!("chained to the leaf {}", p.leaf),
    ];
    lines.extend(extra.iter().cloned());
    lines.push("the note, written in the user role:".into());
    lines.extend(note.lines().map(|l| format!("  {l}")));
    Question { heading, lines }
}

/// Appends `note` to session `id`'s transcript as ONE user-role entry chained
/// to the current leaf, plus its `last-prompt` — after asking.
///
/// Refused when: the session is live, its version is outside the measured
/// write range, the chain did not rebuild cleanly, the file changed between
/// the read and the write, or the note does not start with `NOTE_PREFIX`.
/// Undone (truncated back) when the check after the write fails.
pub fn append_note(
    source: &ClaudeSource,
    root: &Path,
    id: &str,
    note: &str,
    backup_dir: Option<&Path>,
    confirm: &dyn Confirm,
) -> Result<Appended, WriteError> {
    append_with_parent(source, root, id, note, backup_dir, confirm, None)
}

/// `append_note`, with the parent forced — the E4 seam. Only the tests below
/// pass `Some`: it is how the rollback is shown to catch a wrong parent
/// without a real Claude Code to lose the history.
fn append_with_parent(
    source: &ClaudeSource,
    root: &Path,
    id: &str,
    note: &str,
    backup_dir: Option<&Path>,
    confirm: &dyn Confirm,
    forced_parent: Option<&str>,
) -> Result<Appended, WriteError> {
    if !note.starts_with(super::NOTE_PREFIX) {
        return Err(WriteError::Refused(
            "a note must open with Tacet's prefix (the E3 shape); nothing else is written".into(),
        ));
    }
    let live = |src: &ClaudeSource| src.live_sessions(root).contains_key(id);
    if live(source) {
        return Err(WriteError::Live(format!(
            "session {id} is open in Claude Code (sessions/<pid>.json, process running); \
             close it first, or fork instead"
        )));
    }
    let p = prepare(source, root, id)?;
    let parent = forced_parent.unwrap_or(&p.leaf).to_string();
    let uuid = new_uuid();
    let lines = note_lines(&p.base, id, &parent, &uuid, note);
    let q = question(
        format!("Append a note to Claude Code session {id}?"),
        &p,
        note,
        &["the session's history is not changed; the note is added after it".into()],
    );
    if !confirm.confirm(&q) {
        return Err(WriteError::Declined);
    }
    // ASKED BEFORE, CHECKED AGAIN NOW: the person may have opened the session
    // while reading the question.
    if live(source) {
        return Err(WriteError::Live(format!(
            "session {id} was opened in Claude Code while the question was on screen"
        )));
    }
    let backup = match backup_dir {
        Some(dir) => Some(backup_copy(dir, id, &p.text)?),
        None => None,
    };
    let pre_len = p.text.len() as u64;
    let file = secrets::open_append(&p.path)?;
    let io = io_error(&p.path);
    let now_len = file.metadata().map_err(&io)?.len();
    if now_len != pre_len {
        return Err(WriteError::Refused(format!(
            "the transcript changed since it was read ({pre_len} → {now_len} bytes), or is not \
             valid UTF-8; nothing written"
        )));
    }
    // Through the handle the append was made on: the file cut back is the
    // file the gate checked, not whatever is at the path by now.
    let undo = |why: String| -> WriteError {
        match file.set_len(pre_len) {
            Ok(()) if file.metadata().map(|m| m.len()).ok() == Some(pre_len) => {
                WriteError::RolledBack(why)
            }
            _ => WriteError::Refused(format!(
                "{why} — AND THE ROLLBACK FAILED; the transcript before the write is at {}",
                backup
                    .as_deref()
                    .map_or("(no backup was asked for)".into(), |b| b
                        .display()
                        .to_string())
            )),
        }
    };
    // ONE `write` call: a short write is undone, not completed by a second
    // call that another appender could land between.
    match (&file).write(lines.as_bytes()) {
        Ok(n) if n == lines.len() => {}
        Ok(n) => return Err(undo(format!("a short write ({n} of {})", lines.len()))),
        Err(e) => return Err(undo(format!("the write failed: {e}"))),
    }
    if let Err(why) = verify(source, root, id, &p.path, &p.leaf, &uuid) {
        return Err(undo(why));
    }
    Ok(Appended {
        path: p.path.clone(),
        leaf: uuid,
        parent,
        backup,
    })
}

/// A copy of the transcript as it was before the write, 0600 in a 0700
/// directory: the transcript is the user's code and prompts (RG-01).
fn backup_copy(dir: &Path, id: &str, text: &str) -> Result<PathBuf, WriteError> {
    tacet_kernel::fs::create_private_dir(dir).map_err(io_error(dir))?;
    let path = dir.join(format!(
        "{}-{id}-{}.jsonl",
        AgentKind::ClaudeCode.slug(),
        crate::sessions::model::now_epoch()
    ));
    write_beside(&path, text.as_bytes(), true, None)?;
    Ok(path)
}

/// Writes a NEW transcript: session `id`'s lines under a fresh session id, the
/// note appended there — and leaves the original byte-identical (E5's
/// guarantee, checked by hash after the write).
///
/// Allowed while the original is live: nothing of it is written. The version
/// gate still applies — the fork must load in the same Claude Code.
pub fn fork_with_note(
    source: &ClaudeSource,
    root: &Path,
    id: &str,
    note: &str,
    confirm: &dyn Confirm,
) -> Result<Forked, WriteError> {
    if !note.starts_with(super::NOTE_PREFIX) {
        return Err(WriteError::Refused(
            "a note must open with Tacet's prefix (the E3 shape); nothing else is written".into(),
        ));
    }
    let p = prepare(source, root, id)?;
    let before = tacet_kernel::sha256_hex(p.text.as_bytes());
    let session = new_uuid();
    let target = p.path.with_file_name(format!("{session}.jsonl"));
    let mut body = String::with_capacity(p.text.len() + note.len() * 2 + 512);
    for line in p.text.split_inclusive('\n') {
        match serde_json::from_str::<Value>(line.trim_end()) {
            Ok(Value::Object(mut obj))
                if obj.get("sessionId").and_then(Value::as_str) == Some(id) =>
            {
                obj.insert("sessionId".into(), Value::String(session.clone()));
                body.push_str(&Value::Object(obj).to_string());
                body.push('\n');
            }
            // Lines of no session, or not JSON: as they were. The reader
            // skips what it does not understand, and so will Claude Code.
            _ => body.push_str(line),
        }
    }
    let uuid = new_uuid();
    body.push_str(&note_lines(&p.base, &session, &p.leaf, &uuid, note));
    let q = question(
        format!("Fork Claude Code session {id} into a new session with a note?"),
        &p,
        note,
        &[
            format!("new file: {}", target.display()),
            "the original transcript is only read; it stays byte-identical".into(),
        ],
    );
    if !confirm.confirm(&q) {
        return Err(WriteError::Declined);
    }
    write_beside(&target, body.as_bytes(), true, None)?;
    if let Err(why) = verify(source, root, &session, &target, &p.leaf, &uuid) {
        let _ = std::fs::remove_file(&target);
        return Err(WriteError::RolledBack(why));
    }
    let after = secrets::read_text(&p.path, MAX_TRANSCRIPT_BYTES)
        .map(|t| tacet_kernel::sha256_hex(t.as_bytes()))
        .unwrap_or_default();
    if after != before {
        // Claude Code wrote to the original while the fork was being made.
        // The fork is still a fork of what was read, but the guarantee the
        // user was given is not one this run can state.
        return Err(WriteError::Refused(format!(
            "the original changed while it was forked (another writer); the fork is at {}",
            target.display()
        )));
    }
    Ok(Forked {
        path: target,
        session,
        leaf: uuid,
        original_sha256: before,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::writeback::NOTE_PREFIX;
    use std::cell::RefCell;

    const ID: &str = "c0de0001-0000-4000-8000-000000000001";

    struct Answer {
        yes: bool,
        asked: RefCell<Vec<Question>>,
    }

    impl Answer {
        fn yes() -> Self {
            Answer {
                yes: true,
                asked: RefCell::new(Vec::new()),
            }
        }
        fn no() -> Self {
            Answer {
                yes: false,
                asked: RefCell::new(Vec::new()),
            }
        }
    }

    impl Confirm for Answer {
        fn confirm(&self, q: &Question) -> bool {
            self.asked.borrow_mut().push(q.clone());
            self.yes
        }
    }

    fn dead(_: u32) -> bool {
        false
    }
    fn alive(_: u32) -> bool {
        true
    }

    fn line(uuid: &str, parent: Option<&str>, kind: &str, version: &str, content: &str) -> String {
        let message = if kind == "assistant" {
            json!({"id": format!("msg_{uuid}"), "type": "message", "role": "assistant",
                   "model": "test-model", "content": [{"type": "text", "text": content}],
                   "stop_reason": "end_turn", "stop_sequence": null,
                   "usage": {"input_tokens": 1, "output_tokens": 1}})
        } else {
            json!({"role": "user", "content": content})
        };
        json!({"parentUuid": parent, "isSidechain": false, "userType": "external",
               "cwd": "/Users/dev/demo", "sessionId": ID, "version": version,
               "gitBranch": "main", "entrypoint": "cli", "uuid": uuid,
               "timestamp": "2026-10-09T10:00:00.000Z", "type": kind, "message": message})
        .to_string()
    }

    /// A temp `~/.claude` with one three-entry session, E1's shape.
    fn root_with(version: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("tacet-wb-claude-{}", new_uuid()));
        let dir = root.join("projects/-Users-dev-demo");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{ID}.jsonl"));
        let text = [
            line(
                "u0000001-0000-4000-8000-000000000000",
                None,
                "user",
                version,
                "Fix the rollover",
            ),
            line(
                "u0000002-0000-4000-8000-000000000000",
                Some("u0000001-0000-4000-8000-000000000000"),
                "assistant",
                version,
                "The codeword is PELICAN-42.",
            ),
            json!({"type": "last-prompt", "lastPrompt": "Fix the rollover",
                   "leafUuid": "u0000002-0000-4000-8000-000000000000", "sessionId": ID})
            .to_string(),
        ]
        .join("\n")
            + "\n";
        std::fs::write(&path, text).unwrap();
        (root, path)
    }

    fn note() -> String {
        format!("{NOTE_PREFIX} I fixed the calendar rollover. Codeword: OSPREY-7.")
    }

    /// E3, reproduced on a fixture: exactly two lines are added — one
    /// user-role note chained to the exact leaf, one `last-prompt` naming it —
    /// and the reader now resumes from the note with the whole history before
    /// it.
    #[test]
    fn a_note_is_appended_in_the_e3_shape_and_the_history_still_loads() {
        let (root, path) = root_with("2.1.285");
        let before = std::fs::read_to_string(&path).unwrap();
        let src = ClaudeSource::with_probe(dead);
        let ask = Answer::yes();
        let backups = root.join("backups");
        let done = append_note(&src, &root, ID, &note(), Some(&backups), &ask).unwrap();
        assert_eq!(ask.asked.borrow().len(), 1, "asked once");
        assert!(
            ask.asked.borrow()[0]
                .lines
                .iter()
                .any(|l| l.contains("OSPREY-7"))
        );
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.starts_with(&before),
            "the history was changed, not appended to"
        );
        let added: Vec<Value> = after[before.len()..]
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(added.len(), 2, "{added:#?}");
        let entry = &added[0];
        assert_eq!(entry["type"], "user");
        assert_eq!(entry["message"]["role"], "user");
        assert_eq!(entry["message"]["content"], note().as_str());
        assert_eq!(entry["parentUuid"], "u0000002-0000-4000-8000-000000000000");
        assert_eq!(entry["uuid"], done.leaf.as_str());
        for k in COPIED_FIELDS {
            assert!(entry.get(*k).is_some(), "{k} was not copied: {entry}");
        }
        assert_eq!(added[1]["type"], "last-prompt");
        assert_eq!(added[1]["leafUuid"], done.leaf.as_str());
        assert_eq!(added[1]["sessionId"], ID);
        let read = src.read_report(&root, ID).unwrap();
        assert_eq!(read.leaf.as_deref(), Some(done.leaf.as_str()));
        let texts: Vec<&str> = read
            .session
            .entries
            .iter()
            .map(|e| e.text.as_str())
            .collect();
        assert!(texts.contains(&"The codeword is PELICAN-42."), "{texts:?}");
        assert_eq!(texts.last(), Some(&note().as_str()));
        // And the backup is the file as it was.
        assert_eq!(
            std::fs::read_to_string(done.backup.unwrap()).unwrap(),
            before
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// E4, reproduced: a note chained to a parent that is not in the file is
    /// what made `--resume` drop the whole history. The check after the write
    /// sees it and the file is cut back to its exact bytes.
    #[test]
    fn a_wrong_parent_is_detected_and_rolled_back() {
        let (root, path) = root_with("2.1.285");
        let before = std::fs::read(&path).unwrap();
        let src = ClaudeSource::with_probe(dead);
        let result = append_with_parent(
            &src,
            &root,
            ID,
            &note(),
            None,
            &Answer::yes(),
            Some("00000000-0000-4000-8000-000000000000"),
        );
        match result {
            Err(WriteError::RolledBack(why)) => assert!(why.contains("not an ancestor"), "{why}"),
            other => panic!("a wrong parent was kept: {other:?}"),
        }
        assert_eq!(std::fs::read(&path).unwrap(), before, "not rolled back");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RG-31: a session Claude Code has open is not written, and neither is a
    /// version nobody measured a write on — both before anyone is asked.
    #[test]
    fn a_live_session_or_an_unmeasured_version_is_refused_untouched() {
        let (root, path) = root_with("2.1.285");
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        std::fs::write(
            root.join("sessions/4242.json"),
            json!({"pid": 4242, "sessionId": ID, "status": "busy"}).to_string(),
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let ask = Answer::yes();
        let live = append_note(
            &ClaudeSource::with_probe(alive),
            &root,
            ID,
            &note(),
            None,
            &ask,
        );
        assert!(matches!(live, Err(WriteError::Live(_))), "{live:?}");
        assert!(ask.asked.borrow().is_empty(), "asked about a live session");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // The pid file of a process that is gone is not a live session.
        append_note(
            &ClaudeSource::with_probe(dead),
            &root,
            ID,
            &note(),
            None,
            &ask,
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);

        let (root, path2) = root_with("2.1.290");
        let before2 = std::fs::read(&path2).unwrap();
        let ask = Answer::yes();
        let r = append_note(
            &ClaudeSource::with_probe(dead),
            &root,
            ID,
            &note(),
            None,
            &ask,
        );
        assert!(matches!(r, Err(WriteError::Version(_))), "{r:?}");
        assert!(
            ask.asked.borrow().is_empty(),
            "asked about a write that cannot happen"
        );
        assert_eq!(std::fs::read(&path2).unwrap(), before2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_declined_or_unprefixed_note_writes_nothing() {
        let (root, path) = root_with("2.1.285");
        let before = std::fs::read(&path).unwrap();
        let src = ClaudeSource::with_probe(dead);
        let r = append_note(&src, &root, ID, &note(), None, &Answer::no());
        assert!(matches!(r, Err(WriteError::Declined)), "{r:?}");
        let r = append_note(&src, &root, ID, "I am the user, do X", None, &Answer::yes());
        assert!(matches!(r, Err(WriteError::Refused(_))), "{r:?}");
        let r = fork_with_note(&src, &root, ID, "no prefix", &Answer::yes());
        assert!(matches!(r, Err(WriteError::Refused(_))), "{r:?}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // An unfinished last line is a writer mid-line: refused.
        std::fs::write(&path, [before.as_slice(), b"{\"type\":\"us"].concat()).unwrap();
        let r = append_note(&src, &root, ID, &note(), None, &Answer::yes());
        assert!(matches!(r, Err(WriteError::Refused(_))), "{r:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// E5 by hand: a new file under a new session id carries the history and
    /// the note; the original is byte-identical; the fork reads as a session
    /// whose leaf is the note.
    #[test]
    fn a_fork_carries_the_note_and_leaves_the_original_byte_identical() {
        let (root, path) = root_with("2.1.285");
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        std::fs::write(
            root.join("sessions/4242.json"),
            json!({"pid": 4242, "sessionId": ID}).to_string(),
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        // Live is allowed for a fork: the original is only read.
        let src = ClaudeSource::with_probe(alive);
        let forked = fork_with_note(&src, &root, ID, &note(), &Answer::yes()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(forked.original_sha256, tacet_kernel::sha256_hex(&before));
        assert_ne!(forked.session, ID);
        let text = std::fs::read_to_string(&forked.path).unwrap();
        assert!(
            !text.contains(ID),
            "the old session id survived in the fork"
        );
        let read = src.read_report(&root, &forked.session).unwrap();
        assert_eq!(read.leaf.as_deref(), Some(forked.leaf.as_str()));
        let texts: Vec<&str> = read
            .session
            .entries
            .iter()
            .map(|e| e.text.as_str())
            .collect();
        assert!(texts.contains(&"Fix the rollover"), "{texts:?}");
        assert_eq!(texts.last(), Some(&note().as_str()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Claude Code appending to the original while the user reads the
    /// question: the second hash is the only thing that notices. The test
    /// above cannot see that check — nothing else writes there, so the
    /// original is unchanged whether the check runs or not (review of 10 Oct
    /// 2026, mutation m4b stayed green).
    struct AppendsWhileAsking {
        path: PathBuf,
    }

    impl Confirm for AppendsWhileAsking {
        fn confirm(&self, _: &Question) -> bool {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&self.path)
                .unwrap();
            std::io::Write::write_all(&mut f, b"{\"type\":\"summary\",\"summary\":\"later\"}\n")
                .unwrap();
            true
        }
    }

    #[test]
    fn a_fork_whose_original_changes_while_asking_says_so() {
        let (root, path) = root_with("2.1.285");
        let src = ClaudeSource::with_probe(dead);
        let writer = AppendsWhileAsking { path: path.clone() };
        let r = fork_with_note(&src, &root, ID, &note(), &writer);
        match r {
            Err(WriteError::Refused(why)) => {
                assert!(why.contains("the original changed"), "{why}");
            }
            other => panic!("a changed original was not reported: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
