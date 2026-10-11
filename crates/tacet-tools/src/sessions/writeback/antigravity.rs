//! Antigravity: checkbox updates to a conversation's `task.md` (RG-26) —
//! experimental, off by default, and refused by the version gate until a
//! person verifies by hand that the GUI re-reads the file.
//!
//! THE SUPPORTED PATH IS THE HOOK (RG-25). A PreInvocation hook installed with
//! the user's approval runs `tacet handoff hook antigravity`, which reads
//! Tacet's handoff record and returns it as an `ephemeralMessage` in
//! `injectSteps` (`hook_response` in `mod.rs`). Nothing of Antigravity's is
//! written that way.
//!
//! WHY THIS WRITE IS FENCED THREE TIMES. Antigravity's terms (§6) forbid
//! third-party access to the product and its FAQ says writing to its storage
//! is unsupported (ADR 0001, "Antigravity terms of service"). So:
//!
//! 1. `version::check_write_target` refuses every file but `task.md`, and
//!    `task.md` too unless the caller passed the explicit experimental flag;
//! 2. `version::check_write` refuses every Antigravity version, because no
//!    write range was ever measured (the IDE 2.5.5 and `agy` 1.1.10 rows have
//!    `write: None`) — the mechanism is built and tested here, and stays shut
//!    until RG-26's manual check adds a range;
//! 3. a conversation that looks open (its database has a `-wal`, its last step
//!    is RUNNING) or an Antigravity process that is running stops the write
//!    (RG-31): the agent rewrites `task.md` itself, and two writers of one file
//!    lose one of the two.
//!
//! ONLY THE BOX CHANGES. `- [ ]`, `- [/]` and `- [x]` are the three states the
//! file has (MEASURED on IDE 2.5.5); blocked and cancelled have no box and stay
//! in Tacet's handoff record. Every other byte of the file is kept, and
//! `task.md.metadata.json`'s `updatedAt` is moved to now, the field the IDE was
//! seen to keep beside each artifact.
//!
//! THE EDIT MAY BECOME PART OF ANTIGRAVITY'S OWN GIT HISTORY. Under the CLI
//! root (`~/.gemini/antigravity-cli/brain/<id>/`) a conversation's folder is a
//! git repository, and Antigravity commits it on its own — `git log` there
//! shows "jetski Snapshot" commits (seen by the RG-26 review on 10 Oct 2026,
//! on the macOS machine this was written on). A box Tacet ticks is then committed by
//! Antigravity next to the agent's own edits, and stays in that history after
//! anyone unticks it. So whenever `brain/<id>/.git` exists, the question says
//! so before the yes; the check is on the folder, not on the root's name,
//! because whether the IDE root does the same was not looked at.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Confirm, Question, WriteError, now_iso_ms, write_beside};
use crate::sessions::antigravity::{AntigravitySource, flavor, parse_task_md};
use crate::sessions::handoff::{Status, parse_task_md_line};
use crate::sessions::model::{AgentKind, validate_id};
use crate::sessions::{secrets, version};

/// Shown with every experimental write, before the question.
pub const TERMS_WARNING: &str = "Experimental: Antigravity's terms of service (section 6) forbid \
     third-party software from accessing the product, and writing to its storage is unsupported. \
     Tacet changes only task.md checkboxes, only with this flag, and the supported way to report \
     back is the PreInvocation hook (`tacet handoff install antigravity`).";

/// Added to the question when the conversation's folder is a git repository
/// (see the module comment): the change outlives the checkbox.
pub const SNAPSHOT_WARNING: &str = "this conversation's folder is a git repository: \
     Antigravity's automatic \"jetski Snapshot\" commits will record this change in its \
     history, where it stays even if the box is changed back";

/// The largest `task.md` this writer edits. A plan is a page; the line-to-item
/// map below is quadratic in lines and is not run on a file it was not meant
/// for.
pub const MAX_TASK_MD_BYTES: u64 = 64 << 10;

/// Substrings of a process's executable path that mean "Antigravity is
/// running". The product's own name covers the app bundle
/// (`/Applications/Antigravity.app/…`) and the CLI root; the `agy` CLI's
/// binary is matched by its file name. NOT MEASURED against a running
/// Antigravity: an over-broad match only refuses a write, the safe side.
const CLI_NAME: &str = "agy";

/// One checkbox to set.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskUpdate {
    /// The item's title as `task.md` shows it (without its box).
    pub title: String,
    pub status: Status,
}

