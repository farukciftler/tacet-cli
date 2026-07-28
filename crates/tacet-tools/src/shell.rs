//! `shell` — runs ONE allowlisted program with an ARGUMENT VECTOR.
//!
//! WHY IT EXISTS: `run_code` runs inside a shield whose whole point is that it
//! sees no network and no file of this device (see `run_code.rs`). That is
//! correct and deliberate, and it is also why a terminal assistant cannot answer
//! "run the tests" — the one request a terminal assistant is asked most.
//!
//! WHY AN ADDON: this tool breaks the default promise of the product. Everything
//! else either stays inside the working directory or stays inside the shield;
//! this one starts a REAL process with the user's own environment. So it is
//! NOT IN THE CATALOG unless the user installed the addon AND left it open (the
//! same gate `web_search` sits behind, `tacet_web::addon`). With no addon there
//! is no tool to call, no grammar generated for it and no runtime check to
//! forget — absence is the strongest gate there is.
//!
//! THE FOUR THINGS THAT MAKE IT SURVIVABLE, in the order they are hit:
//!
//! 1. THERE IS NO SHELL. `Command::new(program).args(argv)` — no `sh -c`, no
//!    string concatenation, no glob expansion, no variable substitution. A model
//!    that writes `; rm -rf ~` produces an ARGUMENT with that text in it, not a
//!    second command. Injection is closed STRUCTURALLY; there is no text filter
//!    here, and none should ever be added — a filter is a list of the attacks
//!    somebody thought of.
//! 2. THE PROGRAM IS A CLOSED SET. The allowlist the user gave at install time
//!    becomes the `command` field's `Choice` schema, so the grammar cannot even
//!    GENERATE a program outside it (gate 2 of the four gates), and `run` checks
//!    membership a second time for the paths where the grammar is off (eval, a
//!    direct call). An empty list means the tool does not exist: "allow
//!    everything" is not a default that can be reached by accident.
//! 3. THE OUTPUT IS BOUNDED IN THREE WAYS: bytes captured, characters sent to
//!    the model (the rest goes to the DataStore, never through the model), and
//!    seconds allowed before the PROCESS GROUP is killed.
//! 4. NOTHING REACHES A SCREEN RAW. Everything captured goes through
//!    `tacet_mcp::safe_for_screen` before it is stored, shown or sent — the same
//!    gate that already closed the "a remote server repaints your terminal"
//!    hole.
//!
//! WHAT THE ALLOWLIST DOES NOT DO, stated plainly because a security note that
//! overclaims is worse than none: allowing a program allows EVERYTHING THAT
//! PROGRAM CAN DO. `git` can run a hook, `find` has `-exec`, `xargs` runs what
//! it is given. The allowlist is a decision about which programs the user trusts
//! with their machine, not a sandbox. `run_code`'s shield is the sandbox; this
//! tool deliberately has none, which is exactly why it is off by default and
//! why it taints the session.

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolState,
    TraceUpdate, boxed,
};

/// Seconds allowed when the model does not say. A test suite is the common case
/// and 10 (the `run_code` default) killed it mid-compile.
pub const DEFAULT_TIMEOUT: u64 = 30;

/// The longest the model may ask for. The model can SHORTEN this, never raise it
/// — the cap is the user's, not the model's.
pub const MAX_TIMEOUT: u64 = 300;

/// Bytes captured per stream. Everything above is read and DISCARDED (see
/// `read_pipe`): a build log can be hundreds of megabytes.
const OUTPUT_CAP: usize = 20_000;

/// Characters of output that reach the model. The rest lives in the DataStore
/// behind a `source_ref` — the 4096 window is not a place to put a build log.
const MODEL_OUTPUT_CAP: usize = 600;

/// The most arguments one call may carry, and the longest a single argument may
/// be. Not a security boundary (there is no shell to overflow into) but a
/// budget: a schema with no bound lets a confused model emit a megabyte of
/// arguments and stall the turn.
const MAX_ARGS: usize = 24;
const ARG_CAP: usize = 4_096;

/// The wait loop's tick and the grace given to the pipe readers after the
/// process is gone. BOTH NUMBERS ARE COPIED FROM `run_code`, deliberately: that
/// file already paid for them (see the note on the bounded join there).
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const JOIN_GRACE: Duration = Duration::from_secs(2);

// `setsid`/`killpg` WITHOUT A libc CRATE — the same two symbols `run_code`
// declares, for the same reason (zero dependencies; every unix links these).
// THE DECLARATION IS REPEATED rather than shared because the ones in `run_code`
// are private to that module and this arm owns a single file; a duplicate
// `extern` block is legal and costs nothing at run time.
#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
    fn killpg(group: i32, signal: i32) -> i32;
}

#[cfg(unix)]
const SIGKILL: i32 = 9;

// ---------------------------------------------------------------------------
// The allowlist
// ---------------------------------------------------------------------------

/// Is this a plain program name — no path, no option, no metacharacter?
///
/// THE RULE IS NOT WRITTEN HERE, IT IS ASKED. `addon::Shape::CommandName` owns
/// it: that is the shape the installer validates the user's typing against, and
/// a second copy in this file would be a second answer to "what is a command
/// name" — the install path accepting what the run path refuses, or the other
/// way round, which is the worse direction. What this function decides is not
/// the rule but WHEN it is asked: at run time as well, because the registry is
/// a JSON file a human can edit after the installer has had its say.
///
/// Looking the name up is then left to the operating system, with the user's own
/// `PATH` (see `execute`) — the same resolution their shell would do, on a name
/// they themselves allowed.
fn is_bare_program_name(name: &str) -> bool {
    tacet_web::addon::Shape::CommandName.check(name).is_ok()
}

// ---------------------------------------------------------------------------
// The tool
// ---------------------------------------------------------------------------

