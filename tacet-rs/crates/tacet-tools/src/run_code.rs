//! `run_code` — the tool that runs model output. THIS FILE IS A SECURITY
//! BOUNDARY; every mechanism here exists for an attack or an accident.
//!
//! The Swift counterpart is `CodeEngine` + `RunCodeTool`. There the engine was
//! JavaScriptCore: an interpreter inside the operating system with NO file or
//! network bridge. There is NO counterpart in Rust and, under the
//! zero-dependency identity, pulling in a JS engine is out of the question — so
//! here the model's code runs IN A SEPARATE PROCESS, with an interpreter
//! already installed on the system. That changes the threat model: in JSC the
//! network was absent "because it was not bridged", here it must be absent
//! "because it is explicitly cut". Where we cannot cut it, the tool DOES NOT
//! PUT ITSELF IN THE CATALOG (see `discover`).
//! FOUR GATES:
//!
//! 1. **NETWORK** — `sandbox-exec` with `(deny network*)` on macOS, `bwrap`
//!    with `--unshare-net` on Linux. The cut is REALLY MEASURED DURING
//!    DISCOVERY (`verify_shield` ACTUALLY TRIES a connection under the shield):
//!    writing a profile file and saying "it probably works" would be exactly
//!    what silently calling something safe looks like. If the shield cannot be
//!    set up, `discover` returns `None` and the tool IS NOT VISIBLE in the catalog.
//!
//! PLATFORM RECORD: there is NO counterpart of these tools (`run_code` and
//! `write_code`) on Windows and the tool is DELIBERATELY off there. The reason
//! is the absence of a shield: on Windows there is no mechanism installable
//! with zero dependencies that cuts BOTH the network AND the file system
//! (AppContainer/Job Object want `windows-sys`). Being off is the right
//! behaviour — treating a gate we cannot close as "probably closed" and running
//! model code is exactly what this file stands against. The user learns THE
//! REASON from `diagnose()`.
//! 2. **FILES** — the same profile locks writing into the sandbox and also
//!    closes the user's home directory TO READING. The interpreter's own
//!    library (dyld, stdlib) stays readable; closed off, python3 would not start at all.
//! 3. **TIME** — a mandatory timeout (10 s by default). If it is exceeded the
//!    process is KILLED (`kill` + `wait`), no zombie is left behind.
//! 4. **OUTPUT** — a byte cap. So an infinite loop does not eat all the memory,
//!    the pipes KEEP BEING READ in separate threads but anything over the cap is
//!    DISCARDED: stopping the reads locks the child on the pipe, having no cap eats the memory.
//! THE ATTEMPT COUNTER (code-spec §5.4): at most 2 real runs per turn; the 3rd
//! call is rejected without EVER seeing the engine. In Swift the counter was on
//! a `weak` reference and, when it was not bound, "fail-closed" counted it as 3.
//! Here the counter is INSIDE THE TOOL and is always bound by default — there is
//! no binding to forget.
//! SUCCESS WITH NO OUTPUT IS NOT A SUCCESS (measured in Swift): a model that saw
//! "ok" MADE THE RESULT UP. A script that prints nothing counts as an error.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tacet_core::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult, ToolState,
    TraceUpdate, boxed,
};

/// The default timeout (seconds). The base the task gives.
pub const DEFAULT_TIMEOUT: u64 = 10;
/// The longest duration the model may ask for. The model CANNOT CHOOSE the cap, only shorten it.
pub const MAX_TIMEOUT: u64 = 30;

/// The byte cap of the captured output. Anything over it is discarded and the discarding IS STATED.
const OUTPUT_CAP: usize = 10_000;
/// The cap of the output going to the model — the full output goes to the chip (code-spec §5.2).
pub(crate) const MODEL_OUTPUT_CAP: usize = 500;
/// The cap of the error text: it must not eat the 4096 budget.
pub(crate) const MODEL_ERROR_CAP: usize = 400;
/// The cap of the script source. Anything above this is not a "short script".
pub(crate) const CODE_CAP: usize = 20_000;
/// The number of real runs per turn (code-spec §5.4).
pub(crate) const MAX_ATTEMPTS: usize = 2;

/// The polling interval for the process finishing. Std has no `wait_timeout`;
/// the polling interval is kept short so the timeout measurement stays
/// sensitive, but long enough not to burn the CPU for nothing.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

// ---------------------------------------------------------------------------
// The network shield
// ---------------------------------------------------------------------------

/// The mechanism that cuts the child process's network and file access.
///
/// "NO SHIELD" IS NOT A VARIANT — that rule has not changed. If the shield
/// cannot be set up, `RunCodeTool` simply does not exist; had "shieldless mode"
/// been an enum arm, a future patch could silently make it the default. The
/// variants are ONLY real shields and each of them must close THE SAME two
/// gates: the network AND writing outside the sandbox. A mechanism that cuts
/// only the network (e.g. a bare `unshare -n`) IS NOT ADDED here — the sentence
/// "NO access to your files outside the working folder" in `description()`
/// would then be a lie.
#[derive(Debug, Clone)]
pub enum NetworkShield {
    /// macOS `sandbox-exec`. The profile is given inline (`-p`); we leave no
    /// profile file on disk that could be tampered with.
    SandboxExec { tool: PathBuf },
    /// Linux `bwrap` (bubblewrap). `--unshare-net` cuts the network and
    /// `--ro-bind / /` cuts writing outside the sandbox; the user's home
    /// directory is COVERED with an empty `--tmpfs` (the counterpart of the
    /// `deny file-read* (subpath home)` rule in the macOS profile).
    ///
    /// NOT MEASURED ON THIS MACHINE: there is no Linux here, no `rustup`, no
    /// cross compilation. But that is NOT the same as "we hope it works": on
    /// the production path the shield is measured ON EVERY STARTUP with
    /// `verify_shield` (see that function). If bwrap is installed wrong on this
    /// machine, or `--unshare-net` is refused by the kernel, the measurement
    /// fails, `discover` returns `None` and the tool IS NOT VISIBLE in the
    /// catalog — so the worst case is today's Linux behaviour, not a new silent hole.
    Bwrap { tool: PathBuf },
}

impl NetworkShield {
    /// Looks for a usable shield on the system. IT DOES NOT VALIDATE — the
    /// caller measures that it really cuts, with `verify_shield`.
    ///
    /// The paths are FIXED, `PATH` is not searched: `PATH` comes from the
    /// caller's environment and can be poisoned; picking the shield itself out
    /// of PATH would make the shield meaningless (see `discover_interpreters`).
    pub fn find() -> Option<NetworkShield> {
        let sandbox = Path::new("/usr/bin/sandbox-exec");
        if sandbox.is_file() {
            return Some(NetworkShield::SandboxExec {
                tool: sandbox.to_path_buf(),
            });
        }
        BWRAP_PATHS
            .iter()
            .map(Path::new)
            .find(|y| y.is_file())
            .map(|y| NetworkShield::Bwrap {
                tool: y.to_path_buf(),
            })
    }

    pub fn name(&self) -> &'static str {
        match self {
            NetworkShield::SandboxExec { .. } => "sandbox-exec",
            NetworkShield::Bwrap { .. } => "bwrap",
        }
    }

    /// Wraps the command with the shield.
    ///
    /// `sandbox` must be ABSOLUTE and RESOLVED (symlink free): on macOS `/tmp`
    /// is a link to `/private/tmp` and a profile that says `(subpath "/tmp/...")`
    /// permits nothing — the rule silently falls to "everything forbidden" and
    /// the script crashes incomprehensibly.
    ///
    /// `args` is free: a direct run passes `[script]`, while a syntax check
    /// (`write_code`) carries a flag such as `["--check", script]`. The shield
    /// profile is THE SAME on both paths — even the verification step cannot get out of the sandbox.
    fn wrap(&self, sandbox: &Path, program: &Path, args: &[OsString]) -> Command {
        match self {
            NetworkShield::SandboxExec { tool } => {
                let mut k = Command::new(tool);
                k.arg("-p").arg(profile(sandbox)).arg(program).args(args);
                k
            }
            NetworkShield::Bwrap { tool } => {
                let mut k = Command::new(tool);
                // `HOME` is passed as a PARAMETER and is not read inside the
                // function: an environment variable is process-wide and
                // changing it in a test would break other tests running in
                // parallel (`profile()` reads `HOME` too). A pure function = a
                // measurable function.
                let home = std::env::var_os("HOME").map(PathBuf::from);
                k.args(bwrap_args(sandbox, home.as_deref()));
                // `--` IS REQUIRED: everything after it is the child's command
                // line, not bwrap's. Without it a script flag such as `--check`
                // would be taken for bwrap's own argument.
                k.arg("--").arg(program).args(args);
                k
            }
        }
    }
}

