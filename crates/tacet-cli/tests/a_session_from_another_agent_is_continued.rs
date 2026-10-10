//! `tacet chat --from <agent>:<id>` — another coding agent's session,
//! continued here (RG-17, RG-18, RG-20).
//!
//! WHY THIS RUNS THE BINARY. Every piece has a unit test — the reader, the
//! summary, the seed, the taint, the handoff — and none of them can see the
//! one thing the user gets: that `chat` actually calls them, in that order,
//! before the first question is sent. So this drives `tacet chat` on the
//! scripted engine and reads the prompt it sent (`--show-prompt`) and the file
//! it left behind.
//!
//! NOTHING OF THE USER'S IS TOUCHED. `HOME` and `TACET_HOME` point into a temp
//! directory; `CLAUDE_CONFIG_DIR` and `CODEX_HOME` are removed, and the
//! registry names the readers' own fixture tree as the only root, so no
//! default root under the real home is ever looked at. The scripted engine
//! loads no weights.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tacet_tools::sessions::AgentKind;
use tacet_web::addon::{AGENT_SESSIONS, Addon, ROOTS_KEY, Record};

const SESSION: &str = "11111111-0000-4000-8000-111111111111";

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("tacet-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("work")).expect("a temp directory");
        Scratch(p)
    }

    fn handoff(&self) -> PathBuf {
        self.0
            .join("handoff")
            .join(format!("{}-{SESSION}.json", AgentKind::ClaudeCode.slug()))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The readers' own fixture tree for Claude Code — built from the slug so no
/// literal here names an agent program (`network_monopoly`).
fn claude_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tacet-tools/tests/fixtures/sessions")
        .join(AgentKind::ClaudeCode.slug())
        .canonicalize()
        .expect("the fixture tree exists")
}

fn install(scratch: &Scratch, open: bool) {
    let mut record = Record::empty();
    let mut addon = Addon::new(AGENT_SESSIONS, AGENT_SESSIONS)
        .with_setting(ROOTS_KEY, claude_fixtures().display().to_string());
    addon.open = open;
    record.add(addon);
    tacet_web::addon::write_to_path(&scratch.0.join("addons.json"), &record)
        .expect("the registry is written");
}

fn chat(scratch: &Scratch, from: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tacet"))
        .args(["chat", "--engine", "fake", "--script", "the answer."])
        .args(["--show-prompt", "--json", "--dir"])
        .arg(scratch.0.join("work"))
        .args(["--from", from])
        .args(["--message", "what was I in the middle of?"])
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("TACET_MODEL")
        .env_remove("TACET_TOKENIZER")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env("HOME", &scratch.0)
        .env("TACET_HOME", &scratch.0)
        .stdin(Stdio::null())
        .output()
        .expect("the tacet binary runs")
}

fn reference() -> String {
    format!("{}:{SESSION}", AgentKind::ClaudeCode.slug())
}

/// The summary is in the prompt, fenced, ABOVE the question; the transcript
/// itself is not; and the handoff record exists afterwards.
#[test]
fn the_summary_is_sent_with_the_first_question_and_the_handoff_is_started() {
    let scratch = Scratch::new("from-claude");
    install(&scratch, true);
    let out = chat(&scratch, &reference());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "tacet failed:\n{stderr}");

    let prompt = &stderr[stderr.find("--- PROMPT").unwrap_or(0)..];
    let fence = prompt
        .find("<imported_session agent=\"claude\"")
        .unwrap_or_else(|| panic!("no imported session in the prompt:\n{prompt}"));
    let goal = prompt
        .find("Goal (first request): Fix the date rollover in calendar.rs")
        .unwrap_or_else(|| panic!("the summary has no goal:\n{prompt}"));
    let question = prompt
        .rfind("what was I in the middle of?")
        .expect("the question is in the prompt");
    assert!(
        fence < goal && goal < question,
        "the summary must sit above the question it informs"
    );
    // The reasoning is in the transcript and stays in the store.
    assert!(
        !prompt.contains("The month does not carry."),
        "the transcript's reasoning reached the prompt"
    );
    // ...and the prompt points at it (RG-18 second review): `--from` stores
    // the full transcript and sends its reference, the way `session_read`
    // does, after the fence closes.
    let reference = prompt
        .find("source_ref=agent-session#")
        .unwrap_or_else(|| panic!("the prompt points at no stored transcript:\n{prompt}"));
    let close = prompt
        .find("</imported_session>")
        .expect("the fence closes");
    assert!(
        close < reference && reference < question,
        "the reference must sit after the fence and above the question"
    );

    let record = std::fs::read_to_string(scratch.handoff()).expect("the handoff record");
    assert!(record.contains("Fix the date rollover"), "{record}");
    assert!(record.contains("\"by\": \"tacet\""), "{record}");
}