/// What was written.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskMdWritten {
    pub path: PathBuf,
    pub changed: usize,
    /// Whether `task.md.metadata.json` was found and its `updatedAt` moved.
    pub metadata_updated: bool,
}

/// Does any of `processes` look like Antigravity?
pub fn antigravity_running(processes: &[String]) -> bool {
    let product = AgentKind::Antigravity.slug();
    processes.iter().any(|p| {
        let lower = p.to_lowercase();
        let name = lower.rsplit(['/', '\\']).next().unwrap_or_default();
        lower.contains(product) || name == CLI_NAME
    })
}

/// The box character a status is written with; `None` for the two statuses a
/// `task.md` box cannot hold.
fn box_char(status: Status) -> Option<char> {
    match status {
        Status::Pending => Some(' '),
        Status::InProgress => Some('/'),
        Status::Completed => Some('x'),
        Status::Blocked | Status::Cancelled => None,
    }
}

/// `text` with each update's box changed, and how many lines changed.
///
/// An update must name exactly ONE real item (a fenced example is not one);
/// a title that matches none or several is refused, and so is a change the
/// line's own text would read back as something else.
pub fn apply_updates(text: &str, updates: &[TaskUpdate]) -> Result<(String, usize), WriteError> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    // Which lines are items: the reader's own tree parse (fences and all),
    // run on each prefix, grows by one exactly at an item line.
    let mut items: Vec<usize> = Vec::new();
    let mut prefix = String::with_capacity(text.len());
    let mut count = 0;
    for (i, line) in lines.iter().enumerate() {
        prefix.push_str(line);
        let now = parse_task_md(&prefix).len();
        if now > count {
            items.push(i);
            count = now;
        }
    }
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let mut changed = 0;
    for u in updates {
        let want = box_char(u.status).ok_or_else(|| {
            WriteError::Refused(format!(
                "task.md has no box for {}; Tacet's handoff record carries it",
                u.status
            ))
        })?;
        let hits: Vec<usize> = items
            .iter()
            .copied()
            .filter(|&i| {
                parse_task_md_line(lines[i].trim_end_matches(['\n', '\r']))
                    .is_some_and(|item| item.title == u.title.trim())
            })
            .collect();
        let at = match hits.as_slice() {
            [one] => *one,
            [] => {
                return Err(WriteError::Refused(format!(
                    "no task named {:?} in task.md",
                    u.title
                )));
            }
            _ => {
                return Err(WriteError::Refused(format!(
                    "{} tasks are named {:?}; which one is meant cannot be told",
                    hits.len(),
                    u.title
                )));
            }
        };
        let line = &out[at];
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        // `- [?] ` or `* [?] `: the box character sits three bytes in.
        let at_box = indent + 3;
        if line.get(indent + 2..indent + 3) != Some("[") || !line.is_char_boundary(at_box + 1) {
            return Err(WriteError::Refused(format!(
                "{:?} is crossed out, not boxed; it is left as it is",
                u.title
            )));
        }
        let mut edited = String::with_capacity(line.len());
        edited.push_str(&line[..at_box]);
        edited.push(want);
        edited.push_str(&line[at_box + 1..]);
        let read_back = parse_task_md_line(edited.trim_end_matches(['\n', '\r'])).map(|i| i.status);
        if read_back != Some(u.status) {
            return Err(WriteError::Refused(format!(
                "with that box, the line {:?} would read as {read_back:?}, not {}",
                edited.trim_end(),
                u.status
            )));
        }
        if edited != *line {
            changed += 1;
        }
        out[at] = edited;
    }
    Ok((out.concat(), changed))
}

/// Why the write must wait: the conversation or the product looks open.
fn liveness(
    root: &Path,
    id: &str,
    processes: &dyn Fn() -> Option<Vec<String>>,
) -> Result<(), WriteError> {
    let read = AntigravitySource.read_full(root, id)?;
    if read.session.live {
        return Err(WriteError::Live(format!(
            "conversation {id} looks open in Antigravity (its database has a write-ahead log, or \
             its last step is still running)"
        )));
    }
    match processes() {
        None => Err(WriteError::Live(
            "the running processes could not be listed, so an open Antigravity cannot be ruled out"
                .into(),
        )),
        Some(list) if antigravity_running(&list) => Err(WriteError::Live(
            "an Antigravity process is running; quit it first — it rewrites task.md itself".into(),
        )),
        Some(_) => Ok(()),
    }
}

