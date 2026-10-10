//! `tacet handoff` — Tacet's progress, reported back to the agent a session
//! came from (RG-21..27). The mechanisms live in
//! `tacet_tools::sessions::writeback`; this file is the command line around
//! them: which file, which root, and the question on the terminal.
//!
//! EVERY WRITE INTO ANOTHER AGENT'S FILES ASKS HERE (`TerminalConfirm`), and
//! with nobody at the terminal the answer is no — the rule the outbound
//! approval gate follows (audit L31). `update` is the exception on purpose: it
//! writes Tacet's own handoff record, and it is the command the OTHER agents
//! run, unattended, to report back (docs/agent-handoff.md).

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tacet_tools::sessions::claude::ClaudeSource;
use tacet_tools::sessions::codex::CodexSource;
use tacet_tools::sessions::handoff::{self, HandoffRecord, HandoffTask, Status};
use tacet_tools::sessions::model::{AgentKind, SessionError, iso_utc, now_epoch, validate_id};
use tacet_tools::sessions::writeback::{
    self, Confirm, Question, WriteError, antigravity as ag_write, claude as claude_write,
    codex as codex_write,
};
use tacet_tools::sessions_tool::{SessionRoots, guarded};

use crate::cli::HandoffJob;
use crate::ui::{self, BOLD, Color, DIM, YELLOW};

/// The y/N question on the terminal, every line made screen-safe where it
/// meets the terminal: the note and the paths come from files another program
/// wrote.
struct TerminalConfirm;

impl Confirm for TerminalConfirm {
    fn confirm(&self, question: &Question) -> bool {
        confirm_on(question, &ui::Terminal)
    }
}

fn confirm_on(question: &Question, console: &dyn ui::Console) -> bool {
    // NOBODY TO ASK IS A NO, and stdin is not touched (audit L31).
    let Some(prompt) = ui::Prompt::open(console) else {
        eprintln!(
            "  {} — nobody at a terminal to ask, so nothing was written",
            ui::one_line(&question.heading)
        );
        return false;
    };
    let color = Color::setup();
    eprintln!();
    eprintln!("  {}", color.paint(BOLD, &ui::one_line(&question.heading)));
    for line in &question.lines {
        eprintln!("    {}", ui::one_line(line));
    }
    eprint!("  Write it? [y/N] ");
    let _ = std::io::stderr().flush();
    ui::is_yes(prompt.read().as_deref())
}

fn agent_of(text: &str) -> Result<AgentKind, String> {
    AgentKind::from_slug(text).ok_or_else(|| {
        let known: Vec<&str> = AgentKind::ALL.iter().map(|a| a.slug()).collect();
        format!("unknown agent {text:?}; one of: {}", known.join(", "))
    })
}

fn handoff_dir() -> Result<PathBuf, String> {
    handoff::default_dir()
        .ok_or_else(|| "the config directory could not be resolved (set TACET_HOME)".to_string())
}

pub fn run(job: HandoffJob) -> ExitCode {
    run_with(job, dispatch)
}

/// What `tacet handoff` says when the work under it panicked. It cannot say
/// nothing was written: `append_note` writes, then re-reads the chain to
/// check it, and the re-read is a parse like any other.
const STOPPED: &str = "tacet handoff stopped on an internal error while reading another agent's \
                       files; whether anything was written before it stopped is not known";

/// `run` with the work passed in, so a test can hand it work that panics.
fn run_with(
    job: HandoffJob,
    work: impl FnOnce(HandoffJob, &Color) -> Result<(), String>,
) -> ExitCode {
    // `hook` answers on stdout for another program and must never fail it.
    if let HandoffJob::Hook { agent } = &job {
        hook(agent);
        return ExitCode::SUCCESS;
    }
    let color = Color::setup();
    // UNDER `guarded`, EVERY SUBCOMMAND (RG-04): `note` and `tasks` call the
    // Claude Code, Codex and Antigravity readers directly — `locate`, the
    // chain rebuild behind `append_note` and `fork_with_note`, the rollout
    // parse, the task.md parser — with no executor above them, so a reader
    // panicking on a line from a release it has never seen was a backtrace
    // instead of a sentence.
    let result = guarded(|| work(job, &color)).unwrap_or_else(|_| Err(STOPPED.to_string()));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("  {}", color.paint(YELLOW, &message));
            ExitCode::FAILURE
        }
    }
}

