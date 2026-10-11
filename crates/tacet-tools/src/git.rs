//! `git` — a READ-ONLY window onto the repository in the working directory.
//!
//! WHY IT EXISTS: the most frequent job of a terminal assistant is "summarize my
//! changes" / "write me a commit message". Both need the same three answers —
//! what changed, how it changed, what happened before. No network is involved,
//! so the tool sits cleanly inside the four gates.
//!
//! ONE TOOL, THREE ACTIONS — NOT THREE TOOLS. The router budget is 8
//! (`router::MAX_TOOLS`) and the catalog is already full; three separate entries
//! would push three other tools off the list the model ever sees. The action is a
//! `Choice` field, so the grammar forces the model into exactly one of the three.
//!
//! READ-ONLY IS STRUCTURAL, NOT A TEXT FILTER. There is no place in this file
//! where a string coming from the model becomes a git argument. `Action` is a
//! closed enum, each arm builds a FIXED argument vector, and `run_git` is the
//! only door to the binary. `commit`, `push`, `checkout` and `reset` are not
//! "blocked" — they are unwritable: to add one you would have to add an enum
//! variant, a schema choice and an argument builder. A blacklist of forbidden
//! words would have been the wrong shape; the next word is always the one nobody
//! listed.
//!
//! NO SHELL. `std::process::Command` is given an ARRAY of arguments. There is no
//! `sh -c`, no string concatenation and therefore no quoting rule to get wrong: a
//! branch or path containing a space, a quote or a `;` stays a single argument.
//!
//! NOR DOES THE REPOSITORY GET TO RUN ANYTHING. A string from the model is one
//! source of commands; the repository's own `.git/config` is another — it can
//! name an fsmonitor hook, an external diff, a textconv, a filter driver, a
//! gpg program, a promisor remote with an `ext::` URL. A downloaded folder with
//! a `.git/` inside is not the user's own configuration, so every one of those
//! is switched off on the command line (`BASE_ARGS`, `BASE_ENV`,
//! `neutralizing_args`), and each has a test that builds the hostile repository
//! and checks that nothing ran (audit H6).
//!
//! IT DOES NOT LEAVE THE GIVEN DIRECTORY. The working directory is whatever
//! `ToolContext` carries, and every subcommand is narrowed with the `-- .`
//! pathspec — so when the shell was started in a SUBDIRECTORY of a repository,
//! the answer describes that subdirectory and not the whole repository.
//!
//! BULK OUTPUT DOES NOT PASS THROUGH THE MODEL. A single `git diff` fills the
//! 4096-token window on its own. The rule of this repository applies: the body
//! goes into the `DataStore`, the model gets a SHORT summary (file count, +/-
//! lines, the first few names) plus a `source_ref` — the same pattern as
//! `read_document`.
//!
//! "NOT A REPOSITORY" IS AN ANSWER, NOT A FAILURE. A tool error reaches the model
//! as one fixed sentence (`ERROR_MODEL_TEXT`), from which it cannot tell "there
//! is no git here" from "git crashed". Both of those, "git is not installed"
//! and "this git is too old to run with the H6 overrides" (`GitFailure`) come
//! back as ordinary read results with their own wording, so the model can tell
//! the user what is actually going on.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tacet_kernel::context::CancelToken;
use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult,
    TraceUpdate, boxed,
};

/// How many lines of the body are shown to the model directly.
///
/// The same reasoning as `read_document`'s `PREVIEW_WITH_REF`: the model needs to
/// see the SHAPE (which files, which commits), not the content — the content is
/// behind the `source_ref`.
const PREVIEW_LINES: usize = 10;

/// How many commits `log` fetches. The window shows `PREVIEW_LINES` of them; the
/// rest sit in the store for a follow-up step.
const LOG_COUNT: usize = 40;

/// The cap on the body kept from a git run (256 KiB).
///
/// It is not a memory guarantee — `Command::output()` has already buffered the
/// child's whole output by the time we get here — but a `DataStore` record and a
/// chip detail have to stay bounded, and a diff that big is a symptom rather than
/// a document.
const OUTPUT_CAP: usize = 256 * 1024;

/// The git-level options prepended to EVERY invocation.
///
/// They live in `git_command` so that no call site can forget them:
///   * `--no-pager` (and `core.pager=cat` behind it) — git must never hand its
///     output to a pager. There is no terminal on the other end of a chat turn.
///   * `--no-optional-locks` — without it `git status` REFRESHES THE INDEX, i.e.
///     takes `.git/index.lock` and writes. This tool claims to be read-only; a
///     claim that costs nothing to keep is kept.
///
/// THE REST EXISTS BECAUSE "READ-ONLY" HAS TO HOLD AGAINST THE REPOSITORY TOO,
/// not only against the model (audit H6). A repository's own `.git/config` can
/// name programs that git runs on a plain `status`/`diff`; `git clone` does not
/// carry that file, but a downloaded folder or tarball with a `.git/` inside
/// does, and opening tacet there would run them — outside `run_code`'s shield,
/// with the network, without an approval. The audit MEASURED it:
/// `core.fsmonitor="touch MARKER; false"` made `git status` create MARKER, and
/// `diff.external` did the same for `git diff HEAD -- .`. Each override below is
/// guarded by a test in this file — one that builds the hostile repository
/// where such a repository can be built, `every_invocation_carries_the_overrides`
/// where it cannot:
///   * `core.fsmonitor=false` — the fsmonitor hook runs on EVERY index read.
///   * `core.hooksPath=<null device>` — no read-only subcommand should fire a
///     hook today (`post-index-change` needs an index write, which
///     `--no-optional-locks` already stops); pointing the hook directory at
///     nothing means a future git that adds one still finds none.
///   * `safe.bareRepository=explicit` — a folder that IS a bare repository (or
///     has one above it) is never discovered implicitly. This key is honoured
///     only from protected config, and the command line is protected config.
///   * `log.showSignature=false` — a signed commit would otherwise run
///     `gpg.program`, which the repository's config also chooses.
///
/// Command-line `-c` beats every config file, so the repository cannot undo
/// these; and git exports them to its own children (submodule runs, lazy
/// fetch) through `GIT_CONFIG_PARAMETERS`.
const BASE_ARGS: [&str; 12] = [
    "--no-pager",
    "--no-optional-locks",
    "-c",
    "core.fsmonitor=false",
    "-c",
    HOOKS_OFF,
    "-c",
    "core.pager=cat",
    "-c",
    "safe.bareRepository=explicit",
    "-c",
    "log.showSignature=false",
];

/// `core.hooksPath` pointed at the platform's null device: a directory nothing
/// can live in, so no `<hooksPath>/<hook>` ever resolves.
#[cfg(windows)]
const HOOKS_OFF: &str = "core.hooksPath=NUL";
#[cfg(not(windows))]
const HOOKS_OFF: &str = "core.hooksPath=/dev/null";

/// Environment set on EVERY invocation — the parts of H6 that are not config
/// keys.
///   * `GIT_CONFIG_NOSYSTEM` — the system file is not ours to trust either, and
///     the audit asked for it; the user's own `~/.gitconfig` is still read
///     (that is where a git-lfs filter lives, see `neutralizing_args`).
///   * `GIT_NO_LAZY_FETCH` — in a partial clone a MISSING object makes git
///     fetch it from the "promisor" remote, and both the remote's URL and
///     `protocol.ext.allow=always` come from the repository's config: an
///     `ext::sh -c ...` URL is a shell command. Measured by
///     `a_promisor_remote_cannot_turn_a_missing_object_into_a_command`.
///   * `GIT_ALLOW_PROTOCOL=` (empty) — the second belt for the same hole and for
///     anything else that would open a transport: an allow-list that names no
///     protocol, and per git's documentation it OVERRIDES `protocol.*.allow`
///     in config. This tool never talks to a remote, so nothing is lost.
///     Measured on git 2.50.1: with BOTH variables removed the promisor test
///     saw its `ext::` marker; with either one alone it stayed clean — two
///     independent belts, not one belt written twice.
///   * `GIT_TERMINAL_PROMPT=0` — stdin is already closed; this makes git fail
///     instead of trying to open `/dev/tty` for a credential.
const BASE_ENV: [(&str, &str); 4] = [
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_NO_LAZY_FETCH", "1"),
    ("GIT_ALLOW_PROTOCOL", ""),
    ("GIT_TERMINAL_PROMPT", "0"),
];

/// Options for every subcommand that can PRODUCE A DIFF (`diff`, and `log`,
/// which accepts the diff options). The repository's config and
/// `.gitattributes` together choose `diff.external`, `diff.<driver>.command`
/// and `diff.<driver>.textconv` — all three are programs. `status` takes
/// neither flag and needs neither: porcelain v1 prints no patch.
const NO_DIFF_PROGRAMS: [&str; 2] = ["--no-ext-diff", "--no-textconv"];

/// Submodules are looked at by COMMIT only, never by their work tree.
///
/// WHY THIS IS NOT A FREE CHOICE: to say "the submodule has uncommitted
/// changes", git starts a SECOND git inside the submodule, which reads the
/// submodule's own config — a config `neutralizing_args` never saw, so its
/// filter drivers would run. `dirty` keeps "the submodule points at a
/// different commit" (read in-process from the submodule's refs) and gives up
/// only "something inside it is modified". Measured by
/// `a_submodules_own_config_cannot_run_a_filter`.
///
/// `dirty` ALONE IS NOT ENOUGH for a submodule whose commit changed: see
/// `SUBMODULE_AS_HASHES`, which `diff` carries next to this.
const SUBMODULES_BY_COMMIT: &str = "--ignore-submodules=dirty";

