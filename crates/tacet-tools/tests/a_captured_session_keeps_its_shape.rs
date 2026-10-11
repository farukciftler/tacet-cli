//! RG-28: a real session put through `sessions::anonymise` reads, with every
//! reader, exactly as the original did — and says nothing the original said.
//!
//! WHY THE WHOLE FIXTURE TREE. The anonymiser keeps a list of keys and markup
//! the readers depend on; a key missing from it does not leak anything, it
//! silently turns a fixture into a different case (a task status masked to
//! gibberish is a plan with no items; a masked `<USER_REQUEST>` is a title
//! with metadata in it). The only way to know the list is complete for what
//! the readers do today is to run every fixture this crate has through it and
//! compare what the readers make of the two — entry by entry: role, kind,
//! tool and time; the plan's statuses; the ids a listing finds.
//!
//! The planted session at the end is the other half: a capture from a home
//! that is not `dev`, with an e-mail address, a token and the user's name in
//! it, must come out with none of them and still read the same.
//!
//! No real session is read here: the inputs are the committed fixtures and a
//! session written by the test.

use std::path::{Path, PathBuf};

use tacet_tools::sessions::anonymise::Scrub;
use tacet_tools::sessions::antigravity::AntigravitySource;
use tacet_tools::sessions::claude::ClaudeSource;
use tacet_tools::sessions::codex::CodexSource;
use tacet_tools::sessions::{AgentKind, AgentSession, EntryKind, Role, SessionSource, Status};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tacet-capture-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `from` copied to `to` the way a capture would leave it: every text file
/// anonymised by its kind, Claude Code's encoded project directories renamed
/// to the encoding of the masked path, binary placeholders (`.db`, `.zst`)
/// copied as they are — the capture tool refuses those, and they are here
/// only so a listing finds the same ids. File times are carried over: the
/// readers date sessions by them.
fn capture_tree(from: &Path, to: &Path, scrub: &Scrub) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let source = entry.path();
        if entry.file_type().unwrap().is_dir() {
            let renamed = if name.starts_with('-') {
                scrub.text(&name)
            } else {
                name
            };
            capture_tree(&source, &to.join(renamed), scrub);
            continue;
        }
        let target = to.join(&name);
        let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "jsonl" | "json" | "md" => {
                let text = String::from_utf8_lossy(&std::fs::read(&source).unwrap()).into_owned();
                let out = match ext {
                    "jsonl" => scrub.jsonl(&text),
                    "json" => scrub.json(&text),
                    _ => scrub.markdown(&text),
                };
                std::fs::write(&target, out).unwrap();
            }
            _ => {
                std::fs::copy(&source, &target).unwrap();
            }
        }
        let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&target)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }
}

type Shape = (
    Vec<(Role, EntryKind, Option<String>, Option<i64>)>,
    Vec<Status>,
    Option<String>,
    bool,
);

/// What a session is, minus what it says.
fn shape(s: &AgentSession) -> Shape {
    (
        s.entries
            .iter()
            .map(|e| (e.role, e.kind, e.tool.clone(), e.ts))
            .collect(),
        s.plan.iter().map(|p| p.status).collect(),
        s.version.clone(),
        s.title.is_empty(),
    )
}

fn dead(_: u32) -> bool {
    false
}

fn source(agent: AgentKind) -> Box<dyn SessionSource> {
    match agent {
        AgentKind::ClaudeCode => Box::new(ClaudeSource::with_probe(dead)),
        AgentKind::Codex => Box::new(CodexSource),
        AgentKind::Antigravity => Box::new(AntigravitySource),
    }
}

/// Every root a reader is pointed at in this crate's fixtures.
fn roots() -> Vec<(AgentKind, PathBuf)> {
    let mut out = vec![
        // Joined from the slug: a literal that is an agent's name reads to
        // `network_monopoly` as that program being run.
        (
            AgentKind::ClaudeCode,
            fixtures().join(AgentKind::ClaudeCode.slug()),
        ),
        (AgentKind::Codex, fixtures().join("codex/home")),
        (
            AgentKind::Antigravity,
            fixtures().join("antigravity/antigravity-ide"),
        ),
    ];
    for agent in AgentKind::ALL {
        let dir = fixtures().join("corpus").join(agent.slug());
        for version in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = version.path();
            if agent == AgentKind::Antigravity {
                // `<version>/<antigravity-ide|antigravity-cli>`
                for root in std::fs::read_dir(&path).unwrap().flatten() {
                    out.push((agent, root.path()));
                }
            } else {
                out.push((agent, path));
            }
        }
    }
    out
}

