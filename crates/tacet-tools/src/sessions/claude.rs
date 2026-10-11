//! Claude Code sessions, read from `$CLAUDE_CONFIG_DIR` or `~/.claude`.
//!
//! WHAT IS ON DISK (measured on 2.1.285, 9 Oct 2026, across the 30 newest
//! transcripts on one Mac; docs/adr/0001-agent-sessions.md):
//!
//! * `projects/<enc>/<sessionId>.jsonl` — one transcript per session, `<enc>`
//!   the launch directory with every character outside `[A-Za-z0-9]` made `-`.
//! * `sessions/<pid>.json` — one per RUNNING process, `{pid, sessionId, cwd,
//!   status, …}`. A `<pid>.key` sits beside it and is never opened.
//! * `tasks/<listId>/<n>.json` — the task store `TaskCreate`/`TaskUpdate` write
//!   to (RG-10). NOT MEASURED: the machine the brief was measured on had no
//!   such directory, so its shape here is the task tool's own fields
//!   (`id`, `subject`, `status`, `blockedBy`) and the fixtures only.
//!
//! WHY THE CHAIN AND NOT THE FILE ORDER (RG-09). The transcript looks like a
//! log and is not one: timestamps went backwards 1,900 times across the 30
//! files, one file re-appended ~2,800 uuids it already held, a rewind leaves
//! the abandoned branch in place, and a compaction starts a new chain whose
//! first entry has no parent. What Claude Code itself loads on `--resume` is
//! the chain from the newest leaf back through `parentUuid` — so that is what
//! is rebuilt here, and every one of those shapes has a fixture under
//! `tests/fixtures/sessions/claude/`.
//!
//! EVERY FILE GOES THROUGH `secrets` (the credential denylist), and the version
//! of the leaf is put through `version::check_read`; see `read_report`.
//!
//! Claude Code's own documentation calls the format "internal … and changes
//! between versions". So a line this reader does not understand is SKIPPED and
//! counted, never fatal: one odd line must not hide a day's work.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::handoff::Status;
use super::model::{
    AgentKind, AgentSession, Entry, EntryKind, PlanItem, Role, SessionError, SessionSource,
    SessionSummary, newest_first, parse_iso_utc, validate_id,
};
use super::secrets::{self, Refusal};
use super::version;

/// Claude Code's own override for its configuration directory.
pub const CONFIG_DIR_VAR: &str = "CLAUDE_CONFIG_DIR";

/// How much of a transcript a LISTING reads, from the end. Claude Code
/// re-appends its title and last-prompt lines as a session goes on (MEASURED:
/// 3,157 `last-prompt` lines in one file), so the tail holds what a list row
/// needs; the files run to tens of megabytes (340 MB across the 30 newest), and
/// reading each one whole to draw a list is seconds per row.
pub const LIST_TAIL_BYTES: u64 = 64 * 1024;

/// The largest transcript `read` accepts. Not a measurement of the largest
/// file seen — a guard against a runaway one. The file is streamed, so what is
/// held in memory is the parsed text, not the file plus the text.
pub const MAX_TRANSCRIPT_BYTES: u64 = 512 * 1024 * 1024;

/// `sessions/<pid>.json` and `tasks/<list>/<n>.json` are a few hundred bytes;
/// anything far larger is not one.
const MAX_SMALL_FILE_BYTES: u64 = 256 * 1024;

const MAX_TITLE_CHARS: usize = 120;

/// The length past which Claude Code truncates an encoded project directory
/// name and appends a hash (rnd brief, "names >200 chars are truncated +
/// hashed").
const MAX_ENCODED_LEN: usize = 200;

/// Reads Claude Code sessions.
///
/// THE LIVENESS PROBE IS A FIELD so a test can say which pids are alive
/// without starting or killing processes; `ClaudeSource::new()` uses
/// `pid_alive`.
#[derive(Clone, Copy)]
pub struct ClaudeSource {
    alive: fn(u32) -> bool,
}

impl Default for ClaudeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ClaudeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeSource").finish_non_exhaustive()
    }
}

/// A session read in full, with what the reader had to step over to read it.
///
/// `AgentSession` has no room for warnings, and a reader that drops them
/// leaves the user believing a cut-off history is the whole one. The tool that
/// shows a session (RG-17) calls `read_report` and shows `warnings`.
#[derive(Debug, Clone)]
pub struct ClaudeRead {
    pub session: AgentSession,
    pub path: PathBuf,
    /// The uuid the chain was rebuilt from — the one a write-back (RG-22)
    /// would chain its note to.
    pub leaf: Option<String>,
    /// Non-empty lines that were not a JSON object, or a chain entry without a
    /// uuid.
    pub skipped_lines: usize,
    /// The context starts at a `compact_boundary`: what came before it is not
    /// in `session.entries`.
    pub compacted: bool,
    /// The version gate's warning, a chain cut short, skipped lines.
    pub warnings: Vec<String>,
}

impl ClaudeSource {
    pub fn new() -> Self {
        Self { alive: pid_alive }
    }

    /// A source whose "is this process running?" is `alive`.
    pub fn with_probe(alive: fn(u32) -> bool) -> Self {
        Self { alive }
    }

    /// The sessions whose project directory is `cwd`'s — what "my session in
    /// this repository" means. The directory name is matched first and the
    /// recorded `cwd` second, because two launch directories can encode to the
    /// same name (`/a b` and `/a-b` are both `-a-b`).
    ///
    /// The recorded `cwd` is the LEAF's, and a session that `cd`'d into a
    /// subdirectory records that subdirectory; so a recorded cwd at or BELOW
    /// `cwd` is kept. `Path::starts_with` compares components, so `/a-b/x`
    /// is still not under `/a b`.
    pub fn discover_for_cwd(&self, root: &Path, cwd: &Path) -> Vec<SessionSummary> {
        let text = cwd.to_string_lossy();
        self.discover_where(root, |dir| project_dir_matches(dir, &text))
            .into_iter()
            .filter(|s| s.cwd.as_deref().is_none_or(|c| c.starts_with(cwd)))
            .collect()
    }

