//! Ctrl-C as a SIGNAL reaches the tool that is running (audit M4, the half the
//! key thread cannot see).
//!
//! On a terminal the turn indicator holds raw mode and turns the Ctrl-C KEY
//! into a cancel (the wave-2 fix, guarded in `chat.rs`). With no indicator —
//! `--json`, or stdout piped — Ctrl-C is a SIGINT, and its default action
//! killed tacet while the `shell` command it was waiting on, in its own process
//! group, ran on as an orphan. Driven here with a real SIGINT (`kill -INT`) at
//! the binary on the scripted engine; no model and no GPU are involved, and
//! `sleep` is the only program on the addon's list.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("tacet-sigint-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("work")).expect("a temp directory");
        Scratch(p)
    }

    fn home(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tacet(s: &Scratch) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_tacet"));
    c.env_remove("XDG_CONFIG_HOME")
        .env_remove("TACET_MODEL")
        .env_remove("TACET_TOKENIZER")
        .env("HOME", s.home())
        .env("TACET_HOME", s.home())
        .current_dir(s.home().join("work"));
    c
}

fn install_sleep(s: &Scratch) {
    let mut child = tacet(s)
        .args(["addon", "install", "shell"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the tacet binary runs");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"sleep\n\ny\n")
        .expect("answers the installer");
    let out = child.wait_with_output().expect("the installer finishes");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("the `shell` tool is in the catalog"),
        "the shell addon did not install:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn interrupt(pid: u32) {
    let status = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .expect("kill runs");
    assert!(status.success(), "the SIGINT was not delivered");
}

/// Is a process whose command line contains `needle` still alive.
fn still_running(needle: &str) -> bool {
    Command::new("pgrep")
        .args(["-f", needle])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// RED BEFORE THE FIX: the process died of the signal (no exit code, no JSON
/// line), and `sleep 7.31` was still running afterwards.
#[test]
fn a_sigint_during_a_tool_ends_the_tool_and_not_the_shell() {
    let s = Scratch::new("tool");
    install_sleep(&s);
    // A duration nothing else on the machine is sleeping for, so `pgrep`
    // finds this test's command and only this test's.
    let marker = "7.31";
    let started = Instant::now();
    let child = tacet(&s)
        .args(["chat", "--engine", "fake", "--json", "--message", "wait"])
        .args([
            "--script",
            &format!(r#"{{"name":"shell","arguments":{{"command":"sleep","args":["{marker}"]}}}}"#),
            "--script",
            "DONE.",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the tacet binary runs");

    let needle = format!("sleep {marker}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !still_running(&needle) {
        assert!(Instant::now() < deadline, "the tool never started");
        std::thread::sleep(Duration::from_millis(50));
    }
    interrupt(child.id());
    let out = child.wait_with_output().expect("tacet finishes");
    let took = started.elapsed();
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        out.status.signal().is_none(),
        "the SIGINT killed the shell instead of cancelling the turn: {:?}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.lines().any(|l| l.starts_with('{')),
        "the cancelled turn wrote no JSON line:\n{stdout}"
    );
    assert!(
        took < Duration::from_secs(6),
        "the turn waited {took:?} for a tool it was told to stop"
    );
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !still_running(&needle),
        "`{needle}` outlived the Ctrl-C that cancelled its turn"
    );
}

/// Between turns a SIGINT is what it always was: the end of the process. A
/// handler that swallowed every SIGINT would leave a piped session that can
/// only be killed.
#[test]
fn a_sigint_between_turns_still_ends_the_process() {
    let s = Scratch::new("idle");
    let mut child = tacet(&s)
        .args(["chat", "--engine", "fake"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the tacet binary runs");
    // The pipe stays open and empty: the shell sits waiting for a message.
    let stdin = child.stdin.take();
    std::thread::sleep(Duration::from_millis(800));
    interrupt(child.id());
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "an idle session ignored Ctrl-C"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    drop(stdin);
    assert_eq!(status.signal(), Some(2), "{status:?}");
}