fn dispatch(job: HandoffJob, color: &Color) -> Result<(), String> {
    match job {
        HandoffJob::Hook { .. } => Ok(()),
        HandoffJob::Install { agent, path } => install(&agent, path.as_deref(), color),
        HandoffJob::Show { agent, id, json } => show(&agent, &id, json),
        HandoffJob::Update {
            agent,
            id,
            by,
            summary,
            next,
            tasks,
            notes,
        } => update(&agent, &id, &by, summary, next, &tasks, &notes),
        HandoffJob::Note {
            agent,
            id,
            fork,
            program,
            append_rollout,
            root,
        } => note(
            &agent,
            &id,
            fork,
            program.as_deref(),
            append_rollout,
            root.as_deref(),
        ),
        HandoffJob::Tasks {
            agent,
            id,
            set,
            experimental,
            root,
        } => tasks(&agent, &id, &set, experimental, root.as_deref()),
    }
}

/// `tacet handoff hook <agent>`: silence unless there is something to say,
/// exit 0 whatever happens.
fn hook(agent: &str) {
    let Ok(agent) = agent_of(agent) else { return };
    let Some(dir) = handoff::default_dir() else {
        return;
    };
    let mut input = String::new();
    if std::io::stdin()
        .take(writeback::MAX_HOOK_INPUT)
        .read_to_string(&mut input)
        .is_err()
    {
        return;
    }
    if let Some(out) = hook_answer(|| writeback::hook_response(agent, &dir, &input, &tacet_bin())) {
        println!("{out}");
    }
}

/// The hook's answer, or silence if building it panicked (RG-04): the input
/// is JSON another agent wrote, and a panic here exits 101 under that agent,
/// which reports the hook as failed on every session start — the one thing a
/// hook that "must never fail it" cannot do.
fn hook_answer<T>(respond: impl FnOnce() -> Option<T>) -> Option<T> {
    guarded(respond).ok().flatten()
}

/// The path of this binary for the hook command. Canonical, so a hook keeps
/// working when the shell that ran `install` had a relative PATH entry.
fn tacet_bin() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "tacet".to_string())
}

fn install(agent: &str, path: Option<&Path>, color: &Color) -> Result<(), String> {
    let agent = agent_of(agent)?;
    let command = writeback::hook_command(&tacet_bin(), agent);
    let snippet = writeback::install_snippet(agent, &command);
    println!(
        "{} hook for {}: add this to {}",
        writeback::hook_event(agent),
        agent.display_name(),
        writeback::hook_files(agent).join(" or ")
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&snippet).unwrap_or_default()
    );
    if agent != AgentKind::ClaudeCode {
        println!(
            "{}",
            color.paint(
                DIM,
                "This entry follows Claude Code's documented hook shape; it was not run against \
                 this agent on a real install. Check it against the agent's hook documentation."
            )
        );
    }
    let Some(path) = path else {
        println!(
            "{}",
            color.paint(
                DIM,
                "Nothing was written. Give --path <settings file> to have Tacet add it (it asks first)."
            )
        );
        return Ok(());
    };
    match install_into(path, agent, &command, &TerminalConfirm)? {
        Installed::Already => println!("{} already runs this hook.", path.display()),
        Installed::Written { backup } => {
            println!("Added to {}.", path.display());
            if let Some(b) = backup {
                println!("The file as it was is at {}.", b.display());
            }
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
enum Installed {
    Already,
    Written { backup: Option<PathBuf> },
}

/// The largest settings file `install` edits.
const MAX_SETTINGS_BYTES: u64 = 1 << 20;

/// The settings file as it is now: `None` when there is none, an error when
/// the path is not something `install` should edit.
fn read_settings(path: &Path) -> Result<Option<String>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() => Err(format!("{} is not a regular file", path.display())),
        Ok(meta) if meta.len() > MAX_SETTINGS_BYTES => {
            Err(format!("{} is larger than a settings file", path.display()))
        }
        Ok(_) => std::fs::read_to_string(path)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(_) => Ok(None),
    }
}