/// Off until installed: with the addon closed, `--from` reads nothing,
/// starts nothing, and says how to open it.
#[test]
fn a_closed_addon_refuses_and_names_the_install() {
    let scratch = Scratch::new("from-closed");
    install(&scratch, false);
    let out = chat(&scratch, &reference());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "a closed addon read a session:\n{stderr}"
    );
    assert!(
        stderr.contains(&format!("tacet addon install {AGENT_SESSIONS}")),
        "{stderr}"
    );
    assert!(
        !scratch.handoff().exists(),
        "a refused import wrote a handoff"
    );
}

/// An id that could leave its directory is refused before any path is built,
/// and an unknown session is an error, not an empty continuation.
#[test]
fn a_bad_or_unknown_reference_ends_the_run() {
    let scratch = Scratch::new("from-bad");
    install(&scratch, true);
    let slug = AgentKind::ClaudeCode.slug();
    for bad in [
        format!("{slug}:../../.ssh/id_rsa"),
        format!("{slug}:00000000-0000-4000-8000-000000000000"),
        "cursor:abc".to_string(),
    ] {
        let out = chat(&scratch, &bad);
        assert!(
            !out.status.success(),
            "{bad:?} was accepted:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert!(!scratch.0.join("handoff").exists());
}

fn why(scratch: &Scratch, message: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tacet"))
        .args(["why", message])
        .current_dir(scratch.0.join("work"))
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("TACET_MODEL")
        .env_remove("TACET_TOKENIZER")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env("HOME", &scratch.0)
        .env("TACET_HOME", &scratch.0)
        .stdin(Stdio::null())
        .output()
        .expect("the tacet binary runs")
}

/// RG-17 third review: THE USER'S PATH, through the command that prints it.
/// Installed and open, the two readers are in the session's catalog and
/// `sessions_list` heads what the model sees; closed, neither name is
/// anywhere in the catalog — not shown, not left out.
///
/// WHY `tacet why` AND NOT THE CATALOG FUNCTION. The review replaced
/// `addon::is_open(addon::AGENT_SESSIONS)` in `AddonGates::read()` with
/// `false` and the whole workspace stayed green: the catalog tests build the
/// gates by hand, and `--from` / `/resume-from` check the addon themselves.
/// Only a run of the binary over a written `addons.json` sees the gate the
/// model's tool list is actually built from.
#[test]
fn an_open_addon_puts_both_readers_in_the_catalog_and_a_closed_one_does_not() {
    use tacet_tools::sessions_tool::{LIST_TOOL, READ_TOOL};
    let message = "continue my claude code session";

    let open = Scratch::new("why-open");
    install(&open, true);
    let out = why(&open, message);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "tacet why failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let shown = stdout
        .split("tools the model will see")
        .nth(1)
        .and_then(|s| s.split("the guidance block").next())
        .unwrap_or_else(|| panic!("no tool list in:\n{stdout}"));
    let first = shown
        .lines()
        .find_map(|l| l.trim().strip_prefix("1. "))
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or_else(|| panic!("no first tool in:\n{shown}"));
    assert_eq!(
        first, LIST_TOOL,
        "an open addon's lister is not first:\n{stdout}"
    );
    assert!(
        shown.contains(READ_TOOL),
        "an open addon's reader is not shown:\n{stdout}"
    );

    let closed = Scratch::new("why-closed");
    install(&closed, false);
    let out = why(&closed, message);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    for name in [LIST_TOOL, READ_TOOL] {
        assert!(
            !stdout.contains(name),
            "a closed addon's {name} is in the catalog:\n{stdout}"
        );
    }
}