    fn discover_where(&self, root: &Path, keep_dir: impl Fn(&str) -> bool) -> Vec<SessionSummary> {
        let live = self.live_sessions(root);
        let mut out = Vec::new();
        for dir in project_dirs(root) {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !keep_dir(&name) {
                continue;
            }
            for (id, path) in transcripts_in(&dir) {
                // Skipped, not fatal: one refused or unreadable file must not
                // hide the other sessions (SessionSource::discover).
                if let Ok(mut summary) = summarize(&path, &id) {
                    summary.live = live.contains_key(&id);
                    out.push(summary);
                }
            }
        }
        newest_first(&mut out);
        out
    }

    /// `sessionId -> pid` for every session a running process has open.
    ///
    /// A pid file whose process is gone is a session that crashed or was
    /// killed; it is NOT live. A pid that was reused by an unrelated process
    /// reads as live — the safe direction, since live only ever stops a write.
    pub fn live_sessions(&self, root: &Path) -> HashMap<String, u32> {
        let mut out = HashMap::new();
        for path in live_files(&root.join("sessions")) {
            let Ok(text) = secrets::read_text(&path, MAX_SMALL_FILE_BYTES) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let Some(id) = str_field(&v, "sessionId").filter(|id| validate_id(id).is_ok()) else {
                continue;
            };
            let from_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u32>().ok());
            let Some(pid) = v
                .get("pid")
                .and_then(Value::as_u64)
                .and_then(|p| u32::try_from(p).ok())
                .or(from_name)
            else {
                continue;
            };
            if (self.alive)(pid) {
                out.insert(id, pid);
            }
        }
        out
    }

    /// Where session `id` lives. A session id is unique, but a copied project
    /// directory holds a second copy of the same file; the newer one is the one
    /// the user last worked in.
    pub fn locate(&self, root: &Path, id: &str) -> Result<PathBuf, SessionError> {
        validate_id(id)?;
        let name = format!("{id}.jsonl");
        project_dirs(root)
            .into_iter()
            .map(|dir| dir.join(&name))
            .filter(|p| std::fs::symlink_metadata(p).is_ok())
            .max_by_key(|p| modified(p))
            .ok_or_else(|| SessionError::NotFound {
                agent: AgentKind::ClaudeCode,
                id: id.to_string(),
            })
    }

    /// `read`, plus what the reader stepped over and what the version gate
    /// said about it.
    pub fn read_report(&self, root: &Path, id: &str) -> Result<ClaudeRead, SessionError> {
        let path = self.locate(root, id)?;
        let mut scan = Scan::default();
        stream_lines(&path, &mut scan)?;
        let built = rebuild(&scan.nodes);
        let mut warnings = Vec::new();
        if let Some(w) =
            version::check_read(AgentKind::ClaudeCode, None, built.version.as_deref()).warning()
        {
            warnings.push(w.to_string());
        }
        warnings.extend(built.warnings);
        if scan.skipped > 0 {
            warnings.push(format!(
                "{} line(s) of the transcript were not understood and were skipped",
                scan.skipped
            ));
        }
        let plan = match task_store_plan(root, id) {
            Some(plan) => plan,
            None => replay_plan(&scan.nodes),
        };
        let title = scan
            .custom_title
            .clone()
            .or_else(|| scan.ai_title.clone())
            .or_else(|| side_title(&path, id))
            // `last-prompt` before the first prompt, as in `summarize`: the
            // listing cannot see the first prompt of a real file, and a
            // session must have one name.
            .or_else(|| scan.last_prompt.clone())
            .or(built.first_prompt)
            .map(|t| clip_title(&t))
            .unwrap_or_default();
        let session = AgentSession {
            agent: AgentKind::ClaudeCode,
            id: id.to_string(),
            title,
            cwd: built.cwd.map(PathBuf::from),
            started: scan.nodes.values().filter_map(|n| n.ts).min(),
            updated: scan.nodes.values().filter_map(|n| n.ts).max(),
            live: self.live_sessions(root).contains_key(id),
            version: built.version,
            entries: built.entries,
            plan,
        };
        Ok(ClaudeRead {
            session,
            path,
            leaf: built.leaf,
            skipped_lines: scan.skipped,
            compacted: built.compacted,
            warnings,
        })
    }
}

impl SessionSource for ClaudeSource {
    fn agent(&self) -> AgentKind {
        AgentKind::ClaudeCode
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        roots_from(tacet_kernel::env::env_var)
    }

    fn discover(&self, root: &Path) -> Vec<SessionSummary> {
        self.discover_where(root, |_| true)
    }

    fn read(&self, root: &Path, id: &str) -> Result<AgentSession, SessionError> {
        self.read_report(root, id).map(|r| r.session)
    }
}

/// `$CLAUDE_CONFIG_DIR`, else `$HOME/.claude` (`%USERPROFILE%` on Windows).
///
/// A RELATIVE value is ignored, the way `tacet_kernel::env::absolute_env`
/// ignores one for Tacet's own directory: relative to whose working directory?
/// Not this process's — the variable was set for Claude Code's.
fn roots_from(get: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let absolute = |name: &str| {
        get(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    if let Some(dir) = absolute(CONFIG_DIR_VAR) {
        return vec![dir];
    }
    absolute("HOME")
        .or_else(|| absolute("USERPROFILE"))
        .map(|home| vec![home.join(".claude")])
        .unwrap_or_default()
}

/// The directory name Claude Code files a launch directory's sessions under.
///
/// Every UTF-16 unit outside `[A-Za-z0-9]` becomes one `-`. UTF-16, not
/// chars: the encoder is a JavaScript `replace(/[^a-zA-Z0-9]/g, "-")` without
/// the `u` flag, which sees an astral character (an emoji) as two units and
/// writes two dashes for it (INFERRED from the JavaScript semantics, not
/// measured on a real directory).
pub fn project_dir_name(cwd: &str) -> String {
    let mut out = String::with_capacity(cwd.len());
    for c in cwd.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.extend(std::iter::repeat_n('-', c.len_utf16()));
        }
    }
    out
}

