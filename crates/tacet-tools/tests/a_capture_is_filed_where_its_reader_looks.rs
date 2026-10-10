//! RG-28: a captured session file is filed in the corpus where its reader
//! looks for it, under the version it records — by `sessions::corpus::place`,
//! the code `corpus_capture --into` runs, not by a person typing a path.
//!
//! THREE HALVES. The committed corpus is the first: every file in it must be
//! exactly where `place` would put it, so the rule and the tree it describes
//! cannot drift apart. A planted capture is the second: a session from a home
//! that is not `dev`, put through the anonymiser and filed by `place`, must be
//! found by its reader at the filed path — Claude Code's by its MASKED working
//! directory, which is the mistake a hand-typed path makes silently (the
//! original project directory still reads by id, and its name is the unmasked
//! home). The refusals are the third: nothing a file says becomes a path
//! component unless it is the shape it claims.
//!
//! No real session is read here: the inputs are the committed corpus and
//! copies of it written to a scratch directory.

use std::path::{Path, PathBuf};

use tacet_tools::sessions::anonymise::Scrub;
use tacet_tools::sessions::antigravity::AntigravitySource;
use tacet_tools::sessions::claude::ClaudeSource;
use tacet_tools::sessions::codex::CodexSource;
use tacet_tools::sessions::corpus::place;
use tacet_tools::sessions::{AgentKind, EntryKind, Role, SessionSource};

const CC: AgentKind = AgentKind::ClaudeCode;
const CX: AgentKind = AgentKind::Codex;
const AG: AgentKind = AgentKind::Antigravity;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions/corpus")
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "tacet-place-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn dead(_: u32) -> bool {
    false
}

fn source(agent: AgentKind) -> Box<dyn SessionSource> {
    match agent {
        CC => Box::new(ClaudeSource::with_probe(dead)),
        CX => Box::new(CodexSource),
        AG => Box::new(AntigravitySource),
    }
}

type Shape = Vec<(Role, EntryKind, Option<String>, Option<i64>)>;

fn shape(agent: AgentKind, root: &Path, id: &str) -> Shape {
    source(agent)
        .read(root, id)
        .unwrap_or_else(|e| panic!("{} under {}: {e}", id, root.display()))
        .entries
        .iter()
        .map(|e| (e.role, e.kind, e.tool.clone(), e.ts))
        .collect()
}

/// Every text file the corpus holds is where `place` files it. The binary
/// placeholders (`.db`, `.jsonl.zst`) are skipped: the capture refuses them,
/// so they are never placed. Antigravity's version is the one thing the
/// file cannot supply, so it is handed the directory's — as `--version` is.
#[test]
fn every_corpus_file_is_where_place_would_file_it() {
    let mut files = Vec::new();
    walk(&corpus(), &mut files);
    let mut placed = 0;
    for path in files {
        let rel = path.strip_prefix(corpus()).unwrap().to_path_buf();
        let name = rel.to_string_lossy().into_owned();
        if name == "README.md" || name.ends_with(".db") || name.ends_with(".zst") {
            continue;
        }
        let mut parts = rel.components();
        let agent = parts
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .into_owned();
        let version = parts
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .into_owned();
        let asked = (agent == AG.slug()).then_some(version.as_str());
        let text = std::fs::read_to_string(&path).unwrap();
        let filed = place(&path, &text, asked).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(filed, rel, "{name} sits where `place` would not put it");
        placed += 1;
    }
    // 12 Claude Code transcripts, 5 Codex rollouts, 7 Antigravity files.
    assert_eq!(placed, 24, "the corpus changed; recount");
}

/// The home a planted capture comes from, and the user it belongs to: not
/// `dev`, so a path that survives masking shows.
const USER: &str = "zeynepk";
const PROJECT: &str = "acme-billing";

