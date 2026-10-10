//! RG-14 and RG-15 end to end, over the synthetic tree in
//! `tests/fixtures/sessions/antigravity/` — never the user's `~/.gemini`.
//!
//! The fixture is copied into a scratch home first: the tests set file times
//! (a listing is ordered by them and the lag notice is computed from them, and
//! a git checkout leaves them arbitrary), take permissions away from a
//! database, and plant a credential next to the conversations. None of that
//! may touch the tree in the repository.
//!
//! The fixture's three conversations, each standing for a case the brief
//! measured:
//!
//! * `7f3c2a10…` — the IDE shape: a database, a `transcript_full.jsonl` whose
//!   steps are out of order, repeated (step 2 RUNNING then DONE) and torn (step
//!   4 was cut mid-line), a nested `task.md`, and both artifacts with metadata.
//! * `9b1e4d20…` — a database and nothing else (the brain was never written).
//! * `c4d5e6f7…` — no database, only the fallback `transcript.jsonl`, its last
//!   step still RUNNING.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tacet_tools::sessions::antigravity::{AntigravitySource, TranscriptFile, roots_under};
use tacet_tools::sessions::secrets::Refusal;
use tacet_tools::sessions::{
    AgentKind, EntryKind, Role, SessionError, SessionSource, Status, model::parse_iso_utc,
};

const A: &str = "7f3c2a10-5b6e-4c1d-9a8f-0e1d2c3b4a01";
const B: &str = "9b1e4d20-3c4f-4a5b-8c6d-7e8f9a0b1c02";
const C: &str = "c4d5e6f7-1a2b-4c3d-8e9f-0a1b2c3d4e03";