pub struct ShellTool {
    /// Sorted, unique, never empty (see `with_commands`).
    allowed: Vec<String>,
    /// The schema is built once: it is asked for on every turn (prompt + grammar)
    /// and building it allocates the whole `Choice` list each time.
    schema_cache: Mutex<Option<ArgSchema>>,
    description: String,
}

impl ShellTool {
    /// The production entry point: the tool EXISTS only if the addon is
    /// installed, open, and carries a usable allowlist.
    ///
    /// A CORRUPT REGISTRY MEANS NO TOOL (`read().ok()?`), the same fail-closed
    /// rule the addon gate follows everywhere else. Reading "the file is broken,
    /// it was probably open" would hand the model a process launcher on the
    /// strength of a parse error.
    ///
    /// THE STATE AND THE LIST COME FROM ONE READ. `addon::is_open` is the
    /// catalog's gate function and it reads the file itself; calling it here and
    /// then reading again for the settings would let the two answers come from
    /// two different versions of the file — an allowlist from before an edit,
    /// judged open by the state after it.
    pub fn discover() -> Option<ShellTool> {
        use tacet_web::addon;
        // NOT ON WINDOWS, AND NOT BECAUSE IT WOULD FAIL TO COMPILE — it compiles
        // fine. The timeout is what is missing. On unix a run gets its own
        // process group (`setsid`) and the timeout path signals the GROUP, so a
        // build that spawns compilers dies with it. Both of those calls are
        // `#[cfg(unix)]`; on Windows the same path can only do `child.kill()`,
        // which leaves every descendant running with the pipes open. The tool
        // would still return output and still look like it works, while the one
        // promise that bounds it — "a runaway command is stopped" — quietly did
        // not hold.
        //
        // That is the same rule `run_code` follows when no sandbox can be
        // measured: the tool leaves the catalog rather than run with a guarantee
        // it cannot keep. Windows needs a job object (`CreateJobObject` +
        // `TerminateJobObject`), which is a real piece of work and an unmeasured
        // one here. Until someone does it and runs it on Windows, the honest
        // state is absent.
        if !cfg!(unix) {
            return None;
        }
        let record = addon::read().ok()?;
        let entry = record.find(addon::SHELL)?;
        if !entry.open {
            return None;
        }
        ShellTool::with_commands(
            entry
                .values(addon::COMMANDS_KEY)
                .into_iter()
                .map(str::to_string)
                .collect(),
        )
    }

    /// The allowlist given directly — the install path and the tests.
    ///
    /// `None` FOR AN EMPTY LIST, and this is the load-bearing line of the whole
    /// file: a tool with no allowlist would be a tool that can run nothing, which
    /// sooner or later invites a patch that makes "empty" mean "anything". There
    /// is no such state to patch.
    pub fn with_commands(commands: Vec<String>) -> Option<ShellTool> {
        let mut allowed: Vec<String> = commands
            .iter()
            .map(|c| c.trim().to_string())
            .filter(|c| is_bare_program_name(c))
            .collect();
        allowed.sort();
        allowed.dedup();
        if allowed.is_empty() {
            return None;
        }
        // THE PROGRAM LIST IS IN THE DESCRIPTION. The model picks a tool from
        // name + description; without the list it cannot tell whether "run the
        // tests" is answerable here, and a call that fails the membership check
        // has already cost a turn.
        let description = format!(
            "Runs one of these programs on this device and returns its output: {}. Use it when the \
             user asks to run a command, build or test a project, or inspect files with a program \
             from that list. THERE IS NO SHELL: give the program in `command` and every argument \
             as a SEPARATE item of `args` (['test', '--quiet'], not 'test --quiet'). Pipes, \
             redirection, globs, ';' and '&&' DO NOT WORK - they are passed on as plain text. Only \
             the programs listed above can be run; anything else is refused.",
            allowed.join(", ")
        );
        Some(ShellTool {
            allowed,
            schema_cache: Mutex::new(None),
            description,
        })
    }

    /// The programs this tool may run. The install/diagnostic path reads it.
    pub fn allowed(&self) -> &[String] {
        &self.allowed
    }