/// Is `dir_name` the project directory of `cwd`?
///
/// Past 200 characters Claude Code keeps the first 200 and appends `-<hash>`,
/// and the hash depends on the runtime it was built with (Bun's or a
/// JavaScript fallback), so it is not recomputed here: the prefix and the
/// shape of the suffix are matched, and the caller confirms against the
/// `cwd` recorded inside the transcript.
pub fn project_dir_matches(dir_name: &str, cwd: &str) -> bool {
    let encoded = project_dir_name(cwd);
    if encoded.len() <= MAX_ENCODED_LEN {
        return dir_name == encoded;
    }
    dir_name
        .strip_prefix(&encoded[..MAX_ENCODED_LEN])
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|hash| !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Is process `pid` running?
///
/// std has no `kill(pid, 0)` and this crate takes no libc, so:
///
/// * Linux/Android: `/proc/<pid>` exists. A stat, no spawn. A thread id also
///   has a `/proc/<tid>`, so a dead pid reused as a thread id reads as alive —
///   the safe direction.
/// * other Unix (macOS): `/bin/ps -p <pid>`, which exits non-zero when there is
///   no such process. MEASURED on macOS 26.5.2, 10 Oct 2026: exit 1 for a free
///   pid (99999) and for one past `kern.maxproc`, exit 0 for this shell and
///   for pid 1, which belongs to root. `ps`, not `kill -0`: `kill` fails with
///   EPERM on a process of another user and would call it dead. If `ps`
///   cannot be run at all the answer is "alive" — liveness only ever STOPS a
///   write (RG-31), so not knowing must not permit one.
/// * elsewhere: "alive", for the same reason. NOT MEASURED on Windows.
pub fn pid_alive(pid: u32) -> bool {
    // pid 0 is the kernel's, and `ps -p 0` on macOS lists it; a pid above
    // i32::MAX does not exist and `ps` would read it as negative.
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    probe_pid(pid)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn probe_pid(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
fn probe_pid(pid: u32) -> bool {
    use std::process::{Command, Stdio};
    let pid = pid.to_string();
    // An absolute path: the probe must not run whatever `ps` is first on a
    // PATH the user's shell profile built.
    match Command::new("/bin/ps")
        .args(["-p", pid.as_str(), "-o", "pid="])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => true,
    }
}

#[cfg(not(unix))]
fn probe_pid(_pid: u32) -> bool {
    true
}

/// `<root>/projects/*`, directories only.
fn project_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(root.join("projects")) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

/// `(id, path)` of every top-level `<id>.jsonl` in a project directory.
/// Subagent transcripts live one level down (`<id>/subagents/agent-*.jsonl`)
/// and are not sessions of their own.
fn transcripts_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if validate_id(id).is_ok() {
            out.push((id.to_string(), path.clone()));
        }
    }
    out.sort();
    out
}

/// The pid files under `sessions/`: `<digits>.json` and nothing else. The
/// `<pid>.key` beside each is a credential (the brief: NEVER read it), and is
/// left out HERE as well as refused by the gate, so that widening the gate's
/// extension allowlist one day does not quietly start reading keys.
fn live_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("json")
                && p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    out.sort();
    out
}

fn modified(path: &Path) -> Option<i64> {
    let time = std::fs::metadata(path).ok()?.modified().ok()?;
    let secs = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    i64::try_from(secs).ok()
}

/// A listing row from the last `LIST_TAIL_BYTES` of the file.
///
/// `started` is only filled when the tail IS the whole file: the earliest
/// timestamp of a tail is not when the session started, and a wrong timestamp
/// in a listing is worse than a missing one (model.rs).
///
/// AN UNTITLED SESSION IS NAMED BY ITS `last-prompt`, AND THE READ AGREES.
/// The first prompt was preferred, and the listing could only find it when
/// the tail was the whole file. A `claude -p` session has no `ai-title` and
/// is ~280 KB of attachment lines by its first answer (the 11 Oct 2026
/// captures in the corpus), so every real one was listed by its last prompt
/// and opened under its first, and a small one was renamed when it grew past
/// the tail. Claude Code re-appends `last-prompt` every turn, so the tail
/// always has it; the first prompt is only the fallback for a file without.
fn summarize(path: &Path, id: &str) -> Result<SessionSummary, Refusal> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(u64::MAX);
    let tail = secrets::read_tail(path, LIST_TAIL_BYTES)?;
    let mut scan = Scan::default();
    for line in tail.lines() {
        scan.feed(line);
    }
    let leaf = pick_leaf(&scan.nodes);
    let on_leaf = |get: fn(&Node) -> Option<&String>| {
        leaf.and_then(get)
            .or_else(|| {
                // The leaf may be an attachment without the field; the newest
                // entry that has it says the same thing.
                let mut nodes: Vec<&Node> = scan.nodes.values().collect();
                nodes.sort_by_key(|n| n.first);
                nodes.into_iter().rev().find_map(get)
            })
            .cloned()
    };
    let version = on_leaf(|n| n.version.as_ref());
    let cwd = on_leaf(|n| n.cwd.as_ref());
    let whole = size <= LIST_TAIL_BYTES;
    let first_prompt = if whole {
        leaf.and_then(|l| first_prompt(&walk(&scan.nodes, l).0))
    } else {
        None
    };
    let title = scan
        .custom_title
        .or(scan.ai_title)
        .or_else(|| side_title(path, id))
        .or(scan.last_prompt)
        .or(first_prompt)
        .map(|t| clip_title(&t))
        .unwrap_or_default();
    Ok(SessionSummary {
        agent: AgentKind::ClaudeCode,
        id: id.to_string(),
        title,
        cwd: cwd.map(PathBuf::from),
        started: if whole {
            scan.nodes.values().filter_map(|n| n.ts).min()
        } else {
            None
        },
        updated: scan
            .nodes
            .values()
            .filter_map(|n| n.ts)
            .max()
            .or_else(|| modified(path)),
        live: false,
        version,
        path: path.to_path_buf(),
    })
}

