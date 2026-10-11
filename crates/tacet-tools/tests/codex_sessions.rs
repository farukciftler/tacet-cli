//! RG-11..RG-13 against a Codex home on disk: the fixture tree under
//! `tests/fixtures/sessions/codex/home`, copied into a temp directory so a test
//! can add the files that must never be committed (an `auth.json`, a writer
//! lock) and take files away.
//!
//! THE FIXTURES ARE WRITTEN FROM SOURCE, NOT COLLECTED. Codex is not installed
//! on the machine they were written on; every line follows openai/codex @
//! a06545b (`rollout/src/recorder.rs`, `history/src/rollout_payload.rs`,
//! `rollout/src/policy.rs`). Replacing them with rollouts from a real install
//! is RG-28's job, and the version gate warns on every Codex read until then.
//!
//! No test here reads the user's `~/.codex`: every root is a temp directory.
//! Which environment variables name the root is tested in `codex.rs` through
//! `roots_from_env`, with a table instead of the process environment.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use tacet_tools::sessions::codex::{self, CodexSource, HistoryMode};
use tacet_tools::sessions::model::parse_iso_utc;
use tacet_tools::sessions::secrets::Refusal;
use tacet_tools::sessions::{EntryKind, Role, SessionError, SessionSource, Status};

const A: &str = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a01";
const B: &str = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a02";
const C: &str = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a03";
const D: &str = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a04";
const E: &str = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a05";

const B_FILE: &str =
    "sessions/2026/10/02/rollout-2026-10-02T10-00-00-0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a02.jsonl";
const D_FILE: &str = "sessions/2026/09/30/rollout-2026-09-30T07-00-00-0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a04.jsonl.zst";

/// The API key a reader must never return. If this string turns up in any
/// listing or transcript, the gate was walked around.
const KEY: &str = "sk-fixture-codex-key-never-read";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions/codex/home")
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

/// A fresh Codex home: the fixtures, an `auth.json` at the root where Codex
/// keeps it and a second one inside the session tree, and a writer lock for
/// session B.
fn home(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tacet-codex-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    copy_tree(&fixture(), &dir);
    let auth = format!("{{\"OPENAI_API_KEY\":\"{KEY}\"}}\n");
    std::fs::write(dir.join("auth.json"), &auth).unwrap();
    std::fs::write(dir.join("sessions/2026/10/01/auth.json"), &auth).unwrap();
    std::fs::create_dir_all(dir.join(codex::LOCKS_DIR)).unwrap();
    std::fs::write(dir.join(codex::LOCKS_DIR).join(format!("{B}.lock")), "").unwrap();
    dir
}

fn texts(s: &tacet_tools::sessions::AgentSession) -> Vec<(Role, EntryKind, &str)> {
    s.entries
        .iter()
        .map(|e| (e.role, e.kind, e.text.as_str()))
        .collect()
}

/// RG-11: both session trees are walked, one row per rollout, titled by the
/// index (last rename wins) or the first TYPED message, a compressed rollout
/// listed with how to read it — and nothing that is not a rollout.
#[test]
fn the_listing_finds_every_rollout_and_nothing_else() {
    let root = home("list");
    let list = CodexSource.discover(&root);
    let row = |id: &str| {
        list.iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("{id} not listed: {list:#?}"))
    };

    let ids: std::collections::BTreeSet<&str> = list.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, [A, B, C, D, E].into_iter().collect());

    let a = row(A);
    assert_eq!(
        a.title, "Calendar rollover fix",
        "the LAST index line names it"
    );
    assert_eq!(a.version.as_deref(), Some("0.46.0"));
    assert_eq!(a.cwd, Some(PathBuf::from("/work/tacet")));
    assert_eq!(a.started, parse_iso_utc("2026-10-01T09:15:00.100Z"));
    assert_eq!(a.updated, parse_iso_utc("2026-10-01T09:15:30.002Z"));
    assert!(!a.live);

    // Not the environment block that is the first user-role message.
    assert_eq!(row(B).title, "Rename the crate to tacet-io");
    assert!(row(B).live, "B has a writer lock");
    assert_eq!(row(B).version.as_deref(), Some("0.47.0"));
    // A fork is titled by its own first question, not its parent's.
    assert_eq!(row(C).title, "Now update the README too");
    assert_eq!(row(E).title, "An old archived question");
    assert!(row(E).path.starts_with(root.join(codex::ARCHIVED_DIR)));

    let d = row(D);
    assert_eq!(d.title, codex::COMPRESSED_TITLE);
    assert!(d.path.to_string_lossy().ends_with(".jsonl.zst"));

    // Newest first among the rollouts whose clock is in the file.
    let order: Vec<&str> = list
        .iter()
        .map(|s| s.id.as_str())
        .filter(|id| *id != D)
        .collect();
    assert_eq!(order, [C, B, A, E]);

    let shown = format!("{list:?}");
    assert!(!shown.contains(KEY), "{shown}");
    assert!(!shown.contains("auth.json") && !shown.contains("notes.jsonl"));
    let _ = std::fs::remove_dir_all(&root);
}