/// Adds the hook entry to the settings file at `path`, after `confirm`.
///
/// READ OUTSIDE THE SESSION READERS' GATE, ON PURPOSE. `sessions::secrets`
/// refuses `.claude/settings*` because the session READERS never need it and
/// it can carry an API key that would otherwise end up summarised into a
/// prompt. Here the user named the file on the command line, its text goes to
/// no model and is written back to the same place; only the entry being added
/// is printed, never the rest of the file.
fn install_into(
    path: &Path,
    agent: AgentKind,
    command: &str,
    confirm: &dyn Confirm,
) -> Result<Installed, String> {
    let existing = read_settings(path)?;
    let (text, changed) = writeback::merge_hook(existing.as_deref(), agent, command)
        .map_err(|why| format!("{} was not changed: {why}", path.display()))?;
    if !changed {
        return Ok(Installed::Already);
    }
    let group = writeback::hook_group(agent, command);
    let mut lines = vec![
        format!("file: {}", path.display()),
        format!("adds to hooks.{}:", writeback::hook_event(agent)),
    ];
    lines.extend(
        serde_json::to_string_pretty(&group)
            .unwrap_or_default()
            .lines()
            .map(|l| format!("  {l}")),
    );
    if existing.is_some() {
        lines.push(
            "the rest of the file is kept; its keys are written back in sorted order, and a \
             copy of the file as it is now is saved beside it"
                .into(),
        );
    }
    let q = Question {
        heading: format!(
            "Add Tacet's {} hook to this settings file?",
            writeback::hook_event(agent)
        ),
        lines,
    };
    if !confirm.confirm(&q) {
        return Err("not written: declined".into());
    }
    // THE TEXT WAS MERGED BEFORE THE QUESTION, so it is written only if the
    // file is still what it was merged from. Claude Code rewrites its own
    // settings.json (/config, /model) and a person can take as long as they
    // like to answer: writing the stale merge would silently drop whatever
    // landed meanwhile, and "the rest of the file is kept" would be false.
    // Measured in review: a write during the question (`"model":"opus"`) was
    // gone from the file afterwards, kept only in the backup. The same check
    // `antigravity::update_impl` makes on task.md. What is left is the gap
    // between this read and the rename below, microseconds against the
    // seconds a person takes to answer.
    if read_settings(path)? != existing {
        return Err(format!(
            "{} changed while the question was on screen; nothing written",
            path.display()
        ));
    }
    let backup = if existing.is_some() {
        let b = path.with_file_name(format!(
            "{}.tacet-backup-{}",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            now_epoch()
        ));
        // `copy` keeps the permission bits: a settings file holding a key
        // stays as private as its owner made it.
        std::fs::copy(path, &b).map_err(|e| format!("{}: {e}", b.display()))?;
        Some(b)
    } else {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        None
    };
    let temp = path.with_file_name(format!(
        ".{}.tacet.{}.tmp",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    // OPENED 0600 BEFORE THE BYTES LAND, then given the original's mode.
    // `fs::write` creates at 0666 & ~umask (0644 here), and the merged text
    // carries the WHOLE settings file, API key included when it holds one:
    // until the chmod it was readable by every user on the machine. A file
    // that did not exist stays 0600.
    tacet_kernel::fs::write_private(&temp, text.as_bytes()).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: {e}", temp.display())
    })?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&temp, meta.permissions());
    }
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: {e}", path.display())
    })?;
    Ok(Installed::Written { backup })
}