/// `<project>/<id>/custom-title.json`, the per-session title file the brief
/// lists. Its fields were NOT recorded, so the two names the transcript's own
/// title lines use are accepted, and a bare JSON string.
fn side_title(transcript: &Path, id: &str) -> Option<String> {
    let path = transcript.parent()?.join(id).join("custom-title.json");
    if std::fs::symlink_metadata(&path).is_err() {
        return None;
    }
    let text = secrets::read_text(&path, MAX_SMALL_FILE_BYTES).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.as_str()
        .map(str::to_string)
        .or_else(|| title_field(&v, "customTitle"))
        .filter(|t| !t.trim().is_empty())
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// A title line's text: its own field, else `title`. Only `type` was in the
/// measured schema for `ai-title`/`custom-title`; the field names are Claude
/// Code's (`aiTitle`, `customTitle`), and `title` is accepted so a renamed
/// field degrades to a fallback rather than to no title.
fn title_field(v: &Value, key: &str) -> Option<String> {
    str_field(v, key)
        .or_else(|| str_field(v, "title"))
        .filter(|t| !t.trim().is_empty())
}

fn clip_title(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    if line.chars().count() <= MAX_TITLE_CHARS {
        return line.to_string();
    }
    let mut out: String = line.chars().take(MAX_TITLE_CHARS - 1).collect();
    out.push('…');
    out
}

/// Feeds a transcript to `scan` line by line, through the secrets gate.
///
/// STREAMED, NOT `read_text`: a 300 MB transcript held as one `String` and then
/// parsed holds it twice. Invalid UTF-8 is replaced, not fatal (the gate's own
/// rule for `read_text`).
fn stream_lines(path: &Path, scan: &mut Scan) -> Result<(), SessionError> {
    let io = |error| {
        SessionError::Refused(Refusal::Io {
            path: path.to_path_buf(),
            error,
        })
    };
    let handle = secrets::open(path)?;
    let size = handle.metadata().map_err(io)?.len();
    if size > MAX_TRANSCRIPT_BYTES {
        return Err(Refusal::TooLarge {
            path: path.to_path_buf(),
            size,
            cap: MAX_TRANSCRIPT_BYTES,
        }
        .into());
    }
    let mut reader = std::io::BufReader::new(handle.take(MAX_TRANSCRIPT_BYTES));
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).map_err(io)? == 0 {
            break;
        }
        scan.feed(&String::from_utf8_lossy(&buf));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    User,
    Assistant,
    Attachment,
    System,
}

/// One piece of an entry's content, before tool names are known.
#[derive(Debug, Clone)]
enum Part {
    Say {
        role: Role,
        kind: EntryKind,
        text: String,
    },
    Call {
        id: String,
        name: String,
        input: String,
    },
    /// The tool's name is filled from the `tool_use` with this id.
    Result { tool_use_id: String, text: String },
}

/// A `tool_result`, as the plan replay needs it.
#[derive(Debug, Clone)]
struct ResultMeta {
    tool_use_id: String,
    is_error: bool,
    /// The id a `TaskCreate` was given, from `toolUseResult.task.id` or the
    /// result text.
    task_id: Option<String>,
}

/// A `tool_use` of one of the progress tools, input kept for the replay. Other
/// tools' inputs are kept only as text in `parts`.
#[derive(Debug, Clone)]
struct PlanCall {
    id: String,
    name: String,
    input: Value,
}

const PLAN_TOOLS: &[&str] = &["TaskCreate", "TaskUpdate", "TodoWrite"];

/// One chain entry, already reduced to what the reader uses, so the parsed
/// `Value` of a 40 KB tool result is dropped as soon as its text is taken.
#[derive(Debug, Clone)]
struct Node {
    uuid: String,
    parent: Option<String>,
    sidechain: bool,
    kind: NodeKind,
    compact_boundary: bool,
    compact_summary: bool,
    /// System-injected user text (command caveats, reminders): not the user's
    /// words, so never a title.
    meta: bool,
    source_tool_assistant: Option<String>,
    /// An assistant entry's `message.id`. One API response is written as 1–6
    /// entries sharing it (one per content block), so it says where a response
    /// ENDS — which is where its tool results belong.
    message_id: Option<String>,
    ts: Option<i64>,
    version: Option<String>,
    cwd: Option<String>,
    parts: Vec<Part>,
    calls: Vec<PlanCall>,
    results: Vec<ResultMeta>,
    /// The line this uuid was FIRST written on. Content is the last write's
    /// (dedupe, last wins); position is the first's, because a re-appended
    /// uuid did not happen again — replaying it at its new position would put
    /// a `TaskCreate` after the `TaskUpdate` that followed it.
    first: usize,
}

#[derive(Debug, Default)]
struct Scan {
    nodes: HashMap<String, Node>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    last_prompt: Option<String>,
    skipped: usize,
    lines: usize,
}

