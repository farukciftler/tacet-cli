//! What the session promises about the user's data, held on the binary.
//!
//! Each of these defects lived between modules — the gate, the turn loop, the
//! transcript, the receipt log, the update check — where every unit test was
//! green, so each is driven here through the real `tacet` on the scripted
//! engine, in a temp `TACET_HOME`. Nothing of the user's is read or touched,
//! and nothing leaves the machine: the only tool that runs is `shell` with
//! `echo` on its list, and the update check is answered from a cache written
//! here.
//!
//! * AUDIT H2-b. Personal data that reached the prompt without a tool — a
//!   pipe, a resumed transcript, a remembered note — never tainted the
//!   session, so the approval gate in front of every outbound tool stayed
//!   open. `shell` is one of those tools (`EXTERNAL_TOOLS`), and with nobody
//!   to ask the gate answers no: "not sent" where the call used to run.
//! * RG-20. A session continued from another agent (`--from`, `/resume-from`)
//!   is the user's own work and text another program wrote: it taints from
//!   the start, through the same gate. The library function that marks it had
//!   a unit test; nothing checked that `chat` called it, and with the call
//!   disabled every test stayed green.
//! * AUDIT L29. A chain of `--continue` runs kept only its last link.
//! * AUDIT L27 / RG-32. The receipt log kept every chip's words and
//!   `sessions --purge` left it behind.
//! * AUDIT D7. The daily update check ran after `--message` runs and with the
//!   output piped, which the README says it never does.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tacet_tools::sessions::AgentKind;
use tacet_web::addon;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("tacet-promise-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("work")).expect("a temp directory");
        Scratch(p)
    }

    fn home(&self) -> &Path {
        &self.0
    }

    fn work(&self) -> PathBuf {
        self.0.join("work")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tacet(scratch: &Scratch) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_tacet"));
    c.env_remove("XDG_CONFIG_HOME")
        .env_remove("TACET_MODEL")
        .env_remove("TACET_TOKENIZER")
        .env_remove("TACET_TRACE_DUMP")
        // The agent-session readers fall back to these before the registry;
        // removed so no test here can look at a real agent's folder.
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env("HOME", scratch.home())
        .env("TACET_HOME", scratch.home())
        .current_dir(scratch.work());
    c
}

/// Runs `tacet` with `stdin` written to it (or `/dev/null` for `None`) and
/// waits for it.
fn run(mut c: Command, stdin: Option<&str>) -> Output {
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    match stdin {
        None => {
            c.stdin(Stdio::null());
            c.output().expect("the tacet binary runs")
        }
        Some(text) => {
            c.stdin(Stdio::piped());
            let mut child = c.spawn().expect("the tacet binary runs");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(text.as_bytes())
                .expect("feeds stdin");
            child.wait_with_output().expect("tacet finishes")
        }
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Installs the `shell` addon with `echo` as its only command, through the
/// real installer.
fn install_echo(scratch: &Scratch) {
    let mut c = tacet(scratch);
    c.args(["addon", "install", "shell"]);
    let out = run(c, Some("echo\n\ny\n"));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("the `shell` tool is in the catalog"),
        "the shell addon did not install, so there is no gated call to test:\n{}",
        text(&out)
    );
}

/// The scripted turn: one `shell echo`, then an answer.
fn echo_turn(c: &mut Command) {
    c.args(["chat", "--engine", "fake"]);
    c.args([
        "--script",
        r#"{"name":"shell","arguments":{"command":"echo","args":["OUTBOUND"]}}"#,
        "--script",
        "DONE.",
    ]);
}

fn assert_ran(all: &str, why: &str) {
    assert!(
        all.contains("exit 0") && !all.contains("not sent"),
        "{why}: the echo did not run in a clean session, so the gated runs \
         below would prove nothing:\n{all}"
    );
}

fn assert_refused(all: &str, why: &str) {
    assert!(
        !all.contains("exit 0"),
        "{why}: the outbound call ran in a session holding personal data:\n{all}"
    );
    assert!(
        all.contains("not sent"),
        "{why}: the call was neither run nor refused:\n{all}"
    );
}

/// THE CONTROL: with nothing personal in the prompt, `shell echo` runs. Every
/// "not sent" below is measured against this.
#[test]
fn a_clean_one_shot_runs_its_outbound_call() {
    let s = Scratch::new("clean");
    install_echo(&s);
    let mut c = tacet(&s);
    echo_turn(&mut c);
    c.args(["--message", "say something"]);
    assert_ran(&text(&run(c, None)), "control");
}

/// H2-b, the pipe. RED BEFORE THE FIX: `exit 0`, the piped secret's session
/// sent its call out.
#[test]
fn piped_input_taints_the_session_from_the_start() {
    let s = Scratch::new("pipe");
    install_echo(&s);
    let mut c = tacet(&s);
    echo_turn(&mut c);
    c.args(["--message", "summarise this"]);
    assert_refused(
        &text(&run(c, Some("AWS_SECRET_ACCESS_KEY=abc123\n"))),
        "piped stdin",
    );
}

/// H2-b, a replayed transcript. RED BEFORE THE FIX: `exit 0`.
#[test]
fn a_resumed_session_taints_the_session_from_the_start() {
    let s = Scratch::new("resume");
    install_echo(&s);
    let sessions = s.home().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join("20261001T000000Z-1.jsonl"),
        "{\"at\":1,\"role\":\"user\",\"text\":\"my passport is U1234567\"}\n\
         {\"at\":2,\"role\":\"assistant\",\"text\":\"Noted.\"}\n",
    )
    .unwrap();
    let mut c = tacet(&s);
    echo_turn(&mut c);
    c.args(["--message", "go on", "--session", "20261001T000000Z-1"]);
    let all = text(&run(c, None));
    assert!(
        all.contains("2 earlier turns loaded"),
        "the session was not resumed, so nothing was replayed:\n{all}"
    );
    assert_refused(&all, "--session replay");
}