    fn permits(&self, command: &str) -> bool {
        // EXACT MATCH, case included. A case-insensitive compare has already
        // been a hole in this repository once (the approval gate), and here it
        // would be worse than useless: on a case-insensitive filesystem `LS`
        // resolves to `ls` anyway, so forgiveness buys nothing while widening
        // what the user's list means.
        self.allowed.iter().any(|a| a == command)
    }
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

/// What one finished (or killed) command left behind.
enum RunOutcome {
    Finished {
        /// stdout and stderr, already made safe for a screen.
        text: String,
        /// `None` = killed by a signal (a crash, an outside `kill`).
        code: Option<i32>,
        ms: u128,
        truncated: bool,
    },
    /// The deadline passed and the process GROUP was killed.
    TimedOut { ms: u128 },
}

/// Spawns the program, waits with a deadline, and returns whatever came back.
///
/// THE SHAPE IS `run_code::run_program`'S, ON PURPOSE — its own process group so
/// the kill reaches the children, two reader threads so the pipes cannot
/// deadlock each other, a BOUNDED join so a surviving grandchild holding a pipe
/// open costs two seconds instead of the whole turn. Every one of those three
/// was a measured failure over there; rediscovering them here would mean
/// rediscovering the bugs first. The function is not reused because that one is
/// welded to the shield and to a wiped environment, and both are wrong here.
fn execute(program: &str, argv: &[OsString], dir: &Path, timeout: Duration) -> RunResult {
    let mut command = Command::new(program);
    command
        .args(argv)
        .current_dir(dir)
        // NO STDIN. A command that asks a question (a credential prompt, `git
        // add -p`) must fail instantly instead of sitting there until the
        // deadline: the user is looking at a chip, not at a terminal, and there
        // is no way for them to answer.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // THE ENVIRONMENT IS INHERITED, unlike `run_code`, and that IS the decision:
    // `cargo` needs `PATH`, `HOME`, `RUSTUP_*`; wiping them turns "run the
    // tests" into "command not found" and the tool into a trap. The trust
    // boundary here is the allowlist plus the approval gate, not the
    // environment. Four variables are overridden because each of them prevents a
    // real failure rather than a hypothetical one:
    //   * the colour ones: without them the output arrives full of ANSI, and
    //     while `safe_for_screen` strips it, it strips it into SPACES — the
    //     model would be reading confetti.
    //   * the pager ones: a program that pipes itself into `less` waits for a
    //     terminal that is not there. stdin being /dev/null usually ends that,
    //     but "usually" is how a 300-second timeout gets burned.
    command.env("NO_COLOR", "1");
    command.env("CLICOLOR", "0");
    command.env("CARGO_TERM_COLOR", "never");
    command.env("TERM", "dumb");
    command.env("PAGER", "cat");
    command.env("GIT_PAGER", "cat");
    // A git operation that wants a password must fail, not hang.
    command.env("GIT_TERMINAL_PROMPT", "0");
    // `PWD` IS SET TO MATCH `current_dir`. It is inherited from our own process
    // otherwise, and a `PWD` that names a different directory than the one the
    // child is standing in is not cosmetic: `pwd`, `make` and a great many
    // scripts trust that variable over the syscall.
    command.env("PWD", dir);

    // ITS OWN SESSION. On the timeout path the CHILDREN have to be signalled
    // too: `cargo test` is a parent of compilers and test binaries, and killing
    // only the process we spawned leaves them burning the user's CPU with the
    // pipes still open.
    //
    // WHAT THIS DOES NOT REACH, MEASURED: `killpg` kills the process GROUP, and
    // a grandchild that calls `setsid()` itself has left that group. A run with
    // `python3 -c 'os.fork(); os.setsid(); …'` timed out correctly and the
    // grandchild `sleep` was still alive afterwards, in a group of its own. The
    // control case — an ordinary `bash -c "sleep & sleep"` — died completely.
    // So the guarantee is "the command and its ordinary descendants", not "the
    // whole tree"; a deliberate escape needs a job object or a cgroup, which is
    // a different mechanism than a process group. Reachable on any machine whose
    // allow-list holds an interpreter.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe and touches nothing but this
        // process's session id — exactly what `pre_exec` allows. Failure means
        // we are already a group leader, which is harmless.
        unsafe {
            command.pre_exec(|| {
                setsid();
                Ok(())
            });
        }
    }

    let start = Instant::now();
    let mut child = command.spawn()?;
    #[cfg(unix)]
    let group = child.id() as i32;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_sink = std::sync::Arc::new(Mutex::new(PipeSink::default()));
    let err_sink = std::sync::Arc::new(Mutex::new(PipeSink::default()));
    let out_arm = {
        let sink = out_sink.clone();
        std::thread::spawn(move || read_pipe(stdout, &sink))
    };
    let err_arm = {
        let sink = err_sink.clone();
        std::thread::spawn(move || read_pipe(stderr, &sink))
    };

    let mut timed_out = false;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(state)) => break Some(state),
            Ok(None) => {}
            // The poll failed, so we no longer know this process. Killing it and
            // leaving is better than leaving something we cannot account for.
            Err(_) => {
                timed_out = true;
                break None;
            }
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            break None;
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    if timed_out {
        // THE GROUP, then reap. `child.kill()` alone leaves the descendants;
        // `kill` without `wait` leaves a zombie per call.
        #[cfg(unix)]
        unsafe {
            killpg(group, SIGKILL);
        }
        child.kill().ok();
        child.wait().ok();
    }
    let ms = start.elapsed().as_millis();

    join_before(out_arm, JOIN_GRACE);
    join_before(err_arm, JOIN_GRACE);
    let (out, cut_out) = drain(&out_sink);
    let (err, cut_err) = drain(&err_sink);

    if timed_out {
        return Ok(RunOutcome::TimedOut { ms });
    }

    // stderr IS KEPT AND LABELLED. Half the useful output of a build tool comes
    // out of stderr even on success (`cargo` writes its progress there), so
    // dropping it would hide the answer; merging it unlabelled would let the
    // model read a warning as a result.
    let mut text = out;
    if !err.trim().is_empty() {
        if !text.trim().is_empty() {
            text.push('\n');
        }
        text.push_str("--- stderr ---\n");
        text.push_str(&err);
    }

    Ok(RunOutcome::Finished {
        text: screen_safe_block(text.trim_end()),
        code: exit.and_then(|s| s.code()),
        ms,
        truncated: cut_out || cut_err,
    })
}

type RunResult = std::io::Result<RunOutcome>;

/// What a reader thread has collected so far. Shared rather than returned so the
/// main path can abandon the thread and still keep what was read.
#[derive(Default)]
struct PipeSink {
    accumulated: Vec<u8>,
    truncated: bool,
}

/// Reads to the end, STORES up to the cap. Stopping the read would block the
/// child once the pipe filled (and keep it blocked until the deadline); storing
/// everything would let a runaway build log eat memory.
fn read_pipe<R: Read>(pipe: Option<R>, sink: &Mutex<PipeSink>) {
    let Some(mut pipe) = pipe else { return };
    let mut buffer = [0u8; 8192];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut sink = sink.lock().expect("pipe sink lock");
                let slot = OUTPUT_CAP.saturating_sub(sink.accumulated.len());
                if slot == 0 {
                    sink.truncated = true;
                    continue;
                }
                let taken = n.min(slot);
                sink.accumulated.extend_from_slice(&buffer[..taken]);
                if taken < n {
                    sink.truncated = true;
                }
            }
        }
    }
}

