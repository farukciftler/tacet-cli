//! The one shape three agents' sessions are read into.
//!
//! WHY ONE SHAPE AND NOT THREE. Claude Code writes a parent-linked JSONL chain,
//! Codex a typed rollout of Responses-API items, Antigravity a step log next to
//! a SQLite file it does not want read. Everything Tacet does with a session
//! after reading it — list it, summarise it into a turn, seed a handoff, show
//! the plan — is the same for all three, and written three times it would drift
//! three ways. So each reader's only job is to translate into `AgentSession`;
//! nothing downstream ever sees a vendor format.
//!
//! EVERY STRING IN HERE IS PERSONAL AND UNTRUSTED. A transcript holds the code,
//! the command output and the prompts of another tool's session: it taints the
//! Tacet session that reads it (audit H2 / RG-01), and it was partly written by
//! whatever repository that agent was pointed at, so it is defanged at the
//! prompt boundary like any tool result (H3/H4 / RG-02). The types do not
//! defang — which tokens are dangerous depends on the loaded model's
//! vocabulary, and only the engine knows that — they carry the text verbatim so
//! the one place that does defang sees exactly what was on disk.
//!
//! See docs/adr/0001-agent-sessions.md for the decisions behind this module.

use std::fmt;
use std::path::{Path, PathBuf};

use super::handoff::Status;
use super::secrets::Refusal;

/// The three agents whose sessions Tacet can read.
///
/// A closed set: a fourth agent is a fourth reader, a fourth version table and
/// a fourth threat model, not an enum arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    Antigravity,
}

impl AgentKind {
    pub const ALL: [AgentKind; 3] = [
        AgentKind::ClaudeCode,
        AgentKind::Codex,
        AgentKind::Antigravity,
    ];

    /// The short name used in file names (`<config>/handoff/claude-<id>.json`)
    /// and on the command line (`--from claude:<id>`). Stable: a hook another
    /// agent runs reads files by this name, so renaming it orphans every
    /// handoff already on disk.
    pub fn slug(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Antigravity => "antigravity",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "Claude Code",
            AgentKind::Codex => "Codex CLI",
            AgentKind::Antigravity => "Antigravity",
        }
    }

    /// `claude-code` is accepted because it is what people type and what the
    /// progress sheet's "Ajan / Oturum" column was filled with; it is never
    /// WRITTEN, so files on disk have one spelling.
    pub fn from_slug(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claudecode" => Some(AgentKind::ClaudeCode),
            "codex" => Some(AgentKind::Codex),
            "antigravity" => Some(AgentKind::Antigravity),
            _ => None,
        }
    }
}

impl fmt::Display for AgentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

/// Who produced an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
    /// A tool's own output (a Claude `tool_result`, a Codex
    /// `function_call_output`, an Antigravity `RUN_COMMAND` step).
    Tool,
}

/// What an entry is, independently of who produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Text,
    /// Model reasoning. Kept distinct so a summary can leave it out: it is the
    /// largest and least useful part of a transcript to hand to a 4096-token
    /// context.
    Thinking,
    ToolCall,
    ToolResult,
    /// A compaction summary (Claude `isCompactSummary`, Codex `compacted`):
    /// everything before it was replaced by this text.
    Summary,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub role: Role,
    pub kind: EntryKind,
    pub text: String,
    /// The tool's name for `ToolCall`/`ToolResult`, when the format says it.
    pub tool: Option<String>,
    /// Unix seconds, UTC. `None` rather than a guess: Claude's timestamps were
    /// MEASURED to go backwards 1,900 times across 30 files, so order comes
    /// from the chain, never from this field.
    pub ts: Option<i64>,
}

/// One line of the agent's own plan (Claude Task*/TodoWrite, Codex
/// `update_plan`, Antigravity `task.md`), already mapped onto the shared
/// five-state enum.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanItem {
    pub title: String,
    pub status: Status,
}

/// A session, read in full.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentSession {
    pub agent: AgentKind,
    pub id: String,
    pub title: String,
    pub cwd: Option<PathBuf>,
    /// Unix seconds, UTC.
    pub started: Option<i64>,
    pub updated: Option<i64>,
    /// The agent process still has this session open. A write into a live
    /// session becomes a dead branch or a fork (Claude keeps the leaf in
    /// memory), so every write path refuses when this is true.
    pub live: bool,
    /// The agent's own version string, as the file recorded it. Fed to
    /// `version::check_read`/`check_write`; for Claude it is the version on the
    /// LEAF entry, because that is the line a write would chain to.
    pub version: Option<String>,
    pub entries: Vec<Entry>,
    pub plan: Vec<PlanItem>,
}