fn show(agent: &str, id: &str, json: bool) -> Result<(), String> {
    let agent = agent_of(agent)?;
    let record = handoff::read(&handoff_dir()?, agent, id).map_err(|e| e.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&record.to_json()).unwrap_or_default()
        );
    } else {
        print!("{}", record.render_markdown());
    }
    Ok(())
}

/// `ID=REST` → (id, rest), the id checked like a record's own.
fn split_pair(text: &str, what: &str) -> Result<(String, String), String> {
    let (id, rest) = text
        .split_once('=')
        .ok_or_else(|| format!("{what} {text:?} is not ID=…"))?;
    let id = id.trim();
    if id.is_empty() {
        return Err(format!("{what} {text:?} has no id"));
    }
    Ok((id.to_string(), rest.to_string()))
}

/// Applies `--summary`, `--next`, `--task` and `--note` to a record.
fn apply_update(
    record: &mut HandoffRecord,
    by: &str,
    summary: Option<String>,
    next: Option<String>,
    tasks: &[String],
    notes: &[String],
    now: &str,
) -> Result<(), String> {
    if by.trim().is_empty() {
        return Err("--by names who is reporting".into());
    }
    record.by = by.trim().to_string();
    record.updated_at = now.to_string();
    if let Some(s) = summary {
        record.summary = s;
    }
    if let Some(n) = next {
        record.next = n;
    }
    for t in tasks {
        let (id, rest) = split_pair(t, "--task")?;
        let (status, title) = match rest.split_once('=') {
            Some((s, title)) => (s, Some(title.trim().to_string())),
            None => (rest.as_str(), None),
        };
        let status = Status::parse(status).ok_or_else(|| {
            format!(
                "--task {t:?}: {status:?} is not one of pending, in_progress, completed, \
                 blocked, cancelled"
            )
        })?;
        match record.tasks.iter_mut().find(|x| x.id == id) {
            Some(task) => {
                task.status = status;
                if let Some(title) = title.filter(|t| !t.is_empty()) {
                    task.title = title;
                }
                task.updated_at = Some(now.to_string());
            }
            None => {
                let title = title.filter(|t| !t.is_empty()).ok_or_else(|| {
                    format!("--task {t:?}: task {id} is new and needs a title (ID=STATUS=TITLE)")
                })?;
                record.tasks.push(HandoffTask {
                    id,
                    title,
                    status,
                    note: String::new(),
                    updated_at: Some(now.to_string()),
                });
            }
        }
    }
    for n in notes {
        let (id, text) = split_pair(n, "--note")?;
        let task = record
            .tasks
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| format!("--note {n:?}: there is no task {id}"))?;
        task.note = text;
        task.updated_at = Some(now.to_string());
    }
    Ok(())
}

fn update(
    agent: &str,
    id: &str,
    by: &str,
    summary: Option<String>,
    next: Option<String>,
    tasks: &[String],
    notes: &[String],
) -> Result<(), String> {
    let agent = agent_of(agent)?;
    validate_id(id).map_err(|e| e.to_string())?;
    let dir = handoff_dir()?;
    let json = dir.join(format!(
        "{}.json",
        handoff::file_stem(agent, id).map_err(|e| e.to_string())?
    ));
    // An existing record is read through its validation, never overwritten
    // blind: a record that does not validate may be the only copy of what
    // another agent reported, and is left for a person to look at.
    let mut record = if std::fs::symlink_metadata(&json).is_ok() {
        handoff::read(&dir, agent, id).map_err(|e| e.to_string())?
    } else {
        HandoffRecord::new(agent, id)
    };
    apply_update(
        &mut record,
        by,
        summary,
        next,
        tasks,
        notes,
        &iso_utc(now_epoch()),
    )?;
    let path = handoff::write(&dir, &record).map_err(|e| e.to_string())?;
    println!("{}", path.display());
    Ok(())
}

