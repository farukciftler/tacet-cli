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
//! is no git here" from "git crashed". Both of those, and "git is not installed",
//! come back as ordinary read results with their own wording, so the model can
//! tell the user what is actually going on.

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;
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
/// They live in `run_git` so that no call site can forget them:
///   * `--no-pager` — git must never hand its output to a pager. There is no
///     terminal on the other end of a chat turn.
///   * `--no-optional-locks` — without it `git status` REFRESHES THE INDEX, i.e.
///     takes `.git/index.lock` and writes. This tool claims to be read-only; a
///     claim that costs nothing to keep is kept.
const BASE_ARGS: [&str; 2] = ["--no-pager", "--no-optional-locks"];

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

/// Runs `git` in `dir` with exactly `args` (plus `BASE_ARGS`).
///
/// NO TIMEOUT, AND THAT IS A DECISION. The three subcommands are local-only —
/// none of them opens a socket — so the runtime is bounded by the size of the
/// user's own repository, exactly like the blocking `std::fs::read` in
/// `read_document`. A timeout would mean a second process-supervision path next
/// to `run_code`'s, and one that could not be measured by a test on this runner;
/// this repository's recurring failure is precisely the mechanism that is built
/// and never proven.
fn run_git(program: &str, dir: &Path, args: &[&str]) -> std::io::Result<GitRun> {
    let out = Command::new(program)
        .args(BASE_ARGS)
        .args(args)
        .current_dir(dir)
        // stdin is CLOSED: git must never be able to sit waiting on a terminal
        // that a chat turn does not have.
        .stdin(Stdio::null())
        .output()?;
    Ok(GitRun {
        ok: out.status.success(),
        // Paths on disk are not guaranteed to be UTF-8; a lossy read is better
        // than refusing to answer because one file name is odd.
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
    })
}

/// What the working directory turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Repo {
    Present,
    /// There is a git, but we are not inside a work tree.
    Absent,
    /// The binary could not be run at all.
    Unavailable,
}

/// Asks git itself instead of reading its error text.
///
/// WHY NOT MATCH ON "not a git repository": that message is LOCALIZED — on a
/// machine with a non-English git the substring never appears and every directory
/// would look like a repository. `rev-parse --is-inside-work-tree` answers
/// structurally: exit status plus `true`/`false`.
fn probe(program: &str, dir: &Path) -> Repo {
    match run_git(program, dir, &["rev-parse", "--is-inside-work-tree"]) {
        Err(_) => Repo::Unavailable,
        Ok(r) if r.ok && r.stdout.trim() == "true" => Repo::Present,
        // `false` = we are inside the `.git` directory itself; a non-zero exit =
        // no repository. Neither is something this tool can report on.
        Ok(_) => Repo::Absent,
    }
}

/// Does the repository have a commit yet? A fresh `git init` has no `HEAD`, and
/// `git diff HEAD` / `git log` FAIL there rather than returning empty — the state
/// has to be known before the command is chosen, not guessed from the error.
fn has_commit(program: &str, dir: &Path) -> bool {
    run_git(program, dir, &["rev-parse", "--verify", "--quiet", "HEAD"])
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

fn status(program: &str, dir: &Path) -> ToolResult<Report> {
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
            HERE[0],
            HERE[1],
        ],
    )
    .map_err(|_| ToolError::Other("The git command could not be run.".into()))?;
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

fn diff(program: &str, dir: &Path) -> ToolResult<Report> {
    // WHICH DIFF: against `HEAD`, i.e. staged AND unstaged changes together —
    // "what did I change since the last commit" is the question actually being
    // asked. On a repository with no commit yet there is no `HEAD` to compare
    // with, so the staged set is the whole of the change.
    let base: &str = if has_commit(program, dir) {
        "HEAD"
    } else {
        "--cached"
    };

    let stat_run = run_git(program, dir, &["diff", base, "--numstat", HERE[0], HERE[1]])
        .map_err(|_| ToolError::Other("The git command could not be run.".into()))?;
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
    let patch = run_git(program, dir, &["diff", base, HERE[0], HERE[1]])
        .map_err(|_| ToolError::Other("The git command could not be run.".into()))?;
    Ok(Report {
        chip: format!("git diff · {} files", stats.len()),
        summary,
        body: Some(cap_body(&patch.stdout)),
    })
}

// ---------------------------------------------------------------------------
// log
// ---------------------------------------------------------------------------

fn log(program: &str, dir: &Path) -> ToolResult<Report> {
    if !has_commit(program, dir) {
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
            &count,
            "--date=short",
            // A fixed, one-line-per-commit shape: short hash, date, author,
            // subject. `%s` is the subject alone, so a commit body can never turn
            // one commit into forty lines.
            "--pretty=format:%h %ad %an: %s",
            HERE[0],
            HERE[1],
        ],
    )
    .map_err(|_| ToolError::Other("The git command could not be run.".into()))?;
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

        match probe(self.program, &dir) {
            Repo::Unavailable => Ok((
                ToolOutcome::read_ok(
                    "git is not installed",
                    "git_unavailable: the git command line tool could not be run on this device",
                ),
                false,
            )),
            Repo::Absent => Ok((
                ToolOutcome::read_ok(
                    "no git repository here",
                    "no_git_repository: the working directory is not inside a git repository",
                ),
                false,
            )),
            Repo::Present => {
                let report = match action {
                    Action::Status => status(self.program, &dir)?,
                    Action::Diff => diff(self.program, &dir)?,
                    Action::Log => log(self.program, &dir)?,
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
}