/// Sets checkboxes in conversation `id`'s `task.md` — experimental, after the
/// terms warning and a question.
pub fn update_task_md(
    root: &Path,
    id: &str,
    updates: &[TaskUpdate],
    experimental: bool,
    processes: &dyn Fn() -> Option<Vec<String>>,
    confirm: &dyn Confirm,
) -> Result<TaskMdWritten, WriteError> {
    update_impl(root, id, updates, experimental, processes, confirm, true)
}

/// `measured_only`: false only in the tests below, which drive the mechanism
/// while the version table still refuses every Antigravity version.
fn update_impl(
    root: &Path,
    id: &str,
    updates: &[TaskUpdate],
    experimental: bool,
    processes: &dyn Fn() -> Option<Vec<String>>,
    confirm: &dyn Confirm,
    measured_only: bool,
) -> Result<TaskMdWritten, WriteError> {
    validate_id(id)?;
    let path = root.join("brain").join(id).join("task.md");
    version::check_write_target(AgentKind::Antigravity, &path, experimental)?;
    if measured_only {
        // Antigravity transcripts record no version, so the gate is asked
        // about "unrecorded" — and refuses it, as it refuses every version
        // without a measured write range.
        version::check_write(AgentKind::Antigravity, flavor(root), None)?;
    }
    if updates.is_empty() {
        return Err(WriteError::Refused("no checkbox was named".into()));
    }
    liveness(root, id, processes)?;
    let text = secrets::read_text(&path, MAX_TASK_MD_BYTES)?;
    let (new_text, changed) = apply_updates(&text, updates)?;
    let mut lines = vec![
        TERMS_WARNING.to_string(),
        format!("file: {}", path.display()),
    ];
    for u in updates {
        lines.push(format!("  {} → {}", u.title, u.status));
    }
    if std::fs::symlink_metadata(root.join("brain").join(id).join(".git")).is_ok() {
        lines.push(SNAPSHOT_WARNING.to_string());
    }
    let q = Question {
        heading: format!(
            "Change {changed} checkbox(es) in Antigravity conversation {id}'s task.md?"
        ),
        lines,
    };
    if !confirm.confirm(&q) {
        return Err(WriteError::Declined);
    }
    liveness(root, id, processes)?;
    // The file the question was about must be the file replaced.
    if secrets::read_text(&path, MAX_TASK_MD_BYTES)? != text {
        return Err(WriteError::Refused(
            "task.md changed while the question was on screen; nothing written".into(),
        ));
    }
    write_beside(&path, new_text.as_bytes(), false, Some(&path))?;
    let meta = path.with_file_name("task.md.metadata.json");
    let metadata_updated = touch_metadata(&meta)?;
    Ok(TaskMdWritten {
        path,
        changed,
        metadata_updated,
    })
}