/// The note for session `id`, from its handoff record. No record, no note:
/// there is nothing of Tacet's to report.
fn note_for(agent: AgentKind, id: &str) -> Result<String, String> {
    let record = handoff::read(&handoff_dir()?, agent, id).map_err(|e| {
        format!("{e} — there is no readable handoff record to report (tacet handoff show)")
    })?;
    Ok(writeback::note_text(&record))
}

/// The roots to try: the one named, else the ones `chat --from` and the
/// session tools read — the `agent-sessions` addon's folders when it records
/// any, each agent's own defaults when it does not.
///
/// THE SAME RESOLUTION AS THE READERS (review of 10 Oct 2026, RG-22). This
/// used each agent's defaults only, so with the addon pointed at
/// `<tmp>/data/claude` and no `~/.claude`, `chat --from` continued a session
/// and started its record, the hook answered for it, and `handoff note` then
/// said "no Claude Code session … under the known folders" — for the session
/// the other two had just found.
fn roots(agent: AgentKind, root: Option<&Path>) -> Vec<PathBuf> {
    if let Some(r) = root {
        return vec![r.to_path_buf()];
    }
    SessionRoots::Registry.for_agent(agent)
}

/// The folders, for an error that says where it looked and how to look
/// elsewhere.
fn looked_in(roots: &[PathBuf]) -> String {
    let shown: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
    format!(
        "looked in: {}; pass --root <folder> to name another",
        if shown.is_empty() {
            "(no folder)".to_string()
        } else {
            shown.join(", ")
        }
    )
}

fn shown(e: WriteError) -> String {
    e.to_string()
}

fn note(
    agent: &str,
    id: &str,
    fork: bool,
    program: Option<&Path>,
    append_rollout: bool,
    root: Option<&Path>,
) -> Result<(), String> {
    let agent = agent_of(agent)?;
    validate_id(id).map_err(|e| e.to_string())?;
    match agent {
        AgentKind::ClaudeCode => {
            if program.is_some() || append_rollout {
                return Err("--program and --append-rollout are for Codex".into());
            }
            let text = note_for(agent, id)?;
            let source = ClaudeSource::new();
            let tried = roots(agent, root);
            let root = tried
                .iter()
                .find(|r| source.locate(r, id).is_ok())
                .cloned()
                .ok_or_else(|| {
                    format!("no Claude Code session {id} found ({})", looked_in(&tried))
                })?;
            if fork {
                let f = claude_write::fork_with_note(&source, &root, id, &text, &TerminalConfirm)
                    .map_err(shown)?;
                println!("Forked into session {} ({}).", f.session, f.path.display());
                println!("The original is unchanged (sha256 {}).", f.original_sha256);
            } else {
                let backups = handoff_dir()?.join("backups");
                let a = claude_write::append_note(
                    &source,
                    &root,
                    id,
                    &text,
                    Some(&backups),
                    &TerminalConfirm,
                )
                .map_err(shown)?;
                println!("Appended to {} (new leaf {}).", a.path.display(), a.leaf);
                if let Some(b) = a.backup {
                    println!("The transcript as it was is at {}.", b.display());
                }
            }
            Ok(())
        }
        AgentKind::Codex => {
            if fork {
                return Err("--fork is for Claude Code".into());
            }
            let text = note_for(agent, id)?;
            // The Codex home the thread is in — not merely the first folder:
            // the registry's list is every agent's, and its first entry may
            // be `~/.claude`. Its writer lock is checked there and the
            // app-server is started on it (`inject_via_app_server`).
            let tried = roots(agent, root);
            let home = tried
                .iter()
                .find(|r| {
                    !matches!(
                        CodexSource.read_rollout(r, id),
                        Err(SessionError::NotFound { .. })
                    )
                })
                .or_else(|| tried.first())
                .cloned();
            match (program, append_rollout) {
                (Some(_), true) => Err("give --program or --append-rollout, not both".into()),
                (Some(program), false) => {
                    codex_write::inject_via_app_server(
                        program,
                        home.as_deref(),
                        id,
                        &text,
                        &TerminalConfirm,
                        codex_write::APP_SERVER_TIMEOUT,
                    )
                    .map_err(shown)?;
                    println!("The note was added to thread {id}.");
                    Ok(())
                }
                (None, true) => {
                    let home = home.ok_or("the Codex folder could not be resolved (--root)")?;
                    let a = codex_write::append_note(&home, id, &text, &TerminalConfirm)
                        .map_err(shown)?;
                    println!("Appended to {}.", a.path.display());
                    Ok(())
                }
                (None, false) => Err(
                    "the note goes to Codex through its app-server (--program <path to its binary>) or, \
                     as a fallback, --append-rollout; the hook (tacet handoff install) needs neither"
                        .into(),
                ),
            }
        }
        AgentKind::Antigravity => Err(
            "a conversation of Antigravity takes Tacet's report through its PreInvocation hook \
             (tacet handoff install); task.md boxes: tacet handoff tasks"
                .into(),
        ),
    }
}