/// A capture from a real-looking home: the corpus file with its `dev` paths
/// put back to `USER`'s, filed at the place its agent would have written it,
/// then anonymised and filed by `place` under a fresh corpus. The reader,
/// pointed at the version directory `place` chose, must find it — by the
/// masked working directory for Claude Code — and read it exactly as it reads
/// the committed original.
#[test]
fn a_capture_from_a_real_home_is_filed_where_its_reader_finds_it() {
    let scratch = Scratch::new("home");
    let home = scratch.0.join("home");
    let into = scratch.0.join("corpus");
    let scrub = Scrub::new(7_041_993, Some(USER));
    let real = format!("/Users/{USER}/{PROJECT}");
    let real_linux = format!("/home/{USER}/{PROJECT}");

    // Claude Code: `projects/<encoded real cwd>/<id>.jsonl`.
    let id = "c0000285-0000-4000-8000-000000000001";
    let committed = corpus().join(CC.slug()).join("2.1.285");
    let original =
        std::fs::read_to_string(committed.join(format!("projects/-Users-dev-corpus/{id}.jsonl")))
            .unwrap()
            .replace("/Users/dev/corpus", &real);
    let input = home
        .join(format!(".{}", CC.slug()))
        .join("projects")
        .join(format!("-Users-{USER}-{PROJECT}"))
        .join(format!("{id}.jsonl"));
    write(&input, &original);
    let captured = scrub.jsonl(&original);
    let filed = place(&input, &captured, None).unwrap();
    let filed_text = filed.to_string_lossy().into_owned();
    assert!(
        filed.starts_with(Path::new(CC.slug()).join("2.1.285/projects")),
        "{filed_text}"
    );
    assert!(
        !filed_text.contains(USER) && !filed_text.contains("acme"),
        "the capture was filed under its unmasked project directory: {filed_text}"
    );
    write(&into.join(&filed), &captured);
    let root = into.join(CC.slug()).join("2.1.285");
    let masked_cwd = captured
        .lines()
        .find_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            v.get("cwd")?.as_str().map(str::to_string)
        })
        .unwrap();
    let listed = ClaudeSource::with_probe(dead).discover_for_cwd(&root, Path::new(&masked_cwd));
    assert!(
        listed.iter().any(|s| s.id == id),
        "a listing for {masked_cwd} under {} does not find {id}: {listed:?}",
        root.display()
    );
    assert_eq!(shape(CC, &root, id), shape(CC, &committed, id));

    // Codex: `sessions/YYYY/MM/DD/rollout-…`, under the `cli_version` it records.
    let name = "rollout-2026-10-06T11-00-00-0199e000-0047-7000-8000-000000000001.jsonl";
    let id = "0199e000-0047-7000-8000-000000000001";
    let committed = corpus().join(CX.slug()).join("0.47.0");
    let original = std::fs::read_to_string(committed.join("sessions/2026/10/06").join(name))
        .unwrap()
        .replace("/home/dev/corpus", &real_linux);
    let input = home
        .join(format!(".{}", CX.slug()))
        .join("sessions/2026/10/06")
        .join(name);
    write(&input, &original);
    let captured = scrub.jsonl(&original);
    let filed = place(&input, &captured, None).unwrap();
    assert_eq!(
        filed,
        Path::new(CX.slug())
            .join("0.47.0/sessions/2026/10/06")
            .join(name)
    );
    write(&into.join(&filed), &captured);
    assert_eq!(
        shape(CX, &into.join(CX.slug()).join("0.47.0"), id),
        shape(CX, &committed, id)
    );

    // Antigravity: `<root>/brain/<id>/…`, under the version it is told.
    let id = "a0000255-0000-4000-8000-000000000001";
    let log = ".system_generated/logs/transcript_full.jsonl";
    let committed = corpus().join(AG.slug()).join("2.5.5/antigravity-ide");
    let original = std::fs::read_to_string(committed.join("brain").join(id).join(log)).unwrap();
    let input = home
        .join(".gemini/antigravity-ide/brain")
        .join(id)
        .join(log);
    write(&input, &original);
    let captured = scrub.jsonl(&original);
    let filed = place(&input, &captured, Some("2.5.5")).unwrap();
    assert_eq!(
        filed,
        Path::new(AG.slug())
            .join("2.5.5/antigravity-ide/brain")
            .join(id)
            .join(log)
    );
    write(&into.join(&filed), &captured);
    assert_eq!(
        shape(AG, &into.join(AG.slug()).join("2.5.5/antigravity-ide"), id),
        shape(AG, &committed, id)
    );

    // And nothing filed anywhere names the user.
    let mut files = Vec::new();
    walk(&into, &mut files);
    for f in files {
        let rel = f
            .strip_prefix(&into)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(
            !rel.contains(USER) && !text.contains(USER),
            "{rel} still names {USER}"
        );
    }
}

/// Nothing a file records becomes a directory unless it is the shape it
/// claims, and a version the file does not record is not filed under.
#[test]
fn what_a_file_cannot_vouch_for_is_refused() {
    let claude_line = |version: &str, cwd: Option<&str>| {
        let mut v = serde_json::json!({
            "type": "user",
            "sessionId": "c0000285-0000-4000-8000-000000000099",
            "version": version,
            "uuid": "e0000001-0000-4000-8000-000000000000",
        });
        if let Some(cwd) = cwd {
            v["cwd"] = cwd.into();
        }
        v.to_string()
    };
    let transcript = Path::new("p/c0000285-0000-4000-8000-000000000099.jsonl");
    let ok = place(
        transcript,
        &claude_line("2.1.285", Some("/Users/dev/x")),
        None,
    )
    .unwrap();
    assert_eq!(
        ok,
        Path::new(CC.slug())
            .join("2.1.285/projects/-Users-dev-x/c0000285-0000-4000-8000-000000000099.jsonl")
    );

    for (why, input, text, asked) in [
        (
            "a version that walks out of the corpus",
            transcript,
            claude_line("../../2.1.285", Some("/Users/dev/x")),
            None,
        ),
        (
            "a version with a space in it",
            transcript,
            claude_line(" 2.1.285", Some("/Users/dev/x")),
            None,
        ),
        (
            "a version the file does not record",
            transcript,
            claude_line("2.1.285", Some("/Users/dev/x")),
            Some("2.1.293"),
        ),
        (
            "no working directory to encode",
            transcript,
            claude_line("2.1.285", None),
            None,
        ),
        (
            "a working directory past Claude Code's hashed cut",
            transcript,
            claude_line("2.1.285", Some(&format!("/Users/dev/{}", "x".repeat(220)))),
            None,
        ),
        (
            "a Codex version that walks out of the corpus",
            Path::new("s/rollout-2026-10-06T11-00-00-0199e000-0047-7000-8000-000000000001.jsonl"),
            r#"{"type":"session_meta","payload":{"cli_version":"0.47.0/../../x"}}"#.to_string(),
            None,
        ),
        (
            "a rollout not named like one",
            Path::new("s/notes.jsonl"),
            r#"{"type":"session_meta","payload":{"cli_version":"0.47.0"}}"#.to_string(),
            None,
        ),
        (
            "an Antigravity file with no version given",
            Path::new("g/antigravity-ide/brain/a1/task.md"),
            "- [ ] x".to_string(),
            None,
        ),
        (
            "an Antigravity version that is not one",
            Path::new("g/antigravity-ide/brain/a1/task.md"),
            "- [ ] x".to_string(),
            Some("latest"),
        ),
        (
            "a file that records nothing",
            Path::new("t/1.json"),
            "{}".to_string(),
            None,
        ),
    ] {
        let got = place(input, &text, asked);
        assert!(got.is_err(), "{why}: filed at {got:?}");
    }
}