/// What a listing shows: everything except the entries, plus where the
/// session lives so `read` does not have to search again.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub agent: AgentKind,
    pub id: String,
    pub title: String,
    pub cwd: Option<PathBuf>,
    pub started: Option<i64>,
    pub updated: Option<i64>,
    pub live: bool,
    pub version: Option<String>,
    pub path: PathBuf,
}

/// Newest first; a session with no timestamp sorts last rather than first, so
/// a file whose clock could not be read never pushes the real latest session
/// off the top of the list.
pub fn newest_first(list: &mut [SessionSummary]) {
    list.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
}

/// A reader of one agent's local session files.
///
/// THE ROOT IS AN ARGUMENT, NOT STATE. `default_roots` says where the agent
/// keeps its files on this machine (its own env override first); tests and the
/// CLI pass a root explicitly, so no test ever has to point a reader at the
/// user's real `~/.claude`, `~/.codex` or `~/.gemini`.
///
/// CONTRACT FOR IMPLEMENTORS: every file is opened through
/// `secrets::open`/`secrets::read_text`/`secrets::read_tail` — never
/// `std::fs` directly. That gate is the only place the credential denylist
/// lives, and `sessions::tests::every_reader_goes_through_the_secrets_gate`
/// fails the build if a file in this module opens a file any other way.
pub trait SessionSource {
    fn agent(&self) -> AgentKind;

    /// Never created, never written: a missing root is "no sessions".
    fn default_roots(&self) -> Vec<PathBuf>;

    /// Lists sessions under `root`. A file that cannot be parsed is SKIPPED,
    /// not fatal: one corrupt rollout must not hide the other forty.
    fn discover(&self, root: &Path) -> Vec<SessionSummary>;

    /// Reads one session in full. `id` is validated with `validate_id` before
    /// it touches a path.
    fn read(&self, root: &Path, id: &str) -> Result<AgentSession, SessionError>;
}

/// Why a session could not be read.
#[derive(Debug)]
pub enum SessionError {
    /// The id is not one this module will put into a path.
    BadId(String),
    NotFound {
        agent: AgentKind,
        id: String,
    },
    /// The secrets gate refused the file (credential, compressed, not a file…).
    Refused(Refusal),
    /// The file exists but is not in a shape this reader knows.
    Format {
        path: PathBuf,
        line: Option<usize>,
        why: String,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::BadId(why) => write!(f, "not a session id: {why}"),
            SessionError::NotFound { agent, id } => {
                write!(f, "no {} session with id {id}", agent.display_name())
            }
            SessionError::Refused(r) => write!(f, "{r}"),
            SessionError::Format { path, line, why } => match line {
                Some(n) => write!(f, "{}:{n}: {why}", path.display()),
                None => write!(f, "{}: {why}", path.display()),
            },
        }
    }
}

impl std::error::Error for SessionError {}

impl From<Refusal> for SessionError {
    fn from(r: Refusal) -> Self {
        SessionError::Refused(r)
    }
}

/// The longest id accepted. Every id seen in the wild is a UUID (36 chars);
/// the slack is for a future format, not for a path.
pub const MAX_ID_LEN: usize = 128;

/// Is `id` safe to put into a file name?
///
/// WHY THIS IS A SECURITY CHECK. Ids arrive from two untrusted places: the
/// command line (`--from claude:<id>`) and the session files themselves (a
/// Codex `session_meta.id`, a handoff record another agent wrote). Both end up
/// joined onto a directory — `<config>/handoff/<agent>-<id>.json` — and an id of
/// `../../.ssh/authorized_keys` would turn "write the handoff" into "write
/// anywhere the user can". So the alphabet is the one real ids use, and
/// nothing else.
pub fn validate_id(id: &str) -> Result<(), SessionError> {
    if id.is_empty() {
        return Err(SessionError::BadId("empty".into()));
    }
    if id.len() > MAX_ID_LEN {
        return Err(SessionError::BadId(format!(
            "longer than {MAX_ID_LEN} bytes"
        )));
    }
    if id.starts_with('.') || id.contains("..") {
        return Err(SessionError::BadId(format!(
            "{id:?} could name a parent or hidden file"
        )));
    }
    if let Some(c) = id
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Err(SessionError::BadId(format!("{id:?} contains {c:?}")));
    }
    Ok(())
}

/// `claude:<id>` / `codex:<id>` / `antigravity:<id>`, the form `tacet chat
/// --from` takes. The id is validated here, once, so no caller can forget.
pub fn parse_ref(text: &str) -> Result<(AgentKind, String), SessionError> {
    let (agent, id) = text
        .split_once(':')
        .ok_or_else(|| SessionError::BadId(format!("{text:?} is not <agent>:<id>")))?;
    let agent = AgentKind::from_slug(agent)
        .ok_or_else(|| SessionError::BadId(format!("unknown agent {agent:?}")))?;
    let id = id.trim();
    validate_id(id)?;
    Ok((agent, id.to_string()))
}