/// Both trees, read the same way, compared session by session.
fn assert_same_reading(agent: AgentKind, original: &Path, captured: &Path, scrub: &Scrub) {
    let src = source(agent);
    let ids = |root: &Path| {
        let mut v: Vec<(String, bool)> = src
            .discover(root)
            .into_iter()
            .map(|s| (s.id, s.live))
            .collect();
        v.sort();
        v
    };
    let listed = ids(original);
    assert!(!listed.is_empty(), "{}: no sessions", original.display());
    assert_eq!(ids(captured), listed, "{}", original.display());
    for (id, _) in &listed {
        let want = src.read(original, id);
        let got = src.read(captured, id);
        match (&want, &got) {
            (Ok(w), Ok(g)) => {
                assert_eq!(shape(g), shape(w), "{} {id}", original.display());
                // Where there was a working directory there still is one:
                // the original's, masked the way every copy of it is — so
                // the path in `cwd` and the path in a tool call still agree.
                let masked = w
                    .cwd
                    .as_ref()
                    .map(|p| PathBuf::from(scrub.path(&p.to_string_lossy())));
                assert_eq!(g.cwd, masked, "{} {id}", original.display());
            }
            (Err(_), Err(_)) => {}
            _ => panic!(
                "{} {id}: read {} before the capture and {} after",
                original.display(),
                if want.is_ok() {
                    "fine"
                } else {
                    "with an error"
                },
                if got.is_ok() { "fine" } else { "with an error" },
            ),
        }
    }
    if agent == AgentKind::ClaudeCode {
        for (id, _) in &listed {
            let src = ClaudeSource::with_probe(dead);
            let (Ok(w), Ok(g)) = (src.read_report(original, id), src.read_report(captured, id))
            else {
                continue;
            };
            assert_eq!(g.warnings.len(), w.warnings.len(), "{id}: {:?}", g.warnings);
        }
    }
    if agent == AgentKind::Antigravity {
        for (id, _) in &listed {
            let (Ok(w), Ok(g)) = (
                AntigravitySource.read_full(original, id),
                AntigravitySource.read_full(captured, id),
            ) else {
                continue;
            };
            let tree = |r: &tacet_tools::sessions::antigravity::AntigravityRead| {
                r.tasks
                    .iter()
                    .map(|t| (t.depth, t.parent, t.status))
                    .collect::<Vec<_>>()
            };
            assert_eq!(tree(&g), tree(&w), "{id}");
            assert_eq!(g.transcript, w.transcript, "{id}");
            assert_eq!(g.skipped_lines, w.skipped_lines, "{id}");
            let names = |r: &tacet_tools::sessions::antigravity::AntigravityRead| {
                r.artifacts
                    .iter()
                    .map(|a| a.name.clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(names(&g), names(&w), "{id}");
        }
    }
}

/// Every text file under `dir`, concatenated.
fn all_text(dir: &Path) -> String {
    let mut out = String::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ["jsonl", "json", "md"].contains(&ext) {
                out.push_str(&String::from_utf8_lossy(&std::fs::read(&path).unwrap()));
            }
        }
    }
    out
}

