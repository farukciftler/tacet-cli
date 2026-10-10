//! A consent question reaches the person at the terminal — and only them.
//!
//! WHY THIS RUNS THE BINARY. Both defects lived in the space between modules,
//! where no unit test was looking:
//!
//! * AUDIT H1 (RG-03). The approval prompt is asked while the turn indicator is
//!   running, and the indicator held the terminal: raw mode on, a thread
//!   reading every key, and a frame every 100 ms that began with `\r\x1b[2K`.
//!   Measured on this binary under a pty before the fix (10 Oct 2026, macOS):
//!   the question was printed, the next frame erased it, `y⏎` was swallowed,
//!   and the shell sat there until it was killed. The gate, the indicator and
//!   the executor were each correct on their own.
//! * AUDIT L31. With no `--message` the shell reads its MESSAGES from a piped
//!   stdin, and the approval prompt read its answer from the same pipe:
//!   `printf 'run it\ny\nsecond message\n' | tacet chat` approved a tainted
//!   `shell` call with the `y` meant as a message.
//!
//! THE SCENARIO IS THE SAME IN BOTH: the scripted engine calls `shell echo`
//! twice in one turn. The first call runs (a clean session asks nothing) and
//! taints the session — `shell` reads the machine — so the second call is an
//! external tool in a tainted session, which is exactly when the approval gate
//! asks. `echo` is the only program on the addon's list, and the
//! addon is installed into a temp `TACET_HOME`, so nothing of the user's is
//! read or touched and nothing leaves the machine.
//!
//! THE PTY IS `script(1)`, NOT A CRATE: the repository takes no new dependency
//! for a test. The pty tests run on macOS, where `script -q /dev/null cmd…` is
//! the BSD form and is always installed; util-linux spells it differently and
//! that spelling has not been run here, so it is not claimed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("tacet-{tag}-{}", std::process::id()));
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

fn tacet(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_tacet"));
    c.env_remove("XDG_CONFIG_HOME")
        .env_remove("TACET_MODEL")
        .env_remove("TACET_TOKENIZER")
        .env("HOME", home)
        .env("TACET_HOME", home);
    c
}

/// Installs the `shell` addon with `echo` as its only command, through the
/// real installer (piped, so it reads the list and the confirmation from
/// stdin).
fn install_echo(scratch: &Scratch) {
    let mut child = tacet(scratch.home())
        .args(["addon", "install", "shell"])
        .current_dir(scratch.work())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the tacet binary runs");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"echo\n\ny\n")
        .expect("answers the installer");
    let out = child.wait_with_output().expect("the installer finishes");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("the `shell` tool is in the catalog"),
        "the shell addon did not install, so there is no gated call to test:\n{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The engine's script: call `echo` (runs, taints), call it again (gated),
/// then answer — and one answer for every later message.
const SCRIPT: [&str; 5] = [
    r#"{"name":"shell","arguments":{"command":"echo","args":["FIRST"]}}"#,
    r#"{"name":"shell","arguments":{"command":"echo","args":["SECOND"]}}"#,
    "ANSWER-ONE.",
    "ANSWER-TWO.",
    "ANSWER-THREE.",
];

fn script_args(c: &mut Command) {
    c.args(["chat", "--engine", "fake"]);
    for step in SCRIPT {
        c.args(["--script", step]);
    }
}

/// AUDIT L31: a `y` on a pipe is the next message, never consent.
///
/// RED BEFORE THE FIX: the approval read `y` off the pipe — two `exit 0`
/// chips, no `not sent`, and only two messages ever reached the engine, so
/// `ANSWER-THREE.` was never printed.
#[test]
fn a_piped_y_is_a_message_not_consent() {
    let scratch = Scratch::new("consent-pipe");
    install_echo(&scratch);

    let mut c = tacet(scratch.home());
    script_args(&mut c);
    let mut child = c
        .current_dir(scratch.work())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the tacet binary runs");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"run it\ny\nsecond message\n")
        .expect("feeds the messages");
    let out = child.wait_with_output().expect("the shell finishes");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert_eq!(
        all.matches("exit 0").count(),
        1,
        "the gated second call ran — a piped line was taken as consent:\n{all}"
    );
    assert!(
        all.contains("not sent"),
        "the gated call was neither run nor refused:\n{all}"
    );
    assert!(
        !all.contains("Do you allow it?"),
        "a question was put to a pipe, where nobody can read it:\n{all}"
    );
    assert!(
        all.contains("ANSWER-THREE."),
        "the `y` line was swallowed instead of being sent as a message:\n{all}"
    );
}

/// AUDIT L31, the catalog half: a piped session is not handed `db_write`.
///
/// `db_write` asks before every change; on a pipe there is nobody to ask, so
/// the tool could only ever be refused, and a tool that can only refuse is a
/// trap that costs the model a turn (see `session_catalog`). The pipe tests
/// above cannot see this — `Prompt::open` refuses on its own, so the gate
/// verdict is the same either way — which is why reverting the chat layer's
/// `can_ask` to plain `interactive` stayed green until this test.
///
/// THE CONTROL IS `tacet tools`, which reports the catalog an interactive
/// session gets. If it does not list `db_write` either, this machine cannot
/// build the tool at all (no `sqlite3` that passes the write-lock measurement)
/// and the absence below would prove nothing. macOS ships `/usr/bin/sqlite3`,
/// so there that is a failure; elsewhere the test says so and stops.
///
/// RED BEFORE THE FIX: `/tools` in the piped session listed `db_write`.
#[test]
fn a_piped_session_is_not_handed_db_write() {
    let scratch = Scratch::new("consent-pipe-db");
    let mut child = tacet(scratch.home())
        .args(["addon", "install", "db"])
        .current_dir(scratch.work())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the tacet binary runs");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"app.db\n\ny\n")
        .expect("answers the installer");
    let out = child.wait_with_output().expect("the installer finishes");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("the db addon is installed and open"),
        "the db addon did not install:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let listed = |text: &str, name: &str| {
        text.lines()
            .any(|l| l.split_whitespace().next() == Some(name))
    };

    let control = tacet(scratch.home())
        .arg("tools")
        .current_dir(scratch.work())
        .output()
        .expect("tacet tools runs");
    let control = String::from_utf8_lossy(&control.stdout).into_owned();
    if !listed(&control, "db_write") {
        if cfg!(target_os = "macos") {
            panic!(
                "`tacet tools` does not list db_write, so this test measures nothing:\n{control}"
            );
        }
        eprintln!("db_write cannot be built on this machine; the piped-catalog check is skipped");
        return;
    }

    let mut c = tacet(scratch.home());
    c.args(["chat", "--engine", "fake", "--script", "unused"]);
    let mut child = c
        .current_dir(scratch.work())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the tacet binary runs");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"/tools\n")
        .expect("feeds the command");
    let out = child.wait_with_output().expect("the shell finishes");
    let tools = String::from_utf8_lossy(&out.stdout);
    assert!(
        listed(&tools, "db"),
        "`/tools` did not list the catalog, so its silence about db_write means nothing:\n{tools}"
    );
    assert!(
        !listed(&tools, "db_write"),
        "a piped session was handed db_write, a tool nobody can approve:\n{tools}"
    );
}

