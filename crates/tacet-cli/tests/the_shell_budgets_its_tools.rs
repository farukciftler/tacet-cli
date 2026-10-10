//! The shell's prompt, with its `<tools>` block, fits the window it was budgeted for.
//!
//! WHY THIS RUNS THE BINARY. Audit M22: the shell and the eval each built a
//! pass's prompt and each truncated it, in a different order — the eval cut and
//! then attached the tools, so its budget never saw them. The order now lives in
//! one function, `tacet_eval::turn_prompt::build_turn_prompt`, and the eval's
//! tests guard it from the eval's side. The first version of that fix left the
//! shell with its own copy, and moving `with_tools` after `truncate` there kept
//! every `tacet-cli` test green: nothing on this side looked at the prompt the
//! shell actually sends. A unit test of a helper would have the same hole — it
//! checks the helper, not that `chat` calls it — so this drives `tacet chat` end
//! to end, on the scripted engine, and reads what it sent.
//!
//! THE FAKE ENGINE KEEPS THE FLOOR (`engine_setup::setup_engine`): 4096 tokens,
//! a 3072 prompt cap. The selected tools and the system block fit under that on
//! their own (measured 10 Oct 2026: the prompt below is sent at 3071 with the
//! question cut), so a correct shell can always get inside it by cutting the
//! question. With the order reversed the question is cut to fit WITHOUT the
//! tools, the tools are then appended, and the sent prompt is over the cap by
//! roughly their size — while the token count the shell reports is the smaller,
//! pre-tools figure.
//!
//! NOTHING OF THE USER'S IS TOUCHED. `HOME` and `TACET_HOME` point into a temp
//! directory, `--message` keeps memory in-memory and writes no transcript, and
//! the scripted engine loads no weights.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const HEAD: &str = "ZZ-HEAD-OF-THE-INPUT-ZZ";

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("tacet-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("work")).expect("a temp directory");
        Scratch(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The `~N tokens (estimate)` line `--show-prompt` prints under the dump. It is
/// computed from the wire text, so it is the size of what was SENT, not of what
/// the truncator measured.
fn sent_estimate(stderr: &str) -> usize {
    stderr
        .lines()
        .find_map(|l| {
            l.strip_prefix("--- ~")?
                .strip_suffix(" tokens (estimate) ---")?
                .parse()
                .ok()
        })
        .unwrap_or_else(|| panic!("no prompt dump on stderr:\n{stderr}"))
}

#[test]
fn a_long_message_is_cut_to_make_room_for_the_tools_not_sent_over_the_cap() {
    let scratch = Scratch::new("shell-budget");
    // About 7 000 tokens: over the cap with or without the tools, so the
    // truncator has to act whichever order it runs in, and what decides the
    // outcome is only whether the tools were counted.
    let message = format!(
        "{HEAD} {}",
        "and the quarterly budget review totals against the summary from last week ".repeat(400)
    );
    let out = Command::new(env!("CARGO_BIN_EXE_tacet"))
        .args(["chat", "--engine", "fake", "--script", "the answer."])
        .args(["--show-prompt", "--json", "--dir"])
        .arg(scratch.0.join("work"))
        .arg("--message")
        .arg(&message)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("TACET_MODEL")
        .env_remove("TACET_TOKENIZER")
        .env("HOME", &scratch.0)
        .env("TACET_HOME", &scratch.0)
        // Not a terminal and not a pipe with data: `--message` would read a
        // piped stdin as context and change the prompt under test.
        .stdin(Stdio::null())
        .output()
        .expect("the tacet binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "tacet failed:\n{stderr}");

    let cap = tacet_engine::CONTEXT_BUDGET - tacet_engine::GENERATION_SHARE;
    let sent = sent_estimate(&stderr);
    assert!(
        stderr.contains("<tools>"),
        "the pass was offered no tools, so there is nothing to budget:\n{stderr}"
    );
    assert!(
        sent <= cap,
        "the shell sent {sent} tokens into a {cap}-token prompt cap — the tools \
         were attached after the truncation measured the prompt"
    );

    let record: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("--json printed something else ({e}):\n{stdout}"));
    let reported = record["tokens"]["context"]
        .as_u64()
        .expect("tokens.context") as usize;
    assert_eq!(
        reported, sent,
        "the context size the shell reports is not the size of the prompt it \
         sent — the truncation report was taken before the prompt was finished"
    );

    assert!(
        !stderr[stderr.find("--- PROMPT").unwrap_or(0)..].contains(HEAD),
        "the head of an over-long input survived, so nothing was cut to make room"
    );
    assert!(
        stderr.contains("THE START OF YOUR INPUT WAS CUT"),
        "the input was cut without saying so:\n{}",
        &stderr[..stderr.len().min(400)]
    );
}