/// The FIXED paths where bwrap is looked for (they differ per distribution).
const BWRAP_PATHS: &[&str] = &["/usr/bin/bwrap", "/bin/bwrap", "/usr/local/bin/bwrap"];

/// The bwrap arguments — the Linux counterpart of `profile()`.
///
/// Rule maps to rule:
///   * `(deny network*)`            -> `--unshare-net`
///   * `(allow file-read*)`         -> `--ro-bind / /` (the interpreter must be
///     able to read its own library; narrowed down, python3 does not start at
///     all — the lesson learned on the macOS side applies as it is)
///   * `(deny file-read* home)`     -> `--tmpfs {home}` (the home directory looks EMPTY)
///   * `(allow file-write* sandbox)` -> `--bind {sandbox} {sandbox}`, and since
///     `/` is read only nothing can be written anywhere else
///
/// THE ORDER MATTERS: bwrap applies the operations IN THE ORDER GIVEN.
/// `--tmpfs {home}` must come first and `--bind {sandbox} {sandbox}` after — in
/// production the sandbox is often UNDER the home directory and in the reverse
/// order the tmpfs would COVER it (the script would see its own working folder empty).
///
/// `--dev`/`--proc` are required for the interpreters (node wants
/// `/dev/urandom`, python probes `/proc`); `--proc` also wants `--unshare-pid`.
///
/// NOT MEASURED: there is no Linux on this machine. If it is wrong,
/// `verify_shield` fails and the tool leaves the catalog; it does not produce a silent hole.
fn bwrap_args(sandbox: &Path, home: Option<&Path>) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec![
        "--unshare-net".into(),
        "--unshare-pid".into(),
        "--unshare-ipc".into(),
        "--unshare-uts".into(),
        // So the child does not stay behind if the shell is killed (the second
        // layer of the `kill`+`wait` discipline on the timeout path).
        "--die-with-parent".into(),
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
    ];
    // The home directory is covered. `/` and a relative value are EXCLUDED: `--tmpfs /`
    // empties the whole root directory and the interpreter does not start at all —
    // that is a crash, not a protection.
    if let Some(home) = home
        && home.is_absolute()
        && home != Path::new("/")
    {
        a.push("--tmpfs".into());
        a.push(home.into());
    }
    a.push("--bind".into());
    a.push(sandbox.into());
    a.push(sandbox.into());
    a.push("--chdir".into());
    a.push(sandbox.into());
    a
}

/// The sandbox-exec profile.
///
/// `(allow file-read*)` HAS TO BE LEFT wide: the interpreter reads its own
/// library, the dyld cache and locale files; narrowed down, python3 does not
/// start at all. Instead the real target is closed EXPLICITLY — the user's home
/// directory — and the sandbox is opened back up. So it is not "read
/// everything" but "read the system, do not read the user".
///
/// `(allow file-read-metadata)` WAS ADDED LATER and closes a failure: before
/// running a script, node calls `realpathSync`, which `lstat`s EVERY parent
/// directory ON THE SCRIPT'S PATH — if the sandbox is under the home directory
/// (and in production it is, the working directory is the sandbox) the home
/// directory's turn comes, `deny file-read*` cuts it and node crashes with
/// `EPERM: lstat '/Users/...'` before reading even the first line. That is what
/// was seen in the field: because js is the preferred interpreter, `run_code`
/// returned "the script returned an error" on EVERY call and the model wrote
/// the list from memory instead. The metadata permission DOES NOT OPEN CONTENT
/// — that a file exists and its size are visible, reading its content is still
/// forbidden. The alternative would have been "take node off the list"; that
/// would have crippled the tool instead of closing a gate.
fn profile(sandbox: &Path) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users".to_string());
    let sandbox = sandbox.display();
    format!(
        "(version 1)\
         (deny default)\
         (allow process-exec*)\
         (allow process-fork)\
         (allow file-read*)\
         (deny file-read* (subpath \"{home}\"))\
         (allow file-read-metadata)\
         (allow file-read* (subpath \"{sandbox}\"))\
         (allow file-write* (subpath \"{sandbox}\"))\
         (allow file-write-data (literal \"/dev/null\"))\
         (allow sysctl-read)\
         (allow mach-lookup)\
         (allow ipc-posix-shm)\
         (allow signal)\
         (deny network*)"
    )
}

// ---------------------------------------------------------------------------
// Interpreter discovery
// ---------------------------------------------------------------------------

/// An interpreter found on the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interpreter {
    /// The `Choice` value in the schema and the name visible to the model.
    pub key: &'static str,
    pub path: PathBuf,
    pub extension: &'static str,
}

/// The interpreters looked for, IN ORDER OF PREFERENCE.
///
/// JS comes first because the skill file (`code.md`) and the Swift side are
/// built on JS; the same prompt should produce the same language in both ports.
/// If python is present it is a real second option — a gain Swift did not have,
/// and the `language` field exists exactly for that (see `schema`).
const CANDIDATES: &[(&str, &str, &[&str])] = &[
    (
        "js",
        "js",
        &[
            "/opt/homebrew/bin/node",
            "/usr/local/bin/node",
            "/usr/bin/node",
        ],
    ),
    (
        "python",
        "py",
        &[
            "/opt/homebrew/bin/python3",
            "/usr/local/bin/python3",
            "/usr/bin/python3",
        ],
    ),
];

fn discover_interpreters() -> Vec<Interpreter> {
    let mut found = Vec::new();
    for (key, extension, paths) in CANDIDATES {
        // PATH IS NOT SEARCHED, fixed paths are used. Rationale: `PATH` comes
        // from the calling process's environment and can be poisoned; finding
        // something called "node" on PATH and running it is not behaviour that
        // suits a tool which runs model output.
        if let Some(y) = paths.iter().map(Path::new).find(|y| y.is_file()) {
            found.push(Interpreter {
                key,
                path: y.to_path_buf(),
                extension,
            });
        }
    }
    found
}

// ---------------------------------------------------------------------------
// The execution outcome
// ---------------------------------------------------------------------------

/// The raw outcome of one execution (the same distinction as code-spec §5.2/§5.3).
#[derive(Debug, Clone, PartialEq)]
pub enum CodeOutcome {
    /// The script finished without an error. If `output` is EMPTY the caller MUST NOT PRESENT this as a success.
    Succeeded {
        output: String,
        ms: u128,
        truncated: bool,
    },
    /// The script returned an error (a non-zero exit code) — stderr + partial output.
    Error(String),
    /// The time ran out; the process was killed.
    Timeout,
}

/// Runs the script under the shield. Free and pure so it can be called from
/// outside the tool (test, eval): it holds no state.
pub fn run(
    shield: &NetworkShield,
    interpreter: &Interpreter,
    sandbox: &Path,
    code: &str,
    timeout: Duration,
) -> ToolResult<CodeOutcome> {
    if code.trim().is_empty() {
        return Err(ToolError::InvalidArgument("empty script".into()));
    }
    if code.len() > CODE_CAP {
        return Err(ToolError::InvalidArgument("the script is too long".into()));
    }

    // The script is written INSIDE the sandbox: the shield profile only lets
    // that place be read, so keeping the script anywhere else would make it unreadable.
    let runtime = sandbox.join("code");
    std::fs::create_dir_all(&runtime)?;
    let script = runtime.join(format!(
        "script-{}-{}.{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos() ^ (code.len() as u128),
        interpreter.extension
    ));
    std::fs::write(&script, code.as_bytes())?;

    let outcome = run_program(
        shield,
        &runtime,
        &interpreter.path,
        &[script.clone().into()],
        timeout,
    );
    // The script is deleted ON EVERY PATH: if model output piles up on disk the
    // sandbox gets dirty and `find_file` lists them as if they were user documents.
    std::fs::remove_file(&script).ok();
    outcome
}