/// Takes what a reader collected. A poisoned lock (the reader panicked) counts
/// as "nothing was read": a helper thread must not take the turn down with it.
fn drain(sink: &Mutex<PipeSink>) -> (String, bool) {
    let Ok(sink) = sink.lock() else {
        return (String::new(), false);
    };
    (
        String::from_utf8_lossy(&sink.accumulated).into_owned(),
        sink.truncated,
    )
}

/// Joins, but gives up after `grace` and orphans the thread. Std has no timed
/// join; the alternative to giving up is a tool call that never returns.
fn join_before(handle: std::thread::JoinHandle<()>, grace: Duration) {
    let deadline = Instant::now() + grace;
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    let _ = handle.join();
}

/// Makes captured output safe to put on a screen, WITHOUT collapsing it into one
/// line.
///
/// The gate itself is `tacet_mcp::safe_for_screen` — it is CALLED, not
/// reimplemented: it already turns every control character (C0 `ESC` and the
/// 8-bit C1 `CSI` alike) into a space, collapses the result, and caps the
/// length. That function is built for a ONE-LINE chip, and command output is not
/// one line, so it is applied PER LINE, and a line longer than the cap is fed
/// through in cap-sized pieces instead of being cut — a rustc error is exactly
/// the sort of long line the user opened the output to read.
///
/// THE COST IS STATED: leading whitespace does not survive (a tab is a control
/// character, and the gate collapses runs of spaces), so indented output loses
/// its indentation. That is accepted. The alternative is a second, gentler
/// sanitizer living next to the first one, and this repository has already
/// measured what two copies of one security rule cost.
///
/// SANITISED AT THE POINT OF CAPTURE, not at the point of printing: the same
/// text goes to the chip, to the model and to the DataStore, and a copy that
/// escaped the gate would eventually reach a terminal through whichever of the
/// three nobody was watching.
fn screen_safe_block(raw: &str) -> String {
    use tacet_mcp::{SCREEN_LIMIT, safe_for_screen};
    let mut lines = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        for piece in chars.chunks(SCREEN_LIMIT) {
            let safe = safe_for_screen(&piece.iter().collect::<String>());
            if !safe.is_empty() {
                lines.push(safe);
            }
        }
    }
    lines.join("\n")
}

/// Keeps the LAST `cap` characters.
///
/// THE TAIL, NOT THE HEAD, and that differs from `run_code` on purpose: a script
/// prints its answer, a command prints its progress and THEN its answer. The
/// head of `cargo test` is a list of crates being compiled; the line that says
/// whether the tests passed is the last one. The whole output is in the
/// DataStore either way.
fn tail(text: &str, cap: usize) -> (String, bool) {
    let count = text.chars().count();
    if count <= cap {
        return (text.to_string(), false);
    }
    (text.chars().skip(count - cap).collect(), true)
}

// ---------------------------------------------------------------------------
// The tool contract
// ---------------------------------------------------------------------------

impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> ArgSchema {
        if let Some(s) = self.schema_cache.lock().expect("schema lock").clone() {
            return s;
        }
        // `command` IS A `Choice`, NOT A `Text`. This is the file's second
        // structural gate: the grammar turns a closed set into a literal
        // alternation, so a program outside the user's list is not something the
        // model is refused for asking — it is something it CANNOT EMIT. A
        // `Text` field guarded by a check afterwards would be the same rule
        // enforced one layer too late.
        let schema = ArgSchema::object(vec![
            Field::new(
                "command",
                ArgSchema::choice(self.allowed.iter().map(String::as_str))
                    .description("The program to run."),
            )
            .required(),
            Field::new(
                "args",
                ArgSchema::array(ArgSchema::text())
                    .length(None, Some(MAX_ARGS))
                    .description(
                        "Arguments, ONE PER ITEM. They are passed to the program as written; \
                         they are not interpreted by a shell.",
                    ),
            ),
            Field::new(
                "timeout_s",
                ArgSchema::integer()
                    .range(Some(1.0), Some(MAX_TIMEOUT as f64))
                    .description(format!(
                        "Seconds before the command is killed (default {DEFAULT_TIMEOUT})."
                    )),
            ),
        ])
        .description("Run an allowed program in the working folder");
        *self.schema_cache.lock().expect("schema lock") = Some(schema.clone());
        schema
    }

    /// IT TAINTS. The command runs with the user's own environment inside the
    /// user's own folder; whatever it printed is now in the context, and the
    /// next tool that could send data outwards must meet the approval gate.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            // THE SECOND CHECK OF THE SAME RULE, AND IT RUNS BEFORE THE SCHEMA
            // VALIDATION.
            //
            // Why a second check at all: the schema refused this already (and so
            // did the grammar, if it was on) — but "the grammar was on" is not
            // something this file can know, because eval and the CLI call tools
            // directly. Why FIRST: a schema failure returns the one fixed
            // sentence every tool error returns, and from that the model learns
            // only "something was wrong". The one thing it needs here is WHICH
            // programs exist, so that the next turn either names one of them or
            // tells the user what to allow.
            let requested = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !requested.is_empty() && !self.permits(requested) {
                let command = requested;
                return ToolOutcome::new(
                    format!("'{}' is not an allowed command", safe_name(command)),
                    ToolState::Failed(format!("'{}' is not allowed", safe_name(command))),
                    format!(
                        "error: '{}' is not an allowed program. Only these can be run: {}. \
                         Tell the user which program they would need to allow.",
                        safe_name(command),
                        self.allowed.join(", ")
                    ),
                );
            }

            // The rest of the contract is the schema's job — the closed set, the
            // argument list's shape and length, the timeout range are all
            // declared there, and validating them anywhere else would be a
            // second copy of the contract.
            if let Err(e) = self.schema().validate(&args) {
                return ToolOutcome::failed(&e);
            }
            let Some(command) = args.get("command").and_then(|v| v.as_str()) else {
                return ToolOutcome::failed(&ToolError::MissingField("command".into()));
            };

            let argv = match collect_args(&args) {
                Ok(a) => a,
                Err(text) => {
                    return ToolOutcome::new(
                        "The command could not be run",
                        ToolState::Failed("the arguments could not be read".into()),
                        text,
                    );
                }
            };

            let timeout = Duration::from_secs(
                args.get("timeout_s")
                    .and_then(|v| v.as_u64())
                    .filter(|s| *s >= 1 && *s <= MAX_TIMEOUT)
                    .unwrap_or(DEFAULT_TIMEOUT),
            );

            // THE WORKING DIRECTORY IS THE CONTEXT'S, and it is resolved through
            // the sandbox gate rather than used raw: `resolve_path` is the one
            // place that knows what "outside" means, and `canonicalize` settles
            // symlinks before the path is handed to a process (the symlink
            // escape is a closed hole in this repository — it stays closed by
            // going through the same door).
            let dir = match ctx
                .resolve_path(".")
                .and_then(|p| p.canonicalize().map_err(ToolError::Io))
            {
                Ok(p) => p,
                Err(e) => return ToolOutcome::failed(&e),
            };

            let printable = display_command(command, &argv);
            let trace = ctx.start_chip("shell", &format!("Running {command}…"));
            let result = execute(command, &argv, &dir, timeout);
            let (outcome, ran) = match result {
                Ok(run) => (self.report(run, command, timeout, ctx), true),
                // The spawn itself failed: no process was started, so nothing
                // outside changed and the session is not tainted. The most
                // common cause by far is the program not being installed, and
                // saying so is what lets the model answer usefully.
                Err(e) => (
                    ToolOutcome::new(
                        format!("{command} could not be started"),
                        ToolState::Failed(format!("{command} could not be started")),
                        format!(
                            "error: '{command}' could not be started ({}). It may not be \
                             installed on this device.",
                            e.kind()
                        ),
                    ),
                    false,
                ),
            };

            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(printable)
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            // ONLY A COMMAND THAT REALLY RAN TAINTS. A refused or never-started
            // call touched nothing, and tainting on it would tighten the
            // approval gate for the rest of the session over an event that did
            // not happen — an approval seen too often stops being read.
            if ran {
                ctx.taint();
            }
            outcome
        })
    }
}