/// Parses an ISO-8601 instant into Unix seconds, UTC.
///
/// Accepts the shapes the three agents were seen to write:
/// `2026-10-09T12:34:56.789Z` (Claude), `2026-10-09T12:34:56Z`, and an explicit
/// `+03:00` offset. Fractions are dropped. Anything else is `None`, NOT a best
/// guess — a wrong timestamp in a listing is worse than a missing one.
pub fn parse_iso_utc(text: &str) -> Option<i64> {
    let text = text.trim();
    let bytes = text.as_bytes();
    if bytes.len() < 19 || !(bytes[10] == b'T' || bytes[10] == b' ') {
        return None;
    }
    let num = |range: std::ops::Range<usize>| -> Option<u32> {
        let s = text.get(range)?;
        if s.bytes().all(|b| b.is_ascii_digit()) {
            s.parse().ok()
        } else {
            None
        }
    };
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let date = crate::time::DateTime::new(
        num(0..4)? as i64,
        num(5..7)?,
        num(8..10)?,
        num(11..13)?,
        num(14..16)?,
        num(17..19)?,
    )?;
    let mut rest = &text[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        rest = &frac[digits..];
    }
    let offset_seconds = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let body = &rest[1..];
            let (h, m) = body
                .split_once(':')
                .unwrap_or((body.get(0..2)?, body.get(2..)?));
            if h.len() != 2
                || m.len() != 2
                || !(h.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit()))
            {
                return None;
            }
            let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
            if h > 23 || m > 59 {
                return None;
            }
            sign * (h * 3600 + m * 60)
        }
    };
    Some(date.epoch() - offset_seconds)
}

/// Unix seconds to `YYYY-MM-DDTHH:MM:SSZ`. Second precision is enough for a
/// handoff and keeps the field byte-comparable across writers.
pub fn iso_utc(epoch: i64) -> String {
    let d = crate::time::DateTime::from_epoch(epoch);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        d.iso_date(),
        d.clock,
        d.minute,
        d.second
    )
}