/// H2-b, a remembered note. An interactive session (messages on a pipe, so
/// nobody to ask) whose message matches a stored note. RED BEFORE THE FIX:
/// the note was in the prompt and the call still ran.
#[test]
fn an_injected_memory_note_taints_the_turn_it_enters() {
    let s = Scratch::new("memory");
    install_echo(&s);
    std::fs::write(
        s.home().join("memory.json"),
        r#"{"version":1,"next_id":2,"notes":[{"id":1,"text":"The user passport number is U1234567.","kind":"identity","keys":["passport"],"created":1,"active":true}]}"#,
    )
    .unwrap();
    let mut c = tacet(&s);
    echo_turn(&mut c);
    c.arg("--show-prompt");
    let all = text(&run(c, Some("what is my passport number\n")));
    assert!(
        all.contains("U1234567"),
        "the note never reached the prompt, so there was nothing to taint:\n{all}"
    );
    assert_refused(&all, "memory injection");
}

/// The Claude Code session in the readers' own fixture tree.
const CLAUDE_SESSION: &str = "11111111-0000-4000-8000-111111111111";

fn claude_reference() -> String {
    format!("{}:{CLAUDE_SESSION}", AgentKind::ClaudeCode.slug())
}

/// Opens `agent-sessions` beside the shell addon `install_echo` wrote, with
/// the readers' fixture tree as its only folder — built from the slug, so no
/// literal here names an agent program (`network_monopoly`).
fn open_agent_sessions(scratch: &Scratch) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tacet-tools/tests/fixtures/sessions")
        .join(AgentKind::ClaudeCode.slug())
        .canonicalize()
        .expect("the fixture tree exists");
    let path = scratch.home().join("addons.json");
    let mut record = addon::read_from_path(&path).expect("the shell addon's registry");
    record.add(
        addon::Addon::new(addon::AGENT_SESSIONS, addon::AGENT_SESSIONS)
            .with_setting(addon::ROOTS_KEY, fixtures.display().to_string()),
    );
    addon::write_to_path(&path, &record).expect("the registry is written");
}

/// RG-20, `--from`. The import is the only personal input: no pipe, no
/// `--session`, no note. RED BEFORE THE FIX (the review broke the call to
/// `mark_imported` on this path): `exit 0`.
#[test]
fn an_imported_agent_session_taints_the_session_from_the_start() {
    let s = Scratch::new("from");
    install_echo(&s);
    open_agent_sessions(&s);
    let mut c = tacet(&s);
    echo_turn(&mut c);
    c.args(["--from", &claude_reference(), "--message", "go on"]);
    let all = text(&run(c, None));
    assert!(
        all.contains("continuing Claude Code session"),
        "the session was not imported, so nothing was there to taint:\n{all}"
    );
    assert_refused(&all, "--from");
}