impl ShellTool {
    /// Turns a finished run into the two texts: the user's chip and the model's
    /// short result.
    fn report(
        &self,
        run: RunOutcome,
        command: &str,
        timeout: Duration,
        ctx: &ToolContext,
    ) -> ToolOutcome {
        match run {
            RunOutcome::TimedOut { ms } => ToolOutcome::new(
                format!("{command} timed out"),
                ToolState::Failed("the command timed out".into()),
                format!(
                    "error: '{command}' did not finish within {}s and was killed. It may have \
                     done part of its work. Do not repeat it blindly; tell the user, or run a \
                     narrower command.",
                    timeout.as_secs()
                ),
            )
            .raw_output(format!("timed out after {ms} ms")),

            RunOutcome::Finished {
                text,
                code,
                ms,
                truncated,
            } => {
                let status = match code {
                    Some(c) => format!("exit {c}"),
                    // No exit code means a signal killed it — a segfault, or
                    // somebody's `kill`. Reporting it as "exit 0" would be a lie
                    // the model then repeats to the user.
                    None => "killed by a signal".to_string(),
                };
                let (shown, cut) = tail(&text, MODEL_OUTPUT_CAP);
                // THE BULK DOES NOT GO THROUGH THE MODEL. Above the cap the full
                // text lands in the DataStore and the model gets the tail plus a
                // reference, in the ONE wire format (`source_ref_suffix`).
                let suffix = if cut || truncated {
                    let r = ctx.store(
                        "shell",
                        &format!("the full output of '{command}' ({} characters)", text.len()),
                        text.clone(),
                    );
                    format!(
                        "\n(only the last {MODEL_OUTPUT_CAP} characters are shown{}){}",
                        if truncated {
                            "; the command produced more output than was captured"
                        } else {
                            ""
                        },
                        tacet_kernel::source_ref_suffix(r.as_str())
                    )
                } else {
                    String::new()
                };

                let body = if shown.trim().is_empty() {
                    // NO OUTPUT IS A RESULT HERE, NOT AN ERROR — the opposite of
                    // `run_code`, and the difference is real: a script that
                    // prints nothing computed nothing, but `git add .` printing
                    // nothing IS the success case. Saying so explicitly stops
                    // the model inventing output it never saw.
                    "(the command produced no output)".to_string()
                } else {
                    shown
                };

                // A NON-ZERO EXIT IS NOT A TOOL FAILURE. `grep` exits 1 when it
                // finds nothing and `cargo test` exits 101 when a test fails —
                // both are answers the user asked for. `Failed` is kept for the
                // cases where the tool could not do its job at all, so the model
                // is not told to "retry" something that worked perfectly.
                //
                // THE STATE IS `Written`, EVEN FOR `ls`. Nothing here can know
                // whether a command touched the disk, and `Written` is the
                // fail-closed answer: it stops the engine replaying a turn that
                // may already have created a commit. The cost is a lost
                // automatic retry after an unrelated error; the alternative
                // cost is a side effect happening twice.
                ToolOutcome::written(
                    format!("{command} · {status} · {ms} ms"),
                    format!("{status} ({ms} ms)\n{body}{suffix}"),
                )
                .raw_output(text)
            }
        }
    }
}

