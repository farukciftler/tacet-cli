//! Every loop that hands a generation to the executor re-splits it with the
//! catalog first.
//!
//! WHY THIS IS A SOURCE GUARD (audit H5). The engine strips the `<think>` block
//! in `Generation::new`, where no tool name is known, and the glued call shape
//! (`write_filepath: "a.md"` at a line start) is a call only by a CATALOG name.
//! So the engine's split cut it like prose: a review measured
//! `write_filepath: "a.md", content: "Qwen emits <think> first"` reaching the
//! executor as `Qwen emits  first`, and a `</think>` inside one glued argument
//! leaving `delete_filepath: "victim.md"` at a line start as a second call.
//! `Generation::split_against` closes that, but only where it is called, and
//! the shell's turn loop is one 1500-line function no unit test drives. The
//! eval runner's own test (`a_close_tag_inside_a_glued_argument_cannot_launch_another_call`)
//! covers the behaviour; this covers the wiring, in every copy of the loop —
//! including one somebody adds later.
//!
//! AND EACH GATES THE CALL ON `generation.executable`. A block the split keeps
//! on a guess is executor input, calls the model declined in it included; a
//! review ran `web_search` from `<think>I could write <tool_call>web_search
//! query="leak secret" but I will not.</think>` through the production
//! catalog. The split reports the guess, but a loop that does not read the
//! report runs the call anyway.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

// Split in two so this file's own constant is not one of the sites it finds.
const EXECUTES: &str = concat!("execute_raw(", "&generation.text");
const SPLITS: &str =
    "generation.split_against(&executor.catalog().names(), |t| executor.parse_call(t))";
const GATES: &str = "!generation.executable";
const GATES_POSITIVE: &str = "if generation.executable";

/// Each `execute_raw(&generation.text …)` with, for each, whether a CODE line
/// between the `generation` binding above it and the call re-splits it, and
/// whether one gates the call on `executable`.
///
/// COMMENT LINES DO NOT COUNT. The loops explain the split in prose right
/// next to it, and a guard satisfied by that prose would stay green with the
/// call deleted — the failure `the_smoke_check_names_real_flags` records.
fn sites() -> Vec<(String, bool, bool)> {
    let mut files = Vec::new();
    rust_files(&repo_root().join("crates"), &mut files);
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.contains(EXECUTES) || line.trim_start().starts_with("//") {
                continue;
            }
            let binding = (0..i)
                .rev()
                .find(|&j| {
                    lines[j].contains("let mut generation =")
                        || lines[j].contains("let generation =")
                })
                .unwrap_or(0);
            let code = || {
                lines[binding..i]
                    .iter()
                    .filter(|l| !l.trim_start().starts_with("//"))
            };
            let split = code().any(|l| l.contains(SPLITS));
            let gated = code().any(|l| l.contains(GATES) || l.contains(GATES_POSITIVE));
            let rel = file.strip_prefix(repo_root()).unwrap_or(&file);
            out.push((format!("{}:{}", rel.display(), i + 1), split, gated));
        }
    }
    out
}

#[test]
fn every_executing_loop_resplits_with_the_catalog() {
    let found = sites();
    // NOT VACUOUS: the three loops this was written for must be found, or the
    // scan has stopped matching and every assertion below passes on nothing.
    for expected in [
        "crates/tacet-cli/src/chat.rs",
        "crates/tacet-eval/src/runner.rs",
        "crates/tacet-eval/src/tool_selection.rs",
    ] {
        assert!(
            found.iter().any(|(at, _, _)| at.starts_with(expected)),
            "no `{EXECUTES}` found in {expected}: {found:?}"
        );
    }
    let missing: Vec<&String> = found
        .iter()
        .filter(|(_, split, _)| !split)
        .map(|(at, _, _)| at)
        .collect();
    assert!(
        missing.is_empty(),
        "these hand the executor a generation the catalog never re-split, so a \
         glued call in it is cut like prose (audit H5) — call `{SPLITS}` after \
         the generation is bound: {missing:#?}"
    );
}

#[test]
fn every_executing_loop_gates_the_call_on_the_split() {
    let found = sites();
    assert!(
        found.len() >= 3,
        "the scan found fewer loops than it was written for: {found:?}"
    );
    let ungated: Vec<&String> = found
        .iter()
        .filter(|(_, _, gated)| !gated)
        .map(|(at, _, _)| at)
        .collect();
    assert!(
        ungated.is_empty(),
        "these run a call the thinking split could only guess about — a call the \
         model may have declined inside its reasoning (audit H5); skip the \
         executor when `generation.executable` is false: {ungated:#?}"
    );
}

// ---------------------------------------------------------------------------
// The shell's gate, on the binary
// ---------------------------------------------------------------------------
//
// THE SOURCE GUARD ABOVE COUNTS A LINE, NOT WHAT IT DOES. A review changed
// chat.rs's `if last_pass || !generation.executable` to `&&` and every
// tacet-cli test stayed green, both above included: the line still held the
// text. Under that mutation the shell ran a call the model declined inside its
// reasoning on every pass but the last. The eval runner's copy has a
// behavioural test (`runner::tests::a_call_declined_inside_the_thinking_does_not_run`);
// this is the shell's, through `tacet chat` on the scripted engine with
// `shell`/`echo` installed in a temp home.

use std::io::Write;
use std::process::{Command, Stdio};

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("tacet-h5-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("work")).expect("a temp directory");
        Home(p)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tacet(home: &Home) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_tacet"));
    c.env_remove("XDG_CONFIG_HOME")
        .env_remove("TACET_MODEL")
        .env_remove("TACET_TOKENIZER")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env("HOME", &home.0)
        .env("TACET_HOME", &home.0)
        .current_dir(home.0.join("work"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn output(mut c: Command, stdin: &str) -> String {
    c.stdin(Stdio::piped());
    let mut child = c.spawn().expect("the tacet binary runs");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("feeds stdin");
    let out = child.wait_with_output().expect("tacet finishes");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn chat_with(home: &Home, first_pass: &str) -> String {
    let mut c = tacet(home);
    c.args(["chat", "--engine", "fake", "--script", first_pass])
        .args(["--script", "Nothing to run."])
        .args(["--message", "should anything run?"])
        .stdin(Stdio::null());
    let out = c.output().expect("the tacet binary runs");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const CALL: &str = r#"{"name":"shell","arguments":{"command":"echo","args":["H5PROBE"]}}"#;

#[test]
fn the_shell_does_not_run_a_call_declined_inside_the_thinking() {
    let home = Home::new("declined");
    let mut c = tacet(&home);
    c.args(["addon", "install", "shell"]);
    let installed = output(c, "echo\n\ny\n");
    assert!(
        installed.contains("the `shell` tool is in the catalog"),
        "the shell addon did not install:\n{installed}"
    );

    // THE CONTROL: the same call, written as the answer, runs — so a quiet
    // run below is the gate and not a shell that never ran anything.
    let control = chat_with(&home, CALL);
    assert!(
        control.contains("exit 0"),
        "the control call did not run, so the refusal below proves nothing:\n{control}"
    );

    // The review's shape for `web_search`, with `shell`: a key=value call
    // inside the reasoning, turned down in the same sentence. (A JSON call
    // with prose after it is not parsed at all, so it would prove nothing.)
    let declined = chat_with(
        &home,
        r#"<think>I could write <tool_call>shell command="echo" but I will not.</think>Nothing to run."#,
    );
    assert!(
        !declined.contains("exit 0"),
        "a call the model declined inside <think> ran (audit H5):\n{declined}"
    );
}
