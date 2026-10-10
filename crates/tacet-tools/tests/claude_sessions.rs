//! The Claude Code session reader (RG-08 discovery, RG-09 chain, RG-10 plan)
//! over the synthetic corpus in `tests/fixtures/sessions/claude/`.
//!
//! THE CORPUS IS MADE UP. Every line was written for these tests; none was
//! copied from a real transcript (a real one holds the user's code, prompts and
//! paths). Each file stands for one shape the 9 Oct 2026 measurement found on
//! disk: re-appended uuids, timestamps going backwards, a rewind, parallel tool
//! results on side branches, a compaction whose `logicalParentUuid` loops, a
//! parent in another file, and the three ways a plan is recorded.
//!
//! No test here reads the user's real config directory: every root is the
//! fixture directory or a temp copy of it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use tacet_tools::sessions::claude::{
    ClaudeSource, LIST_TAIL_BYTES, pid_alive, project_dir_matches, project_dir_name,
};
use tacet_tools::sessions::{
    AgentKind, EntryKind, PlanItem, Role, SessionError, SessionSource, Status,
};

const BASIC: &str = "11111111-0000-4000-8000-111111111111";
const BRANCHES: &str = "22222222-0000-4000-8000-222222222222";
const COMPACTED: &str = "33333333-0000-4000-8000-333333333333";
const ORPHAN: &str = "44444444-0000-4000-8000-444444444444";
const LOOPED: &str = "55555555-0000-4000-8000-555555555555";
const TASKS: &str = "66666666-0000-4000-8000-666666666666";
const TODOS: &str = "77777777-0000-4000-8000-777777777777";
const STORE: &str = "88888888-0000-4000-8000-888888888888";
const FUTURE: &str = "99999999-0000-4000-8000-999999999999";
const OTHER: &str = "abcdefab-0000-4000-8000-abcdefabcdef";

/// The fixture config directory. Joined from the agent's slug rather than
/// written out: a string literal whose last path component is an agent CLI's
/// name reads to the network-monopoly scan as a program being run.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sessions")
        .join(AgentKind::ClaudeCode.slug())
}

fn dead(_: u32) -> bool {
    false
}

fn only_4242(pid: u32) -> bool {
    pid == 4242
}