/// Reads the `args` array into an argument vector.
///
/// Returns the model-facing error text on refusal. NOTHING IS PARSED OUT OF THE
/// STRINGS: an argument goes to the program exactly as written — that is the
/// whole point of the tool. The only refusals are structural (wrong JSON type,
/// too many, too long) plus the NUL byte, which cannot cross `exec` at all and
/// would otherwise surface as an unexplained spawn failure.
fn collect_args(args: &serde_json::Value) -> Result<Vec<OsString>, String> {
    let Some(list) = args.get("args") else {
        return Ok(Vec::new());
    };
    if list.is_null() {
        return Ok(Vec::new());
    }
    let Some(list) = list.as_array() else {
        return Err("error: 'args' must be a list of strings, one argument per item".to_string());
    };
    if list.len() > MAX_ARGS {
        return Err(format!(
            "error: too many arguments (at most {MAX_ARGS}); use a narrower command"
        ));
    }
    let mut argv = Vec::with_capacity(list.len());
    for item in list {
        let Some(text) = item.as_str() else {
            return Err(
                "error: every item of 'args' must be a string; write numbers as text".to_string(),
            );
        };
        if text.chars().count() > ARG_CAP {
            return Err(format!(
                "error: an argument is longer than {ARG_CAP} characters; put long text in a file \
                 instead"
            ));
        }
        if text.contains('\0') {
            return Err("error: an argument contains a NUL character".to_string());
        }
        argv.push(OsString::from(text));
    }
    Ok(argv)
}

/// The command line as the USER should see it, for the chip's detail view.
///
/// It is not a shell line and does not pretend to be one: the arguments are
/// joined for reading only, and the text goes through the screen gate because a
/// model-written argument is untrusted text on its way to a terminal.
fn display_command(command: &str, argv: &[OsString]) -> String {
    let mut line = String::from(command);
    for a in argv {
        line.push(' ');
        line.push_str(&a.to_string_lossy());
    }
    screen_safe_block(&line)
}