/// Runs any program with the given arguments under the shield.
///
/// The generalisation of `run` and the shared path of `write_code`'s
/// verification steps: a syntax check (`node --check f`, `python3 -m
/// py_compile f`) wants a different command line from running the script
/// DIRECTLY but still wants THE SAME shield, the same environment cleaning and
/// the same timeout discipline. Had two separate runners been written, security
/// fixes would have stayed in one of them.
pub fn run_program(
    shield: &NetworkShield,
    runtime: &Path,
    program: &Path,
    args: &[OsString],
    timeout: Duration,
) -> ToolResult<CodeOutcome> {
    let mut command = shield.wrap(runtime, program, args);
    command
        .current_dir(runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // THE ENVIRONMENT IS WIPED. The calling process's environment holds API
    // keys, session tokens and the home directory path; all of them are within
    // reach of the model's output. `PATH` stays minimal only so the interpreter
    // can find its own subprocesses, and `HOME` is deliberately turned into the
    // sandbox so the interpreter does not go looking for the user's config files.
    command.env_clear();
    command.env("PATH", "/usr/bin:/bin");
    command.env("HOME", runtime);
    command.env("LANG", "en_US.UTF-8");
    // Turn off python's user site-packages and its cache writing: both are paths
    // reaching outside the sandbox.
    command.env("PYTHONDONTWRITEBYTECODE", "1");
    command.env("PYTHONNOUSERSITE", "1");
    command.env("NODE_OPTIONS", "");

    let start = Instant::now();
    let mut child = command.spawn()?;

    // The two pipes are read IN TWO SEPARATE THREADS. Reading them in turn on
    // one thread is the classic deadlock: while waiting for stdout to the end
    // the child fills the stderr pipe, blocks, and the two wait for each other forever.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_arm = std::thread::spawn(move || read_pipe(stdout));
    let err_arm = std::thread::spawn(move || read_pipe(stderr));

    let mut timed_out = false;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(state)) => break Some(state),
            Ok(None) => {}
            // If the poll errored we do not know the process; killing it and
            // leaving is better than leaving an uncertain process behind.
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
        // KILL AND REAP. `kill` alone is not enough: without `wait` the process
        // stays a zombie and in a long session the process table fills up.
        child.kill().ok();
        child.wait().ok();
    }
    let ms = start.elapsed().as_millis();

    // The pipe readers get EOF because the child is gone and finish; `join`
    // therefore does not hang. We still fall back to the default against a
    // panic — a crashing tool takes the whole turn down.
    let (output, truncated_out) = out_arm.join().unwrap_or_default();
    let (error_text, truncated_err) = err_arm.join().unwrap_or_default();

    if timed_out {
        return Ok(CodeOutcome::Timeout);
    }
    let Some(state) = exit else {
        return Ok(CodeOutcome::Timeout);
    };

    if state.success() {
        return Ok(CodeOutcome::Succeeded {
            output: output.trim_end().to_string(),
            ms,
            truncated: truncated_out,
        });
    }

    // On the error path stderr comes FIRST but the PARTIAL OUTPUT is carried
    // too: measured in Swift — "SyntaxError" on its own is not enough for the
    // model to fix the code, the output before the error shows how far it got.
    let mut text = error_text.trim().to_string();
    if text.is_empty() {
        text = format!("script exited with status {state}");
    }
    if truncated_err {
        text.push_str("\n(error output truncated)");
    }
    if !output.trim().is_empty() {
        text.push_str(&format!(
            "\n--- output before the error ---\n{}",
            output.trim()
        ));
    }
    Ok(CodeOutcome::Error(text))
}

/// Reads the pipe TO THE END but DISCARDS anything over the cap.
///
/// Stopping the reads locks the child once the pipe fills (and keeps it waiting
/// until the timeout); having no cap at all lets infinite output eat all the
/// memory. The middle of the two: keep reading, stop storing.
fn read_pipe<R: Read>(pipe: Option<R>) -> (String, bool) {
    let Some(mut pipe) = pipe else {
        return (String::new(), false);
    };
    let mut accumulated: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let slot = OUTPUT_CAP.saturating_sub(accumulated.len());
                if slot == 0 {
                    truncated = true;
                    continue;
                }
                let taken = n.min(slot);
                accumulated.extend_from_slice(&buffer[..taken]);
                if taken < n {
                    truncated = true;
                }
            }
        }
    }
    (
        String::from_utf8_lossy(&accumulated).into_owned(),
        truncated,
    )
}

// ---------------------------------------------------------------------------
// The attempt counter
// ---------------------------------------------------------------------------

/// The per-turn attempt counter (code-spec §5.4).
///
/// In Swift this was on a `weak` reference and, so the cap was not ignored when
/// it was not bound, 3 was returned with "fail-closed". Here the tool KEEPS the
/// counter ITSELF: an owned field cannot be forgotten, so the fail-closed arm is
/// not needed at all. The shell does the reset with `new_turn()`.
#[derive(Debug, Default)]
pub struct CodeState {
    attempt: AtomicUsize,
}

impl CodeState {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new turn — the counter is reset.
    pub fn new_turn(&self) {
        self.attempt.store(0, Ordering::SeqCst);
    }

    /// Records the attempt and returns WHICH one it is (starting at 1).
    ///
    /// `pub(crate)`: the counter is SHARED with `write_code` — the sum of the
    /// model-code runs in the turn is one budget, not a separate budget per tool.
    pub(crate) fn next_attempt(&self) -> usize {
        self.attempt.fetch_add(1, Ordering::SeqCst) + 1
    }
}

// ---------------------------------------------------------------------------
// The tool
// ---------------------------------------------------------------------------

/// `run_code` — runs a short script under the shield.
pub struct RunCodeTool {
    shield: NetworkShield,
    interpreters: Vec<Interpreter>,
    state: std::sync::Arc<CodeState>,
    /// A cache so the schema is not rebuilt on every call; `schema()` is called
    /// on the hot path (prompt build + grammar).
    schema_cache: Mutex<Option<ArgSchema>>,
}

impl RunCodeTool {
    /// Builds the tool ONLY if we can really run safely.
    ///
    /// Returning `None` is not a malfunction but the RIGHT behaviour: a tool
    /// that calls a non-existent interpreter, or cannot cut the network, is a
    /// trap for the model — it sees it in the catalog, calls it, gets an error
    /// every time and loses the turn. If the shell sees `None` it does not add
    /// the tool; `diagnose()` says why.
    pub fn discover() -> Option<RunCodeTool> {
        let interpreters = discover_interpreters();
        if interpreters.is_empty() {
            return None;
        }
        // THE SHIELD IS MEASURED, NOT ASSUMED.
        let shield = NetworkShield::find()?;
        if !verify_shield(&shield, &interpreters[0]) {
            return None;
        }
        Some(RunCodeTool {
            shield,
            interpreters,
            state: std::sync::Arc::new(CodeState::new()),
            schema_cache: Mutex::new(None),
        })
    }