fn source() -> ClaudeSource {
    ClaudeSource::with_probe(dead)
}

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("tacet-claude-reader-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A project directory under a temp root, with one transcript in it.
fn one_session(root: &Path, project: &str, id: &str, lines: &[String]) -> PathBuf {
    let dir = root.join("projects").join(project);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{id}.jsonl"));
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    path
}

fn entry_line(uuid: &str, parent: Option<&str>, kind: &str, content: serde_json::Value) -> String {
    let message = if kind == "assistant" {
        serde_json::json!({"id": format!("msg_{uuid}"), "role": "assistant", "content": content})
    } else {
        serde_json::json!({"role": "user", "content": content})
    };
    serde_json::json!({
        "type": kind,
        "uuid": uuid,
        "parentUuid": parent,
        "isSidechain": false,
        "sessionId": "s",
        "cwd": "/w",
        "version": "2.1.285",
        "timestamp": "2026-10-09T10:00:00.000Z",
        "message": message,
    })
    .to_string()
}

/// `(role, kind, tool, text)` per entry, for comparing a whole context at once.
fn shape(entries: &[tacet_tools::sessions::Entry]) -> Vec<(Role, EntryKind, Option<&str>, &str)> {
    entries
        .iter()
        .map(|e| (e.role, e.kind, e.tool.as_deref(), e.text.as_str()))
        .collect()
}

fn texts(entries: &[tacet_tools::sessions::Entry]) -> Vec<&str> {
    entries.iter().map(|e| e.text.as_str()).collect()
}

// ---------------------------------------------------------------- RG-08 --

/// Every top-level `<id>.jsonl` under `projects/*` is a session; a subagent's
/// transcript one level down is not.
#[test]
fn discovery_lists_every_top_level_transcript_and_no_subagent() {
    let list = source().discover(&fixtures());
    let ids: BTreeSet<&str> = list.iter().map(|s| s.id.as_str()).collect();
    let want: BTreeSet<&str> = [
        BASIC, BRANCHES, COMPACTED, ORPHAN, LOOPED, TASKS, TODOS, STORE, FUTURE, OTHER,
    ]
    .into_iter()
    .collect();
    assert_eq!(ids, want);
    assert!(list.iter().all(|s| s.agent == AgentKind::ClaudeCode));
    // Newest first.
    let updated: Vec<_> = list.iter().map(|s| s.updated).collect();
    let mut sorted = updated.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(updated, sorted);
    let basic = list.iter().find(|s| s.id == BASIC).unwrap();
    assert_eq!(basic.title, "Rollover bug");
    assert_eq!(basic.cwd.as_deref(), Some(Path::new("/Users/dev/demo")));
    assert_eq!(basic.version.as_deref(), Some("2.1.285"));
    assert!(basic.path.ends_with(format!("{BASIC}.jsonl")));
}

/// A listing reads the last `LIST_TAIL_BYTES` of a transcript and no more: a
/// title written only at the head of a long file is not seen by the listing,
/// and is by a full read.
#[test]
fn a_listing_reads_only_the_tail_of_the_transcript() {
    let root = scratch("tail");
    let mut lines = vec![
        serde_json::json!({"type": "ai-title", "aiTitle": "Title at the head"}).to_string(),
        entry_line("t0", None, "user", serde_json::json!("the first prompt")),
    ];
    let filler = "x".repeat(1000);
    let mut parent = "t0".to_string();
    let mut n = 1;
    while lines.iter().map(|l| l.len() + 1).sum::<usize>() < 2 * LIST_TAIL_BYTES as usize {
        let uuid = format!("t{n}");
        lines.push(entry_line(
            &uuid,
            Some(&parent),
            "assistant",
            serde_json::json!([{"type": "text", "text": filler}]),
        ));
        parent = uuid;
        n += 1;
    }
    lines.push(
        serde_json::json!({"type": "last-prompt", "lastPrompt": "the prompt at the tail", "leafUuid": parent})
            .to_string(),
    );
    one_session(&root, "-w", "long", &lines);

    let list = source().discover(&root);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "the prompt at the tail");
    // The earliest timestamp in a tail is not when the session started.
    assert_eq!(list[0].started, None);

    let full = source().read_report(&root, "long").unwrap();
    assert_eq!(full.session.title, "Title at the head");
    assert!(full.session.started.is_some());
    let _ = std::fs::remove_dir_all(&root);
}

/// custom-title beats ai-title beats the first thing the user typed; a slash
/// command's wrapper and a system-injected (`isMeta`) line are not that.
#[test]
fn the_title_is_the_custom_one_then_the_ai_one_then_the_first_prompt() {
    let s = source();
    assert_eq!(s.read(&fixtures(), BASIC).unwrap().title, "Rollover bug");

    let root = scratch("titles");
    let meta = {
        let mut v: serde_json::Value = serde_json::from_str(&entry_line(
            "m1",
            Some("c1"),
            "user",
            serde_json::json!("Caveat: injected by the harness"),
        ))
        .unwrap();
        v["isMeta"] = true.into();
        v.to_string()
    };
    let lines = vec![
        entry_line(
            "c1",
            None,
            "user",
            serde_json::json!("<command-name>/clear</command-name>"),
        ),
        meta,
        entry_line(
            "p1",
            Some("m1"),
            "user",
            serde_json::json!("make the build green\nplease"),
        ),
    ];
    one_session(&root, "-w", "untitled", &lines);
    let mut with_ai = lines.clone();
    with_ai.push(serde_json::json!({"type": "ai-title", "aiTitle": "Green build"}).to_string());
    one_session(&root, "-w", "aititled", &with_ai);

    assert_eq!(
        s.read(&root, "untitled").unwrap().title,
        "make the build green"
    );
    assert_eq!(s.read(&root, "aititled").unwrap().title, "Green build");
    let listed: Vec<(String, String)> = s
        .discover(&root)
        .into_iter()
        .map(|x| (x.id, x.title))
        .collect();
    assert!(listed.contains(&("untitled".into(), "make the build green".into())));
    assert!(listed.contains(&("aititled".into(), "Green build".into())));
    let _ = std::fs::remove_dir_all(&root);
}