fn tasks(
    agent: &str,
    id: &str,
    set: &[String],
    experimental: bool,
    root: Option<&Path>,
) -> Result<(), String> {
    let agent = agent_of(agent)?;
    if agent != AgentKind::Antigravity {
        return Err("task.md boxes exist only in Antigravity conversations".into());
    }
    validate_id(id).map_err(|e| e.to_string())?;
    // Printed whatever happens next: the person typing this should know the
    // terms before the refusals below explain the rest.
    eprintln!("  {}", ag_write::TERMS_WARNING);
    let mut updates = Vec::new();
    for s in set {
        let (title, status) = s
            .rsplit_once('=')
            .ok_or_else(|| format!("--set {s:?} is not TITLE=STATUS"))?;
        let status = Status::parse(status)
            .ok_or_else(|| format!("--set {s:?}: {status:?} is not a status"))?;
        updates.push(ag_write::TaskUpdate {
            title: title.trim().to_string(),
            status,
        });
    }
    let tried = roots(agent, root);
    let root = tried
        .iter()
        .find(|r| r.join("brain").join(id).is_dir())
        .cloned()
        .ok_or_else(|| {
            format!(
                "no Antigravity conversation {id} found ({})",
                looked_in(&tried)
            )
        })?;
    let done = ag_write::update_task_md(
        &root,
        id,
        &updates,
        experimental,
        &writeback::process_names,
        &TerminalConfirm,
    )
    .map_err(shown)?;
    println!(
        "{} checkbox(es) changed in {}.",
        done.changed,
        done.path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Scripted(bool, Cell<usize>);

    impl Confirm for Scripted {
        fn confirm(&self, _: &Question) -> bool {
            self.1.set(self.1.get() + 1);
            self.0
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "tacet-handoff-cmd-{tag}-{}-{}",
            std::process::id(),
            writeback::new_uuid()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// RG-21: the settings file is written only after a yes, keeps what was
    /// there, gets a backup, and a second install changes nothing.
    #[test]
    fn install_writes_only_with_confirmation_and_only_once() {
        let dir = scratch("install");
        let settings = dir.join("settings.json");
        std::fs::write(&settings, r#"{"theme":"dark"}"#).unwrap();
        let cmd = writeback::hook_command("/opt/tacet", AgentKind::ClaudeCode);
        let no = Scripted(false, Cell::new(0));
        assert!(install_into(&settings, AgentKind::ClaudeCode, &cmd, &no).is_err());
        assert_eq!(no.1.get(), 1);
        assert_eq!(
            std::fs::read_to_string(&settings).unwrap(),
            r#"{"theme":"dark"}"#
        );
        let yes = Scripted(true, Cell::new(0));
        let done = install_into(&settings, AgentKind::ClaudeCode, &cmd, &yes).unwrap();
        let Installed::Written {
            backup: Some(backup),
        } = done
        else {
            panic!("{done:?}")
        };
        assert_eq!(
            std::fs::read_to_string(backup).unwrap(),
            r#"{"theme":"dark"}"#
        );
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(
            v["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            cmd.as_str()
        );
        assert_eq!(
            install_into(&settings, AgentKind::ClaudeCode, &cmd, &yes).unwrap(),
            Installed::Already
        );
        assert_eq!(yes.1.get(), 1, "asked again about a change it did not make");
        // A file that does not exist yet is created, no backup needed.
        let fresh = dir.join("nested/hooks.json");
        assert_eq!(
            install_into(&fresh, AgentKind::Codex, &cmd, &yes).unwrap(),
            Installed::Written { backup: None }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Answers yes, after rewriting the file the way Claude Code's /model
    /// does while the question is on screen.
    struct EditsWhileAsking(PathBuf, &'static str);

    impl Confirm for EditsWhileAsking {
        fn confirm(&self, _: &Question) -> bool {
            std::fs::write(&self.0, self.1).unwrap();
            true
        }
    }

    /// RG-21 review: the merge was computed before the question, so a write
    /// made while it was on screen must stop the install rather than be
    /// overwritten by the stale merge — for an existing file and for one that
    /// appeared meanwhile.
    #[test]
    fn install_refuses_a_settings_file_that_changed_while_it_asked() {
        let dir = scratch("install-race");
        let settings = dir.join("settings.json");
        std::fs::write(&settings, r#"{"theme":"dark"}"#).unwrap();
        let cmd = writeback::hook_command("/opt/tacet", AgentKind::ClaudeCode);
        let edited = r#"{"theme":"dark","model":"opus"}"#;
        let asker = EditsWhileAsking(settings.clone(), edited);
        let err = install_into(&settings, AgentKind::ClaudeCode, &cmd, &asker).unwrap_err();
        assert!(err.contains("changed while the question"), "{err}");
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), edited);
        let fresh = dir.join("appeared.json");
        let asker = EditsWhileAsking(fresh.clone(), r#"{"model":"opus"}"#);
        let err = install_into(&fresh, AgentKind::ClaudeCode, &cmd, &asker).unwrap_err();
        assert!(err.contains("changed while the question"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&fresh).unwrap(),
            r#"{"model":"opus"}"#
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RG-21 review: the temp file holds the whole settings file, so it is
    /// created 0600, not at the umask's 0644; a new settings file keeps
    /// that, an existing one keeps its owner's mode.
    #[cfg(unix)]
    #[test]
    fn install_never_writes_the_settings_text_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = scratch("install-mode");
        let cmd = writeback::hook_command("/opt/tacet", AgentKind::ClaudeCode);
        let yes = Scripted(true, Cell::new(0));
        let fresh = dir.join("settings.json");
        install_into(&fresh, AgentKind::ClaudeCode, &cmd, &yes).unwrap();
        assert_eq!(mode(&fresh), 0o600, "a new settings file was not private");
        let owned = dir.join("owned.json");
        std::fs::write(&owned, r#"{"theme":"dark"}"#).unwrap();
        std::fs::set_permissions(&owned, std::fs::Permissions::from_mode(0o640)).unwrap();
        install_into(&owned, AgentKind::ClaudeCode, &cmd, &yes).unwrap();
        assert_eq!(mode(&owned), 0o640, "the owner's mode was not kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RG-27: what another agent runs to report back lands in the record, a
    /// new task needs a title, and a status outside the five is refused.
    #[test]
    fn update_applies_tasks_notes_and_refuses_what_the_protocol_does_not_say() {
        let mut r = HandoffRecord::new(AgentKind::Codex, "s-1");
        r.tasks.push(HandoffTask {
            id: "1".into(),
            title: "split the parser".into(),
            status: Status::InProgress,
            note: String::new(),
            updated_at: None,
        });
        let now = "2026-10-10T12:00:00Z";
        apply_update(
            &mut r,
            AgentKind::Codex.slug(),
            Some("done half".into()),
            None,
            &["1=completed".into(), "2=blocked=write the docs".into()],
            &["2=waits on review".into()],
            now,
        )
        .unwrap();
        assert_eq!(r.by, AgentKind::Codex.slug());
        assert_eq!(r.updated_at, now);
        assert_eq!(r.summary, "done half");
        assert_eq!(r.tasks[0].status, Status::Completed);
        assert_eq!(r.tasks[1].title, "write the docs");
        assert_eq!(r.tasks[1].status, Status::Blocked);
        assert_eq!(r.tasks[1].note, "waits on review");
        for (task, why) in [
            ("3=pending", "needs a title"),
            ("1=done", "is not one of"),
            ("nothing", "is not ID="),
        ] {
            let e = apply_update(
                &mut r,
                AgentKind::Codex.slug(),
                None,
                None,
                &[task.into()],
                &[],
                now,
            )
            .unwrap_err();
            assert!(e.contains(why), "{task}: {e}");
        }
        assert!(apply_update(&mut r, " ", None, None, &[], &[], now).is_err());
    }

    /// The protocol document names commands; each must exist (CLAUDE.md: a
    /// command the page tells the reader to type is a claim to re-derive).
    #[test]
    fn the_handoff_document_names_only_real_subcommands() {
        use clap::CommandFactory;
        let doc = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/agent-handoff.md"),
        )
        .unwrap();
        let shell = crate::cli::Shell::command();
        let handoff = shell
            .find_subcommand("handoff")
            .expect("the handoff subcommand exists");
        let mut named = Vec::new();
        for (i, _) in doc.match_indices("tacet handoff ") {
            let word: String = doc[i + "tacet handoff ".len()..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase())
                .collect();
            if !word.is_empty() {
                named.push(word);
            }
        }
        assert!(
            named.len() >= 5,
            "the document names too few commands: {named:?}"
        );
        for word in &named {
            assert!(
                handoff.find_subcommand(word).is_some(),
                "docs/agent-handoff.md tells the reader to type `tacet handoff {word}`, which \
                 does not exist"
            );
        }
        // And every flag a `tacet handoff` line shows exists on some handoff
        // job. Other lines may quote the agents' own flags (`--resume`).
        let flags: std::collections::BTreeSet<String> = handoff
            .get_subcommands()
            .flat_map(|s| {
                s.get_arguments()
                    .filter_map(|a| a.get_long().map(String::from))
            })
            .collect();
        let mut shown = 0;
        for line in doc.lines().filter(|l| l.contains("tacet handoff ")) {
            for (i, _) in line.match_indices(" --") {
                let flag: String = line[i + 3..]
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                    .collect();
                if !flag.is_empty() {
                    shown += 1;
                    assert!(
                        flags.contains(&flag),
                        "the document shows --{flag}; no such flag"
                    );
                }
            }
        }
        assert!(
            shown >= 3,
            "the document shows too few flags to check: {shown}"
        );
    }

    /// RG-04: a reader that panics under `tacet handoff note` / `tasks` is a
    /// failed command with a sentence, not an unwind out of `main`. `run` is
    /// `run_with(job, dispatch)`, so this is the production frame with only
    /// the work swapped for one that panics the way a reader would.
    #[test]
    fn a_reader_that_panics_under_handoff_is_a_failure_not_a_crash() {
        let job = HandoffJob::Show {
            agent: AgentKind::ClaudeCode.slug().into(),
            id: "11111111-0000-4000-8000-111111111111".into(),
            json: false,
        };
        let code = run_with(job, |_, _| {
            panic!("the chain rebuild met a line no version of the reader has seen")
        });
        assert_eq!(code, ExitCode::FAILURE);
    }

    /// RG-04: the hook answers for another agent and exits 0 whatever
    /// happens; a panic while building its answer is silence.
    #[test]
    fn a_hook_whose_answer_panics_stays_silent() {
        assert_eq!(
            hook_answer(|| -> Option<String> { panic!("the agent's hook input broke the parser") }),
            None
        );
        assert_eq!(
            hook_answer(|| Some("said".to_string())),
            Some("said".to_string())
        );
    }
}