/// RG-20, `/resume-from` inside the shell — the handler that changes the
/// session in place, which had no test at all. Messages on a pipe (nobody to
/// ask, the same as the memory test): first the CONTROL, a plain message
/// whose echo runs; then the same session continues another agent's work and
/// the next message's echo must be refused. RED BEFORE THE FIX: the second
/// echo ran.
#[test]
fn resume_from_in_the_shell_taints_the_turns_after_it() {
    let s = Scratch::new("resume-from");
    install_echo(&s);
    open_agent_sessions(&s);

    let mut control = tacet(&s);
    echo_turn(&mut control);
    assert_ran(&text(&run(control, Some("say something\n"))), "piped shell");

    let mut c = tacet(&s);
    echo_turn(&mut c);
    c.arg("--show-prompt");
    let all = text(&run(
        c,
        Some(&format!("/resume-from {}\nwhat next\n", claude_reference())),
    ));
    assert!(
        all.contains("continuing Claude Code session"),
        "/resume-from did not import, so nothing was there to taint:\n{all}"
    );
    assert!(
        all.contains("Goal (first request): Fix the date rollover"),
        "the summary never reached the next question's prompt:\n{all}"
    );
    // RG-18 second review: the in-shell verb stores the transcript too.
    assert!(
        all.contains("source_ref=agent-session#"),
        "/resume-from sent a summary with no reference to the transcript:\n{all}"
    );
    assert_refused(&all, "/resume-from");
}

/// L29: three `--continue` runs; the third sees all four earlier turns.
/// RED BEFORE THE FIX: "(continuing — 2 earlier turns loaded)".
#[test]
fn a_chain_of_continues_keeps_the_first_turn() {
    let s = Scratch::new("chain");
    for (i, message) in ["first", "second", "third"].iter().enumerate() {
        let mut c = tacet(&s);
        c.args(["chat", "--engine", "fake", "--script", "ok."]);
        c.args(["--continue", "--message", message]);
        let all = text(&run(c, None));
        let expected = format!("(continuing — {} earlier turns loaded)", i * 2);
        if i == 0 {
            assert!(all.contains("no stored session to continue"), "{all}");
        } else {
            assert!(
                all.contains(&expected),
                "run {} did not load the whole conversation (wanted {expected:?}):\n{all}",
                i + 1
            );
        }
    }
}

/// L27 / RG-32 on the binary: the receipt log holds no chip words, and
/// `sessions --purge` takes it. RED BEFORE THE FIX: the receipt line carried
/// the echoed argument, and the purge left `receipts.jsonl` in place.
#[test]
fn receipts_hold_no_words_and_the_purge_takes_them() {
    let s = Scratch::new("receipts");
    install_echo(&s);
    let mut c = tacet(&s);
    c.args(["chat", "--engine", "fake", "--continue"]);
    c.args([
        "--script",
        r#"{"name":"shell","arguments":{"command":"echo","args":["PASSPORT-U1234567"]}}"#,
        "--script",
        "DONE.",
        "--message",
        "echo it",
    ]);
    let all = text(&run(c, None));
    assert_ran(&all, "the receipt run");

    let log = s.home().join("receipts.jsonl");
    let raw = std::fs::read_to_string(&log).expect("a receipt was written");
    assert!(
        !raw.contains("U1234567") && !raw.contains("echo"),
        "the receipt log carries the chip's words:\n{raw}"
    );

    let mut c = tacet(&s);
    c.args(["sessions", "--purge"]);
    let out = run(c, None);
    assert!(out.status.success(), "{}", text(&out));
    assert!(!log.exists(), "the purge left the receipt log behind");
    assert!(
        !s.home().join("receipts.head").exists(),
        "the purge left the receipt head behind"
    );
    assert!(
        !s.home().join("sessions").exists(),
        "the purge left the sessions"
    );
}

/// D7: with the check on and a newer version in a fresh cache, neither a
/// `--message` run nor an interactive session on pipes prints the notice —
/// so neither would have asked GitHub either. RED BEFORE THE FIX: both
/// printed "update available".
#[test]
fn a_script_is_never_told_about_updates() {
    let s = Scratch::new("update");
    let mut c = tacet(&s);
    c.args(["config", "set", "update.check", "on"]);
    let out = run(c, None);
    assert!(out.status.success(), "{}", text(&out));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        s.home().join("update.json"),
        format!("{{\"checked_at\":{now},\"latest\":\"v999.0.0\"}}\n"),
    )
    .unwrap();

    let mut one_shot = tacet(&s);
    one_shot.args(["chat", "--engine", "fake", "--message", "hi"]);
    let all = text(&run(one_shot, None));
    assert!(
        !all.contains("update available"),
        "a --message run reached the update check:\n{all}"
    );

    let mut piped = tacet(&s);
    piped.args(["chat", "--engine", "fake"]);
    let all = text(&run(piped, Some("hi\n")));
    assert!(all.contains("Tacet"), "the session did not answer:\n{all}");
    assert!(
        !all.contains("update available"),
        "a session with its output piped reached the update check:\n{all}"
    );
}