/// A session is live when a `sessions/<pid>.json` names it AND that pid is
/// running. A pid file whose process is gone is a crash, not a live session.
#[test]
fn a_session_is_live_only_while_its_process_runs() {
    let root = fixtures();
    let live = ClaudeSource::with_probe(only_4242);
    let map = live.live_sessions(&root);
    assert_eq!(map.get(BASIC), Some(&4242));
    // 4343.json names BRANCHES, but 4343 is not running.
    assert!(!map.contains_key(BRANCHES));
    let list = live.discover(&root);
    let flag = |id: &str| list.iter().find(|s| s.id == id).unwrap().live;
    assert!(flag(BASIC));
    assert!(!flag(BRANCHES));
    assert!(live.read(&root, BASIC).unwrap().live);
    assert!(!live.read(&root, BRANCHES).unwrap().live);

    let none = source();
    assert!(none.live_sessions(&root).is_empty());
    assert!(!none.read(&root, BASIC).unwrap().live);
}

/// The real probe: this process is running; a child that has been waited for
/// is not.
///
/// UNIX ONLY, because only there can the probe say "not running": elsewhere
/// `probe_pid` answers "alive" for every pid by design (liveness only stops a
/// write, so not knowing must not permit one), and this test, unconditional,
/// failed by construction on the windows-latest CI job — which also has no
/// `true` to spawn. The Windows expectation is the next test. On Linux this is
/// the run of the `/proc/<pid>` branch; it was written on macOS, where the
/// `/bin/ps` branch is what runs.
#[cfg(unix)]
#[test]
fn the_pid_probe_tells_a_running_process_from_a_reaped_one() {
    assert!(pid_alive(std::process::id()));
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    assert!(
        !pid_alive(pid),
        "pid {pid} was reaped and still reads as alive"
    );
    assert!(!pid_alive(0));
    assert!(!pid_alive(u32::MAX));
}