/// Moves `updatedAt` to now in an existing metadata file, keeping every other
/// field. A missing file is not created (Tacet does not add files to
/// Antigravity's storage); a file that is not a JSON object is left alone.
fn touch_metadata(meta: &Path) -> Result<bool, WriteError> {
    if std::fs::symlink_metadata(meta).is_err() {
        return Ok(false);
    }
    let text = secrets::read_text(meta, 64 << 10)?;
    let Ok(Value::Object(mut obj)) = serde_json::from_str::<Value>(&text) else {
        return Ok(false);
    };
    obj.insert("updatedAt".into(), Value::String(now_iso_ms()));
    write_beside(
        meta,
        Value::Object(obj).to_string().as_bytes(),
        false,
        Some(meta),
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::writeback::new_uuid;
    use std::cell::RefCell;

    const ID: &str = "7f3c2a10-5b6e-4c1d-9a8f-0e1d2c3b4a01";

    const TASK_MD: &str = "# Fix the rollover bug\n\n- [x] Reproduce the bug\n    - [x] Read the scheduler\n- [/] Fix the day boundary\n    - [ ] Handle DST\n- [ ] Update the changelog (blocked: waiting for the release number)\n\n```markdown\n- [ ] Handle DST\n```\n";

    struct Answer(bool, RefCell<Vec<Question>>);

    impl Confirm for Answer {
        fn confirm(&self, q: &Question) -> bool {
            self.1.borrow_mut().push(q.clone());
            self.0
        }
    }

    fn yes() -> Answer {
        Answer(true, RefCell::new(Vec::new()))
    }

    fn quiet() -> Option<Vec<String>> {
        Some(vec!["/bin/zsh".into(), "/usr/libexec/logd".into()])
    }

    fn root() -> PathBuf {
        let root = std::env::temp_dir()
            .join(format!("tacet-wb-ag-{}", new_uuid()))
            .join("antigravity-ide");
        let brain = root.join("brain").join(ID);
        std::fs::create_dir_all(brain.join(".system_generated/logs")).unwrap();
        std::fs::write(
            brain.join(".system_generated/logs/transcript_full.jsonl"),
            "{\"step_index\":0,\"type\":\"USER_INPUT\",\"source\":\"USER_EXPLICIT\",\"status\":\"DONE\",\"created_at\":\"2026-10-09T09:00:00Z\",\"content\":\"<USER_REQUEST>fix it</USER_REQUEST>\"}\n",
        )
        .unwrap();
        std::fs::write(brain.join("task.md"), TASK_MD).unwrap();
        std::fs::write(
            brain.join("task.md.metadata.json"),
            "{\"summary\":\"plan\",\"updatedAt\":\"2026-10-09T09:03:00Z\"}",
        )
        .unwrap();
        root
    }

    fn set(title: &str, status: Status) -> TaskUpdate {
        TaskUpdate {
            title: title.into(),
            status,
        }
    }

    /// RG-26: the box changes, nothing else does, the fenced example of the
    /// same title is not the item, and `updatedAt` moves.
    #[test]
    fn only_the_named_box_changes_and_updated_at_moves() {
        let root = root();
        let ask = yes();
        let done = update_impl(
            &root,
            ID,
            &[
                set("Handle DST", Status::Completed),
                set("Fix the day boundary", Status::Completed),
            ],
            true,
            &quiet,
            &ask,
            false,
        )
        .unwrap();
        assert_eq!(done.changed, 2);
        assert!(done.metadata_updated);
        let text = std::fs::read_to_string(&done.path).unwrap();
        let expected = TASK_MD
            .replace("- [/] Fix the day boundary", "- [x] Fix the day boundary")
            .replacen("    - [ ] Handle DST", "    - [x] Handle DST", 1);
        assert_eq!(text, expected);
        assert!(
            text.contains("```markdown\n- [ ] Handle DST\n```"),
            "{text}"
        );
        let meta: Value = serde_json::from_str(
            &std::fs::read_to_string(done.path.with_file_name("task.md.metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(meta["summary"], "plan");
        assert_ne!(meta["updatedAt"], "2026-10-09T09:03:00Z");
        // The terms warning is part of the question.
        assert!(ask.1.borrow()[0].lines[0].starts_with("Experimental:"));
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    /// The three fences: no flag, the version gate (every Antigravity version,
    /// today), and a target that is not task.md. Nothing asked, nothing
    /// written.
    #[test]
    fn without_the_flag_or_a_measured_version_nothing_is_written() {
        let root = root();
        let path = root.join("brain").join(ID).join("task.md");
        let ask = yes();
        let r = update_task_md(
            &root,
            ID,
            &[set("Handle DST", Status::Completed)],
            false,
            &quiet,
            &ask,
        );
        assert!(
            matches!(r, Err(WriteError::Version(ref w)) if w.reason.contains("experimental")),
            "{r:?}"
        );
        let r = update_task_md(
            &root,
            ID,
            &[set("Handle DST", Status::Completed)],
            true,
            &quiet,
            &ask,
        );
        // Antigravity records no version, and "could not tell" is refused
        // like "never verified": no write range exists for either.
        assert!(
            matches!(r, Err(WriteError::Version(ref w)) if w.reason.contains("version")),
            "{r:?}"
        );
        assert!(ask.1.borrow().is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TASK_MD);
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    /// RG-31: an open conversation, a running Antigravity, or a process list
    /// that could not be taken — each stops the write, BEFORE the question.
    /// Asking about a write the check after the yes will refuse wastes the
    /// user's yes; the Claude and Codex tests assert nothing was asked, and
    /// this one did not until the review of 10 Oct 2026 deleted the first
    /// check and watched it stay green (mutation m9b).
    #[test]
    fn an_open_conversation_or_a_running_antigravity_stops_the_write() {
        let root = root();
        let path = root.join("brain").join(ID).join("task.md");
        let one = [set("Handle DST", Status::Completed)];
        let running = || {
            Some(vec![
                "/Applications/Antigravity.app/Contents/MacOS/Electron".to_string(),
            ])
        };
        let cli = || Some(vec![format!("/Users/u/.local/bin/{CLI_NAME}")]);
        let unknown = || None;
        for probe in [&running as &dyn Fn() -> Option<Vec<String>>, &cli, &unknown] {
            let answer = yes();
            let r = update_impl(&root, ID, &one, true, probe, &answer, false);
            assert!(matches!(r, Err(WriteError::Live(_))), "{r:?}");
            assert_eq!(
                answer.1.borrow().len(),
                0,
                "asked about a write that cannot happen"
            );
        }
        std::fs::create_dir_all(root.join("conversations")).unwrap();
        std::fs::write(root.join(format!("conversations/{ID}.db")), "x").unwrap();
        std::fs::write(root.join(format!("conversations/{ID}.db-wal")), "x").unwrap();
        let answer = yes();
        let r = update_impl(&root, ID, &one, true, &quiet, &answer, false);
        assert!(matches!(r, Err(WriteError::Live(_))), "{r:?}");
        assert_eq!(
            answer.1.borrow().len(),
            0,
            "asked about a write that cannot happen"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TASK_MD);
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    /// RG-26: a conversation folder that is a git repository (the CLI root's
    /// are, and Antigravity commits them as "jetski Snapshot") is named in the
    /// question; one that is not, is not.
    #[test]
    fn a_snapshotted_conversation_says_so_in_the_question() {
        let root = root();
        let one = [set("Handle DST", Status::Completed)];
        let no = Answer(false, RefCell::new(Vec::new()));
        let r = update_impl(&root, ID, &one, true, &quiet, &no, false);
        assert!(matches!(r, Err(WriteError::Declined)), "{r:?}");
        assert!(
            !no.1.borrow()[0].lines.iter().any(|l| l.contains("jetski")),
            "{:?}",
            no.1.borrow()[0]
        );
        std::fs::create_dir_all(root.join("brain").join(ID).join(".git")).unwrap();
        let no = Answer(false, RefCell::new(Vec::new()));
        let r = update_impl(&root, ID, &one, true, &quiet, &no, false);
        assert!(matches!(r, Err(WriteError::Declined)), "{r:?}");
        let asked = no.1.borrow();
        assert!(
            asked[0].lines.iter().any(|l| l == SNAPSHOT_WARNING),
            "{:?}",
            asked[0]
        );
        assert!(SNAPSHOT_WARNING.contains("jetski Snapshot"));
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn an_unknown_ambiguous_or_boxless_update_is_refused() {
        let twice = "- [ ] a\n- [ ] a\n- ~~gone~~\n- [ ] blocked one (blocked: x)\n";
        for (u, why) in [
            (set("missing", Status::Completed), "no task named"),
            (set("a", Status::Completed), "2 tasks"),
            (set("gone", Status::Completed), "crossed out"),
            (set("blocked one", Status::Blocked), "no box for"),
            // Pending would still read as blocked: refused, not half-done.
            (set("blocked one", Status::Pending), "would read as"),
        ] {
            match apply_updates(twice, std::slice::from_ref(&u)) {
                Err(WriteError::Refused(w)) => assert!(w.contains(why), "{w}"),
                other => panic!("{u:?}: {other:?}"),
            }
        }
        let (text, n) = apply_updates(twice, &[set("blocked one", Status::Completed)]).unwrap();
        assert_eq!(n, 1);
        assert!(text.ends_with("- [x] blocked one (blocked: x)\n"), "{text}");
    }

    /// Antigravity (or the user) editing task.md while the question is on
    /// screen. Replacing the re-read with `if false` kept the suite green
    /// (RG-21 review of 10 Oct 2026); the write would then replace their edit
    /// with a version made from the text before it.
    struct EditsWhileAsking(PathBuf);

    impl Confirm for EditsWhileAsking {
        fn confirm(&self, _: &Question) -> bool {
            let mut text = std::fs::read_to_string(&self.0).unwrap();
            text.push_str("- [ ] Added by the agent meanwhile\n");
            std::fs::write(&self.0, text).unwrap();
            true
        }
    }

    #[test]
    fn a_task_md_that_changes_while_asking_is_not_written() {
        let root = root();
        let path = root.join("brain").join(ID).join("task.md");
        let r = update_impl(
            &root,
            ID,
            &[set("Handle DST", Status::Completed)],
            true,
            &quiet,
            &EditsWhileAsking(path.clone()),
            false,
        );
        assert!(
            matches!(r, Err(WriteError::Refused(ref w)) if w.contains("changed while the question")),
            "{r:?}"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            format!("{TASK_MD}- [ ] Added by the agent meanwhile\n")
        );
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }
}