/// The current time in Unix seconds. A clock before 1970 reads as 0 rather
/// than panicking — a handoff with a wrong date is recoverable, a crash in the
/// middle of a write is not.
pub fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::secrets;

    /// The id check is what keeps a session id out of `../`. Every one of these
    /// came from thinking about where an id is joined onto a path.
    #[test]
    fn an_id_that_could_leave_its_directory_is_refused() {
        for bad in [
            "",
            "..",
            "../../.ssh/authorized_keys",
            "a/b",
            "a\\b",
            ".hidden",
            "x..y",
            "id with space",
            "nul\0byte",
            "ü-unicode",
        ] {
            assert!(validate_id(bad).is_err(), "accepted {bad:?}");
        }
        assert!(validate_id(&"a".repeat(MAX_ID_LEN + 1)).is_err());
        // The real shapes: a Claude/Codex/Antigravity UUID, a Claude subagent id.
        for good in [
            "a035ad6d-1c2b-4f6e-9d1a-0b8c7e6f5a43",
            "agent-a1b2c3",
            "rollout_2026.x",
        ] {
            assert!(validate_id(good).is_ok(), "refused {good:?}");
        }
    }

    #[test]
    fn a_reference_names_the_agent_and_a_safe_id() {
        let (agent, id) = parse_ref("claude-code:a035ad6d").unwrap();
        assert_eq!((agent, id.as_str()), (AgentKind::ClaudeCode, "a035ad6d"));
        assert_eq!(parse_ref("codex:abc").unwrap().0, AgentKind::Codex);
        assert!(parse_ref("antigravity:../x").is_err());
        assert!(parse_ref("cursor:abc").is_err());
        assert!(parse_ref("no-colon").is_err());
    }

    #[test]
    fn every_slug_reads_back_as_its_agent() {
        for agent in AgentKind::ALL {
            assert_eq!(AgentKind::from_slug(agent.slug()), Some(agent));
        }
    }

    #[test]
    fn iso_timestamps_parse_in_the_shapes_the_agents_write() {
        let z = parse_iso_utc("2026-10-09T12:34:56Z").unwrap();
        assert_eq!(parse_iso_utc("2026-10-09T12:34:56.789Z"), Some(z));
        assert_eq!(parse_iso_utc("2026-10-09T15:34:56+03:00"), Some(z));
        assert_eq!(parse_iso_utc("2026-10-09T09:34:56-0300"), Some(z));
        assert_eq!(iso_utc(z), "2026-10-09T12:34:56Z");
        for bad in [
            "2026-10-09",
            "2026-13-09T00:00:00Z",
            "2026-10-09T12:34:56",
            "2026-10-09T12:34:56.Z",
            "yesterday",
        ] {
            assert_eq!(parse_iso_utc(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_session_without_a_clock_sorts_last() {
        let s = |id: &str, updated: Option<i64>| SessionSummary {
            agent: AgentKind::Codex,
            id: id.into(),
            title: String::new(),
            cwd: None,
            started: None,
            updated,
            live: false,
            version: None,
            path: PathBuf::new(),
        };
        let mut list = vec![s("old", Some(1)), s("none", None), s("new", Some(9))];
        newest_first(&mut list);
        let ids: Vec<_> = list.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["new", "old", "none"]);
    }

    /// A minimal source over a made-up format: one `<id>.jsonl` per session,
    /// each line `{"role","text","ts"}`. It exists to hold the trait to its
    /// contract — ids validated before a path is built, every open through the
    /// secrets gate — on something small enough to read in one screen.
    struct FixtureSource;

    impl SessionSource for FixtureSource {
        fn agent(&self) -> AgentKind {
            AgentKind::ClaudeCode
        }
        fn default_roots(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn discover(&self, root: &Path) -> Vec<SessionSummary> {
            let Ok(dir) = std::fs::read_dir(root) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for entry in dir.flatten() {
                let path = entry.path();
                let Some(id) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
                else {
                    continue;
                };
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl")
                    || validate_id(&id).is_err()
                {
                    continue;
                }
                if let Ok(session) = self.read(root, &id) {
                    out.push(SessionSummary {
                        agent: session.agent,
                        id,
                        title: session.title,
                        cwd: None,
                        started: session.started,
                        updated: session.updated,
                        live: false,
                        version: None,
                        path,
                    });
                }
            }
            newest_first(&mut out);
            out
        }
        fn read(&self, root: &Path, id: &str) -> Result<AgentSession, SessionError> {
            validate_id(id)?;
            let path = root.join(format!("{id}.jsonl"));
            let text = secrets::read_text(&path, 1 << 20)?;
            let mut entries = Vec::new();
            for (n, line) in text.lines().enumerate() {
                let v: serde_json::Value =
                    serde_json::from_str(line).map_err(|e| SessionError::Format {
                        path: path.clone(),
                        line: Some(n + 1),
                        why: e.to_string(),
                    })?;
                entries.push(Entry {
                    role: if v["role"] == "user" {
                        Role::User
                    } else {
                        Role::Assistant
                    },
                    kind: EntryKind::Text,
                    text: v["text"].as_str().unwrap_or_default().to_string(),
                    tool: None,
                    ts: v["ts"].as_str().and_then(parse_iso_utc),
                });
            }
            Ok(AgentSession {
                agent: self.agent(),
                id: id.to_string(),
                title: entries.first().map(|e| e.text.clone()).unwrap_or_default(),
                cwd: None,
                started: entries.first().and_then(|e| e.ts),
                updated: entries.iter().filter_map(|e| e.ts).max(),
                live: false,
                version: None,
                entries,
                plan: Vec::new(),
            })
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tacet-sessions-model-{name}-{}-{}",
            std::process::id(),
            now_epoch()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_source_lists_reads_and_refuses_through_the_gate() {
        let root = scratch("fixture");
        std::fs::write(
            root.join("s1.jsonl"),
            "{\"role\":\"user\",\"text\":\"fix the rollover\",\"ts\":\"2026-10-09T10:00:00Z\"}\n\
             {\"role\":\"assistant\",\"text\":\"done\",\"ts\":\"2026-10-09T10:05:00Z\"}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("s2.jsonl"),
            "{\"role\":\"user\",\"text\":\"later\",\"ts\":\"2026-10-09T11:00:00Z\"}\n",
        )
        .unwrap();
        let list = FixtureSource.discover(&root);
        assert_eq!(
            list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["s2", "s1"]
        );
        let s1 = FixtureSource.read(&root, "s1").unwrap();
        assert_eq!(s1.title, "fix the rollover");
        assert_eq!(s1.entries.len(), 2);
        assert!(matches!(
            FixtureSource.read(&root, "../s1"),
            Err(SessionError::BadId(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The point of routing every read through one gate: a session file that is
    /// really a symlink to a credential is refused by a reader that never
    /// thought about credentials at all.
    #[cfg(unix)]
    #[test]
    fn a_session_file_that_links_to_a_credential_is_refused_by_any_reader() {
        let root = scratch("link");
        let auth = root.join("auth.json");
        std::fs::write(&auth, "{\"OPENAI_API_KEY\":\"sk-secret\"}").unwrap();
        std::os::unix::fs::symlink(&auth, root.join("s3.jsonl")).unwrap();
        match FixtureSource.read(&root, "s3") {
            Err(SessionError::Refused(Refusal::Secret { .. })) => {}
            other => panic!("a link to auth.json was read: {other:?}"),
        }
        assert!(FixtureSource.discover(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