/// A submodule whose COMMIT moved is shown as two hashes, never as a patch.
///
/// `dirty` above covers only a submodule whose WORK TREE changed. When its
/// commit changed, `diff.submodule=diff` in the repository's config makes
/// `git diff` start a child `git diff` inside the submodule; that child reads
/// the submodule's own config and does not inherit `--no-ext-diff` or
/// `--no-textconv`, so the submodule's `diff.external` and textconv ran.
/// The flag beats the config key. Measured by
/// `a_repository_cannot_make_diff_run_inside_a_submodule`: without it the
/// test saw `["sub-ext"]`.
const SUBMODULE_AS_HASHES: &str = "--submodule=short";

/// Environment variables the neutralizing `--config-env` entries read from.
///
/// `--config-env` and not `-c`: `-c` splits `name=value` at the FIRST `=`, and
/// a filter driver's name is a config subsection, which may itself contain
/// `=`. `--config-env` splits at the LAST `=`, and our variable names have
/// none. Measured: with the same overrides written as `-c`, a driver named
/// `a=b.c` stayed live and `a_repository_clean_filter_cannot_run` saw its
/// marker.
const ENV_EMPTY: &str = "TACET_GIT_EMPTY";
const ENV_FALSE: &str = "TACET_GIT_FALSE";

/// The pathspec that pins every subcommand to the working directory.
///
/// git DISCOVERS its repository by walking upwards, so without this a shell
/// started in `repo/crates/x` would answer with the whole repository. `--` also
/// ends option parsing, so the `.` can never be read as a flag.
const HERE: [&str; 2] = ["--", "."];

// ---------------------------------------------------------------------------
// The action
// ---------------------------------------------------------------------------

/// The closed set of things this tool can do. THE ONLY producer of git argument
/// vectors — see the read-only note at the top of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Status,
    Diff,
    Log,
}

impl Action {
    /// The single source of the schema's choice list and of the parser below.
    /// Written out twice, the two would drift and the model would be offered a
    /// value the parser rejects.
    pub const ALL: [Action; 3] = [Action::Status, Action::Diff, Action::Log];

    pub fn name(&self) -> &'static str {
        match self {
            Action::Status => "status",
            Action::Diff => "diff",
            Action::Log => "log",
        }
    }

    fn parse(raw: &str) -> ToolResult<Action> {
        let raw = raw.trim().to_ascii_lowercase();
        Action::ALL
            .into_iter()
            .find(|a| a.name() == raw)
            .ok_or_else(|| ToolError::InvalidArgument(format!("unknown git action: {raw}")))
    }
}

// ---------------------------------------------------------------------------
// Running the binary
// ---------------------------------------------------------------------------

/// The outcome of one git run. `stderr` is NOT carried: `output()` pipes it, so a
/// git message can never land on the user's screen in the middle of a chat, and
/// forwarding it further would only put file paths into places that do not need
/// them. An unexpected failure is reported as one honest sentence instead.
struct GitRun {
    ok: bool,
    stdout: String,
}

/// Why a git run produced no `GitRun` at all.
///
/// TWO VARIANTS BECAUSE THEY ARE TWO DIFFERENT ANSWERS. Folding `Unsupported`
/// into `ok: false` made `probe` call every real repository "not a
/// repository" on a git without `--show-scope` (git < 2.26, e.g. Ubuntu
/// 20.04's 2.25.1) — measured by
/// `a_git_too_old_for_the_overrides_is_not_reported_as_no_repository`.
#[derive(Debug)]
enum GitFailure {
    /// The binary could not be started at all.
    Spawn,
    /// git ran but rejected the command line that keeps the repository from
    /// running programs, or could not read the config that line is built
    /// from. Running on without it is exactly what H6 forbids, so this is a
    /// refusal — reported as one, not as the absence of a repository.
    Unsupported,
    /// The call was stopped while git ran, and git was killed and reaped.
    Stopped,
}

impl GitFailure {
    /// What `status`, `diff` and `log` report when a run produced nothing.
    fn into_tool_error(self) -> ToolError {
        match self {
            GitFailure::Stopped => {
                ToolError::Other("git was stopped before it finished; nothing was read.".into())
            }
            GitFailure::Spawn | GitFailure::Unsupported => {
                ToolError::Other("The git command could not be run.".into())
            }
        }
    }
}

/// git's exit status for a usage error, including an option it does not know.
/// Every git option this file passes is fixed, so on a current git this code
/// never appears; on an older one it means "too old for our overrides".
const GIT_USAGE_EXIT: i32 = 129;

/// THE ONLY PLACE A git `Command` IS BUILT: the base options, the base
/// environment, the neutralizing overrides, then `args`.
///
/// Split out of `run_git` so a test can read the finished argument and
/// environment lists without running anything — the guard for the overrides
/// no hostile fixture can trip today (`core.hooksPath`, `GIT_CONFIG_NOSYSTEM`).
fn git_command(program: &str, dir: &Path, neutralize: &[String], args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(BASE_ARGS)
        .args(neutralize)
        .args(args)
        .current_dir(dir)
        .envs(BASE_ENV)
        .env(ENV_EMPTY, "")
        .env(ENV_FALSE, "false")
        // stdin is CLOSED: git must never be able to sit waiting on a terminal
        // that a chat turn does not have.
        .stdin(Stdio::null());
    cmd
}

/// The config keys that make a filter driver run a program, or make its
/// failure fatal. Matched against git's canonical key (`filter.<name>.<key>`).
const FILTER_KEYS: &str = r"^filter\..*\.(clean|smudge|process|required)$";

/// The scopes whose config is the USER'S OWN and therefore trusted: a git-lfs
/// filter set by `git lfs install` lives in `global`. Everything else —
/// `local`, `worktree`, and any scope a future git invents — came with the
/// repository, so it is neutralized. Fail closed: an unknown name is untrusted.
const TRUSTED_SCOPES: [&str; 3] = ["global", "system", "command"];

/// Turns `git config -z --show-scope --name-only --get-regexp FILTER_KEYS`
/// output into the overrides that switch those drivers off.
///
/// WHY A FILTER CANNOT BE SWITCHED OFF BY A FIXED FLAG LIKE THE OTHERS: there is
/// no `--no-filters` for `status`/`diff`, and the driver's NAME is chosen by the
/// repository (`.gitattributes`: `*.txt filter=anything`). And it is not
/// hypothetical: an archive-extracted work tree has stat data that matches no
/// index entry, so `git status` re-hashes every tracked file and runs its
/// `clean` filter on the way. The only handle is to read the names and blank
/// them on the command line, which beats the config file. An empty `clean`/
/// `process` is "no filter" to git; `required=false` keeps a now-empty driver
/// from turning into a fatal error that would hide the whole answer.
///
/// Output shape with `-z`: `scope NUL key NUL`, repeated.
fn neutralizing_args(raw: &str) -> Vec<String> {
    let mut drivers: Vec<&str> = Vec::new();
    let mut tokens = raw.split('\0');
    while let (Some(scope), Some(key)) = (tokens.next(), tokens.next()) {
        if TRUSTED_SCOPES.contains(&scope) {
            continue;
        }
        let Some(rest) = key.strip_prefix("filter.") else {
            continue;
        };
        let Some((name, _)) = rest.rsplit_once('.') else {
            continue;
        };
        if !drivers.contains(&name) {
            drivers.push(name);
        }
    }
    let mut args = Vec::new();
    for name in drivers {
        for key in ["clean", "smudge", "process"] {
            args.push(format!("--config-env=filter.{name}.{key}={ENV_EMPTY}"));
        }
        args.push(format!("--config-env=filter.{name}.required={ENV_FALSE}"));
    }
    args
}

/// Runs `git` in `dir` with exactly `args` (plus everything `git_command` adds).
///
/// Every run is preceded by a `git config` read that finds the repository's
/// filter drivers (`neutralizing_args`). Reading config runs nothing; it costs
/// one extra process per call, a few milliseconds against a turn that
/// generates tokens.
///
/// NO TIMEOUT OF ITS OWN, AND THAT IS A DECISION. The three subcommands are
/// local-only — none of them opens a socket — so the runtime is bounded by the
/// size of the user's own repository, exactly like the blocking
/// `std::fs::read` in `read_document`.
///
/// BUT IT STOPS WHEN ASKED (audit M30, review of 10 Oct 2026). The executor
/// gives every call a deadline (`DEFAULT_DEADLINE`, 120 s) and passes Ctrl-C
/// through `stop`. With a plain `output()` neither reached git: on a
/// repository where `status --untracked-files=all` outlived the deadline, the
/// executor abandoned the worker after its grace and reported the call as
/// stopped, while the git child ran on, orphaned, for the rest of the session.
/// `output_unless_stopped` kills and reaps it instead.
fn run_git(
    program: &str,
    dir: &Path,
    args: &[&str],
    stop: &CancelToken,
) -> Result<GitRun, GitFailure> {
    let scan = git_command(
        program,
        dir,
        &[],
        &[
            "config",
            "-z",
            "--show-scope",
            "--name-only",
            "--get-regexp",
            FILTER_KEYS,
        ],
    );
    let scan = output_unless_stopped(scan, stop)?;
    // Exit 1 is git's "no key matched" — the common case. Anything else means
    // the config could not be read (129: `--show-scope` unknown, git < 2.26;
    // 128: a config file git cannot parse), and running on WITHOUT the
    // overrides would be running exactly the repository this guards against:
    // fail closed, and say so.
    let neutralize = match scan.status.code() {
        Some(0) => neutralizing_args(&String::from_utf8_lossy(&scan.stdout)),
        Some(1) => Vec::new(),
        _ => return Err(GitFailure::Unsupported),
    };
    let out = output_unless_stopped(git_command(program, dir, &neutralize, args), stop)?;
    // git 2.26–2.30 has `--show-scope` but not `--config-env`, so it rejects
    // the overrides of a repository that defines a filter. That is a usage
    // error (129), never a "not a repository" (128) — keep them apart.
    if out.status.code() == Some(GIT_USAGE_EXIT) {
        return Err(GitFailure::Unsupported);
    }
    Ok(GitRun {
        ok: out.status.success(),
        // Paths on disk are not guaranteed to be UTF-8; a lossy read is better
        // than refusing to answer because one file name is odd.
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
    })
}