/// RG-12 + RG-13 on a legacy rollout: the model stream once (the events that
/// repeat it are not shown twice), Codex's own context as system text, a
/// malformed line skipped and numbered, `cli_version` kept, and the plan the
/// LAST accepted `update_plan` set.
#[test]
fn a_legacy_rollout_reads_with_its_plan() {
    let root = home("legacy");
    let s = CodexSource.read(&root, A).unwrap();
    assert_eq!(s.title, "Calendar rollover fix");
    assert_eq!(s.version.as_deref(), Some("0.46.0"));
    assert!(!s.live);
    assert_eq!(
        texts(&s),
        [
            (
                Role::System,
                EntryKind::Text,
                "<permissions instructions>workspace-write</permissions instructions>"
            ),
            (
                Role::System,
                EntryKind::Text,
                "<environment_context>\n  <cwd>/work/tacet</cwd>\n  <shell>zsh</shell>\n</environment_context>"
            ),
            (
                Role::User,
                EntryKind::Text,
                "Fix the date rollover bug in calendar.rs"
            ),
            (
                Role::Assistant,
                EntryKind::Thinking,
                "Looking at how calendar.rs adds days"
            ),
            (
                Role::Assistant,
                EntryKind::ToolCall,
                "{\"explanation\":\"Start with a failing test\",\"plan\":[{\"step\":\"Reproduce the rollover\",\"status\":\"in_progress\"},{\"step\":\"Fix add_days\",\"status\":\"pending\"}]}"
            ),
            (Role::Tool, EntryKind::ToolResult, "Plan updated"),
            (
                Role::Assistant,
                EntryKind::ToolCall,
                "{\"command\":[\"cargo\",\"test\",\"-p\",\"tacet-tools\"],\"workdir\":\"/work/tacet\"}"
            ),
            (
                Role::Tool,
                EntryKind::ToolResult,
                "test result: ok. 412 passed"
            ),
            (
                Role::Assistant,
                EntryKind::ToolCall,
                "{\"plan\":[{\"step\":\"Reproduce the rollover\",\"status\":\"completed\"},{\"step\":\"Fix add_days\",\"status\":\"completed\"},{\"step\":\"Run the whole suite\",\"status\":\"in_progress\"}]}"
            ),
            (Role::Tool, EntryKind::ToolResult, "Plan updated"),
            (
                Role::Assistant,
                EntryKind::Text,
                "Fixed the rollover in add_days; the suite passes."
            ),
        ]
    );
    assert_eq!(s.entries[7].tool.as_deref(), Some("shell"));
    let plan: Vec<(&str, Status)> = s
        .plan
        .iter()
        .map(|p| (p.title.as_str(), p.status))
        .collect();
    assert_eq!(
        plan,
        [
            ("Reproduce the rollover", Status::Completed),
            ("Fix add_days", Status::Completed),
            ("Run the whole suite", Status::InProgress),
        ]
    );

    let file = CodexSource.read_rollout(&root, A).unwrap();
    assert_eq!(file.rollout.history_mode, HistoryMode::Legacy);
    assert_eq!(file.rollout.skipped, [10], "the truncated line, by number");
    assert!(!file.archived);
    let _ = std::fs::remove_dir_all(&root);
}