/// Where no probe exists the answer is "alive" for any pid that could exist —
/// a session there always reads as open and is never written to — and still
/// "not alive" for the two that cannot. NOT MEASURED on Windows: this states
/// what the code promises there, so a change to the fallback is a decision
/// someone makes, not a drift.
#[cfg(not(unix))]
#[test]
fn without_a_probe_every_possible_pid_reads_as_alive() {
    assert!(pid_alive(std::process::id()));
    let mut child = std::process::Command::new("cmd")
        .args(["/C", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    assert!(
        pid_alive(pid),
        "pid {pid} was reaped and the fallback said so"
    );
    assert!(!pid_alive(0));
    assert!(!pid_alive(u32::MAX));
}

/// The two tests above are split by platform, and the split is only visible
/// on the platform it protects: dropping `#[cfg(unix)]` stays green on every
/// Mac and Linux box and fails only on the Windows CI job (RG-08 review — the
/// test shipped unconditional). So the attributes are checked here, where any
/// machine runs them, against the probe they mirror.
#[test]
fn the_pid_probe_tests_are_split_where_the_probe_is() {
    let tests = include_str!("claude_sessions.rs");
    let probe = include_str!("../src/sessions/claude.rs");
    let attrs_of = |text: &str, item: &str| -> Vec<String> {
        let lines: Vec<&str> = text.lines().collect();
        let at = lines
            .iter()
            .position(|l| l.trim_start().starts_with(item))
            .unwrap_or_else(|| panic!("no `{item}`"));
        lines[..at]
            .iter()
            .rev()
            .map(|l| l.trim())
            .take_while(|l| l.starts_with("#[") || l.starts_with("///"))
            .filter(|l| l.starts_with("#["))
            .map(str::to_string)
            .collect()
    };
    assert!(
        attrs_of(
            tests,
            "fn the_pid_probe_tells_a_running_process_from_a_reaped_one("
        )
        .contains(&"#[cfg(unix)]".to_string()),
        "the reaped-pid test runs where the probe answers \"alive\" for every pid"
    );
    assert!(
        attrs_of(
            tests,
            "fn without_a_probe_every_possible_pid_reads_as_alive("
        )
        .contains(&"#[cfg(not(unix))]".to_string())
    );
    // The fallback the Windows test expects is the one the code has.
    assert!(
        attrs_of(probe, "fn probe_pid(_pid: u32) -> bool")
            .contains(&"#[cfg(not(unix))]".to_string())
    );
}

#[test]
fn a_project_directory_is_the_launch_directory_with_every_other_character_a_dash() {
    assert_eq!(project_dir_name("/Users/dev/demo"), "-Users-dev-demo");
    assert_eq!(
        project_dir_name("/Users/dev/my.app_v2 (copy)"),
        "-Users-dev-my-app-v2--copy-"
    );
    // One dash per UTF-16 unit: `ü` is one, an astral emoji is two.
    assert_eq!(project_dir_name("/a/ü/\u{1F600}"), "-a-----");

    let s = source();
    let demo: BTreeSet<String> = s
        .discover_for_cwd(&fixtures(), Path::new("/Users/dev/demo"))
        .into_iter()
        .map(|x| x.id)
        .collect();
    assert_eq!(demo.len(), 9);
    assert!(!demo.contains(OTHER));
    let other: Vec<String> = s
        .discover_for_cwd(&fixtures(), Path::new("/Users/dev/other"))
        .into_iter()
        .map(|x| x.id)
        .collect();
    assert_eq!(other, [OTHER]);
}

/// `/a b` and `/a-b` both encode to `-a-b`; the `cwd` recorded in the
/// transcript says which session is whose.
#[test]
fn two_launch_directories_that_encode_alike_are_told_apart_by_the_recorded_cwd() {
    let root = scratch("collide");
    let line = |cwd: &str| {
        serde_json::json!({
            "type": "user", "uuid": "u1", "parentUuid": null, "cwd": cwd,
            "version": "2.1.285", "timestamp": "2026-10-09T10:00:00Z",
            "message": {"role": "user", "content": format!("in {cwd}")},
        })
        .to_string()
    };
    one_session(&root, "-a-b", "spaced", &[line("/a b")]);
    one_session(&root, "-a-b", "dashed", &[line("/a-b")]);
    one_session(&root, "-a-b", "deeper", &[line("/a b/sub")]);
    let ids = |cwd: &str| -> BTreeSet<String> {
        source()
            .discover_for_cwd(&root, Path::new(cwd))
            .into_iter()
            .map(|s| s.id)
            .collect()
    };
    assert_eq!(ids("/a b"), ["deeper", "spaced"].map(String::from).into());
    assert_eq!(ids("/a-b"), ["dashed"].map(String::from).into());
    let _ = std::fs::remove_dir_all(&root);
}

/// Past 200 characters the directory name is cut and a hash appended; the
/// hash is not recomputed, the prefix and the suffix's shape are matched.
#[test]
fn a_long_launch_directory_matches_its_truncated_and_hashed_name() {
    let cwd = format!("/{}", "d".repeat(260));
    let encoded = project_dir_name(&cwd);
    assert_eq!(encoded.len(), 261);
    let dir = format!("{}-1x9z0q", &encoded[..200]);
    assert!(project_dir_matches(&dir, &cwd));
    assert!(!project_dir_matches(&encoded[..200], &cwd), "no hash");
    assert!(!project_dir_matches(&format!("{}-", &encoded[..200]), &cwd));
    assert!(!project_dir_matches(
        &format!("{}-ab/c", &encoded[..200]),
        &cwd
    ));
    let other = format!("/{}", "e".repeat(260));
    assert!(!project_dir_matches(&dir, &other));
    // At or under 200 it is the whole name, exactly.
    assert!(project_dir_matches("-short", "/short"));
    assert!(!project_dir_matches("-short-abc", "/short"));
}

/// The reader goes through the secrets gate: a transcript that is a link to
/// Claude Code's credential file is not listed and not read.
#[cfg(unix)]
#[test]
fn a_transcript_that_links_to_the_credential_file_is_neither_listed_nor_read() {
    let root = scratch("link");
    std::fs::write(
        root.join(".credentials.json"),
        "{\"accessToken\":\"sk-ant-oat01-not-a-real-token\"}",
    )
    .unwrap();
    let dir = root.join("projects").join("-w");
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(root.join(".credentials.json"), dir.join("linked.jsonl")).unwrap();
    one_session(
        &root,
        "-w",
        "honest",
        &[entry_line("h1", None, "user", "hi".into())],
    );

    let ids: Vec<String> = source().discover(&root).into_iter().map(|s| s.id).collect();
    assert_eq!(ids, ["honest"]);
    match source().read(&root, "linked") {
        Err(SessionError::Refused(r)) => {
            assert!(r.to_string().contains("credential"), "{r}");
        }
        other => panic!("the credential link was read: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The version gate: an unmeasured version is read, and says so.
#[test]
fn an_unmeasured_version_is_read_with_a_warning() {
    let future = source().read_report(&fixtures(), FUTURE).unwrap();
    assert_eq!(future.session.version.as_deref(), Some("9.9.9"));
    assert_eq!(
        texts(&future.session.entries),
        ["from the future", "Hello from 9.9.9."]
    );
    assert!(
        future
            .warnings
            .iter()
            .any(|w| w.contains("9.9.9") && w.contains("has not been read")),
        "{:?}",
        future.warnings
    );
    let known = source().read_report(&fixtures(), BRANCHES).unwrap();
    assert!(known.warnings.is_empty(), "{:?}", known.warnings);
}

#[test]
fn an_id_is_checked_before_it_becomes_a_path() {
    assert!(matches!(
        source().read(&fixtures(), "../sessions/4242"),
        Err(SessionError::BadId(_))
    ));
    assert!(matches!(
        source().read(&fixtures(), "00000000-0000-4000-8000-000000000000"),
        Err(SessionError::NotFound { .. })
    ));
    // A missing root is "no sessions", not an error.
    assert!(source().discover(&fixtures().join("absent")).is_empty());
}

// ---------------------------------------------------------------- RG-09 --

/// The basic transcript, rebuilt: metadata lines ignored, a malformed line,
/// a uuid-less entry and a non-object line skipped and counted, the
/// re-appended entry's LAST content kept, the old uuid re-appended at the end
/// not mistaken for the leaf, the sidechain entry after the leaf left out.
#[test]
fn the_chain_is_rebuilt_from_the_newest_leaf_with_duplicates_last_wins() {
    let read = source().read_report(&fixtures(), BASIC).unwrap();
    assert_eq!(
        shape(&read.session.entries),
        [
            (
                Role::User,
                EntryKind::Text,
                None,
                "Fix the date rollover in calendar.rs"
            ),
            (
                Role::Assistant,
                EntryKind::Thinking,
                None,
                "The month does not carry."
            ),
            (Role::Assistant, EntryKind::Text, None, "Looking."),
            (
                Role::Assistant,
                EntryKind::ToolCall,
                Some("Read"),
                "{\"file_path\":\"/Users/dev/demo/calendar.rs\"}"
            ),
            (
                Role::Tool,
                EntryKind::ToolResult,
                Some("Read"),
                "fn rollover() {}"
            ),
            (
                Role::Assistant,
                EntryKind::Text,
                None,
                "Fixed: the rollover now carries the month."
            ),
        ]
    );
    assert_eq!(
        read.leaf.as_deref(),
        Some("b0000005-0000-4000-8000-000000000000")
    );
    assert_eq!(read.skipped_lines, 3);
    assert!(
        read.warnings.iter().any(|w| w.contains("3 line(s)")),
        "{:?}",
        read.warnings
    );
    assert!(!read.compacted);
    // Timestamps went backwards in the file (the tool result is stamped before
    // the call); started/updated are the extremes, order is the chain's.
    let s = &read.session;
    assert_eq!(
        s.started,
        tacet_tools::sessions::model::parse_iso_utc("2026-10-09T10:00:00Z")
    );
    assert_eq!(
        s.updated,
        tacet_tools::sessions::model::parse_iso_utc("2026-10-09T10:00:20Z")
    );
    assert_eq!(s.cwd.as_deref(), Some(Path::new("/Users/dev/demo")));
}

/// Parallel tool calls: one result sits on a side branch and is attached by
/// `sourceToolAssistantUUID` — after the whole response that made the calls,
/// not between its two `tool_use` entries. A rewound branch is not context.
#[test]
fn a_sibling_tool_result_is_attached_and_a_rewound_branch_is_not() {
    let read = source().read_report(&fixtures(), BRANCHES).unwrap();
    assert_eq!(
        shape(&read.session.entries),
        [
            (Role::User, EntryKind::Text, None, "Run both checks"),
            (
                Role::Assistant,
                EntryKind::ToolCall,
                Some("Bash"),
                "{\"command\":\"make test\"}"
            ),
            (
                Role::Assistant,
                EntryKind::ToolCall,
                Some("Grep"),
                "{\"pattern\":\"TODO\"}"
            ),
            (
                Role::Tool,
                EntryKind::ToolResult,
                Some("Grep"),
                "no TODO left"
            ),
            (
                Role::Tool,
                EntryKind::ToolResult,
                Some("Bash"),
                "tests passed"
            ),
            (Role::Assistant, EntryKind::Text, None, "Both passed."),
            (
                Role::User,
                EntryKind::Text,
                None,
                "Actually, write the changelog"
            ),
            (Role::Assistant, EntryKind::Text, None, "Changelog written."),
        ]
    );
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
}

/// After a compaction the context starts at the summary. The boundary's
/// `logicalParentUuid` points at its own descendant here; following it would
/// go round, and it is not followed.
#[test]
fn a_compaction_starts_the_context_and_its_logical_parent_is_not_followed() {
    let read = source().read_report(&fixtures(), COMPACTED).unwrap();
    assert!(read.compacted);
    assert_eq!(
        shape(&read.session.entries),
        [
            (
                Role::System,
                EntryKind::Summary,
                None,
                "Summary: the rollover was fixed in calendar.rs."
            ),
            (Role::User, EntryKind::Text, None, "continue with the tests"),
            (Role::Assistant, EntryKind::Text, None, "Tests added."),
        ]
    );
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
    // The summary is not the user's words, so not the title either.
    assert_eq!(read.session.title, "continue with the tests");
}

/// A parent that is not in this file ends the chain with a warning, and the
/// part that IS here is kept.
#[test]
fn a_parent_outside_the_file_ends_the_chain_with_a_warning() {
    let read = source().read_report(&fixtures(), ORPHAN).unwrap();
    assert_eq!(
        texts(&read.session.entries),
        ["picked up where the last file stopped", "Continuing."]
    );
    assert!(
        read.warnings
            .iter()
            .any(|w| w.contains("e000dead-0000-4000-8000-000000000000")
                && w.contains("not in this file")),
        "{:?}",
        read.warnings
    );
}

/// A `parentUuid` loop is cut where it closes, with a warning; the read ends.
#[test]
fn a_parent_loop_is_cut_with_a_warning() {
    let read = source().read_report(&fixtures(), LOOPED).unwrap();
    assert_eq!(
        texts(&read.session.entries),
        [
            "third of the loop",
            "first of the loop",
            "second of the loop",
            "the leaf"
        ]
    );
    assert!(
        read.warnings.iter().any(|w| w.contains("loops back")),
        "{:?}",
        read.warnings
    );
}

/// A chain that is nothing but a loop has no leaf; that is said, not shown as
/// an empty session.
#[test]
fn a_chain_that_is_only_a_loop_says_so() {
    let root = scratch("closed-loop");
    one_session(
        &root,
        "-w",
        "closed",
        &[
            entry_line("l1", Some("l2"), "user", "one".into()),
            entry_line("l2", Some("l1"), "user", "two".into()),
        ],
    );
    let read = source().read_report(&root, "closed").unwrap();
    assert!(read.session.entries.is_empty());
    assert!(
        read.warnings.iter().any(|w| w.contains("no chain")),
        "{:?}",
        read.warnings
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------- RG-10 --

fn plan(items: &[(&str, Status)]) -> Vec<PlanItem> {
    items
        .iter()
        .map(|(title, status)| PlanItem {
            title: title.to_string(),
            status: *status,
        })
        .collect()
}

/// TaskCreate/TaskUpdate replayed in file order: a failed call did not
/// happen, an id missing from the structured result is read from its text,
/// a pending task with an open blocker is blocked, a deleted one cancelled.
/// The first create re-appended at the end of the file does not reset task 1,
/// a subagent's task is not this session's, and a TodoWrite in a session that
/// uses tasks is not the plan.
#[test]
fn the_plan_is_the_replay_of_every_task_call() {
    let s = source().read(&fixtures(), TASKS).unwrap();
    assert_eq!(
        s.plan,
        plan(&[
            ("Write changelog", Status::Completed),
            ("Tag the release", Status::Blocked),
            ("Announce", Status::Pending),
            ("Clean up", Status::Cancelled),
        ])
    );
}

/// With no task calls, the last TodoWrite is the plan.
#[test]
fn without_task_calls_the_last_todo_write_is_the_plan() {
    let s = source().read(&fixtures(), TODOS).unwrap();
    assert_eq!(
        s.plan,
        plan(&[
            ("Delete dead code", Status::Completed),
            ("Rename modules", Status::InProgress),
            ("Update docs", Status::Pending),
        ])
    );
}

/// `tasks/<sessionId>/` is the task list itself and beats the replay; its
/// files are taken in numeric id order (`10` after `2`).
#[test]
fn the_task_store_beats_the_replay() {
    let s = source().read(&fixtures(), STORE).unwrap();
    assert_eq!(
        s.plan,
        plan(&[
            ("From the store", Status::InProgress),
            ("Second", Status::Blocked),
            ("Tenth", Status::Completed),
        ])
    );
}
