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

/// Seven seconds and six digits of fraction no other run will draw: long enough
/// to still be sleeping when the SIGINT lands, short enough that a failure
/// leaves nothing behind for long.
fn unique_seconds() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let mixed = (u64::from(std::process::id()) * 1_000_003 + nanos) % 1_000_000;
    format!("7.{mixed:06}")
}

/// RED BEFORE THE FIX: the process died of the signal (no exit code, no JSON
/// line), and its `sleep` was still running afterwards.
#[test]
fn a_sigint_during_a_tool_ends_the_tool_and_not_the_shell() {
    let s = Scratch::new("tool");
    install_sleep(&s);
    // A duration nothing else on the machine is sleeping for, so `pgrep`
    // finds this test's command and only this test's. It was the constant
    // `7.31`, and that was not true: a `sleep 7.31` left by an earlier run, or
    // by a second checkout's tests running beside this one, satisfied the
    // "started" wait and then the "outlived" check, and the test failed
    // (measured: once in a parallel workspace run, once in 3 quick reruns,
    // then 14 of 14 with no stale `sleep` alive). Drawn from this process
    // and the clock, it can only be this run's.
    let marker = unique_seconds();
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

/// Ctrl-C as a KEY at a real terminal (audit M4, the form the audit measured).
///
/// The tests above send the signal with `kill -INT`, and the indicator's key
/// thread has only a unit test (`ui::tests::ctrl_c_marks_…`). Neither is what
/// the audit did: it pressed ctrl-c at a pty during `shell sleep 9` and watched
/// the chip read `exit 0 · 9018 ms`. This drives the binary through
/// `script(1)` — the pty harness `consent_reaches_the_person.rs` already uses,
/// no crate — types `\x03` while the tool sleeps, and holds three things: the
/// chip comes back well before the sleep would have ended, the `sleep` is gone,
/// and the shell answers the next message.
///
/// WHICH PATH CARRIES THE KEY IS NOT ASSERTED. With the indicator in raw mode
/// the byte is a key and `swallow_key` sets `CANCEL`; with the terminal cooked
/// the pty's line discipline turns it into a SIGINT and `interrupt::on_sigint`
/// does. Either is the fix; what the person sees is what is held. Both end in
/// the tool's context watching `CANCEL` (`chat::session_context`).
///
/// RED WITHOUT IT (11 Oct 2026, this Mac): with `session_context` building a
/// plain `ToolContext::new`, the screen showed `(cancelling…)` at once and then
/// `shell · sleep · exit 0 · 9554 ms` — the audit's defect, byte for byte — and
/// the "came back early" assertion failed at 9.17 s.
///
/// macOS only: `script -q /dev/null cmd…` is the BSD spelling; util-linux's is
/// different and has not been run here, so it is not claimed.
#[cfg(target_os = "macos")]
mod at_a_terminal {
    use super::*;
    use std::io::Read;
    use std::sync::{Arc, Mutex};

    /// Nine seconds — the audit's `sleep 9` — and a fraction no other run draws.
    fn nine_seconds() -> String {
        format!("9.{}", &unique_seconds()[2..])
    }

    #[test]
    fn ctrl_c_typed_during_a_tool_stops_it_and_the_shell_lives_on() {
        let s = Scratch::new("pty");
        install_sleep(&s);
        let marker = nine_seconds();
        let call =
            format!(r#"{{"name":"shell","arguments":{{"command":"sleep","args":["{marker}"]}}}}"#);

        let mut c = Command::new("/usr/bin/script");
        c.args(["-q", "/dev/null", env!("CARGO_BIN_EXE_tacet")])
            .args(["chat", "--engine", "fake"])
            .args(["--script", &call])
            .args(["--script", "AFTER-STOP.", "--script", "STILL-HERE."])
            .args(["--script", "STILL-HERE."]);
        let mut child = c
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("TACET_MODEL")
            .env_remove("TACET_TOKENIZER")
            .env("HOME", s.home())
            .env("TACET_HOME", s.home())
            .current_dir(s.home().join("work"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("script(1) runs");
        let mut input = child.stdin.take().expect("stdin");
        let mut stdout = child.stdout.take().expect("stdout");
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&output);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock().expect("output").extend_from_slice(&buf[..n]);
            }
        });
        let text = || String::from_utf8_lossy(&output.lock().expect("output")).into_owned();
        let wait_for = |needle: &str, from: usize, secs: u64| -> Option<usize> {
            let deadline = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < deadline {
                let t = text();
                if let Some(at) = t.get(from..).and_then(|t| t.find(needle)) {
                    return Some(from + at + needle.len());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            None
        };
        let mut type_text = |keys: &str| {
            input
                .write_all(keys.as_bytes())
                .expect("types into the pty");
            input.flush().expect("flush");
        };

        let ready = wait_for("/help", 0, 20)
            .unwrap_or_else(|| panic!("the shell never started:\n{}", text()));
        type_text("wait\r");

        let needle = format!("sleep {marker}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !still_running(&needle) {
            assert!(
                Instant::now() < deadline,
                "the tool never started:\n{}",
                text()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        // Let the sleep be under way, the way a person notices and reacts.
        std::thread::sleep(Duration::from_millis(500));
        let pressed = Instant::now();
        let before = text().len().max(ready);
        type_text("\x03");

        // The turn ends: the indicator's "(stopped)", or the chip's own failure.
        let back = wait_for("stopped", before, 12);
        let took = pressed.elapsed();
        let screen = text();
        let back = back
            .unwrap_or_else(|| panic!("the turn never said it stopped after ctrl-c:\n{screen}"));
        assert!(
            took < Duration::from_secs(5),
            "ctrl-c reached the tool only after {took:?} — the {marker}s sleep \
             ran out instead of being stopped:\n{screen}"
        );
        assert!(
            !screen[before..].contains(" exit 0"),
            "the stopped tool reported a clean exit:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !still_running(&needle),
            "`{needle}` outlived the ctrl-c typed at its turn"
        );

        type_text("still there?\r");
        assert!(
            wait_for("STILL-HERE.", back, 15).is_some(),
            "the shell did not answer after ctrl-c — the key ended the program, \
             not the turn:\n{}",
            text()
        );
        type_text("/exit\r");
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}