impl Scan {
    fn feed(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let pos = self.lines;
        self.lines += 1;
        let v = match serde_json::from_str::<Value>(line) {
            Ok(v) if v.is_object() => v,
            _ => {
                self.skipped += 1;
                return;
            }
        };
        let kind = match v.get("type").and_then(Value::as_str).unwrap_or_default() {
            "user" => NodeKind::User,
            "assistant" => NodeKind::Assistant,
            "attachment" => NodeKind::Attachment,
            "system" => NodeKind::System,
            "custom-title" => {
                if let Some(t) = title_field(&v, "customTitle") {
                    self.custom_title = Some(t);
                }
                return;
            }
            "ai-title" => {
                if let Some(t) = title_field(&v, "aiTitle") {
                    self.ai_title = Some(t);
                }
                return;
            }
            "last-prompt" => {
                if let Some(t) = str_field(&v, "lastPrompt").filter(|t| !t.trim().is_empty()) {
                    self.last_prompt = Some(t);
                }
                return;
            }
            // file-history-snapshot, queue-operation, cost-state, progress…:
            // metadata, not context.
            _ => return,
        };
        let Some(mut node) = Node::from_value(&v, kind, pos) else {
            self.skipped += 1;
            return;
        };
        if let Some(old) = self.nodes.get(&node.uuid) {
            node.first = old.first;
        }
        self.nodes.insert(node.uuid.clone(), node);
    }
}

impl Node {
    fn from_value(v: &Value, kind: NodeKind, pos: usize) -> Option<Node> {
        let uuid = str_field(v, "uuid").filter(|u| !u.is_empty())?;
        let flag = |key: &str| v.get(key).and_then(Value::as_bool).unwrap_or(false);
        let mut node = Node {
            uuid,
            parent: str_field(v, "parentUuid").filter(|u| !u.is_empty()),
            sidechain: flag("isSidechain"),
            kind,
            compact_boundary: kind == NodeKind::System
                && v.get("subtype").and_then(Value::as_str) == Some("compact_boundary"),
            compact_summary: flag("isCompactSummary"),
            meta: flag("isMeta"),
            source_tool_assistant: str_field(v, "sourceToolAssistantUUID"),
            message_id: if kind == NodeKind::Assistant {
                v.get("message").and_then(|m| str_field(m, "id"))
            } else {
                None
            },
            ts: v
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_iso_utc),
            version: str_field(v, "version"),
            cwd: str_field(v, "cwd"),
            parts: Vec::new(),
            calls: Vec::new(),
            results: Vec::new(),
            first: pos,
        };
        match kind {
            NodeKind::User => node.read_user(v),
            NodeKind::Assistant => node.read_assistant(v),
            // A boundary's text ("Conversation compacted") says nothing the
            // summary after it does not; other system lines (a local
            // command's output, an API error) are context.
            NodeKind::System if !node.compact_boundary => {
                if let Some(text) = str_field(v, "content").filter(|t| !t.is_empty()) {
                    node.parts.push(Part::Say {
                        role: Role::System,
                        kind: EntryKind::Text,
                        text,
                    });
                }
            }
            // Attachments (hook output, file mentions, reminders) are many
            // subtypes with no measured schema each; reading them would put
            // guesses into the context.
            NodeKind::System | NodeKind::Attachment => {}
        }
        Some(node)
    }

    fn read_user(&mut self, v: &Value) {
        let content = &v["message"]["content"];
        let (role, kind) = if self.compact_summary {
            (Role::System, EntryKind::Summary)
        } else if self.meta {
            (Role::System, EntryKind::Text)
        } else {
            (Role::User, EntryKind::Text)
        };
        if let Some(text) = content.as_str() {
            self.parts.push(Part::Say {
                role,
                kind,
                text: text.to_string(),
            });
            return;
        }
        let Some(items) = content.as_array() else {
            return;
        };
        let results = items.iter().filter(|i| i["type"] == "tool_result").count();
        for item in items {
            match item["type"].as_str().unwrap_or_default() {
                "text" => {
                    if let Some(text) = item["text"].as_str() {
                        self.parts.push(Part::Say {
                            role,
                            kind,
                            text: text.to_string(),
                        });
                    }
                }
                "image" => self.parts.push(Part::Say {
                    role,
                    kind: EntryKind::Text,
                    text: "[image]".into(),
                }),
                "tool_result" => {
                    let Some(id) = item["tool_use_id"].as_str() else {
                        continue;
                    };
                    let text = content_text(&item["content"]);
                    // `toolUseResult` belongs to the ENTRY; with two results
                    // in one entry it cannot be told which it describes.
                    let task_id = if results == 1 {
                        task_id_of(&v["toolUseResult"])
                    } else {
                        None
                    }
                    .or_else(|| task_id_in_text(&text));
                    self.results.push(ResultMeta {
                        tool_use_id: id.to_string(),
                        is_error: item["is_error"].as_bool().unwrap_or(false),
                        task_id,
                    });
                    self.parts.push(Part::Result {
                        tool_use_id: id.to_string(),
                        text,
                    });
                }
                _ => {}
            }
        }
    }

    fn read_assistant(&mut self, v: &Value) {
        let content = &v["message"]["content"];
        if let Some(text) = content.as_str() {
            self.parts.push(Part::Say {
                role: Role::Assistant,
                kind: EntryKind::Text,
                text: text.to_string(),
            });
            return;
        }
        for item in content.as_array().into_iter().flatten() {
            match item["type"].as_str().unwrap_or_default() {
                "text" => {
                    if let Some(text) = item["text"].as_str() {
                        self.parts.push(Part::Say {
                            role: Role::Assistant,
                            kind: EntryKind::Text,
                            text: text.to_string(),
                        });
                    }
                }
                "thinking" => {
                    if let Some(text) = item["thinking"].as_str().filter(|t| !t.is_empty()) {
                        self.parts.push(Part::Say {
                            role: Role::Assistant,
                            kind: EntryKind::Thinking,
                            text: text.to_string(),
                        });
                    }
                }
                "tool_use" => {
                    let (Some(id), Some(name)) = (item["id"].as_str(), item["name"].as_str())
                    else {
                        continue;
                    };
                    if PLAN_TOOLS.contains(&name) {
                        self.calls.push(PlanCall {
                            id: id.to_string(),
                            name: name.to_string(),
                            input: item["input"].clone(),
                        });
                    }
                    self.parts.push(Part::Call {
                        id: id.to_string(),
                        name: name.to_string(),
                        input: item["input"].to_string(),
                    });
                }
                _ => {}
            }
        }
    }
}