/// `cmd.output()`, except that a fired `stop` kills the child, reaps it and
/// answers `Stopped` within one poll.
///
/// THE PIPES ARE DRAINED ON THEIR OWN THREADS while the child is polled:
/// `git diff` on a large change writes more than a pipe holds, and a child
/// blocked on a full pipe never exits for `try_wait` to see. On a stop the
/// readers are not joined — a grandchild could hold the pipe open — they end
/// when the last writer does.
fn output_unless_stopped(mut cmd: Command, stop: &CancelToken) -> Result<Output, GitFailure> {
    use std::io::Read;
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| GitFailure::Spawn)?;
    fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if stop.is_cancelled() => {
                // KILL AND THEN REAP, as `calendar::run_osascript` learned:
                // without the `wait` the child stays a zombie.
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitFailure::Stopped);
            }
            Ok(None) => std::thread::sleep(STOP_POLL),
            Err(_) => return Err(GitFailure::Spawn),
        }
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// How often a running git is checked for a stop. The process tools poll at
/// 10 ms; the executor looks up every 20 ms and waits `ABANDON_GRACE` (3 s),
/// so this is well inside both.
const STOP_POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// What the working directory turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Repo {
    Present,
    /// There is a git, but we are not inside a work tree.
    Absent,
    /// The binary could not be run at all.
    Unavailable,
    /// git runs, but not with the overrides H6 needs (see `GitFailure`).
    Unsupported,
    /// The call was stopped (Ctrl-C or the executor's deadline) while git
    /// was still answering.
    Stopped,
}

/// Asks git itself instead of reading its error text.
///
/// WHY NOT MATCH ON "not a git repository": that message is LOCALIZED — on a
/// machine with a non-English git the substring never appears and every directory
/// would look like a repository. `rev-parse --is-inside-work-tree` answers
/// structurally: exit status plus `true`/`false`.
fn probe(program: &str, dir: &Path, stop: &CancelToken) -> Repo {
    match run_git(program, dir, &["rev-parse", "--is-inside-work-tree"], stop) {
        Err(GitFailure::Spawn) => Repo::Unavailable,
        Err(GitFailure::Stopped) => Repo::Stopped,
        Err(GitFailure::Unsupported) => Repo::Unsupported,
        Ok(r) if r.ok && r.stdout.trim() == "true" => Repo::Present,
        // `false` = we are inside the `.git` directory itself; a non-zero exit =
        // no repository. Neither is something this tool can report on.
        Ok(_) => Repo::Absent,
    }
}

/// Does the repository have a commit yet? A fresh `git init` has no `HEAD`, and
/// `git diff HEAD` / `git log` FAIL there rather than returning empty — the state
/// has to be known before the command is chosen, not guessed from the error.
fn has_commit(program: &str, dir: &Path, stop: &CancelToken) -> bool {
    run_git(
        program,
        dir,
        &["rev-parse", "--verify", "--quiet", "HEAD"],
        stop,
    )
    .map(|r| r.ok)
    .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

/// The result of one action, before it is turned into a `ToolOutcome`.
struct Report {
    /// The line the user sees on the chip.
    chip: String,
    /// The SHORT text the model sees. The `source_ref` suffix is added by the
    /// caller — a single crossing point, so the wire format cannot be written two
    /// different ways (see `source_ref_suffix`).
    summary: String,
    /// The full output. `None` when everything already fits in `summary`:
    /// producing a reference to data the model has in hand adds an indirection
    /// and no information.
    body: Option<String>,
}

impl Report {
    /// Nothing to report — a clean tree, no commits, no changes.
    fn empty(chip: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            chip: chip.into(),
            summary: summary.into(),
            body: None,
        }
    }
}

/// Cuts a body at a LINE boundary. A half-written diff hunk is worse than a short
/// one: the model tries to reconstruct it and invents the rest.
fn cap_body(body: &str) -> String {
    if body.len() <= OUTPUT_CAP {
        return body.to_string();
    }
    let mut end = OUTPUT_CAP;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    let cut = body[..end].rfind('\n').map(|i| i + 1).unwrap_or(end);
    format!(
        "{}(+{} more bytes not shown)\n",
        &body[..cut],
        body.len() - cut
    )
}

/// The first `PREVIEW_LINES` lines plus an explicit note about what was left out.
/// Silence here reads to the model as "that is all there was".
fn preview(lines: &[String], noun: &str) -> String {
    let mut s = String::new();
    for line in lines.iter().take(PREVIEW_LINES) {
        s.push_str(line);
        s.push('\n');
    }
    let hidden = lines.len().saturating_sub(PREVIEW_LINES);
    if hidden > 0 {
        s.push_str(&format!("(+{hidden} more {noun} not shown)\n"));
    }
    s
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

/// Pulls a readable branch name out of the porcelain `## ` line.
///
/// The line comes in three shapes: `main...origin/main [ahead 1]`, plain `main`,
/// and `No commits yet on main`. The tracking part and the ahead/behind counter
/// are cut; the rest is passed through, because on a fresh repository the
/// sentence itself is the useful answer.
fn branch_name(raw: &str) -> String {
    let head = raw.split('[').next().unwrap_or(raw);
    let head = head.split("...").next().unwrap_or(head);
    head.trim().to_string()
}

fn status(program: &str, dir: &Path, stop: &CancelToken) -> ToolResult<Report> {
    let run = run_git(
        program,
        dir,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            // Without this git reports an untracked FOLDER as a single entry and
            // "3 new files" is shown to the user as one line.
            "--untracked-files=all",
            SUBMODULES_BY_COMMIT,
            HERE[0],
            HERE[1],
        ],
        stop,
    )
    .map_err(GitFailure::into_tool_error)?;
    if !run.ok {
        return Err(ToolError::Other("The git status could not be read.".into()));
    }

    let mut branch = String::new();
    let mut entries: Vec<String> = Vec::new();
    for line in run.stdout.lines() {
        match line.strip_prefix("## ") {
            Some(rest) => branch = branch_name(rest),
            None if !line.trim().is_empty() => entries.push(line.to_string()),
            None => {}
        }
    }
    let branch = if branch.is_empty() {
        "unknown".to_string()
    } else {
        branch
    };

    if entries.is_empty() {
        return Ok(Report::empty(
            "git status · clean",
            format!("git_status: on branch {branch}, the working tree is clean (no changes)"),
        ));
    }

    // The two columns of porcelain v1: X = the index, Y = the work tree. A file
    // can be in BOTH (staged, then edited again), so the counters deliberately
    // overlap and the wording says so rather than inventing a total.
    let (mut staged, mut unstaged, mut untracked) = (0usize, 0usize, 0usize);
    for entry in &entries {
        let b = entry.as_bytes();
        let x = *b.first().unwrap_or(&b' ');
        let y = *b.get(1).unwrap_or(&b' ');
        if x == b'?' {
            untracked += 1;
            continue;
        }
        if x != b' ' {
            staged += 1;
        }
        if y != b' ' {
            unstaged += 1;
        }
    }

    let headline = format!(
        "git_status: on branch {branch}, {} files touched ({staged} staged, {unstaged} unstaged, \
         {untracked} untracked)",
        entries.len()
    );
    let summary = format!("{headline}\n{}", preview(&entries, "files"));
    Ok(Report {
        chip: format!("git status · {} files", entries.len()),
        summary,
        // The full list only earns a reference when the preview really hid
        // something.
        body: (entries.len() > PREVIEW_LINES).then(|| cap_body(&run.stdout)),
    })
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

/// One line of `--numstat`: added, removed, path. `None` counts mean a binary
/// file — git writes `-` there, and calling that zero would understate the change.
struct FileStat {
    added: Option<u64>,
    removed: Option<u64>,
    path: String,
}

fn parse_numstat(raw: &str) -> Vec<FileStat> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let added = parts.next()?;
            let removed = parts.next()?;
            let path = parts.next()?;
            Some(FileStat {
                added: added.parse().ok(),
                removed: removed.parse().ok(),
                path: path.to_string(),
            })
        })
        .collect()
}

fn diff(program: &str, dir: &Path, stop: &CancelToken) -> ToolResult<Report> {
    // WHICH DIFF: against `HEAD`, i.e. staged AND unstaged changes together —
    // "what did I change since the last commit" is the question actually being
    // asked. On a repository with no commit yet there is no `HEAD` to compare
    // with, so the staged set is the whole of the change.
    let base: &str = if has_commit(program, dir, stop) {
        "HEAD"
    } else {
        "--cached"
    };

    let stat_run = run_git(
        program,
        dir,
        &[
            "diff",
            NO_DIFF_PROGRAMS[0],
            NO_DIFF_PROGRAMS[1],
            SUBMODULES_BY_COMMIT,
            SUBMODULE_AS_HASHES,
            base,
            "--numstat",
            HERE[0],
            HERE[1],
        ],
        stop,
    )
    .map_err(GitFailure::into_tool_error)?;
    if !stat_run.ok {
        return Err(ToolError::Other("The git diff could not be read.".into()));
    }
    let stats = parse_numstat(&stat_run.stdout);
    if stats.is_empty() {
        return Ok(Report::empty(
            "git diff · no changes",
            "git_diff: there are no changes against the last commit",
        ));
    }

    let added: u64 = stats.iter().filter_map(|s| s.added).sum();
    let removed: u64 = stats.iter().filter_map(|s| s.removed).sum();
    let lines: Vec<String> = stats
        .iter()
        .map(|s| match (s.added, s.removed) {
            (Some(a), Some(r)) => format!("+{a} -{r}\t{}", s.path),
            // Binary: the counts are meaningless, saying so is not.
            _ => format!("binary\t{}", s.path),
        })
        .collect();

    let headline = format!(
        "git_diff: {} files changed, +{added} -{removed} lines",
        stats.len()
    );
    let summary = format!("{headline}\n{}", preview(&lines, "files"));

    // THE PATCH ITSELF NEVER GOES TO THE MODEL. This is the case the bypass
    // channel exists for: the summary above is a few lines, the hunks below are
    // easily tens of thousands of tokens, and a follow-up step reaches them
    // through the source_ref.
    let patch = run_git(
        program,
        dir,
        &[
            "diff",
            NO_DIFF_PROGRAMS[0],
            NO_DIFF_PROGRAMS[1],
            SUBMODULES_BY_COMMIT,
            SUBMODULE_AS_HASHES,
            base,
            HERE[0],
            HERE[1],
        ],
        stop,
    )
    .map_err(GitFailure::into_tool_error)?;
    Ok(Report {
        chip: format!("git diff · {} files", stats.len()),
        summary,
        body: Some(cap_body(&patch.stdout)),
    })
}