#[test]
fn every_fixture_reads_the_same_after_a_capture() {
    let out = scratch("tree");
    let scrub = Scrub::new(0x5eed, None);
    for (n, (agent, root)) in roots().into_iter().enumerate() {
        // The root's own name is kept: the Antigravity reader takes the
        // product from it, and the version gate the flavor.
        let name = root.file_name().unwrap().to_owned();
        let captured = out.join(n.to_string()).join(name);
        capture_tree(&root, &captured, &scrub);
        assert_same_reading(agent, &root, &captured, &scrub);
        // And it says none of what it said: words from the fixtures' paths
        // and prompts, each checked only where the original has it, and at
        // least one per tree so this cannot pass on a tree it never looked at.
        let (before, after) = (all_text(&root), all_text(&captured));
        let probes: Vec<&str> = ["corpus", "demo", "rollover", "Reproduce", "Explain"]
            .into_iter()
            .filter(|w| before.contains(w))
            .collect();
        assert!(!probes.is_empty(), "{}: no probe word", root.display());
        for word in probes {
            assert!(
                !after.contains(word),
                "{}: {word:?} survived",
                root.display()
            );
        }
    }
    let _ = std::fs::remove_dir_all(&out);
}

/// A capture off a machine that is not the corpus's: another home, the
/// user's name, an address, a token. None of it survives; the session reads
/// the same, under the project directory of its masked `cwd`.
///
/// The last three lines are the review's hostile shapes: user data under the
/// key names the anonymiser keeps for structure — a CRM answer in a
/// tool_result, a tool call's input, a top-level `source` naming the user
/// with a capital letter. Before keeping looked at the value's shape and at
/// whether it sat inside a tool's data, every one of them came out verbatim,
/// the e-mail address included. The two lines after them are the second
/// review's: the same answer as object keys and numbers, which the version
/// that masked only values let through byte for byte.
#[test]
fn a_planted_capture_loses_what_identifies_it_and_keeps_its_reading() {
    let dir = scratch("planted");
    let id = "d0000001-0000-4000-8000-000000000001";
    let cwd = "/Users/alice/Clients/AcmeCorp";
    let lines = [
        serde_json::json!({"type":"user","uuid":"u1","parentUuid":null,"sessionId":id,
            "cwd":cwd,"gitBranch":"alice/acme-invoices","version":"2.1.285",
            "timestamp":"2026-10-09T10:00:00.000Z","isSidechain":false,"userType":"external",
            "message":{"role":"user","content":"Email alice@acmecorp.example the invoice; key sk-ant-api03-SECRET"}}),
        serde_json::json!({"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":id,
            "cwd":cwd,"version":"2.1.285","timestamp":"2026-10-09T10:00:05.000Z","isSidechain":false,
            "message":{"id":"msg_1","role":"assistant","model":"claude-x","content":[
                {"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/Users/alice/Clients/AcmeCorp/invoice.rs"}}]}}),
        serde_json::json!({"type":"user","uuid":"u2","parentUuid":"a1","sessionId":id,
            "cwd":cwd,"version":"2.1.285","timestamp":"2026-10-09T10:00:06.000Z","isSidechain":false,
            "sourceToolAssistantUUID":"a1",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"fn bill_alice() {}"}]}}),
        serde_json::json!({"type":"ai-title","aiTitle":"Invoice for AcmeCorp","sessionId":id}),
        serde_json::json!({"type":"assistant","uuid":"a2","parentUuid":"u2","sessionId":id,
            "cwd":cwd,"version":"2.1.285","timestamp":"2026-10-09T10:00:07.000Z","isSidechain":false,
            "message":{"id":"msg_2","role":"assistant","model":"claude-x","content":[
                {"type":"tool_use","id":"t2","name":"mcp__crm__lookup","input":
                    {"id":"AcmeCorp-contract-77","status":"AcmeCorp pending","type":"AcmeCorp confidential"}}]}}),
        serde_json::json!({"type":"user","uuid":"u3","parentUuid":"a2","sessionId":id,
            "cwd":cwd,"version":"2.1.285","timestamp":"2026-10-09T10:00:08.000Z","isSidechain":false,
            "sourceToolAssistantUUID":"a2","source":"Alice's laptop",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":
                r#"{"id":"jane.doe@clientbank.com","status":"overdue: Jane Doe owes 40k","source":"Clientbank CRM","version":"Q3 board deck"}"#}]}}),
        // The second review's shapes: the same answer moved into object
        // KEYS and NUMBERS — a call's input keyed by a client, a result
        // keyed by a person and an address, an MCP result's
        // `structuredContent` keyed by a name.
        serde_json::json!({"type":"assistant","uuid":"a3","parentUuid":"u3","sessionId":id,
            "cwd":cwd,"version":"2.1.285","timestamp":"2026-10-09T10:00:09.000Z","isSidechain":false,
            "message":{"id":"msg_3","role":"assistant","model":"claude-x","content":[
                {"type":"tool_use","id":"t3","name":"mcp__crm__lookup","input":
                    {"customers":{"AcmeCorp Holdings":{"owed":40000}}}}]}}),
        serde_json::json!({"type":"user","uuid":"u4","parentUuid":"a3","sessionId":id,
            "cwd":cwd,"version":"2.1.285","timestamp":"2026-10-09T10:00:10.000Z","isSidechain":false,
            "sourceToolAssistantUUID":"a3",
            "toolUseResult":{"structuredContent":{"Bob Smith":1}},
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t3","content":
                r#"{"Jane Doe":{"iban_no":4012888888881881,"salary":185000},"jdoe@clientbank.com":true}"#}]}}),
        // The third review's shapes: card numbers under `id`, a key the
        // readers take task ids from, as a number and as a string; and a
        // tool's answer in a `progress` line, an envelope no reader opens.
        serde_json::json!({"type":"user","uuid":"u5","parentUuid":"u4","sessionId":id,
            "cwd":cwd,"version":"2.1.285","timestamp":"2026-10-09T10:00:11.000Z","isSidechain":false,
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t3","content":
                r#"{"rows":[{"id":4012888888881881},{"id":"5500005555555559"}]}"#}]}}),
        serde_json::json!({"type":"progress","uuid":"p1","parentUuid":"u5","sessionId":id,
            "timestamp":"2026-10-09T10:00:12.000Z",
            "data":{"type":"mcp_progress","result":{"AcmeCorp":{"card":4_111_111_111_111_111_u64}}}}),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let project = tacet_tools::sessions::claude::project_dir_name(cwd);
    let original = dir.join("original");
    std::fs::create_dir_all(original.join("projects").join(&project)).unwrap();
    std::fs::write(
        original
            .join("projects")
            .join(&project)
            .join(format!("{id}.jsonl")),
        &text,
    )
    .unwrap();

    let scrub = Scrub::new(7, Some("alice"));
    let out = scrub.jsonl(&text);
    for leak in [
        "alice",
        "Alice",
        "AcmeCorp",
        "acme",
        "Clients",
        "invoice",
        "@",
        "sk-ant-",
        "SECRET",
        "jane",
        "Jane",
        "clientbank",
        "Clientbank",
        "overdue",
        "owes",
        "board",
        "deck",
        "contract",
        "confidential",
        "laptop",
        "customers",
        "Holdings",
        "owed",
        "40000",
        "Bob",
        "Smith",
        "iban",
        "salary",
        "4012888888881881",
        "185000",
        "jdoe",
        "5500005555555559",
        "4111111111111111",
        "card",
    ] {
        assert!(!out.contains(leak), "{leak:?} survived:\n{out}");
    }
    let masked_cwd = scrub.path(cwd);
    assert!(masked_cwd.starts_with("/Users/dev/"), "{masked_cwd}");
    let captured = dir.join("captured");
    let captured_project = captured
        .join("projects")
        .join(tacet_tools::sessions::claude::project_dir_name(&masked_cwd));
    std::fs::create_dir_all(&captured_project).unwrap();
    std::fs::write(captured_project.join(format!("{id}.jsonl")), &out).unwrap();
    // The directory a capture is filed under is the encoding of the masked
    // path, and masking the original directory's name gives the same one.
    assert_eq!(
        scrub.text(&project),
        tacet_tools::sessions::claude::project_dir_name(&masked_cwd)
    );

    assert_same_reading(AgentKind::ClaudeCode, &original, &captured, &scrub);
    let got = ClaudeSource::with_probe(dead).read(&captured, id).unwrap();
    assert_eq!(got.cwd.as_deref(), Some(Path::new(&masked_cwd)));
    assert_eq!(got.version.as_deref(), Some("2.1.285"));
    let _ = std::fs::remove_dir_all(&dir);
}