/// RG-12, paginated: the line without an ordinal is skipped, the events that
/// paginated mode writes instead of `user_message` are not shown twice, and
/// the last ordinal is reported for whoever appends next.
#[test]
fn a_paginated_rollout_reads_by_ordinal() {
    let root = home("paginated");
    let file = CodexSource.read_rollout(&root, B).unwrap();
    let r = &file.rollout;
    assert_eq!(r.history_mode, HistoryMode::Paginated);
    assert_eq!(r.skipped, [5]);
    assert_eq!(r.last_ordinal, Some(5));
    assert_eq!(r.ordinal_gaps, 0);
    let s = CodexSource.read(&root, B).unwrap();
    assert!(s.live);
    assert_eq!(
        s.entries
            .iter()
            .map(|e| (e.role, e.text.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                Role::System,
                "<environment_context>\n  <cwd>/work/other</cwd>\n</environment_context>"
            ),
            (Role::User, "Rename the crate to tacet-io"),
            (Role::Assistant, "Renamed it in Cargo.toml and lib.rs."),
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A paginated fork does not copy its parent's lines: `history_base` points
/// at them. Read alone, the fork would start mid-conversation.
#[test]
fn a_paginated_fork_reads_its_parents_prefix() {
    let root = home("fork");
    let s = CodexSource.read(&root, C).unwrap();
    assert_eq!(s.title, "Now update the README too");
    assert_eq!(
        s.entries
            .iter()
            .map(|e| (e.role, e.text.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                Role::System,
                "<environment_context>\n  <cwd>/work/other</cwd>\n</environment_context>"
            ),
            (Role::User, "Rename the crate to tacet-io"),
            (Role::User, "Now update the README too"),
            (Role::Assistant, "README updated."),
        ],
        "the parent's lines below ordinal 4, then the fork's own"
    );
    let file = CodexSource.read_rollout(&root, C).unwrap();
    assert_eq!(file.rollout.missing_prefix, None);
    assert_eq!(file.rollout.last_ordinal, Some(6));
    assert_eq!(
        file.rollout
            .history_base
            .as_ref()
            .map(|b| b.rollout_id.as_str()),
        Some(B)
    );

    // The parent gone: the fork is still read, and says what is missing.
    std::fs::remove_file(root.join(B_FILE)).unwrap();
    let file = CodexSource.read_rollout(&root, C).unwrap();
    assert!(
        file.rollout
            .missing_prefix
            .as_deref()
            .is_some_and(|m| m.contains(B)),
        "{:?}",
        file.rollout.missing_prefix
    );
    assert_eq!(file.rollout.entries.len(), 2);
    let _ = std::fs::remove_dir_all(&root);
}

/// RG-11: a `.jsonl.zst` is refused with a message that says how to get it
/// read — and once the user has done that, next to the original, the plain
/// file is the one read and listed.
#[test]
fn a_compressed_rollout_is_refused_until_it_is_decompressed() {
    let root = home("zst");
    match CodexSource.read(&root, D) {
        Err(SessionError::Refused(r @ Refusal::Compressed { .. })) => {
            let msg = r.to_string();
            assert!(msg.contains("zstd -d"), "{msg}");
            assert!(msg.contains(D), "{msg}");
        }
        other => panic!("a .zst rollout was not refused as compressed: {other:?}"),
    }

    let decompressed = "{\"timestamp\":\"2026-09-30T07:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a04\",\"cli_version\":\"0.45.0\"}}\n\
         {\"timestamp\":\"2026-09-30T07:00:01.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"decompressed question\"}}\n";
    let only_row = |root: &Path| {
        let rows: Vec<_> = CodexSource
            .discover(root)
            .into_iter()
            .filter(|r| r.id == D)
            .collect();
        assert_eq!(rows.len(), 1, "one row per id: {rows:#?}");
        rows[0].path.clone()
    };

    // Next to the original, as the refusal says.
    let plain = root.join(D_FILE.trim_end_matches(".zst"));
    std::fs::write(&plain, decompressed).unwrap();
    assert_eq!(
        CodexSource.read(&root, D).unwrap().title,
        "decompressed question"
    );
    assert_eq!(only_row(&root), plain);

    // In the OTHER tree: the walk meets the `.zst` first, and the readable
    // copy must still win. (Within one directory the plain name sorts first,
    // so only this case exercises the rule.)
    std::fs::remove_file(&plain).unwrap();
    let archived = root.join(codex::ARCHIVED_DIR).join(
        Path::new(D_FILE)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .trim_end_matches(".zst"),
    );
    std::fs::write(&archived, decompressed).unwrap();
    assert_eq!(
        CodexSource.read(&root, D).unwrap().title,
        "decompressed question"
    );
    assert_eq!(only_row(&root), archived);
    let _ = std::fs::remove_dir_all(&root);
}

/// A listing reads a head and a tail, not the file. A first line longer than
/// the head (Codex puts the whole base instructions into `session_meta`) is
/// read in full rather than dropped, and the last activity comes from the
/// tail when the head stops short of it.
#[test]
fn a_rollout_longer_than_the_head_is_listed_from_its_head_and_tail() {
    let root = home("head");
    let dir = root.join("sessions/2026/10/04");
    std::fs::create_dir_all(&dir).unwrap();
    let pad = "x".repeat(codex::HEAD_BYTES as usize + 1024);

    let long_meta = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a0b";
    std::fs::write(
        dir.join(format!("rollout-2026-10-04T09-00-00-{long_meta}.jsonl")),
        format!(
            "{{\"timestamp\":\"2026-10-04T09:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{long_meta}\",\"cli_version\":\"0.47.0\",\"base_instructions\":{{\"text\":\"{pad}\"}}}}}}\n\
             {{\"timestamp\":\"2026-10-04T09:00:01.000Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"after a long meta\"}}}}\n"
        ),
    )
    .unwrap();

    let long_middle = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a0c";
    std::fs::write(
        dir.join(format!("rollout-2026-10-04T10-00-00-{long_middle}.jsonl")),
        format!(
            "{{\"timestamp\":\"2026-10-04T10:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{long_middle}\",\"cli_version\":\"0.47.0\"}}}}\n\
             {{\"timestamp\":\"2026-10-04T10:00:01.000Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"a long tool output follows\"}}}}\n\
             {{\"timestamp\":\"2026-10-04T10:00:02.000Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call_output\",\"call_id\":\"k\",\"output\":\"{pad}\"}}}}\n\
             {{\"timestamp\":\"2026-10-04T11:30:00.000Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"done much later\"}}}}\n"
        ),
    )
    .unwrap();

    let list = CodexSource.discover(&root);
    let row = |id: &str| {
        list.iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("{id} not listed"))
    };
    assert_eq!(row(long_meta).title, "after a long meta");
    assert_eq!(row(long_middle).title, "a long tool output follows");
    assert_eq!(
        row(long_middle).updated,
        parse_iso_utc("2026-10-04T11:30:00Z"),
        "the last activity is in the tail"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Liveness is the writer lock's presence, and nothing else.
#[test]
fn a_session_is_live_while_its_writer_lock_exists() {
    let root = home("live");
    assert!(CodexSource.read(&root, B).unwrap().live);
    assert!(!CodexSource.read(&root, A).unwrap().live);
    std::fs::remove_file(root.join(codex::LOCKS_DIR).join(format!("{B}.lock"))).unwrap();
    assert!(!CodexSource.read(&root, B).unwrap().live);
    assert!(!codex::is_live(&root, "../auth"));
    let _ = std::fs::remove_dir_all(&root);
}

/// Archived sessions are read like active ones, and say where they came from
/// (a write path must refuse them). With no `response_item`, the events are
/// the transcript.
#[test]
fn an_archived_session_reads_and_says_it_is_archived() {
    let root = home("archived");
    let file = CodexSource.read_rollout(&root, E).unwrap();
    assert!(file.archived);
    let s = CodexSource.read(&root, E).unwrap();
    assert_eq!(s.version.as_deref(), Some("0.40.0"));
    assert_eq!(
        s.entries
            .iter()
            .map(|e| (e.role, e.text.as_str()))
            .collect::<Vec<_>>(),
        [
            (Role::User, "An old archived question"),
            (Role::Assistant, "An old answer"),
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A rollout name in front of a credential: listed by nobody, read by
/// nobody. The reader never thinks about credentials; the gate does.
#[cfg(unix)]
#[test]
fn a_rollout_that_links_to_auth_json_is_refused() {
    let root = home("link");
    let id = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a09";
    std::os::unix::fs::symlink(
        root.join("auth.json"),
        root.join(format!(
            "sessions/2026/10/01/rollout-2026-10-01T10-00-00-{id}.jsonl"
        )),
    )
    .unwrap();
    assert!(!CodexSource.discover(&root).iter().any(|s| s.id == id));
    match CodexSource.read(&root, id) {
        Err(SessionError::Refused(Refusal::Secret { .. })) => {}
        other => panic!("a link to auth.json was read: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A file with a rollout's name and nothing of a rollout inside is not a
/// session: not listed, and a read says so rather than returning nothing.
#[test]
fn a_rollout_with_no_rollout_line_is_not_a_session() {
    let root = home("junk");
    let id = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a0a";
    std::fs::write(
        root.join(format!(
            "sessions/2026/10/01/rollout-2026-10-01T11-00-00-{id}.jsonl"
        )),
        "garbage\n{\"no\":\"type\"}\n",
    )
    .unwrap();
    assert!(!CodexSource.discover(&root).iter().any(|s| s.id == id));
    match CodexSource.read(&root, id) {
        Err(SessionError::Format { line: Some(1), .. }) => {}
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        CodexSource.read(&root, "../../auth"),
        Err(SessionError::BadId(_))
    ));
    let _ = std::fs::remove_dir_all(&root);
}