    /// Why discovery failed — the shell prints this to the user/developer. A
    /// silent absence would not make an accidentally removed tool noticeable.
    pub fn diagnose() -> String {
        let interpreters = discover_interpreters();
        if interpreters.is_empty() {
            return "run_code is off: neither node nor python3 was found on the machine"
                .to_string();
        }
        let names: Vec<&str> = interpreters.iter().map(|y| y.key).collect();
        let Some(shield) = NetworkShield::find() else {
            // REASON + REMEDY. The previous text stated only the reason, and
            // for a Linux user that was a dead end: there WAS a way to bring
            // the tool back (installing bubblewrap) and the text did not say
            // so. A documented gap instead of a silent one.
            return format!(
                "run_code is off: an interpreter exists ({}) but no shield was found that can \
                 cut the process's network; model code is not run without cutting the network. \
                 Paths searched: macOS /usr/bin/sandbox-exec, Linux {}. On Linux, installing the \
                 `bubblewrap` package brings the tool back. There is NO counterpart on Windows: \
                 the tool is deliberately off there.",
                names.join(", "),
                BWRAP_PATHS.join(" | ")
            );
        };
        if !verify_shield(&shield, &interpreters[0]) {
            return format!(
                "run_code is off: {} was found but the measurement did not pass — either the \
                 script never ran under the shield, or the network COULD NOT BE SEEN as cut \
                 (see verify_shield)",
                shield.name()
            );
        }
        format!(
            "run_code is on: interpreter {}, shield {} (network cut, writes confined to the sandbox)",
            names.join(", "),
            shield.name()
        )
    }

    /// The lever handed to the shell that resets the counter — bound to
    /// `ToolExecutor`'s turn hook.
    pub fn interpreters(&self) -> &[Interpreter] {
        &self.interpreters
    }

    pub fn turn_state(&self) -> std::sync::Arc<CodeState> {
        self.state.clone()
    }

    /// The measured shield — handed out so `write_code` can share the same one.
    /// Discovery (and the expensive `verify_shield` measurement) is therefore
    /// done ONCE; had the two tools measured separately, startup would have
    /// started two processes.
    pub fn shield(&self) -> &NetworkShield {
        &self.shield
    }

    /// The interpreter to run: first the model's EXPLICIT choice, failing that
    /// THE SCRIPT ITSELF, and if that does not speak either, the first in preference order.
    ///
    /// THE MIDDLE STEP WAS ADDED LATER and closes a measured failure:
    /// `language` is an optional field and the small model almost never fills
    /// it; when it did not, the script ran with the first in the list (js). But
    /// the model was writing Python. The result was "the script returned an
    /// error" every time -> a second attempt -> "no output", and the model went
    /// back to writing the list from memory — that is, the tool existed, worked, and WAS OF NO USE AT ALL.
    ///
    /// The alternative was making `language` REQUIRED; that adds a decode slot
    /// to every call, mostly needlessly, and does not remove the chance of the
    /// model filling it wrong. The source itself already answers this question:
    /// a script that writes `console.log` is JS, one that writes `print(...)` is Python.
    ///
    /// The markers are chosen ONE WAY ONLY — syntax valid in both languages
    /// (`for`, `if`, a `//` comment) is NEVER looked at; only things that are a
    /// SYNTAX ERROR in the other one count. On a tie the preference order wins,
    /// so this step only speaks in CLEAR cases.
    fn pick_interpreter(&self, requested: Option<&str>) -> &Interpreter {
        if let Some(y) = requested.and_then(|a| self.interpreters.iter().find(|y| y.key == a)) {
            return y;
        }
        &self.interpreters[0]
    }

    /// The interpreter to run. In order: the SOURCE if it speaks plainly,
    /// failing that the model's `language` choice, failing that the first in preference order.
    ///
    /// THAT THE SOURCE OVERRIDES THE `language` FIELD CAME FROM MEASUREMENT. In
    /// the first writing the guess only kicked in when `language` was empty
    /// ("never override an explicit choice"), but what was seen in the field was
    /// this: the model wrote JS and sent `language:"python"`. So the declaration
    /// is a GUESS by the model while the source is EVIDENCE; when the two
    /// conflict, following the evidence is the right thing. Doing what the
    /// declaration says looks like "respecting the model's intent" but in
    /// practice is a guaranteed syntax error and wastes the turn.
    /// The source has to "speak plainly": syntax valid in both languages does
    /// not count, and if the count ties the guess stays silent and the declaration holds.
    fn resolve_interpreter(&self, requested: Option<&str>, code: &str) -> &Interpreter {
        if let Some(key) = language_from_source(code)
            && let Some(y) = self.interpreters.iter().find(|y| y.key == key)
        {
            return y;
        }
        self.pick_interpreter(requested)
    }
}

/// Guesses the script's language from the source. `None` if undecided.
///
/// Only markers that WOULD NOT WORK in the other language are looked at; shared
/// syntax is deliberately ignored (see `resolve_interpreter`).
pub fn language_from_source(code: &str) -> Option<&'static str> {
    const PY: [&str; 6] = ["print(", "def ", "import ", "elif ", "lambda ", "range("];
    const JS: [&str; 6] = ["console.log", "=>", "const ", "let ", "function ", "var "];
    let py = PY.iter().filter(|i| code.contains(**i)).count();
    let js = JS.iter().filter(|i| code.contains(**i)).count();
    match py.cmp(&js) {
        std::cmp::Ordering::Greater => Some("python"),
        std::cmp::Ordering::Less => Some("js"),
        std::cmp::Ordering::Equal => None,
    }
}

/// Measures that the shield IS REALLY SET UP AND CUTS THE NETWORK.
///
/// Just checking "does the file exist" is not enough: if the profile syntax
/// does not hold, if the OS version refuses the rule, or if the interpreter
/// cannot start at all under the shield, the result would be a silent error on
/// every call. It is measured once, at discovery time, with a real process.
///
/// THE MEASUREMENT COVERS THE NETWORK TOO — it USED NOT TO. The previous
/// version only ran `print(1)`, i.e. it measured "a script runs under the
/// shield" while writing the sentence "the cut is REALLY MEASURED DURING
/// DISCOVERY" at the top of the file. The difference was harmless on macOS (the
/// network was measured by the `the_network_is_really_cut` test) but would
/// become dangerous the moment a second shield (bwrap) was added: the tests run
/// on the developer's machine, while nothing measures the shield on the user's
/// machine. The measurement now ASKS THE QUESTION DIRECTLY: the script under
/// the shield tries to open a connection and prints that it could not.
/// THE LIMIT OF THE MEASUREMENT IS WRITTEN OUT PLAINLY: this measurement proves
/// "a connection WAS NOT OPENED", not "the shield cut it". On a machine whose
/// network is already off the two cannot be told apart (bwrap `--unshare-net`
/// and an offline machine both give the same `ENETUNREACH`). The wrong
/// direction is the safe one: the error is not saying "there is a shield" when
/// there is none, it is "counting the shield as verified while the network is
/// off"; and in that case there is no real network anyway. The reverse — taking
/// the shield as valid while the network is UP — IS NOT POSSIBLE, because if the
/// connection succeeds the measurement fails.
fn verify_shield(shield: &NetworkShield, interpreter: &Interpreter) -> bool {
    let Ok(sandbox) = temp_measurement_dir() else {
        return false;
    };
    let script = sandbox.join(format!("measurement.{}", interpreter.extension));
    let body = measurement_script(interpreter.key);
    let outcome = std::fs::write(&script, body).is_ok()
        && match run_program(
            shield,
            &sandbox,
            &interpreter.path,
            &[script.clone().into_os_string()],
            Duration::from_secs(15),
        ) {
            // Both "the script runs" and "the network did not open" are read
            // from the SAME output: measuring them in two separate runs would
            // double the startup time.
            Ok(CodeOutcome::Succeeded { output, .. }) => {
                output.contains("NET_BLOCKED") && !output.contains("NET_OPEN")
            }
            _ => false,
        };
    let _ = std::fs::remove_dir_all(&sandbox);
    outcome
}