#[cfg(target_os = "macos")]
mod under_a_pty {
    use super::*;
    use std::io::Read;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// The shell, running on a pty, with a handle to type into it and
    /// everything it has written so far.
    struct Session {
        child: std::process::Child,
        input: std::process::ChildStdin,
        output: Arc<Mutex<Vec<u8>>>,
    }

    impl Session {
        fn start(scratch: &Scratch) -> Session {
            let mut c = Command::new("/usr/bin/script");
            // `-q /dev/null`: no banner, no typescript file. Everything after is
            // the command to run on the pty.
            c.args(["-q", "/dev/null", env!("CARGO_BIN_EXE_tacet")]);
            script_args(&mut c);
            let mut child = c
                .env_remove("XDG_CONFIG_HOME")
                .env_remove("TACET_MODEL")
                .env_remove("TACET_TOKENIZER")
                .env("HOME", scratch.home())
                .env("TACET_HOME", scratch.home())
                .current_dir(scratch.work())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("script(1) runs");
            let input = child.stdin.take().expect("stdin");
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
            Session {
                child,
                input,
                output,
            }
        }

        fn text(&self) -> String {
            String::from_utf8_lossy(&self.output.lock().expect("output")).into_owned()
        }

        /// Waits for `needle` to appear after byte `from`; the byte offset just
        /// past it, or `None` on timeout.
        fn wait_for(&self, needle: &str, from: usize, secs: u64) -> Option<usize> {
            let deadline = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < deadline {
                let text = self.text();
                if let Some(at) = text.get(from..).and_then(|t| t.find(needle)) {
                    return Some(from + at + needle.len());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            None
        }

        fn type_text(&mut self, keys: &str) {
            self.input
                .write_all(keys.as_bytes())
                .expect("types into the pty");
            self.input.flush().expect("flush");
        }
    }

    impl Drop for Session {
        /// A shell that is still running after the test has its PTY taken away:
        /// killing `script(1)` closes the master, and the shell gets the hangup
        /// a closed terminal sends. The shell itself is never signalled — the
        /// scripted engine holds no GPU, but the habit is the point.
        fn drop(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Drives one turn up to the approval question and answers it with `keys`.
    /// Returns the whole screen once `until` has appeared.
    fn answer_the_question(tag: &str, keys: &str, until: &str) -> String {
        answer_then_talk(tag, keys, until, None)
    }

    /// `answer_the_question`, and then — if `next` is given — one more message
    /// typed at the shell and the text its answer must contain. A shell that
    /// answers it is still alive; one that the answer killed cannot.
    fn answer_then_talk(tag: &str, keys: &str, until: &str, next: Option<(&str, &str)>) -> String {
        let scratch = Scratch::new(tag);
        install_echo(&scratch);
        let mut session = Session::start(&scratch);

        let ready = session
            .wait_for("/help", 0, 20)
            .unwrap_or_else(|| panic!("the shell never started:\n{}", session.text()));
        session.type_text("run it\r");

        let asked = session
            .wait_for("Do you allow it? [y/N] ", ready, 20)
            .unwrap_or_else(|| panic!("the approval was never asked:\n{}", session.text()));

        // THE QUESTION STAYS ON SCREEN. Before the fix the spinner's next frame
        // (`\r\x1b[2K`, every 100 ms) erased it; four frames' worth of waiting
        // is enough to see one.
        std::thread::sleep(Duration::from_millis(400));
        let after_question = session.text()[asked..].to_string();
        assert!(
            !after_question.contains("\x1b[2K"),
            "something erased the line the question is on: {after_question:?}"
        );

        session.type_text(keys);
        let screen = match session.wait_for(until, asked, 15) {
            Some(_) => session.text(),
            None => panic!(
                "typing {keys:?} at the approval did not reach the gate — the \
                 question is open and the answer went nowhere:\n{}",
                session.text()
            ),
        };
        let screen = match next {
            None => screen,
            Some((message, reply)) => {
                let from = screen.len();
                session.type_text(message);
                match session.wait_for(reply, from, 15) {
                    Some(_) => session.text(),
                    None => panic!(
                        "the shell did not answer the next message after {keys:?} — \
                         the answer to the question ended the session:\n{}",
                        session.text()
                    ),
                }
            }
        };
        session.type_text("/exit\r");
        screen
    }

    /// AUDIT H1 / RG-03: `y⏎` at the approval APPROVES — the second `echo`
    /// runs. `\r` is what a terminal's enter key sends; the pty's line
    /// discipline is what turns it into the newline a cooked read ends on, so
    /// this is the real key, not a stand-in.
    ///
    /// RED BEFORE THE FIX: the shell hung at the question; `ANSWER-ONE.` never
    /// appeared.
    #[test]
    fn yes_typed_at_the_approval_approves() {
        let screen = answer_the_question("consent-pty-yes", "y\r", "ANSWER-ONE.");
        assert_eq!(
            screen.matches("exit 0").count(),
            2,
            "the approved call did not run:\n{screen}"
        );
        assert!(
            !screen.contains("not sent"),
            "a typed `y` was read as a refusal:\n{screen}"
        );
    }

    /// The other answer reaches the gate too — and is a refusal, not a hang.
    #[test]
    fn no_typed_at_the_approval_refuses() {
        let screen = answer_the_question("consent-pty-no", "n\r", "ANSWER-ONE.");
        assert!(
            screen.contains("not sent"),
            "a typed `n` did not refuse:\n{screen}"
        );
        assert_eq!(
            screen.matches("exit 0").count(),
            1,
            "the refused call ran:\n{screen}"
        );
    }

    /// Asserts the gated call was refused — `not sent`, and only the first
    /// `echo` ran.
    fn refused(screen: &str, keys: &str) {
        assert!(
            screen.contains("not sent"),
            "{keys:?} at the approval did not refuse:\n{screen}"
        );
        assert_eq!(
            screen.matches("exit 0").count(),
            1,
            "{keys:?} at the approval let the gated call run:\n{screen}"
        );
    }

    /// ESC THEN ENTER IS A NO (review of H1). The pty delivers the three keys
    /// together, as a quick typist would; crossterm reads `\x1b\r` as
    /// Alt+Enter.
    ///
    /// RED BEFORE THE FIX: two `exit 0` chips, no `not sent` — Alt+Enter was an
    /// enter, the line was `y`, and the call was approved.
    #[test]
    fn escape_then_enter_at_the_approval_refuses() {
        let screen = answer_the_question("consent-pty-esc", "y\x1b\r", "ANSWER-ONE.");
        refused(&screen, "y Esc Enter");
    }

    /// CTRL-C DECLINES THE QUESTION AND THE SHELL LIVES ON — the turn's rule,
    /// "ctrl-c cancels the turn, never the program", holds at a question too.
    ///
    /// RED BEFORE THE FIX: with the answer read cooked, `\x03` was a SIGINT;
    /// `script(1)` exited with status 2 and `ANSWER-ONE.` never appeared.
    #[test]
    fn ctrl_c_at_the_approval_refuses_and_the_shell_survives() {
        let screen = answer_then_talk(
            "consent-pty-ctrlc",
            "\x03",
            "ANSWER-ONE.",
            Some(("still there?\r", "ANSWER-TWO.")),
        );
        refused(&screen, "ctrl-c");
    }

    /// CTRL-D DECLINES.
    ///
    /// RED BEFORE THE FIX: cooked mode echoed `^D` and the question kept
    /// waiting; `ANSWER-ONE.` never appeared.
    #[test]
    fn ctrl_d_at_the_approval_refuses() {
        let screen = answer_the_question("consent-pty-ctrld", "\x04", "ANSWER-ONE.");
        refused(&screen, "ctrl-d");
    }
}