/// A `tool_result`'s content: a string, or text blocks (and images).
fn content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| match block["type"].as_str() {
            Some("text") => block["text"].as_str().map(str::to_string),
            Some("image") => Some("[image]".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `toolUseResult: {task: {id, subject}}`, the brief's measured shape of a
/// `TaskCreate` result.
fn task_id_of(result: &Value) -> Option<String> {
    id_text(&result["task"]["id"])
}

/// `Task #4 created …`: the first `#<digits>` of a result's text, for a result
/// whose structured half is missing (an older version, or an entry with two
/// results).
fn task_id_in_text(text: &str) -> Option<String> {
    let at = text.find('#')?;
    let digits: String = text[at + 1..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (!digits.is_empty()).then_some(digits)
}

/// Picks the leaf the context is rebuilt from: the newest chain entry that is
/// not a sidechain's and that no other main-chain entry names as its parent.
///
/// "Newest" is by FIRST appearance in the file, not by timestamp (timestamps
/// were measured going backwards) and not by "last line with a uuid" (a
/// re-appended old uuid would then be the leaf, and the walk from it would
/// drop everything after it). `last-prompt.leafUuid` is not used: a stale one
/// cannot be told from a current one, the file order can.
fn pick_leaf(nodes: &HashMap<String, Node>) -> Option<&Node> {
    let parents: HashSet<&str> = nodes
        .values()
        .filter(|n| !n.sidechain)
        .filter_map(|n| n.parent.as_deref())
        .collect();
    nodes
        .values()
        .filter(|n| !n.sidechain && !parents.contains(n.uuid.as_str()))
        .max_by_key(|n| n.first)
}

/// A uuid from the file, as it may appear in a warning: the file is
/// untrusted, and a 10 KB "uuid" is not one to print.
fn shown(uuid: &str) -> String {
    if validate_id(uuid).is_ok() && uuid.len() <= 64 {
        uuid.to_string()
    } else {
        "<malformed uuid>".to_string()
    }
}

/// The chain from `leaf` back to its root, oldest first.
///
/// `parentUuid` ONLY. `logicalParentUuid` (a compact boundary's pointer to
/// the history it replaced) is never followed: what came before a compaction
/// is not in the context Claude Code resumes with, and in 1 of the 30
/// measured files following it looped. A `parentUuid` loop is cut too, and a
/// parent not in this file (a session continued from another file) ends the
/// chain — both with a warning, because the history shown is then shorter
/// than the history there was.
fn walk<'a>(nodes: &'a HashMap<String, Node>, leaf: &'a Node) -> (Vec<&'a Node>, Vec<String>) {
    let mut path = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    let mut cur = Some(leaf.uuid.as_str());
    while let Some(uuid) = cur {
        let Some(node) = nodes.get(uuid) else {
            warnings.push(format!(
                "the chain continues at {}, which is not in this file; earlier history is not shown",
                shown(uuid)
            ));
            break;
        };
        if !seen.insert(uuid) {
            warnings.push(format!(
                "parentUuid loops back to {}; the chain was cut there",
                shown(uuid)
            ));
            break;
        }
        path.push(node);
        cur = node.parent.as_deref();
    }
    path.reverse();
    (path, warnings)
}

/// The first thing the user typed on the chain, for a session without a title
/// line. A slash command's wrapper (`<command-name>/clear</command-name>`)
/// and a local command's echo are the shell talking, not the user.
fn first_prompt(path: &[&Node]) -> Option<String> {
    path.iter()
        .filter(|n| n.kind == NodeKind::User && !n.meta && !n.compact_summary)
        .flat_map(|n| n.parts.iter())
        .find_map(|p| match p {
            Part::Say {
                role: Role::User,
                kind: EntryKind::Text,
                text,
            } if !text.trim().is_empty()
                && !text.trim_start().starts_with("<command-")
                && !text.trim_start().starts_with("<local-command-") =>
            {
                Some(text.clone())
            }
            _ => None,
        })
}

#[derive(Debug, Default)]
struct Rebuilt {
    entries: Vec<Entry>,
    leaf: Option<String>,
    version: Option<String>,
    cwd: Option<String>,
    compacted: bool,
    first_prompt: Option<String>,
    warnings: Vec<String>,
}

/// The chain with its SIBLING TOOL RESULTS put back.
///
/// Parallel tool calls can answer on side branches: two results share a parent
/// and the chain continues from only one of them, so the parent walk never
/// visits the other. Each result entry names the assistant entry that made the
/// call (`sourceToolAssistantUUID`), and a result whose call is on the chain
/// belongs to the context even though its parent link does not lead there.
///
/// WHERE IT GOES. After the whole API response that made the call — the
/// assistant entries sharing its `message.id` — and after the results already
/// on the chain that answer it. Attaching right after its parent instead put a
/// result between two `tool_use` entries of the same response whenever its
/// parent was the first of them, which no transcript Claude Code itself
/// replays looks like.
fn with_sibling_results<'a>(nodes: &'a HashMap<String, Node>, path: &[&'a Node]) -> Vec<&'a Node> {
    let pos: HashMap<&str, usize> = path
        .iter()
        .enumerate()
        .map(|(i, n)| (n.uuid.as_str(), i))
        .collect();
    let mut answered: HashSet<&str> = path
        .iter()
        .flat_map(|n| n.results.iter().map(|r| r.tool_use_id.as_str()))
        .collect();
    let mut candidates: Vec<&Node> = nodes
        .values()
        .filter(|n| {
            !n.sidechain
                && n.kind == NodeKind::User
                && !n.results.is_empty()
                && !pos.contains_key(n.uuid.as_str())
                && n.source_tool_assistant
                    .as_deref()
                    .is_some_and(|s| pos.contains_key(s))
        })
        .collect();
    // File order, so that of two copies of one answer the first is kept.
    candidates.sort_by_key(|n| n.first);
    let mut after: HashMap<usize, Vec<&Node>> = HashMap::new();
    for node in candidates {
        // A result already on the chain (or attached a line ago) is not shown
        // twice; a re-appended answer under a new uuid would be.
        if node
            .results
            .iter()
            .all(|r| answered.contains(r.tool_use_id.as_str()))
        {
            continue;
        }
        answered.extend(node.results.iter().map(|r| r.tool_use_id.as_str()));
        let source = pos[node.source_tool_assistant.as_deref().unwrap_or_default()];
        let parent = node.parent.as_deref().and_then(|p| pos.get(p).copied());
        let mut at = parent.map_or(source, |p| p.max(source));
        let response = path[source].message_id.as_deref();
        while let Some(next) = path.get(at + 1) {
            let same_response = next.kind == NodeKind::Assistant
                && response.is_some()
                && next.message_id.as_deref() == response;
            let a_result = next.kind == NodeKind::User && !next.results.is_empty();
            if !(same_response || a_result) {
                break;
            }
            at += 1;
        }
        after.entry(at).or_default().push(node);
    }
    let mut ordered = Vec::with_capacity(path.len());
    for (i, node) in path.iter().enumerate() {
        ordered.push(*node);
        if let Some(list) = after.get(&i) {
            ordered.extend(list.iter().copied());
        }
    }
    ordered
}

/// The context Claude Code would resume with, as `Entry`s.
fn rebuild(nodes: &HashMap<String, Node>) -> Rebuilt {
    let Some(leaf) = pick_leaf(nodes) else {
        // Entries, and not one that is nobody's parent: the main chain is a
        // closed loop. An empty session with no word would read as "nothing
        // happened here".
        let mut out = Rebuilt::default();
        if nodes.values().any(|n| !n.sidechain) {
            out.warnings.push(
                "every entry names another as its parent (a loop); no chain could be rebuilt"
                    .into(),
            );
        }
        return out;
    };
    let (path, warnings) = walk(nodes, leaf);
    let ordered = with_sibling_results(nodes, &path);

    // Tool names by call id, from every entry: a result's call can sit on a
    // branch the walk did not take.
    let mut names: HashMap<&str, &str> = HashMap::new();
    for node in nodes.values() {
        for part in &node.parts {
            if let Part::Call { id, name, .. } = part {
                names.insert(id, name);
            }
        }
    }

    let mut entries = Vec::new();
    for node in &ordered {
        for part in &node.parts {
            let entry = match part {
                Part::Say { role, kind, text } => Entry {
                    role: *role,
                    kind: *kind,
                    text: text.clone(),
                    tool: None,
                    ts: node.ts,
                },
                Part::Call { name, input, .. } => Entry {
                    role: Role::Assistant,
                    kind: EntryKind::ToolCall,
                    text: input.clone(),
                    tool: Some(name.clone()),
                    ts: node.ts,
                },
                Part::Result { tool_use_id, text } => Entry {
                    role: Role::Tool,
                    kind: EntryKind::ToolResult,
                    text: text.clone(),
                    tool: names.get(tool_use_id.as_str()).map(|n| n.to_string()),
                    ts: node.ts,
                },
            };
            entries.push(entry);
        }
    }

    // The version and cwd of the LEAF (model.rs: the line a write would chain
    // to), falling back up the chain for a leaf that lacks the field.
    let up = |get: fn(&Node) -> Option<&String>| path.iter().rev().find_map(|n| get(n)).cloned();
    Rebuilt {
        entries,
        leaf: Some(leaf.uuid.clone()),
        version: up(|n| n.version.as_ref()),
        cwd: up(|n| n.cwd.as_ref()),
        compacted: path.iter().any(|n| n.compact_boundary),
        first_prompt: first_prompt(&path),
        warnings,
    }
}

/// A task as the replay tracks it.
#[derive(Debug, Clone)]
struct TaskState {
    title: String,
    status: String,
    blocked_by: Vec<String>,
}

/// The final plan, from replaying every `TaskCreate`/`TaskUpdate` in the file
/// — or, when there were none, the last `TodoWrite` (RG-10).
///
/// THE WHOLE FILE, NOT THE CHAIN. The task list lives on disk, not in the
/// context: a task created before a compaction, or on a branch the user
/// rewound past, still exists. Sidechain calls are a subagent's and are left
/// out with the subagent.
///
/// A call whose result says it failed did not happen; a call with no result
/// at all (the session was killed mid-call) is applied for an update and
/// skipped for a create, because a create without its result has no id for
/// anything to refer to.
fn replay_plan(nodes: &HashMap<String, Node>) -> Vec<PlanItem> {
    let mut ordered: Vec<&Node> = nodes.values().filter(|n| !n.sidechain).collect();
    ordered.sort_by_key(|n| n.first);
    let results: HashMap<&str, &ResultMeta> = ordered
        .iter()
        .flat_map(|n| n.results.iter())
        .map(|r| (r.tool_use_id.as_str(), r))
        .collect();

    let mut order: Vec<String> = Vec::new();
    let mut tasks: HashMap<String, TaskState> = HashMap::new();
    let mut todos: Option<Vec<PlanItem>> = None;
    for call in ordered.iter().flat_map(|n| n.calls.iter()) {
        let result = results.get(call.id.as_str());
        if result.is_some_and(|r| r.is_error) {
            continue;
        }
        let input = &call.input;
        match call.name.as_str() {
            "TaskCreate" => {
                let Some(id) = result.and_then(|r| r.task_id.clone()) else {
                    continue;
                };
                let title = str_field(input, "subject").unwrap_or_default();
                if !tasks.contains_key(&id) {
                    order.push(id.clone());
                }
                tasks.insert(
                    id,
                    TaskState {
                        title,
                        status: "pending".into(),
                        blocked_by: Vec::new(),
                    },
                );
            }
            "TaskUpdate" => {
                let Some(task) = id_text(&input["taskId"]).and_then(|id| tasks.get_mut(&id)) else {
                    continue;
                };
                if let Some(status) = str_field(input, "status")
                    && Status::from_claude_task(&status, false).is_some()
                {
                    task.status = status;
                }
                if let Some(subject) = str_field(input, "subject").filter(|s| !s.is_empty()) {
                    task.title = subject;
                }
                for blocker in input["addBlockedBy"].as_array().into_iter().flatten() {
                    if let Some(b) = id_text(blocker)
                        && !task.blocked_by.contains(&b)
                    {
                        task.blocked_by.push(b);
                    }
                }
            }
            "TodoWrite" => {
                let list = input["todos"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|todo| {
                        Some(PlanItem {
                            title: str_field(todo, "content")?,
                            status: Status::from_claude_todo(todo["status"].as_str()?)?,
                        })
                    })
                    .collect();
                todos = Some(list);
            }
            _ => {}
        }
    }
    if order.is_empty() {
        return todos.unwrap_or_default();
    }
    let states: Vec<(String, TaskState)> = order
        .into_iter()
        .filter_map(|id| tasks.get(&id).cloned().map(|t| (id, t)))
        .collect();
    plan_from_tasks(&states)
}

/// Tasks to plan items. A pending task is BLOCKED while one of its blockers is
/// still open; a blocker that was completed or deleted no longer blocks, and
/// one this list does not know is assumed open (the conservative reading).
fn plan_from_tasks(tasks: &[(String, TaskState)]) -> Vec<PlanItem> {
    let closed = |id: &str| {
        tasks
            .iter()
            .find(|(tid, _)| tid == id)
            .is_some_and(|(_, t)| t.status == "completed" || t.status == "deleted")
    };
    tasks
        .iter()
        .filter_map(|(_, t)| {
            let open_blockers = t.blocked_by.iter().any(|b| !closed(b));
            Some(PlanItem {
                title: t.title.clone(),
                status: Status::from_claude_task(&t.status, open_blockers)?,
            })
        })
        .collect()
}

/// The plan from `tasks/<id>/`, the task store itself, when it exists and
/// holds at least one task: it is what the tools wrote, so it beats a replay
/// of the calls. The list id is the session id unless a team or
/// `CLAUDE_CODE_TASK_LIST_ID` named another list; then this finds nothing and
/// the replay answers.
fn task_store_plan(root: &Path, id: &str) -> Option<Vec<PlanItem>> {
    let dir = root.join("tasks").join(id);
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut tasks: Vec<(String, TaskState)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = secrets::read_text(&path, MAX_SMALL_FILE_BYTES) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let (Some(tid), Some(title), Some(status)) = (
            id_text(&v["id"]),
            str_field(&v, "subject"),
            str_field(&v, "status"),
        ) else {
            continue;
        };
        let blocked_by = v["blockedBy"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(id_text)
            .collect();
        tasks.push((
            tid,
            TaskState {
                title,
                status,
                blocked_by,
            },
        ));
    }
    if tasks.is_empty() {
        return None;
    }
    // Numeric ids in numeric order (`10` after `9`), anything else after them.
    tasks.sort_by(|(a, _), (b, _)| {
        (a.parse::<u64>().ok().is_none(), a.parse::<u64>().ok(), a).cmp(&(
            b.parse::<u64>().ok().is_none(),
            b.parse::<u64>().ok(),
            b,
        ))
    });
    Some(plan_from_tasks(&tasks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    /// RG-08: Claude Code's own override wins, `HOME/.claude` otherwise, and a
    /// relative override is not resolved against this process's directory.
    #[test]
    fn the_root_is_the_config_dir_override_then_home() {
        assert_eq!(
            roots_from(env(&[(CONFIG_DIR_VAR, "/cfg/cc"), ("HOME", "/home/u")])),
            [PathBuf::from("/cfg/cc")]
        );
        assert_eq!(
            roots_from(env(&[("HOME", "/home/u")])),
            [Path::new("/home/u").join(".claude")]
        );
        assert_eq!(
            roots_from(env(&[(CONFIG_DIR_VAR, "rel/cc"), ("HOME", "/home/u")])),
            [Path::new("/home/u").join(".claude")]
        );
        assert_eq!(
            roots_from(env(&[(CONFIG_DIR_VAR, ""), ("HOME", "/home/u")])),
            [Path::new("/home/u").join(".claude")]
        );
        assert!(roots_from(env(&[])).is_empty());
    }

    /// The pid files are `<digits>.json`; the `<pid>.key` beside each, and
    /// anything else, is not even put to the gate.
    #[test]
    fn only_numeric_json_files_are_pid_files() {
        let dir = std::env::temp_dir().join(format!("tacet-claude-pids-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "4242.json",
            "4242.key",
            "x42.json",
            "42.json.bak",
            "notes.json",
        ] {
            std::fs::write(dir.join(name), "{}").unwrap();
        }
        assert_eq!(live_files(&dir), [dir.join("4242.json")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_task_id_is_read_from_a_result_text_when_the_structure_is_missing() {
        assert_eq!(
            task_id_in_text("Task #12 created successfully: x").as_deref(),
            Some("12")
        );
        assert_eq!(task_id_in_text("no id here"), None);
        assert_eq!(task_id_in_text("a # alone"), None);
    }
}