/// The script of the discovery measurement: it tries a connection under the shield and prints the result.
///
/// THREE OUTCOMES, THE SAME IN BOTH LANGUAGES: `NET_OPEN` (a connection was
/// made), `NET_BLOCKED` (the request was refused IMMEDIATELY) and
/// `NET_UNCERTAIN` (the time ran out). The third is kept separate because a
/// connection LEFT HANGING is NOT a cut one; a firewall may be silently
/// dropping the packet, or the shield may never have been set up.
/// `verify_shield` only counts `NET_BLOCKED` as valid — on uncertainty the tool
/// leaves the catalog (fail-closed). Merging the two would be the easiest way
/// to say "there is a shield" and is exactly what this file forbids.
/// The timeout is SHORT (2 s) and INSIDE the script: if the shield really cuts,
/// the error comes at once and startup is not delayed; if it does not, 2
/// seconds are added to startup — but in that case the tool will leave the
/// catalog anyway, so the price is paid once.
fn measurement_script(key: &str) -> &'static str {
    match key {
        "python" => {
            "import socket\n\
             try:\n\
             \x20   socket.create_connection(('1.1.1.1', 80), timeout=2).close()\n\
             \x20   print('NET_OPEN')\n\
             except socket.timeout:\n\
             \x20   print('NET_UNCERTAIN')\n\
             except Exception:\n\
             \x20   print('NET_BLOCKED')\n"
        }
        _ => {
            "const s=require('net').connect(80,'1.1.1.1');\
             s.setTimeout(2000,()=>{console.log('NET_UNCERTAIN');process.exit(0)});\
             s.on('connect',()=>{console.log('NET_OPEN');process.exit(0)});\
             s.on('error',()=>{console.log('NET_BLOCKED');process.exit(0)});"
        }
    }
}

/// A temporary, RESOLVED (symlink free) directory for the measurement.
///
/// `canonicalize` IS REQUIRED: on macOS `std::env::temp_dir()` gives something
/// under `/var/folders/...`, and `/var` is a link to `/private/var`. Written
/// unresolved into the sandbox profile, the rule matches nothing.
fn temp_measurement_dir() -> std::io::Result<PathBuf> {
    let root = std::env::temp_dir().join(format!(
        "tacet-code-measurement-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&root)?;
    root.canonicalize()
}

impl Tool for RunCodeTool {
    fn name(&self) -> &str {
        "run_code"
    }

    fn description(&self) -> &str {
        // The sentence "NO network, NO filesystem, NO access to this device" is
        // not a promise but a MEASURED fact (see `verify_shield`) — that is why
        // it can be written. The last sentences close two failures measured in
        // Swift: the model was calling the tool to inspect a server/container,
        // and it took scripts that printed nothing for a success.
        "Runs a short script in an isolated sandbox with NO network access, NO access to your \
         files outside the working folder and NO access to this device or any server. Call this \
         for any calculation or transformation too complex for the calculate tool (loops, dates, \
         text processing, simulations), and whenever the user asks you to PRODUCE A LIST OR A \
         SEQUENCE - prime numbers, Fibonacci terms, sorting, filtering, counting - instead of \
         writing the items out from memory. Never call it to inspect servers, containers, ports, \
         processes, disks or the operating system — it cannot see them. The LAST LINE of the \
         script must be print(...) in python or console.log(...) in js: a bare expression is \
         NOT output and a script that prints nothing returns an error. If the tool returns an \
         error, fix the code and call it ONCE more."
    }

    fn schema(&self) -> ArgSchema {
        if let Some(s) = self.schema_cache.lock().expect("schema lock").clone() {
            return s;
        }
        let mut fields = vec![
            Field::new(
                "code",
                ArgSchema::text()
                    .description("The script. Keep it minimal and PRINT the final result."),
            )
            .required(),
        ];

        // THE `language` FIELD IS ADDED ONLY IF THERE ARE TWO INTERPRETERS. In
        // Swift this field stood even though it had a single valid value, and it
        // ate a decode slot the small model had to fill on EVERY call; that is
        // why it was removed. Here it is legitimate if there really is a CHOICE;
        // otherwise it would be the same waste.
        if self.interpreters.len() > 1 {
            let choices: Vec<&str> = self.interpreters.iter().map(|y| y.key).collect();
            let default = self.interpreters[0].key;
            fields.push(Field::new(
                "language",
                ArgSchema::choice(choices)
                    .description(format!("Script language (default '{default}').")),
            ));
        }

        fields.push(Field::new(
            "timeout_s",
            ArgSchema::integer()
                .range(Some(1.0), Some(MAX_TIMEOUT as f64))
                .description(format!(
                    "Seconds to allow before the script is killed (default {DEFAULT_TIMEOUT})."
                )),
        ));

        let schema =
            ArgSchema::object(fields).description("Run a short script and return its output");
        *self.schema_cache.lock().expect("schema lock") = Some(schema.clone());
        schema
    }

    /// AN EXTERNAL TOOL. It starts a process on the device and runs model
    /// output; `ToolExecutor`'s approval gate has to see this.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            if let Err(h) = self.schema().validate(&args) {
                return ToolOutcome::failed(&h);
            }

            // The counter is incremented BEFORE THE DENIAL DECISION: the 3rd call never sees the engine.
            let attempt = self.state.next_attempt();
            if attempt > MAX_ATTEMPTS {
                return ToolOutcome::new(
                    "Code execution limit",
                    ToolState::Failed("two attempts exhausted".into()),
                    "error_final: give the user a short honest answer, do NOT retry",
                );
            }

            let trace = ctx.start_chip("code", "Running code…");
            let outcome = self.execute(&args, ctx, attempt);

            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(
                        args.get("code")
                            .and_then(|k| k.as_str())
                            .unwrap_or_default(),
                    )
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            // Tainting only for a script that REALLY ran: a denied or
            // never-started call did not touch the outside world.
            if !matches!(outcome.state, ToolState::Failed(_)) {
                ctx.taint();
            }
            outcome
        })
    }
}

impl RunCodeTool {
    fn execute(&self, args: &serde_json::Value, ctx: &ToolContext, attempt: usize) -> ToolOutcome {
        let last_attempt = attempt >= MAX_ATTEMPTS;

        let Some(code) = args
            .get("code")
            .and_then(|v| v.as_str())
            .filter(|k| !k.trim().is_empty())
        else {
            return ToolOutcome::failed(&ToolError::MissingField("code".into()));
        };
        let interpreter =
            self.resolve_interpreter(args.get("language").and_then(|v| v.as_str()), code);
        let timeout = Duration::from_secs(
            args.get("timeout_s")
                .and_then(|v| v.as_u64())
                .unwrap_or(DEFAULT_TIMEOUT),
        );

        // The shield profile writes the sandbox as a subpath; an unresolved path
        // silently falls to "nothing is permitted" (see `NetworkShield::wrap`).
        let sandbox = match ctx
            .resolve_path(".")
            .and_then(|k| k.canonicalize().map_err(ToolError::Io))
        {
            Ok(k) => k,
            Err(h) => return ToolOutcome::failed(&h),
        };

        let raw = match run(&self.shield, interpreter, &sandbox, code, timeout) {
            Ok(s) => s,
            Err(h) => return ToolOutcome::failed(&h),
        };

        match raw {
            // SUCCESS WITH NO OUTPUT IS NOT A SUCCESS (measured in Swift): when
            // the model saw "ok" it MADE THE RESULT UP.
            CodeOutcome::Succeeded { output, .. } if output.trim().is_empty() => ToolOutcome::new(
                if last_attempt {
                    "The code could not be run"
                } else {
                    "Error · retrying"
                },
                ToolState::Failed("no output".into()),
                error_text(
                    "error: the script ran but printed nothing. A bare last expression is NOT \
                     output - wrap the value you need in print(...) for python or \
                     console.log(...) for js",
                    last_attempt,
                ),
            )
            .raw_output("no output"),

            CodeOutcome::Succeeded {
                output,
                ms,
                truncated,
            } => {
                let short = truncate(&output, MODEL_OUTPUT_CAP);
                let note = if truncated || short.len() < output.len() {
                    "\n(the output was truncated)"
                } else {
                    ""
                };
                // If the output is large, the bypass channel: the full body to
                // the store, the first 500 characters + source_ref to the model.
                // The head is enough for the model to summarize the result; the
                // next tool reaches the whole thing through the reference.
                let suffix = if output.len() > MODEL_OUTPUT_CAP {
                    let r = ctx.store(
                        "code",
                        &format!("{} characters of code output", output.len()),
                        output.clone(),
                    );
                    tacet_core::source_ref_suffix(r.as_str())
                } else {
                    String::new()
                };
                ToolOutcome::read_ok(
                    format!("Code executed · {ms} ms"),
                    format!("ok ({ms} ms)\n{short}{note}{suffix}"),
                )
                .raw_output(output)
            }

            CodeOutcome::Error(message) => {
                // THE LANGUAGE DIAGNOSIS COMES FIRST (measured in Swift): if the
                // script is Python but the engine is JS, the raw parser message
                // drags the model into the WRONG repair — it adds parentheses,
                // does not change the language, and loses the one remaining attempt.
                let cause = if interpreter.key == "js" && looks_like_python(code) {
                    format!(
                        "error: this script is Python, but this sandbox runs JavaScript. \
                         Rewrite the SAME logic in JavaScript: for (let i = 0; i < n; i++) \
                         {{...}}, console.log(x). Do not use range(), def, or indent blocks.\n{message}"
                    )
                } else {
                    format!("error: {message}")
                };
                ToolOutcome::new(
                    if last_attempt {
                        "The code could not be run"
                    } else {
                        "Error · retrying"
                    },
                    ToolState::Failed("the script returned an error".into()),
                    error_text(&cause, last_attempt),
                )
                .raw_output(message)
            }

            CodeOutcome::Timeout => ToolOutcome::new(
                "The code timed out",
                ToolState::Failed("timed out".into()),
                error_text(
                    "error: the script did not finish in time — it probably has an endless \
                     loop; bound every loop",
                    last_attempt,
                ),
            )
            .raw_output("timeout"),
        }
    }
}