// ---------------------------------------------------------------------------
// log
// ---------------------------------------------------------------------------

fn log(program: &str, dir: &Path, stop: &CancelToken) -> ToolResult<Report> {
    if !has_commit(program, dir, stop) {
        return Ok(Report::empty(
            "git log · no commits",
            "git_log: the repository has no commits yet",
        ));
    }
    let count = format!("--max-count={LOG_COUNT}");
    let run = run_git(
        program,
        dir,
        &[
            "log",
            // The flag as well as `log.showSignature=false` in BASE_ARGS: a
            // signed commit makes git run `gpg.program`, which the repository's
            // config chooses. Two belts, each enough on its own.
            "--no-show-signature",
            NO_DIFF_PROGRAMS[0],
            NO_DIFF_PROGRAMS[1],
            &count,
            "--date=short",
            // A fixed, one-line-per-commit shape: short hash, date, author,
            // subject. `%s` is the subject alone, so a commit body can never turn
            // one commit into forty lines.
            "--pretty=format:%h %ad %an: %s",
            HERE[0],
            HERE[1],
        ],
        stop,
    )
    .map_err(GitFailure::into_tool_error)?;
    if !run.ok {
        return Err(ToolError::Other("The git log could not be read.".into()));
    }

    let lines: Vec<String> = run
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    if lines.is_empty() {
        // There ARE commits (has_commit said so) but none of them touch this
        // directory — a different fact from "no history", and the model has to be
        // able to tell them apart.
        return Ok(Report::empty(
            "git log · no commits here",
            "git_log: no commits touch this directory",
        ));
    }

    let headline = format!("git_log: the last {} commits touching here", lines.len());
    Ok(Report {
        chip: format!("git log · {} commits", lines.len()),
        summary: format!("{headline}\n{}", preview(&lines, "commits")),
        body: (lines.len() > PREVIEW_LINES).then(|| cap_body(&run.stdout)),
    })
}

// ---------------------------------------------------------------------------
// The tool
// ---------------------------------------------------------------------------

pub struct GitTool {
    /// The binary name. ALWAYS `"git"` in production — `new()` is the only public
    /// constructor and it is the only value it can take.
    ///
    /// The field exists so the "git is not installed" branch is MEASURABLE: the
    /// test points it at a name that is not on `PATH`. `&'static str` on purpose —
    /// nothing that arrives at runtime, from the model or from a file, can reach
    /// this field.
    program: &'static str,
}

impl Default for GitTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GitTool {
    pub fn new() -> Self {
        Self { program: "git" }
    }

    #[cfg(test)]
    fn with_program(program: &'static str) -> Self {
        Self { program }
    }
}

impl Tool for GitTool {
    fn name(&self) -> &str {
        "git"
    }

    /// THE WORDING IS ALSO A ROUTER SIGNAL, and that was MEASURED, not guessed.
    ///
    /// `router::Router` scores a tool by looking for the profile's hints inside
    /// its NAME AND DESCRIPTION; there is no `git`/`commit` trigger in the
    /// message-side list, so this tool only ever scores under the General profile
    /// ("summarize", "list", "find"...). With the first draft — "'status' = which
    /// files changed ... wants a commit message written" — the eval case
    /// `git-commit-message` FELL OFF the budget (8 then, `MAX_TOOLS` now) and the
    /// model never saw the
    /// tool. The verbs were changed to the ones the General profile actually
    /// looks for ("lists", "write"); they are the natural words for what the tool
    /// does, so nothing was invented to game the score. Anyone rewording this
    /// must re-run `the_expected_tool_does_not_drop_out_of_the_routers_budget`.
    fn description(&self) -> &str {
        "Reads the state of the git repository in the working directory: \
         'status' reports THE CURRENT BRANCH and which files changed, 'diff' \
         reads the changes, 'log' lists recent commits. Call this when the user \
         asks which branch they are on, what they changed, or wants a commit \
         message written. READ-ONLY: it cannot commit, push or undo anything."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new(
                "action",
                // A Choice, not text: the grammar turns this into a literal
                // alternation, so the model CANNOT produce a fourth value. That is
                // what makes "read-only" a property of the shape rather than of a
                // filter someone has to remember to keep updated.
                ArgSchema::choice(Action::ALL.iter().map(|a| a.name())).description(
                    "status = changed files, diff = the changes themselves, \
                     log = recent commits",
                ),
            )
            .required(),
        ])
        .description("Read the git repository state (read-only)")
    }

    /// The file names, the commit messages and the code in a diff are the user's
    /// own data. Once they are in the window, a later web/mcp call would carry
    /// them off the device — so the session is tainted and the next external tool
    /// meets the approval gate.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let trace = ctx.start_chip("git", "Reading git…");

            let (outcome, tainted) = match self.inspect(&args, ctx) {
                Ok(pair) => pair,
                Err(e) => (ToolOutcome::failed(&e), false),
            };

            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(args.to_string())
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            // Only a run that REALLY read repository content taints. "There is no
            // git here" and "git is not installed" touched no data of the user's;
            // tainting for them would push the user into approval prompts that buy
            // nothing. (`ToolExecutor` applies a second, coarser rule of its own —
            // this one is the tool's own honest account.)
            if tainted {
                ctx.taint();
            }
            outcome
        })
    }
}

impl GitTool {
    /// The synchronous body. Returns `(outcome, did_we_read_repository_content)`.
    fn inspect(&self, args: &Value, ctx: &ToolContext) -> ToolResult<(ToolOutcome, bool)> {
        let raw = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::MissingField("action".into()))?;
        let action = Action::parse(raw)?;

        // THE SANDBOX GATE, EVEN THOUGH THE MODEL GIVES NO PATH. The working
        // directory is resolved through the same door as every other tool: it
        // proves the directory exists and canonicalizes it, so git is never handed
        // a `current_dir` that a symlink has moved somewhere else.
        let dir = crate::sandbox_path::resolve_existing_dir(ctx, ".")?;