/// A program name on its way into a message. It came from the model, so it is
/// untrusted text — even on the refusal path, where it is by definition a name
/// the user never approved.
fn safe_name(name: &str) -> String {
    tacet_mcp::safe_for_screen(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tacet_kernel::{InMemoryDataStore, SilentReporter};

    /// The core has no runtime and this crate must not pick one — the same
    /// minimal executor the other tool tests use.
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut future = pin!(future);
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-shell-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn tool(commands: &[&str]) -> ShellTool {
        ShellTool::with_commands(commands.iter().map(|c| c.to_string()).collect())
            .expect("a non-empty allowlist produces a tool")
    }

    fn call(tool: &ShellTool, root: &Path, args: serde_json::Value) -> (ToolOutcome, ToolContext) {
        let store = Arc::new(InMemoryDataStore::new());
        let mut ctx = ToolContext::new(store, root, Arc::new(SilentReporter));
        let outcome = block_on(tool.run(args, &mut ctx));
        (outcome, ctx)
    }

    // --- The allowlist ---------------------------------------------------

    /// AN EMPTY LIST MEANS NO TOOL. The rule that makes "allow everything"
    /// unreachable: there is no ShellTool to reach it with.
    #[test]
    fn an_empty_allowlist_produces_no_tool() {
        assert!(ShellTool::with_commands(vec![]).is_none());
        assert!(ShellTool::with_commands(vec!["   ".into()]).is_none());
        // Nothing but unusable entries is still an empty list.
        assert!(ShellTool::with_commands(vec!["/bin/sh".into(), "../sh".into()]).is_none());
    }

    /// A PATH IS NOT A PROGRAM NAME, and neither is "a command with its
    /// arguments". Entries like that are DROPPED rather than stored: they would
    /// put the user's filesystem layout into the prompt (the list becomes the
    /// `Choice` schema) and give this file a path to reason about.
    ///
    /// The rule itself belongs to `addon::Shape::CommandName`; what is measured
    /// here is that this file ASKS it, so a hand-edited registry cannot smuggle
    /// `/tmp/x` or `sh -c` into the allowlist.
    #[test]
    fn only_bare_program_names_survive_the_allowlist() {
        let t = ShellTool::with_commands(vec![
            "cargo".into(),
            "/usr/bin/ls".into(),
            "../sh".into(),
            "git".into(),
            "".into(),
            "grep".into(),
            "-rf".into(),
            "..".into(),
            "rm -rf /".into(),
            "cargo".into(),
        ])
        .expect("three usable entries");
        assert_eq!(t.allowed(), ["cargo", "git", "grep"]);
        assert!(is_bare_program_name("rust-analyzer"));
        assert!(!is_bare_program_name("a b"));
        assert!(!is_bare_program_name("a;b"));
        assert!(!is_bare_program_name("$PATH"));
    }

    /// THE SEAM WITH THE REGISTRY, measured rather than assumed.
    ///
    /// `discover` reads the allowlist out of the addon record under
    /// `addon::SHELL` / `addon::COMMANDS_KEY`, and it trusts
    /// `Shape::CommandName` to be the shape the installer validated those
    /// entries against. If the registry ever describes the shell addon with a
    /// different key or a different shape, this tool would read an empty list
    /// (no tool at all) or a list nobody checked — both silent. The failure
    /// belongs on a test, not in the field.
    ///
    /// The addon's `tools` field is deliberately NOT asserted here: it is
    /// documentation for `addon list`, the catalog is what actually adds the
    /// tool, and the catalog is registered by another owner.
    #[test]
    fn the_registry_describes_the_addon_this_tool_reads() {
        use tacet_web::addon;
        let definition =
            addon::definition(addon::SHELL).expect("the registry knows the shell addon");
        let setting = definition
            .settings
            .iter()
            .find(|s| s.key == addon::COMMANDS_KEY)
            .expect("the shell addon asks for a command list");
        assert!(setting.many, "the allowlist is a MANY-valued setting");
        assert!(setting.required, "an addon with no list can run nothing");
        assert_eq!(
            setting.shape,
            addon::Shape::CommandName,
            "the run-time check in this file asks exactly this shape"
        );
    }

    /// GATE 2 IS REAL: the program list is a CLOSED SET in the schema, so the
    /// grammar cannot generate anything else. If this field ever becomes a
    /// `Text`, the structural gate is gone and only the runtime check below is
    /// left — this test is what notices.
    #[test]
    fn the_command_field_is_a_closed_set_of_the_allowed_programs() {
        let t = tool(&["cargo", "ls"]);
        let schema = t.schema();
        let field = schema
            .fields()
            .iter()
            .find(|f| f.name == "command")
            .expect("a command field");
        assert!(field.required);
        match &field.schema.kind {
            tacet_kernel::SchemaKind::Choice { choices } => {
                assert_eq!(choices, &vec!["cargo".to_string(), "ls".to_string()]);
            }
            other => panic!("`command` must be a closed set, it is {other:?}"),
        }
        // The schema refuses a program outside the set on its own.
        assert!(schema.validate(&json!({ "command": "rm" })).is_err());
        assert!(schema.validate(&json!({ "command": "ls" })).is_ok());
    }

    /// THE TOOL TOUCHES THE USER'S MACHINE — the approval gate has to see it.
    #[test]
    fn the_tool_taints_the_session() {
        assert!(tool(&["ls"]).taints_session());
    }

    // --- The refusal path ------------------------------------------------

    /// A PROGRAM OUTSIDE THE LIST IS REFUSED, and refused BEFORE anything is
    /// started. The evidence that nothing ran: the session is not tainted and
    /// the file the command would have created does not exist.
    #[test]
    fn a_program_outside_the_allowlist_is_refused() {
        let root = temp_dir("not-allowed");
        let victim = root.join("victim.txt");
        std::fs::write(&victim, "still here").expect("fixture");

        let (outcome, ctx) = call(
            &tool(&["ls"]),
            &root,
            json!({ "command": "rm", "args": [victim.to_string_lossy()] }),
        );

        assert!(matches!(outcome.state, ToolState::Failed(_)));
        assert!(victim.exists(), "a refused command must not have run");
        assert!(
            !ctx.session_tainted(),
            "a command that never ran must not taint the session"
        );
        // The model is told what it MAY use, so the next turn can be right.
        assert!(outcome.to_model.contains("ls"));
    }

    /// CASE IS NOT FORGIVEN. `ls` in the list does not make `LS` allowed — on a
    /// case-insensitive filesystem forgiveness would quietly widen the user's
    /// list to every spelling of every entry.
    #[test]
    fn the_allowlist_is_case_sensitive() {
        let root = temp_dir("case");
        let (outcome, _) = call(&tool(&["ls"]), &root, json!({ "command": "LS" }));
        assert!(matches!(outcome.state, ToolState::Failed(_)));
    }

    // --- No shell --------------------------------------------------------

    /// THE CLAIM OF THE WHOLE FILE: an argument is DATA, not a command.
    ///
    /// `; rm -rf`, `$(...)`, `&&`, `*` and `|` are handed to `echo` and come
    /// back as the text they are. If a shell ever creeps in between, the sentinel
    /// file disappears and the output stops matching.
    #[cfg(unix)]
    #[test]
    fn an_argument_is_not_interpreted_as_a_command() {
        let root = temp_dir("injection");
        let victim = root.join("victim.txt");
        std::fs::write(&victim, "still here").expect("fixture");
        let payload = format!("hello; rm -rf {}", victim.display());

        let (outcome, _) = call(
            &tool(&["echo"]),
            &root,
            json!({ "command": "echo", "args": [payload, "$HOME", "&& whoami", "*"] }),
        );

        assert!(
            victim.exists(),
            "an argument was executed: the file it names is gone"
        );
        assert!(outcome.to_model.contains("hello; rm -rf"));
        // `$HOME` was NOT expanded and `*` was NOT globbed.
        assert!(outcome.to_model.contains("$HOME"));
        assert!(outcome.to_model.contains('*'));
        assert!(outcome.to_model.contains("&& whoami"));
    }

    /// The command runs in the CONTEXT'S folder, not in the process's.
    #[cfg(unix)]
    #[test]
    fn the_command_runs_in_the_working_directory() {
        let root = temp_dir("cwd").canonicalize().expect("canonical fixture");
        let (outcome, _) = call(&tool(&["pwd"]), &root, json!({ "command": "pwd" }));
        assert!(
            outcome
                .to_model
                .contains(&root.to_string_lossy().to_string()),
            "the command did not run in the working directory: {}",
            outcome.to_model
        );
    }

    /// A COMMAND THAT RAN TAINTS THE SESSION, whatever its exit code — the
    /// output is in the context either way, so the next outgoing call must meet
    /// the approval gate.
    #[cfg(unix)]
    #[test]
    fn a_command_that_ran_taints_the_session() {
        let root = temp_dir("taint");
        let (outcome, ctx) = call(
            &tool(&["echo"]),
            &root,
            json!({ "command": "echo", "args": ["hi"] }),
        );
        assert!(ctx.session_tainted());
        assert_eq!(outcome.state, ToolState::Written);
        assert!(outcome.to_model.contains("exit 0"));
        assert!(outcome.to_model.contains("hi"));
    }

    /// A NON-ZERO EXIT IS AN ANSWER, NOT A TOOL FAILURE: `false` exits 1 and the
    /// model must be able to report that instead of being told to retry.
    #[cfg(unix)]
    #[test]
    fn a_non_zero_exit_is_reported_not_treated_as_a_failure() {
        let root = temp_dir("exit");
        let (outcome, _) = call(&tool(&["false"]), &root, json!({ "command": "false" }));
        assert!(
            !matches!(outcome.state, ToolState::Failed(_)),
            "a command that ran and failed is still a result"
        );
        assert!(outcome.to_model.contains("exit 1"));
        // Empty output is stated, not left for the model to fill in.
        assert!(outcome.to_model.contains("no output"));
    }

    // --- The bounds ------------------------------------------------------

    /// THE TIMEOUT REALLY KILLS. `sleep 30` under a one-second deadline has to
    /// come back in about a second, not in thirty.
    #[cfg(unix)]
    #[test]
    fn the_timeout_kills_the_command() {
        let root = temp_dir("timeout");
        let start = Instant::now();
        let (outcome, ctx) = call(
            &tool(&["sleep"]),
            &root,
            json!({ "command": "sleep", "args": ["30"], "timeout_s": 1 }),
        );
        let elapsed = start.elapsed();
        assert!(matches!(outcome.state, ToolState::Failed(_)));
        assert!(
            elapsed < Duration::from_secs(10),
            "the deadline did not bite: {elapsed:?}"
        );
        assert!(outcome.to_model.contains("did not finish"));
        // It DID run, so the session is tainted even though it was killed.
        assert!(ctx.session_tainted());
    }

    /// BIG OUTPUT DOES NOT GO THROUGH THE MODEL. What the model sees is bounded
    /// and the whole thing is in the DataStore behind a reference.
    #[cfg(unix)]
    #[test]
    fn large_output_is_cut_and_the_rest_goes_to_the_store() {
        let root = temp_dir("large");
        // Twelve arguments of 4000 characters: well past both the model cap and
        // the capture cap, and produced without a shell.
        let chunk = "x".repeat(4_000);
        let args: Vec<String> = (0..12).map(|_| chunk.clone()).collect();

        let (outcome, _) = call(
            &tool(&["echo"]),
            &root,
            json!({ "command": "echo", "args": args }),
        );

        assert!(
            outcome.to_model.chars().count() < MODEL_OUTPUT_CAP + 400,
            "the model was sent {} characters",
            outcome.to_model.chars().count()
        );
        assert!(outcome.to_model.contains("source_ref="));
        // The full text is in the store, and it is bigger than what the model saw.
        let raw = outcome.raw_output.expect("raw output");
        assert!(raw.len() > MODEL_OUTPUT_CAP);
    }

    /// THE ARGUMENT LIST IS BOUNDED, and a bad shape is refused with a sentence
    /// the model can act on rather than a spawn failure.
    #[test]
    fn a_malformed_argument_list_is_refused() {
        let root = temp_dir("args");
        let many: Vec<String> = (0..MAX_ARGS + 1).map(|i| i.to_string()).collect();
        let (outcome, ctx) = call(
            &tool(&["echo"]),
            &root,
            json!({ "command": "echo", "args": many }),
        );
        assert!(matches!(outcome.state, ToolState::Failed(_)));
        assert!(!ctx.session_tainted(), "nothing was started");

        // A number is not a string: the schema catches it before this file does,
        // and either way nothing runs.
        let (outcome, _) = call(
            &tool(&["echo"]),
            &root,
            json!({ "command": "echo", "args": [7] }),
        );
        assert!(matches!(outcome.state, ToolState::Failed(_)));
    }

    // --- The screen gate -------------------------------------------------

    /// NOTHING REACHES A SCREEN RAW. A command whose output carries a
    /// screen-clearing escape sequence must arrive with no control characters
    /// left in it — in the chip's detail view, in the model's text, and in the
    /// store, because all three end up on a terminal eventually.
    #[cfg(unix)]
    #[test]
    fn control_characters_do_not_survive_the_output() {
        let root = temp_dir("ansi");
        let hostile = "\u{1b}[2J\u{1b}[H\u{1b}[1mall files deleted\u{1b}[0m";
        let (outcome, _) = call(
            &tool(&["echo"]),
            &root,
            json!({ "command": "echo", "args": [hostile] }),
        );
        let raw = outcome.raw_output.clone().unwrap_or_default();
        for text in [&outcome.to_model, &outcome.chip_text, &raw] {
            assert!(
                !text.chars().any(|c| c.is_control() && c != '\n'),
                "a control character survived: {text:?}"
            );
        }
        // The visible part is still there — sanitising must not eat the answer.
        assert!(outcome.to_model.contains("all files deleted"));
    }

    /// A LONG LINE IS WRAPPED, NOT CUT. The screen gate caps a single line at
    /// `SCREEN_LIMIT`; feeding it the line in pieces keeps the content, which is
    /// the difference between reading a compiler error and reading its first
    /// eighty characters.
    #[test]
    fn a_long_line_survives_the_screen_gate_in_pieces() {
        let line = "e".repeat(1_000);
        let safe = screen_safe_block(&line);
        assert_eq!(safe.chars().filter(|c| *c == 'e').count(), 1_000);
        assert!(!safe.contains('…'), "content was cut instead of wrapped");
        for piece in safe.lines() {
            assert!(piece.chars().count() <= tacet_mcp::SCREEN_LIMIT);
        }
    }

    /// The tail is what survives truncation: the answer of a command is at the
    /// END of its output.
    #[test]
    fn truncation_keeps_the_end_of_the_output() {
        let (kept, cut) = tail("abcdef", 3);
        assert_eq!(kept, "def");
        assert!(cut);
        let (kept, cut) = tail("ab", 3);
        assert_eq!(kept, "ab");
        assert!(!cut);
    }
}