/// The credential the IDE keeps beside the conversations. Synthetic; if it
/// shows up in anything a reader returns, the reader opened `state.vscdb`.
const TOKEN: &str = "ya29.SYNTHETIC-oauthToken-must-never-be-read";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions/antigravity")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A scratch home holding a copy of the fixture's IDE root at
/// `<home>/.gemini/antigravity-ide`, and that root.
fn scratch(name: &str) -> (PathBuf, PathBuf) {
    let home = std::env::temp_dir().join(format!(
        "tacet-antigravity-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&home);
    let root = roots_under(&home)[0].clone();
    copy_tree(&fixture().join("antigravity-ide"), &root);
    (home, root)
}

fn epoch(iso: &str) -> i64 {
    parse_iso_utc(iso).unwrap()
}

fn touch(path: &Path, at: i64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(UNIX_EPOCH + Duration::from_secs(at as u64))
        .unwrap();
}

fn transcript(root: &Path, id: &str, name: &str) -> PathBuf {
    root.join("brain")
        .join(id)
        .join(".system_generated/logs")
        .join(name)
}

fn db(root: &Path, id: &str) -> PathBuf {
    root.join("conversations").join(format!("{id}.db"))
}

/// The times the tests reason with: the log of A last written at T, its
/// database 75 s later (the IDE's measured >60 s lag), B's database an hour
/// earlier, C's log a day earlier.
const T: &str = "2026-10-09T09:03:00Z";

fn set_times(root: &Path) {
    let t = epoch(T);
    touch(&transcript(root, A, "transcript_full.jsonl"), t);
    touch(&db(root, A), t + 75);
    touch(&db(root, B), t - 3600);
    touch(&transcript(root, C, "transcript.jsonl"), t - 86_400);
}

#[test]
fn a_listing_names_every_conversation_newest_first_with_its_first_request() {
    let (home, root) = scratch("list");
    set_times(&root);
    let list = AntigravitySource.discover(&root);
    let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, [A, B, C], "{list:#?}");

    let a = &list[0];
    assert_eq!(a.agent, AgentKind::Antigravity);
    // Step 0's request, as one line, even though it is not the first USER_INPUT
    // line a reader that ignored step order would meet in a reshuffled file.
    assert_eq!(
        a.title,
        "Fix the midnight rollover bug in the scheduler and add a test for it."
    );
    assert_eq!(
        a.cwd.as_deref(),
        Some(Path::new("/Users/dev/src/rollover app"))
    );
    assert_eq!(a.started, Some(epoch("2026-10-09T09:00:00Z")));
    // The database's time, not the log's: the log trails it.
    assert_eq!(a.updated, Some(epoch(T) + 75));
    assert!(!a.live, "no -wal and the last step is DONE");
    assert!(a.path.ends_with("transcript_full.jsonl"));

    let b = &list[1];
    assert_eq!(b.title, "", "a database is never opened for a title");
    assert_eq!(b.updated, Some(epoch(T) - 3600));
    assert_eq!(b.path, db(&root, B));

    let c = &list[2];
    assert_eq!(c.title, "Summarise the open pull requests");
    assert!(c.live, "its last step is still RUNNING");
    assert!(c.path.ends_with("transcript.jsonl"));
    let _ = std::fs::remove_dir_all(&home);
}

/// SQLite in WAL mode writes to `<id>.db-wal` and reaches `<id>.db` only at a
/// checkpoint, and keeps the `-wal` only while a connection is open.
#[test]
fn the_write_ahead_log_dates_the_conversation_and_marks_it_open() {
    let (home, root) = scratch("wal");
    set_times(&root);
    let wal = root.join("conversations").join(format!("{A}.db-wal"));
    std::fs::write(&wal, "synthetic").unwrap();
    touch(&wal, epoch(T) + 500);
    let a = AntigravitySource
        .discover(&root)
        .into_iter()
        .find(|s| s.id == A)
        .unwrap();
    assert_eq!(a.updated, Some(epoch(T) + 500));
    assert!(a.live);
    let read = AntigravitySource.read_full(&root, A).unwrap();
    assert!(read.session.live);
    assert_eq!(read.lag_seconds, Some(500));
    let _ = std::fs::remove_dir_all(&home);
}

/// The listing takes a database's time from its directory entry alone: with
/// every permission taken away, the conversation is still listed.
#[cfg(unix)]
#[test]
fn a_database_is_listed_without_being_opened() {
    use std::os::unix::fs::PermissionsExt;
    let (home, root) = scratch("unreadable");
    set_times(&root);
    for id in [A, B] {
        std::fs::set_permissions(db(&root, id), std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    let list = AntigravitySource.discover(&root);
    assert_eq!(list.len(), 3, "{list:#?}");
    assert_eq!(list[1].updated, Some(epoch(T) - 3600));
    let b = AntigravitySource.read_full(&root, B).unwrap();
    assert_eq!(b.transcript, TranscriptFile::Missing);
    for id in [A, B] {
        std::fs::set_permissions(db(&root, id), std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_read_dedupes_by_step_keeps_the_last_state_and_says_the_log_trails() {
    let (home, root) = scratch("read");
    set_times(&root);
    let read = AntigravitySource.read_full(&root, A).unwrap();
    let s = &read.session;
    assert_eq!(read.transcript, TranscriptFile::Full);
    assert_eq!(read.skipped_lines, 1, "step 4 was torn mid-line");
    assert_eq!(read.lag_seconds, Some(75));

    let shape: Vec<(Role, EntryKind, Option<&str>)> = s
        .entries
        .iter()
        .map(|e| (e.role, e.kind, e.tool.as_deref()))
        .collect();
    assert_eq!(
        shape,
        [
            (Role::User, EntryKind::Text, None),
            (Role::Assistant, EntryKind::Thinking, None),
            (Role::Assistant, EntryKind::Text, None),
            (Role::Assistant, EntryKind::ToolCall, Some("view_file")),
            (Role::Tool, EntryKind::ToolResult, Some("VIEW_FILE")),
            (Role::Tool, EntryKind::ToolResult, Some("CODE_ACTION")),
            (
                Role::Assistant,
                EntryKind::ToolCall,
                Some("replace_file_content")
            ),
            (Role::User, EntryKind::Text, None),
            (Role::Tool, EntryKind::ToolResult, Some("RUN_COMMAND")),
        ]
    );
    // Step 2 was logged RUNNING (no content) and then DONE: the DONE line wins.
    assert_eq!(
        s.entries[4].text,
        "fn next_run(now: Time) -> Time { now + DAY }"
    );
    assert_eq!(s.entries[8].text, "test result: ok. 12 passed; 0 failed");
    assert_eq!(s.started, Some(epoch("2026-10-09T09:00:00Z")));
    assert_eq!(s.updated, Some(epoch(T) + 75));
    assert!(!s.live);
    assert_eq!(s.version, None, "nothing in brain/ records the app version");

    assert!(
        read.notices.iter().any(|n| n.contains("75 s after")),
        "{:?}",
        read.notices
    );
    assert!(read.notices.iter().any(|n| n.contains("1 line(s)")));

    // A caller that only has the trait still sees the notices, as Tacet's.
    let via_trait = AntigravitySource.read(&root, A).unwrap();
    assert_eq!(
        via_trait.entries.len(),
        s.entries.len() + read.notices.len()
    );
    let tail = via_trait.entries.last().unwrap();
    assert_eq!((tail.role, tail.kind), (Role::System, EntryKind::Text));
    assert!(tail.text.starts_with("[Tacet] "));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_log_newer_than_its_database_carries_no_lag_notice() {
    let (home, root) = scratch("nolag");
    set_times(&root);
    touch(&db(&root, A), epoch(T) - 10);
    let read = AntigravitySource.read_full(&root, A).unwrap();
    assert_eq!(read.lag_seconds, None);
    assert!(
        !read.notices.iter().any(|n| n.contains("s after")),
        "{:?}",
        read.notices
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_plan_and_the_artifacts_are_read_beside_the_log() {
    let (home, root) = scratch("artifacts");
    set_times(&root);
    let read = AntigravitySource.read_full(&root, A).unwrap();
    let tree: Vec<(usize, Option<usize>, Status, &str)> = read
        .tasks
        .iter()
        .map(|t| (t.depth, t.parent, t.status, t.title.as_str()))
        .collect();
    assert_eq!(
        tree,
        [
            (0, None, Status::Completed, "Reproduce the bug"),
            (1, Some(0), Status::Completed, "Read the scheduler"),
            (1, Some(0), Status::Completed, "Write a failing test"),
            (0, None, Status::InProgress, "Fix the day boundary"),
            (1, Some(3), Status::Pending, "Handle DST"),
            (0, None, Status::Blocked, "Update the changelog"),
        ]
    );
    let plan: Vec<(&str, Status)> = read
        .session
        .plan
        .iter()
        .map(|p| (p.title.as_str(), p.status))
        .collect();
    assert_eq!(plan.len(), 6);
    assert_eq!(plan[3], ("Fix the day boundary", Status::InProgress));

    let names: Vec<&str> = read.artifacts.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["implementation_plan.md", "walkthrough.md"]);
    let plan_md = &read.artifacts[0];
    assert!(plan_md.text.starts_with("# Implementation plan"));
    assert_eq!(
        plan_md.summary.as_deref(),
        Some("Wrap next_run at midnight and add a regression test.")
    );
    assert_eq!(plan_md.updated, Some(epoch("2026-10-09T09:00:50Z")));
    assert_eq!(
        (plan_md.user_facing, plan_md.request_feedback),
        (Some(true), Some(true))
    );
    assert_eq!(read.artifacts[1].request_feedback, None);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_fallback_log_is_read_and_said_to_be_the_fallback() {
    let (home, root) = scratch("fallback");
    set_times(&root);
    let read = AntigravitySource.read_full(&root, C).unwrap();
    assert_eq!(read.transcript, TranscriptFile::Fallback);
    assert_eq!(read.session.title, "Summarise the open pull requests");
    assert_eq!(read.session.entries.len(), 2);
    assert!(read.session.live);
    assert_eq!(read.lag_seconds, None, "no database to trail");
    assert!(
        read.notices.iter().any(|n| n.contains("transcript.jsonl")),
        "{:?}",
        read.notices
    );

    let b = AntigravitySource.read_full(&root, B).unwrap();
    assert_eq!(b.transcript, TranscriptFile::Missing);
    assert!(b.session.entries.is_empty());
    assert!(b.notices.iter().any(|n| n.contains("does not parse")));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn an_id_is_checked_before_it_becomes_a_path() {
    let (home, root) = scratch("ids");
    assert!(matches!(
        AntigravitySource.read(&root, "../../.ssh"),
        Err(SessionError::BadId(_))
    ));
    assert!(matches!(
        AntigravitySource.read(&root, "00000000-0000-4000-8000-000000000000"),
        Err(SessionError::NotFound { .. })
    ));
    let _ = std::fs::remove_dir_all(&home);
}

/// The IDE's `state.vscdb` holds the user's `oauthToken`. A log that links
/// to it is refused — and refused, not skipped over to the fallback log, which
/// would show a session that looks complete while hiding why it is not.
#[cfg(unix)]
#[test]
fn a_log_that_links_to_the_token_store_is_refused_not_fallen_back_from() {
    let (home, root) = scratch("vscdb");
    set_times(&root);
    let vscdb = root.join("state.vscdb");
    std::fs::write(&vscdb, format!("{{\"oauthToken\":\"{TOKEN}\"}}\n")).unwrap();
    let full = transcript(&root, A, "transcript_full.jsonl");
    std::fs::remove_file(&full).unwrap();
    std::os::unix::fs::symlink(&vscdb, &full).unwrap();
    std::fs::write(
        transcript(&root, A, "transcript.jsonl"),
        "{\"step_index\":0,\"type\":\"USER_INPUT\",\"content\":\"decoy\"}\n",
    )
    .unwrap();

    match AntigravitySource.read_full(&root, A) {
        Err(SessionError::Refused(Refusal::Secret { .. })) => {}
        other => panic!("a log linking to state.vscdb was not refused: {other:?}"),
    }
    let list = AntigravitySource.discover(&root);
    assert!(list.iter().all(|s| s.id != A), "{list:#?}");
    assert!(
        !format!("{list:?}").contains(TOKEN),
        "the token reached a listing"
    );

    // The same for the plan: a task.md that links to the token store.
    let task = root.join("brain").join(C).join("task.md");
    std::os::unix::fs::symlink(&vscdb, &task).unwrap();
    assert!(matches!(
        AntigravitySource.read_full(&root, C),
        Err(SessionError::Refused(Refusal::Secret { .. }))
    ));
    let _ = std::fs::remove_dir_all(&home);
}

/// `state.vscdb` sits in the root a listing walks; it is never mistaken for a
/// conversation and never read for a title. The brain directory carrying its
/// name is the case only the DISCOVERY walk can stop: the log inside it is
/// called `transcript_full.jsonl`, a name the gate lets through (the gate
/// checks middle components only against `.ssh`-like directories), so if the
/// walk did not skip a credential-named entry itself, its first request — the
/// token — would be the listing's title.
#[test]
fn the_token_store_beside_the_conversations_is_never_a_session() {
    let (home, root) = scratch("beside");
    set_times(&root);
    std::fs::write(
        root.join("state.vscdb"),
        format!("{{\"oauthToken\":\"{TOKEN}\"}}\n"),
    )
    .unwrap();
    std::fs::write(
        root.join("conversations").join("state.vscdb"),
        format!("{{\"oauthToken\":\"{TOKEN}\"}}\n"),
    )
    .unwrap();
    let decoy = transcript(&root, "state.vscdb", "transcript_full.jsonl");
    std::fs::create_dir_all(decoy.parent().unwrap()).unwrap();
    std::fs::write(
        &decoy,
        format!("{{\"step_index\":0,\"type\":\"USER_INPUT\",\"content\":\"{TOKEN}\"}}\n"),
    )
    .unwrap();
    let list = AntigravitySource.discover(&root);
    assert_eq!(list.len(), 3);
    assert!(!format!("{list:?}").contains(TOKEN));
    for s in &list {
        let read = AntigravitySource.read_full(&root, &s.id).unwrap();
        assert!(!format!("{read:?}").contains(TOKEN), "{}", s.id);
    }
    let _ = std::fs::remove_dir_all(&home);
}