        let stop = ctx.cancel_token();
        match probe(self.program, &dir, stop) {
            Repo::Unavailable => Ok((
                ToolOutcome::read_ok(
                    "git is not installed",
                    "git_unavailable: the git command line tool could not be run on this device",
                ),
                false,
            )),
            // Not tainted: git refused before reading anything of the user's.
            Repo::Unsupported => Ok((
                ToolOutcome::read_ok(
                    "git too old for safe reading",
                    "git_unsupported: this git is too old or the repository's config could \
                     not be read (needs git >= 2.31), so it was not run",
                ),
                false,
            )),
            // Before anything of the user's was read: not tainted.
            Repo::Stopped => Err(GitFailure::Stopped.into_tool_error()),
            Repo::Absent => Ok((
                ToolOutcome::read_ok(
                    "no git repository here",
                    "no_git_repository: the working directory is not inside a git repository",
                ),
                false,
            )),
            Repo::Present => {
                let report = match action {
                    Action::Status => status(self.program, &dir, stop)?,
                    Action::Diff => diff(self.program, &dir, stop)?,
                    Action::Log => log(self.program, &dir, stop)?,
                };
                Ok((self.outcome(ctx, report), true))
            }
        }
    }

    /// Turns a `Report` into an outcome, putting the body into the store when
    /// there is one.
    fn outcome(&self, ctx: &ToolContext, report: Report) -> ToolOutcome {
        let Report {
            chip,
            summary,
            body,
        } = report;
        match body {
            Some(body) => {
                let source = ctx.store("git", &summary, body.clone());
                // The chip detail carries the FULL body: what goes to the model is
                // truncated, what the user can open is not (the second layer of
                // transparency).
                ToolOutcome::summarize(chip, summary, source.as_str()).raw_output(body)
            }
            None => ToolOutcome::read_ok(chip, summary.clone()).raw_output(summary),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tacet_kernel::{InMemoryDataStore, SilentReporter, SourceRef, ToolState};

    /// The core has no tokio and this crate must not pick a runtime either — the
    /// same minimal executor as the other tool tests.
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
            "tacet-git-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    /// Runs git in the fixture. The identity is given on the COMMAND LINE (`-c`):
    /// the runner may have no `user.name`/`user.email` configured, and this must
    /// not touch the machine's own git configuration.
    fn git_fixture(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git must be installed on the runner");
        assert!(
            out.status.success(),
            "fixture command failed: {args:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_repo(dir: &Path) {
        // `init.defaultBranch` is given as a command-line override so that a git
        // too old to know the setting simply ignores it instead of failing.
        git_fixture(dir, &["-c", "init.defaultBranch=main", "init", "-q"]);
    }

    fn commit(dir: &Path, message: &str) {
        git_fixture(dir, &["add", "-A"]);
        git_fixture(
            dir,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-q",
                "-m",
                message,
            ],
        );
    }

    fn write(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("folder");
        }
        std::fs::write(path, content).expect("write");
    }

    fn context(root: &Path, store: Arc<InMemoryDataStore>) -> ToolContext {
        ToolContext::new(store, root, Arc::new(SilentReporter))
    }

    fn call(root: &Path, action: &str) -> (ToolOutcome, ToolContext, Arc<InMemoryDataStore>) {
        let store = Arc::new(InMemoryDataStore::new());
        let mut ctx = context(root, Arc::clone(&store));
        let outcome = block_on(GitTool::new().run(json!({ "action": action }), &mut ctx));
        (outcome, ctx, store)
    }

    /// A DIRECTORY THAT IS NOT A REPOSITORY IS AN ANSWER, NOT AN ERROR.
    ///
    /// The claim being measured: the model can say "there is no git here". Had
    /// this come back as a `ToolError` the model would have received
    /// `ERROR_MODEL_TEXT` and been unable to tell it from a crash.
    #[test]
    fn a_directory_without_a_repository_answers_instead_of_failing() {
        let root = temp_dir("norepo");
        for action in ["status", "diff", "log"] {
            let (outcome, ctx, _) = call(&root, action);
            assert!(
                matches!(outcome.state, ToolState::Read),
                "{action}: {:?}",
                outcome.state
            );
            assert!(
                outcome.to_model.starts_with("no_git_repository"),
                "{action}: {}",
                outcome.to_model
            );
            assert_ne!(outcome.to_model, tacet_kernel::ERROR_MODEL_TEXT);
            // Nothing of the user's was read, so nothing was tainted.
            assert!(!ctx.session_tainted(), "{action} tainted the session");
        }
        std::fs::remove_dir_all(&root).ok();
    }

    /// A CLEAN REPOSITORY says the tree is clean and STORES NOTHING — a
    /// source_ref pointing at an empty body would send the model chasing data that
    /// is not there.
    #[test]
    fn a_clean_repository_reports_clean() {
        let root = temp_dir("clean");
        init_repo(&root);
        write(&root, "a.txt", "one\n");
        commit(&root, "first");

        let (outcome, ctx, store) = call(&root, "status");
        assert!(matches!(outcome.state, ToolState::Read));
        assert!(
            outcome.to_model.contains("the working tree is clean"),
            "{}",
            outcome.to_model
        );
        assert!(!outcome.to_model.contains("source_ref"));
        assert!(tacet_kernel::DataStore::of_kind(store.as_ref(), "git").is_empty());
        // Reading the repository — even a clean one — reveals branch names, so it
        // taints.
        assert!(ctx.session_tainted());

        let (outcome, _, _) = call(&root, "diff");
        assert!(
            outcome.to_model.contains("no changes"),
            "{}",
            outcome.to_model
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// A FRESH `git init` HAS NO `HEAD`. `git diff HEAD` and `git log` FAIL
    /// there; the tool has to answer rather than pass the failure on.
    #[test]
    fn a_repository_without_commits_is_handled() {
        let root = temp_dir("nocommit");
        init_repo(&root);
        write(&root, "new.txt", "hello\n");
        git_fixture(&root, &["add", "new.txt"]);

        let (log_outcome, _, _) = call(&root, "log");
        assert!(matches!(log_outcome.state, ToolState::Read));
        assert!(
            log_outcome.to_model.contains("no commits yet"),
            "{}",
            log_outcome.to_model
        );

        // The staged file IS the whole change when there is no HEAD to compare
        // against.
        let (diff_outcome, _, _) = call(&root, "diff");
        assert!(matches!(diff_outcome.state, ToolState::Read));
        assert!(
            diff_outcome.to_model.contains("1 files changed, +1 -0"),
            "{}",
            diff_outcome.to_model
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// THE REAL JOB: a repository with changes. Status counts them, diff
    /// summarizes them, and THE PATCH DOES NOT PASS THROUGH THE MODEL.
    #[test]
    fn changes_are_summarized_and_the_patch_stays_in_the_store() {
        let root = temp_dir("changes");
        init_repo(&root);
        write(&root, "kept.txt", "one\ntwo\nthree\n");
        commit(&root, "first");
        // A tracked file edited, a new file staged, an untracked file left alone.
        write(&root, "kept.txt", "one\ntwo\nthree\nfour\n");
        write(&root, "staged.txt", "s\n");
        git_fixture(&root, &["add", "staged.txt"]);
        write(&root, "loose.txt", "l\n");

        let (status_outcome, _, _) = call(&root, "status");
        assert!(
            status_outcome.to_model.contains("3 files touched"),
            "{}",
            status_outcome.to_model
        );
        assert!(
            status_outcome
                .to_model
                .contains("(1 staged, 1 unstaged, 1 untracked)"),
            "{}",
            status_outcome.to_model
        );
        assert!(status_outcome.to_model.contains("kept.txt"));
        assert!(status_outcome.to_model.contains("loose.txt"));

        let (outcome, ctx, store) = call(&root, "diff");
        assert!(
            outcome.to_model.contains("2 files changed, +2 -0 lines"),
            "{}",
            outcome.to_model
        );
        // THE BYPASS CHANNEL: a reference goes to the model, the hunks do not.
        assert!(outcome.to_model.contains("source_ref=git#1"));
        assert!(
            !outcome.to_model.contains("@@"),
            "a diff hunk leaked into the model window: {}",
            outcome.to_model
        );
        // ...and the body really is reachable through the reference.
        let record = ctx.from_store(&SourceRef("git#1".into())).expect("record");
        assert!(record.body.contains("@@"), "{}", record.body);
        assert!(record.body.contains("+four"));
        assert_eq!(
            tacet_kernel::DataStore::of_kind(store.as_ref(), "git").len(),
            1
        );
        assert!(ctx.session_tainted());
        std::fs::remove_dir_all(&root).ok();
    }

    /// A LONG HISTORY IS TRUNCATED FOR THE MODEL AND KEPT WHOLE IN THE STORE.
    /// The number in the note is measured, not asserted by eye: a silent cut is
    /// read by the model as "that was all".
    #[test]
    fn a_long_history_is_truncated_with_an_explicit_note() {
        let root = temp_dir("history");
        init_repo(&root);
        for i in 0..14 {
            write(&root, "file.txt", &format!("line {i}\n"));
            commit(&root, &format!("commit number {i}"));
        }

        let (outcome, ctx, _) = call(&root, "log");
        assert!(
            outcome.to_model.contains("(+4 more commits not shown)"),
            "{}",
            outcome.to_model
        );
        assert!(outcome.to_model.contains("source_ref=git#1"));
        // The newest commit is in the window, the oldest is only in the store.
        assert!(outcome.to_model.contains("commit number 13"));
        assert!(!outcome.to_model.contains("commit number 0:"));
        let record = ctx.from_store(&SourceRef("git#1".into())).expect("record");
        assert_eq!(record.body.lines().count(), 14);
        assert!(record.body.contains("commit number 0"));
        std::fs::remove_dir_all(&root).ok();
    }

    /// IT DOES NOT LEAVE THE GIVEN DIRECTORY. The context points at a
    /// SUBDIRECTORY of the repository; the answer must describe that subdirectory,
    /// not the whole repository.
    #[test]
    fn the_answer_stays_inside_the_given_directory() {
        let root = temp_dir("scope");
        init_repo(&root);
        write(&root, "outside.txt", "o\n");
        write(&root, "sub/inside.txt", "i\n");
        commit(&root, "first");
        write(&root, "outside.txt", "o\nchanged\n");
        write(&root, "sub/inside.txt", "i\nchanged\n");

        let sub = root.join("sub");
        let (outcome, _, _) = call(&sub, "status");
        assert!(
            outcome.to_model.contains("1 files touched"),
            "{}",
            outcome.to_model
        );
        assert!(outcome.to_model.contains("sub/inside.txt"));
        assert!(
            !outcome.to_model.contains("outside.txt"),
            "a change from outside the given directory leaked in: {}",
            outcome.to_model
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// THE SCHEMA IS THE READ-ONLY GUARANTEE. `commit`/`push`/`checkout`/
    /// `reset` are not filtered out — they cannot be expressed. This test measures
    /// the shape (a closed choice set) rather than a word list, because a word list
    /// always misses the next word.
    #[test]
    fn the_schema_makes_writing_actions_impossible() {
        let schema = GitTool::new().schema();
        let field = &schema.fields()[0];
        assert_eq!(field.name, "action");
        assert!(field.required);
        let tacet_kernel::SchemaKind::Choice { choices } = &field.schema.kind else {
            panic!("the action field must be a Choice: {:?}", field.schema.kind);
        };
        assert_eq!(choices, &["status", "diff", "log"]);
        // The schema's own validator refuses anything outside the set — that is the
        // same gate the generation grammar is built from.
        for forbidden in ["commit", "push", "checkout", "reset", "status; rm -rf /"] {
            assert!(
                schema.validate(&json!({ "action": forbidden })).is_err(),
                "the schema accepted a writing action: {forbidden}"
            );
        }
        // And the parser agrees with the schema, so the two cannot drift.
        for forbidden in ["commit", "push", "checkout", "reset"] {
            assert!(Action::parse(forbidden).is_err());
        }
    }

    /// A MISSING `git` BINARY IS ALSO AN ANSWER. Measured through the seam
    /// described on the `program` field — production can only ever pass "git".
    #[test]
    fn a_missing_git_binary_is_reported_as_a_result() {
        let root = temp_dir("nogit");
        let store = Arc::new(InMemoryDataStore::new());
        let mut ctx = context(&root, store);
        let outcome = block_on(
            GitTool::with_program("tacet-git-does-not-exist")
                .run(json!({"action": "status"}), &mut ctx),
        );
        assert!(matches!(outcome.state, ToolState::Read));
        assert!(
            outcome.to_model.starts_with("git_unavailable"),
            "{}",
            outcome.to_model
        );
        assert!(!ctx.session_tainted());
        std::fs::remove_dir_all(&root).ok();
    }

    /// A missing or unknown `action` fails at the tool boundary, and the model
    /// gets the fixed error text — not a half answer it could build on.
    #[test]
    fn a_bad_action_fails_at_the_boundary() {
        let root = temp_dir("badaction");
        init_repo(&root);
        for args in [
            json!({}),
            json!({"action": ""}),
            json!({"action": "commit"}),
        ] {
            let store = Arc::new(InMemoryDataStore::new());
            let mut ctx = context(&root, store);
            let outcome = block_on(GitTool::new().run(args.clone(), &mut ctx));
            assert!(
                matches!(outcome.state, ToolState::Failed(_)),
                "{args} was accepted"
            );
            assert_eq!(outcome.to_model, tacet_kernel::ERROR_MODEL_TEXT);
            assert!(!ctx.session_tainted());
        }
        std::fs::remove_dir_all(&root).ok();
    }

    /// The body cap cuts at a LINE boundary and says how much it dropped.
    #[test]
    fn the_body_cap_cuts_at_a_line_boundary() {
        let body: String = (0..40_000).map(|i| format!("line {i}\n")).collect();
        let capped = cap_body(&body);
        assert!(capped.len() < body.len());
        assert!(capped.contains("more bytes not shown"));
        // Every kept line is whole: the last one before the note ends with the text
        // of a full line, not half of one.
        let kept: Vec<&str> = capped.lines().collect();
        let last = kept[kept.len() - 2];
        assert!(
            body.contains(&format!("{last}\n")),
            "half a line kept: {last}"
        );
        // A short body is passed through untouched.
        assert_eq!(cap_body("a\nb\n"), "a\nb\n");
    }

    /// THE TOOL REALLY REACHES THE MODEL. Being in the catalog is not enough:
    /// the router only shows 8 of the 12 tools, and a tool that never enters
    /// that 8 is a mechanism built and not connected — the recurring failure of
    /// this repository.
    ///
    /// The claim is measured on the PRODUCTION catalog with the web gate OPEN,
    /// i.e. the widest, hardest case: with 12 tools four of them drop, and `git`
    /// sits in the tail of the order. If a reworded description loses the hints
    /// the General profile looks for, this test fails right here rather than in
    /// a model run three layers up.
    #[test]
    fn the_router_shows_git_when_the_message_is_about_the_repository() {
        let store = Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (catalog, _, _) = crate::catalog::production_catalog_with(&store, &memory, None, true);
        let router = crate::router::Router::new();
        for message in [
            "Which files have I changed in this git repository?",
            "Summarize my git changes and write me a commit message.",
        ] {
            let selection: Vec<String> = router
                .select(message, &catalog)
                .iter()
                .map(|t| t.name().to_string())
                .collect();
            assert!(
                selection.iter().any(|n| n == "git"),
                "git dropped out of the router budget for {message:?}: {selection:?}"
            );
        }
    }

    /// The branch line of porcelain output comes in three shapes; all three
    /// have to yield a readable name.
    #[test]
    fn the_branch_name_is_read_out_of_every_porcelain_shape() {
        assert_eq!(branch_name("main"), "main");
        assert_eq!(branch_name("main...origin/main"), "main");
        assert_eq!(
            branch_name("main...origin/main [ahead 2, behind 1]"),
            "main"
        );
        assert_eq!(
            branch_name("No commits yet on main"),
            "No commits yet on main"
        );
    }

    // -----------------------------------------------------------------------
    // H6: the repository's own config must not run anything.
    //
    // Every test below builds the hostile repository FIRST (commits made with
    // a clean config), THEN writes the hostile keys, then calls the tool. The
    // measurement is a marker file that the hostile command would create; it
    // lives OUTSIDE the work tree so it can never show up in `status` itself.
    // -----------------------------------------------------------------------

    /// git's stdout, for the fixtures that need an object id.
    fn git_out(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git must be installed on the runner");
        assert!(
            out.status.success(),
            "fixture command failed: {args:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A shell command that creates `marker` and then behaves like `cat`, so a
    /// filter or textconv that DID run would still produce sane output and the
    /// only evidence is the marker.
    fn touch_then_cat(marker: &Path) -> String {
        format!("touch '{}'; cat", marker.display())
    }

    /// Moves a file's mtime far into the past WITHOUT changing its content.
    ///
    /// This is the archive-extraction case: stat data that matches no index
    /// entry, so git has to hash the file again — and the hash goes through the
    /// clean filter. A content change alone would NOT do: a size change lets
    /// `status` say "modified" without reading a byte.
    fn age(path: &Path) {
        let file = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open for mtime");
        file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000))
            .expect("set mtime");
    }

    fn assert_nothing_ran(markers: &Path, what: &str) {
        let left: Vec<String> = std::fs::read_dir(markers)
            .expect("markers dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            left.is_empty(),
            "{what}: the repository's config ran a program — markers created: {left:?}"
        );
    }

    /// THE AUDIT'S OWN REPRODUCTION. `core.fsmonitor` is consulted on every
    /// index read, so all three actions are measured, not just `status`.
    #[test]
    fn a_repository_config_cannot_run_an_fsmonitor_hook() {
        let root = temp_dir("h6-fsmonitor");
        let markers = temp_dir("h6-fsmonitor-markers");
        init_repo(&root);
        write(&root, "a.txt", "one\n");
        commit(&root, "first");
        write(&root, "a.txt", "one\ntwo\n");

        let marker = markers.join("fsmonitor");
        git_fixture(
            &root,
            &[
                "config",
                "core.fsmonitor",
                &format!("touch '{}'; false", marker.display()),
            ],
        );

        for action in ["status", "diff", "log"] {
            let (outcome, _, _) = call(&root, action);
            assert!(
                matches!(outcome.state, ToolState::Read),
                "{action}: {:?} {}",
                outcome.state,
                outcome.to_model
            );
            assert_nothing_ran(&markers, action);
        }
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&markers).ok();
    }

    /// `diff.external` (the audit's second reproduction) and its per-file
    /// cousin `diff.<driver>.command`, chosen through `.gitattributes`. The
    /// answer must still be the real diff — switching the program off must not
    /// switch the tool off.
    #[test]
    fn a_repository_config_cannot_run_an_external_diff() {
        let root = temp_dir("h6-extdiff");
        let markers = temp_dir("h6-extdiff-markers");
        init_repo(&root);
        // Two files ON PURPOSE: where a driver's `command` applies it shadows
        // `diff.external` (measured — with `a.txt` alone only the driver's
        // marker appeared), so `b.md` carries no attribute and is the file
        // `diff.external` is measured on.
        write(&root, ".gitattributes", "*.txt diff=evil\n");
        write(&root, "a.txt", "one\n");
        write(&root, "b.md", "one\n");
        commit(&root, "first");
        write(&root, "a.txt", "one\ntwo\n");
        write(&root, "b.md", "one\ntwo\n");

        git_fixture(
            &root,
            &[
                "config",
                "diff.external",
                &format!("touch '{}'; true", markers.join("external").display()),
            ],
        );
        git_fixture(
            &root,
            &[
                "config",
                "diff.evil.command",
                &format!("touch '{}'; true", markers.join("driver").display()),
            ],
        );

        let (outcome, ctx, _) = call(&root, "diff");
        assert_nothing_ran(&markers, "diff");
        assert!(
            outcome.to_model.contains("2 files changed, +2 -0"),
            "{}",
            outcome.to_model
        );
        // The stored patch is git's own, not whatever an external tool printed.
        let record = ctx.from_store(&SourceRef("git#1".into())).expect("record");
        assert!(record.body.contains("+two"), "{}", record.body);

        let (_, _, _) = call(&root, "log");
        assert_nothing_ran(&markers, "log");
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&markers).ok();
    }

    /// `diff.<driver>.textconv` runs on BOTH sides of a diff by default.
    #[test]
    fn a_repository_config_cannot_run_a_textconv() {
        let root = temp_dir("h6-textconv");
        let markers = temp_dir("h6-textconv-markers");
        init_repo(&root);
        write(&root, ".gitattributes", "*.txt diff=evil\n");
        write(&root, "a.txt", "one\n");
        commit(&root, "first");
        write(&root, "a.txt", "one\ntwo\n");

        git_fixture(
            &root,
            &[
                "config",
                "diff.evil.textconv",
                &touch_then_cat(&markers.join("textconv")),
            ],
        );

        let (outcome, _, _) = call(&root, "diff");
        assert_nothing_ran(&markers, "diff");
        assert!(
            outcome.to_model.contains("1 files changed, +1 -0"),
            "{}",
            outcome.to_model
        );
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&markers).ok();
    }

    /// FILTER DRIVERS — the vector the audit did not list, found while fixing
    /// H6: the driver name is the repository's choice, so no fixed flag covers
    /// it (see `neutralizing_args`).
    ///
    /// `driver` is attached to `*.txt`; the keys `config` builds (given the
    /// markers directory) are written into the repository's config only AFTER
    /// the commit. `same.txt` keeps its content and loses its stat match (the
    /// extracted-archive case); `edited.txt` really changes — so `status` has
    /// to hash one and `diff` has to read the other, both through the filter.
    ///
    /// ONE DRIVER PER REPOSITORY, and that was measured: with a `clean` and a
    /// failing `process` driver in the same fixture, the `process` failure
    /// stopped git before the `clean` driver was reached, so the unprotected
    /// run showed one marker and could not tell the two belts apart.
    fn assert_filter_is_neutralized(
        label: &str,
        driver: &str,
        config: impl Fn(&Path) -> Vec<(String, String)>,
    ) {
        let root = temp_dir(label);
        let markers = temp_dir(&format!("{label}-markers"));
        init_repo(&root);
        write(&root, ".gitattributes", &format!("*.txt filter={driver}\n"));
        write(&root, "same.txt", "unchanged\n");
        write(&root, "edited.txt", "one\n");
        commit(&root, "first");
        age(&root.join("same.txt"));
        write(&root, "edited.txt", "one\ntwo\n");
        for (key, value) in config(&markers) {
            git_fixture(&root, &["config", &key, &value]);
        }

        let (status_outcome, _, _) = call(&root, "status");
        assert_nothing_ran(&markers, "status");
        // A neutralized driver is "no filter", not a fatal `required` failure:
        // the answer is still there and still right.
        assert!(
            matches!(status_outcome.state, ToolState::Read),
            "{:?} {}",
            status_outcome.state,
            status_outcome.to_model
        );
        assert!(
            status_outcome.to_model.contains("1 files touched"),
            "{}",
            status_outcome.to_model
        );
        assert!(status_outcome.to_model.contains("edited.txt"));

        let (diff_outcome, _, _) = call(&root, "diff");
        assert_nothing_ran(&markers, "diff");
        assert!(
            diff_outcome.to_model.contains("1 files changed, +1 -0"),
            "{}",
            diff_outcome.to_model
        );
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&markers).ok();
    }

    /// A `required` `clean` filter. Named `a=b.c` ON PURPOSE — `-c` would split
    /// that name at the wrong `=`, which is why the overrides use
    /// `--config-env`; `required` is there to prove a blanked driver is "no
    /// filter" rather than a fatal error that hides the answer.
    #[test]
    fn a_repository_clean_filter_cannot_run() {
        assert_filter_is_neutralized("h6-clean", "a=b.c", |markers| {
            vec![
                (
                    "filter.a=b.c.clean".to_string(),
                    touch_then_cat(&markers.join("clean")),
                ),
                ("filter.a=b.c.required".to_string(), "true".to_string()),
            ]
        });
    }

    /// A long-running `process` filter — the git-lfs shape, and a separate
    /// key from `clean`, so blanking `clean` alone would leave it live.
    #[test]
    fn a_repository_process_filter_cannot_run() {
        assert_filter_is_neutralized("h6-process", "proc", |markers| {
            vec![(
                "filter.proc.process".to_string(),
                format!("touch '{}'; false", markers.join("process").display()),
            )]
        });
    }

    /// A submodule is a second repository with a second config, read by a
    /// second git that `neutralizing_args` never saw. Its filter must not run
    /// either — which is what `SUBMODULES_BY_COMMIT` buys.
    #[test]
    fn a_submodules_own_config_cannot_run_a_filter() {
        let root = temp_dir("h6-submodule");
        let markers = temp_dir("h6-submodule-markers");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).expect("sub dir");
        init_repo(&sub);
        write(&sub, ".gitattributes", "*.txt filter=evil\n");
        write(&sub, "inner.txt", "inside\n");
        commit(&sub, "sub first");

        init_repo(&root);
        write(&root, "top.txt", "top\n");
        // `add -A` records `sub` as a gitlink — a submodule without
        // `.gitmodules`, which git still walks into.
        commit(&root, "super first");

        age(&sub.join("inner.txt"));
        git_fixture(
            &sub,
            &[
                "config",
                "filter.evil.clean",
                &touch_then_cat(&markers.join("sub-clean")),
            ],
        );

        for action in ["status", "diff"] {
            let (outcome, _, _) = call(&root, action);
            assert!(
                matches!(outcome.state, ToolState::Read),
                "{action}: {:?} {}",
                outcome.state,
                outcome.to_model
            );
            assert_nothing_ran(&markers, action);
        }
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&markers).ok();
    }

    /// THE BYPASS THE FIRST H6 FIX MISSED (found by review): with
    /// `diff.submodule=diff` in the OUTER config, `git diff` shows a moved
    /// submodule by starting a child `git diff` inside it. That child reads the
    /// submodule's own config (`.git/modules/sub/config`) and does NOT inherit
    /// `--no-ext-diff`/`--no-textconv`, so the submodule's `diff.external` and
    /// `diff.<driver>.textconv` both ran. `--ignore-submodules=dirty` does not
    /// help: the submodule's COMMIT really did change.
    ///
    /// `status.submoduleSummary` is the same shape for `status` (a child
    /// `git submodule summary`), so it is set too and `status` is measured.
    #[test]
    fn a_repository_cannot_make_diff_run_inside_a_submodule() {
        let root = temp_dir("h6-subdiff");
        let source = temp_dir("h6-subdiff-source");
        let markers = temp_dir("h6-subdiff-markers");
        init_repo(&source);
        write(&source, ".gitattributes", "*.txt diff=tx\n");
        write(&source, "inner.txt", "inside\n");
        commit(&source, "sub first");

        init_repo(&root);
        write(&root, "top.txt", "top\n");
        commit(&root, "super first");
        // A REAL submodule, so its config lives in `.git/modules/sub/config` —
        // the place the review measured. `protocol.file.allow` only lets this
        // FIXTURE clone from a local folder; the tool never clones.
        git_fixture(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                &source.to_string_lossy(),
                "sub",
            ],
        );
        commit(&root, "add sub");

        // Move the submodule's HEAD one commit: the change `dirty` still shows.
        let sub = root.join("sub");
        write(&sub, "inner.txt", "inside\nmoved\n");
        commit(&sub, "sub second");

        git_fixture(
            &sub,
            &[
                "config",
                "diff.external",
                &format!("touch '{}'; true", markers.join("sub-ext").display()),
            ],
        );
        git_fixture(
            &sub,
            &[
                "config",
                "diff.tx.textconv",
                &touch_then_cat(&markers.join("sub-textconv")),
            ],
        );
        git_fixture(&root, &["config", "diff.submodule", "diff"]);
        git_fixture(&root, &["config", "status.submoduleSummary", "true"]);

        let (outcome, _, _) = call(&root, "diff");
        assert_nothing_ran(&markers, "diff (diff.submodule=diff)");
        // The moved submodule is still REPORTED — by commit, not by its patch.
        assert!(outcome.to_model.contains("sub"), "{}", outcome.to_model);

        // `diff.external` shadows the textconv (measured by the review: the
        // textconv marker appeared only once `diff.external` was unset), so
        // the textconv is measured on its own.
        git_fixture(&sub, &["config", "--unset", "diff.external"]);
        for action in ["diff", "status", "log"] {
            let (outcome, _, _) = call(&root, action);
            assert!(
                matches!(outcome.state, ToolState::Read),
                "{action}: {:?} {}",
                outcome.state,
                outcome.to_model
            );
            assert_nothing_ran(&markers, action);
        }
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&source).ok();
        std::fs::remove_dir_all(&markers).ok();
    }

    /// A SIGNED COMMIT makes `log` run `gpg.program` when `log.showSignature`
    /// is on — and both keys can sit in the repository's config. The signature
    /// is fake on purpose: git calls the program to VERIFY it, so the call is
    /// the whole attack and the signature never has to be valid.
    ///
    /// Unix only: the stand-in gpg is a shell script with the executable bit.
    #[cfg(unix)]
    #[test]
    fn a_signed_commit_cannot_run_the_repositorys_gpg_program() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_dir("h6-gpg");
        let markers = temp_dir("h6-gpg-markers");
        let tools = temp_dir("h6-gpg-tools");
        init_repo(&root);
        write(&root, "a.txt", "one\n");
        commit(&root, "first");

        // A ROOT commit over the same tree: with the first commit as its parent
        // it would change nothing under `-- .`, and `log` would skip it as
        // TREESAME before ever looking at the signature.
        let tree = git_out(&root, &["rev-parse", "HEAD^{tree}"]);
        let object = format!(
            "tree {tree}\n\
             author Test <test@example.com> 1700000000 +0000\n\
             committer Test <test@example.com> 1700000000 +0000\n\
             gpgsig -----BEGIN PGP SIGNATURE-----\n \n iQ==\n -----END PGP SIGNATURE-----\n\
             \nsigned commit\n"
        );
        let object_file = tools.join("commit.txt");
        std::fs::write(&object_file, object).expect("commit object");
        let signed = git_out(
            &root,
            &[
                "hash-object",
                "-t",
                "commit",
                "-w",
                &object_file.to_string_lossy(),
            ],
        );
        git_fixture(&root, &["update-ref", "refs/heads/main", &signed]);

        let fake_gpg = tools.join("gpg");
        std::fs::write(
            &fake_gpg,
            format!(
                "#!/bin/sh\ntouch '{}'\nexit 1\n",
                markers.join("gpg").display()
            ),
        )
        .expect("fake gpg");
        std::fs::set_permissions(&fake_gpg, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        git_fixture(&root, &["config", "log.showSignature", "true"]);
        git_fixture(
            &root,
            &["config", "gpg.program", &fake_gpg.to_string_lossy()],
        );

        let (outcome, _, _) = call(&root, "log");
        assert_nothing_ran(&markers, "log");
        assert!(
            outcome.to_model.contains("signed commit"),
            "{}",
            outcome.to_model
        );
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&markers).ok();
        std::fs::remove_dir_all(&tools).ok();
    }

    /// A PARTIAL CLONE fetches a missing object from its promisor remote, and
    /// the repository's config chooses that remote's URL — `ext::` runs a
    /// command, and `protocol.ext.allow=always` sits in the same config. The
    /// whole `.git/` is the attacker's, so "an object is missing" costs them
    /// one `rm`.
    ///
    /// Unix only: the `ext::` command is `sh`.
    #[cfg(unix)]
    #[test]
    fn a_promisor_remote_cannot_turn_a_missing_object_into_a_command() {
        let root = temp_dir("h6-promisor");
        let markers = temp_dir("h6-promisor-markers");
        init_repo(&root);
        write(&root, "a.txt", "one\n");
        commit(&root, "first");

        let blob = git_out(&root, &["rev-parse", "HEAD:a.txt"]);
        let loose = root.join(".git/objects").join(&blob[..2]).join(&blob[2..]);
        std::fs::remove_file(&loose).expect("drop the blob");
        write(&root, "a.txt", "one\ntwo\n");

        // `% ` is ext::'s escaped space: the remote helper runs
        // `sh -c "touch <marker>"`.
        let url = format!("ext::sh -c touch% {}", markers.join("ext").display());
        for (key, value) in [
            ("core.repositoryformatversion", "1"),
            ("extensions.partialClone", "origin"),
            ("remote.origin.url", url.as_str()),
            ("remote.origin.promisor", "true"),
            ("protocol.ext.allow", "always"),
        ] {
            git_fixture(&root, &["config", key, value]);
        }

        // The answer itself may well be a failure — the blob really is gone —
        // and that is the honest outcome. The measurement is that nothing ran.
        let _ = call(&root, "diff");
        assert_nothing_ran(&markers, "diff");
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&markers).ok();
    }

    /// `safe.bareRepository=explicit`: a folder that IS a bare repository is
    /// not silently adopted as one. Measured on `run_git` directly, because
    /// through the tool both outcomes read "no repository here" (a bare
    /// repository has no work tree) and the test could not go red.
    #[test]
    fn an_implicit_bare_repository_is_not_discovered() {
        let root = temp_dir("h6-bare");
        git_fixture(&root, &["init", "--bare", "-q"]);
        let run = run_git(
            "git",
            &root,
            &["rev-parse", "--git-dir"],
            &CancelToken::new(),
        )
        .expect("git runs");
        assert!(
            !run.ok,
            "a bare repository was discovered implicitly: {}",
            run.stdout
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// THE OVERRIDES NO FIXTURE CAN TRIP TODAY — `core.hooksPath` (no
    /// read-only subcommand fires a hook), `GIT_CONFIG_NOSYSTEM` (a test cannot
    /// write the system file), the transport belts — are measured on the
    /// finished command line. `git_command` is the only builder, so what is
    /// true here is true of every invocation; and every git-level option must
    /// come BEFORE the subcommand, where git reads it.
    #[test]
    fn every_invocation_carries_the_overrides() {
        let cmd = git_command("git", Path::new("."), &[], &["status"]);
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let subcommand = args
            .iter()
            .position(|a| a == "status")
            .expect("the subcommand is passed");
        #[cfg(windows)]
        let hooks = "core.hooksPath=NUL";
        #[cfg(not(windows))]
        let hooks = "core.hooksPath=/dev/null";
        for setting in [
            "core.fsmonitor=false",
            hooks,
            "core.pager=cat",
            "safe.bareRepository=explicit",
            "log.showSignature=false",
        ] {
            let at = args
                .iter()
                .position(|a| a == setting)
                .unwrap_or_else(|| panic!("missing -c {setting}: {args:?}"));
            assert_eq!(args[at - 1], "-c", "{setting} is not a -c value: {args:?}");
            assert!(at < subcommand, "{setting} comes after the subcommand");
        }
        for flag in ["--no-pager", "--no-optional-locks"] {
            let at = args.iter().position(|a| a == flag).expect(flag);
            assert!(at < subcommand, "{flag} comes after the subcommand");
        }

        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        for (key, value) in [
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_NO_LAZY_FETCH", "1"),
            ("GIT_ALLOW_PROTOCOL", ""),
            ("GIT_TERMINAL_PROMPT", "0"),
            (ENV_EMPTY, ""),
            (ENV_FALSE, "false"),
        ] {
            assert!(
                envs.contains(&(key.to_string(), Some(value.to_string()))),
                "missing env {key}={value:?}: {envs:?}"
            );
        }
    }

    /// AUDIT M30, review of 10 Oct 2026: a git that outlives the call's
    /// deadline, or a Ctrl-C, used to keep running orphaned after the
    /// executor abandoned the worker — `output()` never looked at the token.
    /// A stand-in that only sleeps plays the slow repository; it writes its
    /// pid so the test can see it is gone, not merely no longer waited for.
    #[cfg(unix)]
    #[test]
    fn a_stop_kills_the_git_that_is_running() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};
        let root = temp_dir("git-stop");
        let pid_file = root.join("pid");
        let program = root.join("slow-git");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\necho $$ > '{}'\nexec sleep 20\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

        // The stop is said once the stand-in is known to be running — under
        // a loaded test run a fixed delay fired before it had started.
        let stop = CancelToken::new();
        let stopper = {
            let stop = stop.clone();
            let pid_file = pid_file.clone();
            std::thread::spawn(move || {
                let waiting = Instant::now();
                while !pid_file.exists() && waiting.elapsed() < Duration::from_secs(10) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                std::thread::sleep(Duration::from_millis(50));
                stop.cancel();
                Instant::now()
            })
        };
        let run = run_git(program.to_str().unwrap(), &root, &["status"], &stop);
        let stopped_at = stopper.join().unwrap();
        assert!(matches!(run, Err(GitFailure::Stopped)), "{:?}", run.err());
        assert!(
            stopped_at.elapsed() < Duration::from_secs(2),
            "the run waited {:?} after the stop for a git it had been told to stop",
            stopped_at.elapsed()
        );
        let pid = std::fs::read_to_string(&pid_file).expect("the stand-in ran");
        let alive = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status()
            .expect("kill runs")
            .success();
        assert!(!alive, "git {} is still running after the stop", pid.trim());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// WHOSE filters are switched off. The repository's (`local`,
    /// `worktree`, and any scope git might add later) — yes. The user's own
    /// (`global`, where `git lfs install` writes) — no: blanking git-lfs would
    /// make every LFS file read as modified.
    #[test]
    fn only_the_repositorys_filter_drivers_are_neutralized() {
        let raw = "global\0filter.lfs.clean\0\
                   global\0filter.lfs.process\0\
                   local\0filter.a=b.c.clean\0\
                   local\0filter.a=b.c.required\0\
                   worktree\0filter.wt.smudge\0\
                   somethingnew\0filter.odd.process\0\
                   command\0filter.mine.clean\0";
        let args = neutralizing_args(raw);
        let joined = args.join(" ");
        assert!(!joined.contains("filter.lfs."), "{joined}");
        assert!(!joined.contains("filter.mine."), "{joined}");
        for name in ["a=b.c", "wt", "odd"] {
            for key in ["clean", "smudge", "process"] {
                let want = format!("--config-env=filter.{name}.{key}={ENV_EMPTY}");
                assert!(args.contains(&want), "missing {want}: {args:?}");
            }
            let want = format!("--config-env=filter.{name}.required={ENV_FALSE}");
            assert!(args.contains(&want), "missing {want}: {args:?}");
        }
        // One driver mentioned by two keys is neutralized once.
        assert_eq!(args.len(), 3 * 4, "{args:?}");
        assert!(neutralizing_args("").is_empty());
    }

    /// A stand-in for an OLDER git: a script that rejects one option the way
    /// git rejects an unknown one (usage, exit 129) and hands everything else
    /// to the real git. Leaked into a `&'static str` because that is the only
    /// type the `program` seam takes — on purpose, see the field.
    #[cfg(unix)]
    fn git_rejecting(dir: &Path, option_glob: &str) -> &'static str {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("old-git");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do\n  case \"$a\" in\n    {option_glob}) \
                 echo \"unknown option: $a\" >&2; exit 129;;\n  esac\ndone\nexec git \"$@\"\n"
            ),
        )
        .expect("write wrapper");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod wrapper");
        Box::leak(script.to_string_lossy().into_owned().into_boxed_str())
    }

    fn call_with(program: &'static str, root: &Path, action: &str) -> (ToolOutcome, ToolContext) {
        let store = Arc::new(InMemoryDataStore::new());
        let mut ctx = context(root, store);
        let outcome =
            block_on(GitTool::with_program(program).run(json!({ "action": action }), &mut ctx));
        (outcome, ctx)
    }

    fn assert_unsupported(program: &'static str, root: &Path, what: &str) {
        for action in ["status", "diff", "log"] {
            let (outcome, ctx) = call_with(program, root, action);
            assert!(
                matches!(outcome.state, ToolState::Read),
                "{what} {action}: {:?}",
                outcome.state
            );
            assert!(
                outcome.to_model.starts_with("git_unsupported"),
                "{what} {action}: a real repository was reported as: {}",
                outcome.to_model
            );
            assert!(!ctx.session_tainted(), "{what} {action} tainted");
        }
    }

    /// AN OLD GIT IS NOT "NO REPOSITORY". The filter scan needs
    /// `git config --show-scope` (git 2.26) and the overrides need
    /// `--config-env` (git 2.31). Refusing to run without them is the H6 fix
    /// working; calling a real, committed repository `no_git_repository` is a
    /// false answer — the exact "no git here" vs "git failed" confusion the
    /// module header rules out. Measured before the fix: both stand-ins below
    /// made `status` say `no_git_repository`.
    #[cfg(unix)]
    #[test]
    fn a_git_too_old_for_the_overrides_is_not_reported_as_no_repository() {
        let root = temp_dir("old-git");
        let bin = temp_dir("old-git-bin");
        init_repo(&root);
        write(&root, "a.txt", "one\n");
        commit(&root, "first");

        // Ubuntu 20.04's git 2.25: no `--show-scope`, in EVERY repository.
        let pre_2_26 = git_rejecting(&bin, "--show-scope");
        assert_unsupported(pre_2_26, &root, "git<2.26");

        // git 2.26–2.30: the scan works, but a repository WITH a filter needs
        // `--config-env`, which these versions reject.
        let bin2 = temp_dir("old-git-bin2");
        let pre_2_31 = git_rejecting(&bin2, "--config-env=*");
        let (outcome, _) = call_with(pre_2_31, &root, "status");
        // ...and without a filter there is nothing to override, so that git
        // still answers normally — the refusal is no wider than the hole.
        assert!(
            outcome.to_model.contains("the working tree is clean"),
            "git 2.26-2.30, no filter: {}",
            outcome.to_model
        );
        git_fixture(&root, &["config", "filter.f.clean", "cat"]);
        assert_unsupported(pre_2_31, &root, "git<2.31 with a filter");

        for dir in [&root, &bin, &bin2] {
            std::fs::remove_dir_all(dir).ok();
        }
    }
}