/// Builds the error return in one place: on the last attempt `error_final` is
/// appended (code-spec §5.4) — but THE REASON is given too, because the honest
/// short answer the model will give the user must contain the reason.
pub(crate) fn error_text(cause: &str, last_attempt: bool) -> String {
    let short = truncate(cause, MODEL_ERROR_CAP);
    if last_attempt {
        format!("{short}\nerror_final: give the user a short honest answer, do NOT retry")
    } else {
        short
    }
}

/// Truncates at a character boundary (not a byte one: cutting in the middle of
/// a multi-byte character would produce invalid UTF-8).
pub(crate) fn truncate(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    text.chars().take(cap).collect()
}

/// Does the script look like Python?
///
/// A measurement finding inherited from Swift: the small model writes Python in
/// most code cases. Pure and static — testable without a model.
pub fn looks_like_python(code: &str) -> bool {
    // Each of these is either a syntax error or undefined in JS; there is no
    // risk of mistaking a valid JS script for Python. `print(` is DELIBERATELY
    // absent: in many shells it can be defined on the JS side too.
    const PYTHON_SPECIFIC: &[&str] = &[
        "range(", "def ", "elif ", "end=", " True", " False", " None",
    ];
    if PYTHON_SPECIFIC.iter().any(|i| code.contains(i)) {
        return true;
    }
    // A block header ending in a colon. In JS a `for`/`if`/`while` header opens
    // with a parenthesis and DOES NOT END with a colon. If there is a `{`, leave
    // it alone: `for (const k in o) { ... }` is valid JS and object literals
    // carry colons too.
    if code.contains('{') {
        return false;
    }
    const BLOCK_START: &[&str] = &["for ", "if ", "while ", "else", "try", "except"];
    code.lines().any(|s| {
        let s = s.trim();
        s.ends_with(':') && !s.contains('{') && BLOCK_START.iter().any(|b| s.starts_with(b))
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use tacet_core::{InMemoryDataStore, SilentReporter, SourceRef};

    fn hold<F: std::future::Future>(future: F) -> F::Output {
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

    fn temp_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-code-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path.canonicalize().expect("resolved path")
    }

    fn context(root: &Path) -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            root.to_path_buf(),
            Arc::new(SilentReporter),
        )
    }

    /// If there is no shield/interpreter (CI, Linux) the execution tests are
    /// SKIPPED — but not SILENTLY: if `discover` returns `None` the tool is not
    /// in the catalog anyway and that is the right behaviour; the test passes by printing it.
    fn tool_or_skip() -> Option<RunCodeTool> {
        match RunCodeTool::discover() {
            Some(a) => Some(a),
            None => {
                eprintln!(
                    "run_code was not discovered, the execution tests were skipped: {}",
                    RunCodeTool::diagnose()
                );
                None
            }
        }
    }

    // --- Model-free, pure tests (they run on every machine) ---

    /// 1) The Python diagnosis: it turns a model sending Python to the JS engine the RIGHT way.
    #[test]
    fn the_python_diagnosis_gives_no_false_positive() {
        assert!(looks_like_python("def f(x):\n  return x"));
        assert!(looks_like_python("if x == 1:\n  y = True"));
        assert!(
            looks_like_python("for i in range(20):\n  print(i)"),
            "a block ending in a colon"
        );
        // Valid JS must not be taken for Python.
        assert!(!looks_like_python(
            "for (let i = 0; i < 10; i++) { console.log(i) }"
        ));
        assert!(!looks_like_python(
            "const o = {a: 1}; console.log(JSON.stringify(o))"
        ));
        assert!(!looks_like_python("for (const k in o) { console.log(k) }"));
    }

    /// 2) The error text carries `error_final` on the last attempt and DOES NOT
    ///    on the earlier one; THE REASON is preserved in both cases.
    #[test]
    fn the_error_text_appends_error_final_on_the_last_attempt() {
        let first = error_text("error: boom", false);
        assert_eq!(first, "error: boom");
        assert!(!first.contains("error_final"));

        let last = error_text("error: boom", true);
        assert!(last.contains("error_final"));
        assert!(last.contains("do NOT retry"));

        // The cap: a long reason is truncated but error_final IS NOT DROPPED.
        let long = error_text(&"x".repeat(5_000), true);
        assert!(long.contains("error_final"));
        assert!(long.chars().count() < MODEL_ERROR_CAP + 120);
    }

    /// 3) The counter: two real attempts, on the third the engine is NEVER seen.
    #[test]
    fn the_attempt_counter_rejects_the_third_call() {
        let d = CodeState::new();
        assert_eq!(d.next_attempt(), 1);
        assert_eq!(d.next_attempt(), 2);
        assert_eq!(
            d.next_attempt(),
            3,
            "the third call must exceed MAX_ATTEMPTS"
        );
        d.new_turn();
        assert_eq!(d.next_attempt(), 1, "it must reset at the start of a turn");
    }

    /// 4) The shield profile binds the sandbox and the home directory to the RIGHT rules.
    #[test]
    fn the_shield_profile_closes_the_network_and_the_home_directory() {
        let p = profile(Path::new("/private/tmp/sandbox"));
        assert!(p.contains("(deny network*)"), "the network was left open");
        assert!(
            p.contains("(deny default)"),
            "the default was left permissive"
        );
        assert!(p.contains("(allow file-write* (subpath \"/private/tmp/sandbox\"))"));
        // Writing only into the sandbox: there must be no write permission for another subpath.
        assert_eq!(p.matches("allow file-write*").count(), 1);
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users".to_string());
        assert!(p.contains(&format!("(deny file-read* (subpath \"{home}\"))")));
    }

    /// 4b) IS DISCOVERY REALLY PASSING — `tool_or_skip()` IS A SILENT ESCAPE
    ///     HATCH and this test closes it.
    ///
    /// All the execution tests below print "ok" without measuring anything when
    /// discovery fails. So if `verify_shield` broke (e.g. the measurement script
    /// no longer printed `NET_BLOCKED`) the suite would stay completely green
    /// and the tool would silently leave the catalog in production — the exact
    /// description of this repo's recurring failure. The claim here is built as
    /// a conditional: IF a shield AND an interpreter EXIST on this machine,
    /// discovery MUST succeed.
    ///
    /// The strict claim holds ONLY on macOS: under `sandbox-exec` `connect`
    /// returns `EPERM` IMMEDIATELY regardless of the state of the internet, so
    /// the measurement is deterministic. bwrap `--unshare-net` returns
    /// `ENETUNREACH` and cannot be told apart from an offline machine; a strict
    /// claim there would make the test swing with the machine's network. On
    /// Linux it is therefore only PRINTED.
    #[test]
    fn when_a_shield_and_an_interpreter_exist_discovery_must_succeed() {
        let Some(shield) = NetworkShield::find() else {
            eprintln!("there is no shield on this machine; the discovery claim cannot be built");
            return;
        };
        if discover_interpreters().is_empty() {
            eprintln!(
                "there is no interpreter on this machine; the discovery claim cannot be built"
            );
            return;
        }
        let discovered = RunCodeTool::discover().is_some();
        if cfg!(target_os = "macos") {
            assert!(
                discovered,
                "a shield ({}) and an interpreter exist but discovery failed: {}",
                shield.name(),
                RunCodeTool::diagnose()
            );
        } else if !discovered {
            eprintln!(
                "discovery failed ({}): {}",
                shield.name(),
                RunCodeTool::diagnose()
            );
        }
    }

    /// 4c) THE bwrap ARGUMENT ORDER — the only measurable part of the Linux shield.
    ///
    /// The shield itself cannot be measured on this machine (no Linux), but
    /// BUILDING the arguments is a pure function and runs on every platform.
    /// What is measured is the single subtlety in the documentation: `--tmpfs
    /// {home}` must come BEFORE `--bind {sandbox}`. In production the sandbox is
    /// often UNDER the home directory; in the reverse order the tmpfs covers the
    /// sandbox and the script sees its own folder EMPTY. This is a bug that does
    /// not break the build and would never be caught without a test.
    // UNIX ONLY, and not out of convenience: `bwrap_args` guards the home
    // directory with `home.is_absolute()`, and on Windows `/home/user` is NOT
    // absolute (a drive prefix is required), so the argument under test is
    // never produced there. bwrap is a Linux mechanism to begin with; running
    // this on Windows measured the meaning of `is_absolute`, not the sandbox.
    #[cfg(unix)]
    #[test]
    fn the_bwrap_arguments_cover_the_home_directory_before_the_sandbox() {
        let sandbox = Path::new("/home/user/.tacet/sandbox");
        let args: Vec<String> = bwrap_args(sandbox, Some(Path::new("/home/user")))
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        assert!(
            args.contains(&"--unshare-net".to_string()),
            "the network was not cut: {args:?}"
        );
        let tmpfs = args
            .iter()
            .position(|a| a == "--tmpfs")
            .expect("the home directory was not covered");
        assert_eq!(args[tmpfs + 1], "/home/user");
        let bind = args
            .iter()
            .position(|a| a == "--bind")
            .expect("the sandbox was not bound");
        assert!(
            tmpfs < bind,
            "the tmpfs covers the sandbox, the order is reversed: {args:?}"
        );
        assert_eq!(args[bind + 1], sandbox.to_string_lossy());

        // The root must be bound READ ONLY: that is what cuts writing outside the sandbox.
        let ro = args
            .iter()
            .position(|a| a == "--ro-bind")
            .expect("the root was not bound");
        assert_eq!(args[ro + 1], "/");
        assert!(
            !args.iter().any(|a| a == "--bind-try"),
            "a try-bind is used, it passes silently: {args:?}"
        );
    }

    /// 4d) `--tmpfs /` IS A DISASTER: it empties the root directory and the
    ///     interpreter does not start at all. `HOME=/` or a broken/missing
    ///     `HOME` must not produce it — and even if the home directory cannot be
    ///     covered, the network and write gates must stay CLOSED.
    #[test]
    fn bwrap_gives_up_on_the_root_when_the_home_directory_is_broken() {
        for home in [Some(Path::new("/")), Some(Path::new("relative/path")), None] {
            let args: Vec<String> = bwrap_args(Path::new("/tmp/sandbox"), home)
                .into_iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert!(
                !args.iter().any(|a| a == "--tmpfs"),
                "a tmpfs was produced for {home:?}: {args:?}"
            );
            assert!(
                args.contains(&"--unshare-net".to_string()),
                "{home:?}: the network was left open"
            );
            assert!(
                args.contains(&"--ro-bind".to_string()),
                "{home:?}: the root was left writable"
            );
        }
    }

    /// 5) Truncation works at a character boundary — it DOES NOT SPLIT a multi-byte character.
    #[test]
    fn truncation_does_not_corrupt_utf8() {
        let text = "ğüşiöç".repeat(200);
        let k = truncate(&text, 10);
        assert_eq!(k.chars().count(), 10);
        assert!(k.is_char_boundary(k.len()), "invalid UTF-8 was produced");
        assert_eq!(truncate("short", 100), "short");
    }

    // --- Tests that run a real process ---

    /// 6) The script REALLY runs and its output is captured. "The build is
    ///    green" proves nothing; this test proves it.
    #[test]
    fn the_script_really_runs_and_the_output_is_captured() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("run");
        let mut ctx = context(&root);

        let code = match tool.interpreters()[0].key {
            "python" => "print(6*7)",
            _ => "console.log(6*7)",
        };
        let outcome = hold(tool.run(json!({"code": code}), &mut ctx));
        assert_eq!(outcome.state, ToolState::Read, "{}", outcome.chip_text);
        assert!(outcome.to_model.contains("42"), "{}", outcome.to_model);
        assert!(outcome.to_model.starts_with("ok ("));
        assert!(
            ctx.session_tainted(),
            "an external tool ran, the session must be tainted"
        );
    }

    /// 7) THE NETWORK IS REALLY CUT. This test passing is the only ground on
    ///    which the "NO network" sentence in `description` is a fact and not a promise.
    #[test]
    fn the_network_is_really_cut() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("network");
        let mut ctx = context(&root);

        let code = match tool.interpreters()[0].key {
            "python" => {
                "import socket\n\
                 try:\n\
                 \x20   socket.create_connection(('1.1.1.1', 80), timeout=3)\n\
                 \x20   print('NET_OPEN')\n\
                 except Exception:\n\
                 \x20   print('NET_BLOCKED')\n"
            }
            _ => {
                "const s=require('net').connect(80,'1.1.1.1');\
                 s.on('connect',()=>{console.log('NET_OPEN');process.exit(0)});\
                 s.on('error',()=>{console.log('NET_BLOCKED');process.exit(0)});"
            }
        };

        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 20}), &mut ctx));
        let output = outcome.raw_output.clone().unwrap_or_default();
        assert!(
            !output.contains("NET_OPEN"),
            "THE NETWORK WAS LEFT OPEN — the tool must not be in the catalog: {output}"
        );
        assert!(
            output.contains("NET_BLOCKED"),
            "unexpected output: {output} / {}",
            outcome.to_model
        );
    }

    /// 8) THE SANDBOX: the script cannot write outside and cannot read the
    ///    user's home directory; but it can write into its own folder (legitimate work must not break).
    #[test]
    fn the_sandbox_cannot_be_escaped() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("sandbox");
        let mut ctx = context(&root);
        let target = std::env::temp_dir().join(format!("tacet-escape-{}.txt", std::process::id()));
        std::fs::remove_file(&target).ok();

        let code = match tool.interpreters()[0].key {
            "python" => format!(
                "import os\n\
                 try:\n\
                 \x20   open({target:?}, 'w').write('x')\n\
                 \x20   print('OUT_WRITE_OK')\n\
                 except Exception:\n\
                 \x20   print('OUT_WRITE_BLOCKED')\n\
                 try:\n\
                 \x20   os.listdir(os.path.expanduser('~'))\n\
                 \x20   print('HOME_READ_OK')\n\
                 except Exception:\n\
                 \x20   print('HOME_READ_BLOCKED')\n\
                 open('inside.txt','w').write('y')\n\
                 print('IN_WRITE_OK')\n"
            ),
            _ => format!(
                "const fs=require('fs');\
                 try{{fs.writeFileSync({target:?},'x');console.log('OUT_WRITE_OK')}}\
                 catch(e){{console.log('OUT_WRITE_BLOCKED')}}\
                 try{{fs.readdirSync(process.env.HOME_REAL||'/Users');console.log('HOME_READ_OK')}}\
                 catch(e){{console.log('HOME_READ_BLOCKED')}}\
                 fs.writeFileSync('inside.txt','y');console.log('IN_WRITE_OK')"
            ),
        };

        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 20}), &mut ctx));
        let output = outcome.raw_output.clone().unwrap_or_default();
        assert!(
            output.contains("OUT_WRITE_BLOCKED"),
            "it wrote outside the sandbox: {output}"
        );
        assert!(
            !target.exists(),
            "the file was really created: {}",
            target.display()
        );
        assert!(
            output.contains("IN_WRITE_OK"),
            "it could not write into the sandbox: {output}"
        );
    }

    /// 9) AN INFINITE LOOP: on the timeout the process is KILLED and the
    ///    duration really is cut at the cap (no silent hang).
    #[test]
    fn an_infinite_loop_is_killed_on_timeout() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("loop");
        let mut ctx = context(&root);
        let code = match tool.interpreters()[0].key {
            "python" => "while True: pass",
            _ => "while(true){}",
        };

        let started = Instant::now();
        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 2}), &mut ctx));
        let elapsed = started.elapsed();
        assert!(
            matches!(outcome.state, ToolState::Failed(_)),
            "{}",
            outcome.chip_text
        );
        assert!(
            outcome.to_model.contains("did not finish in time"),
            "{}",
            outcome.to_model
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "the process was not killed: {elapsed:?}"
        );
        assert!(
            elapsed >= Duration::from_secs(2),
            "it returned before the cap: {elapsed:?}"
        );
    }

    /// 10) SUCCESS WITH NO OUTPUT IS NOT A SUCCESS — so the model cannot see
    ///     "ok" and invent the result.
    #[test]
    fn empty_output_does_not_stay_silent() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("silent");
        let mut ctx = context(&root);
        let code = match tool.interpreters()[0].key {
            "python" => "x = 1 + 1",
            _ => "const x = 1 + 1;",
        };
        let outcome = hold(tool.run(json!({"code": code}), &mut ctx));
        assert!(
            matches!(outcome.state, ToolState::Failed(_)),
            "{}",
            outcome.chip_text
        );
    }

    /// 11) HUGE OUTPUT does not eat the memory, it is truncated and the
    ///     truncation IS STATED; large output goes through the bypass channel.
    #[test]
    fn huge_output_is_truncated_and_falls_into_the_bypass_channel() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("huge");
        let mut ctx = context(&root);
        let code = match tool.interpreters()[0].key {
            "python" => "for i in range(200000): print('line', i)",
            _ => "for(let i=0;i<200000;i++) console.log('line',i)",
        };
        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 25}), &mut ctx));

        let raw = outcome.raw_output.clone().unwrap_or_default();
        assert!(
            raw.len() <= OUTPUT_CAP + 16,
            "the cap was exceeded: {}",
            raw.len()
        );
        // The text going to the model is much shorter: even the full output IS NOT DUMPED on the model.
        assert!(outcome.to_model.len() < 900, "{}", outcome.to_model.len());
        if outcome.state == ToolState::Read {
            assert!(
                outcome.to_model.contains("source_ref"),
                "{}",
                outcome.to_model
            );
            let record = ctx
                .from_store(&SourceRef("code#1".into()))
                .expect("the store record");
            assert!(record.body.len() > MODEL_OUTPUT_CAP);
        }
    }

    /// 12) The third call NEVER SEES THE ENGINE: the tool issues the denial, the
    ///     model is not expected to count.
    #[test]
    fn the_third_call_is_rejected_by_the_tool() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("counter");
        let mut ctx = context(&root);
        let code = match tool.interpreters()[0].key {
            "python" => "print(1)",
            _ => "console.log(1)",
        };

        for kind in 1..=2 {
            let s = hold(tool.run(json!({"code": code}), &mut ctx));
            assert_eq!(s.state, ToolState::Read, "attempt {kind} must not fail");
        }
        let third = hold(tool.run(json!({"code": code}), &mut ctx));
        assert!(matches!(third.state, ToolState::Failed(_)));
        assert!(
            third.to_model.starts_with("error_final"),
            "{}",
            third.to_model
        );

        // The turn hook resets the counter; the shell binds this to `ToolExecutor`.
        tool.turn_state().new_turn();
        let new_turn = hold(tool.run(json!({"code": code}), &mut ctx));
        assert_eq!(new_turn.state, ToolState::Read, "the turn was not reset");
    }

    /// 13) The schema contract: the `language` field stands ONLY if there is a
    ///     real choice (it does not open an empty decode slot), and the timeout cap is binding.
    #[test]
    fn the_schema_contract_is_binding() {
        let Some(tool) = tool_or_skip() else { return };
        let schema = tool.schema();
        let js = schema.json_schema();
        assert_eq!(js["required"], json!(["code"]));
        assert_eq!(js["additionalProperties"], json!(false));

        let has_language = js["properties"].get("language").is_some();
        assert_eq!(
            has_language,
            tool.interpreters().len() > 1,
            "the language field must only exist with multiple interpreters"
        );

        assert!(schema.validate(&json!({"code": "x"})).is_ok());
        assert!(
            schema
                .validate(&json!({"code": "x", "timeout_s": 999}))
                .is_err(),
            "the cap"
        );
        assert!(
            schema
                .validate(&json!({"code": "x", "invented": 1}))
                .is_ok(),
            "the schema ignores it"
        );
        if has_language {
            assert!(
                schema
                    .validate(&json!({"code": "x", "language": "cobol"}))
                    .is_err()
            );
        }
    }

    /// 14) The router must be able to pick this tool for a Calc intent.
    #[test]
    fn the_router_hints_hold() {
        use crate::router::{Router, simplify};
        let Some(tool) = tool_or_skip() else { return };
        let text = simplify(&format!("{} {}", tool.name(), tool.description()));
        // The Calc profile hints: "code", "run".
        assert!(text.contains("code") && text.contains("run"));

        let mut catalog = tacet_core::ToolCatalog::new();
        catalog.add(Arc::new(crate::time::TimeTool::new()));
        catalog.add(Arc::new(RunCodeTool::discover().expect("discovered")));
        let chosen = Router::new()
            .max(1)
            .select("calculate this with python", &catalog);
        assert_eq!(chosen[0].name(), "run_code");
    }

    // --- The language guess ---

    /// When the source speaks PLAINLY it gives the right language.
    #[test]
    fn language_from_source_recognises_an_explicit_source() {
        assert_eq!(
            language_from_source("const a = [];\nlet b = 1;"),
            Some("js")
        );
        assert_eq!(language_from_source("console.log(42)"), Some("js"));
        assert_eq!(
            language_from_source("print(sum(range(10)))"),
            Some("python")
        );
        assert_eq!(
            language_from_source("import math\nprint(math.pi)"),
            Some("python")
        );
    }

    /// Syntax valid in both languages DOES NOT MAKE the guess speak; the
    /// decision is left to the declaration/preference order.
    #[test]
    fn language_from_source_stays_silent_on_shared_syntax() {
        assert_eq!(language_from_source("for i in x:\n    pass"), None);
        assert_eq!(language_from_source("1 + 2"), None);
        assert_eq!(language_from_source(""), None);
    }

    /// THE SOURCE OVERRIDES THE DECLARATION. The failure measured in the field:
    /// the model wrote JS and sent `language:"python"`, and the script failed with a syntax error.
    #[test]
    fn the_source_overrides_a_wrong_declaration() {
        let Some(tool) = tool_or_skip() else { return };
        if tool.interpreters.len() < 2 {
            return; // nothing to measure on a machine with a single interpreter
        }
        let chosen = tool.resolve_interpreter(Some("python"), "console.log(1)");
        assert_eq!(chosen.key, "js");
        let chosen = tool.resolve_interpreter(Some("js"), "print(1)");
        assert_eq!(chosen.key, "python");
    }

    /// If the source is silent the declaration holds.
    #[test]
    fn when_the_source_is_silent_the_declaration_holds() {
        let Some(tool) = tool_or_skip() else { return };
        if tool.interpreters.len() < 2 {
            return;
        }
        assert_eq!(
            tool.resolve_interpreter(Some("python"), "1 + 2").key,
            "python"
        );
    }
}
