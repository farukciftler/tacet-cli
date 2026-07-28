//! Session persistence — the chat that survives `/quit`.
//!
//! ─── PRIVACY RECORD (the shell MUST show this; see `PRIVACY_NOTICE`) ────────
//!
//! WHAT IS STORED: the conversation itself. Every turn's ROLE (user, assistant,
//! tool result), its TEXT VERBATIM, the names of the tools that ran on that
//! turn, and the second it happened.
//!
//! PIPED INPUT IS PART OF THE MESSAGE. `cat report.md | tacet -m "summarise"`
//! stores the whole of `report.md` here, because what gets replayed on
//! `--continue` has to be what the model was actually given. The notice says
//! so out loud: a user who pipes a file expects the ANSWER to be kept, not the
//! file, and that gap is exactly where a `.env` ends up on disk unnoticed. That is the most personal data this
//! program touches — more personal than `memory.json`, because notes are a
//! summary and this is the transcript.
//!
//! WHERE: `<config dir>/sessions/`, one `*.jsonl` file per session — that is
//! `~/.tacet/sessions/` unless `XDG_CONFIG_HOME` or `TACET_HOME` moved it
//! (`tacet config path` prints the resolved directory). NOTHING LEAVES THE
//! MACHINE: this module opens no socket and is not allowed to (the network
//! monopoly lives in `tacet-web`/`tacet-mcp`).
//!
//! HOW IT IS DELETED: `Session::purge_all()` removes the whole `sessions/`
//! directory, or `rm -rf` on the path above does exactly the same thing — there
//! is no index, no second copy and no database, so deleting the files IS the
//! deletion. Individual files can be deleted by hand with no side effects.
//!
//! PERMISSIONS: the directory is 0700 and every file 0600, stamped through
//! `tacet_kernel::fs` rather than left to the umask — see that module for why
//! (measured: umask 022 gives a 0755/0644 pair that every second local account
//! on the machine can read).
//!
//! ─── WHY JSONL AND NOT ONE JSON OBJECT ──────────────────────────────────────
//!
//! A turn is APPENDED as one line the moment it finishes. If the process dies
//! mid-write, the damage is confined to the LAST line: that line is dropped on
//! read and everything before it is still a valid conversation. Written as a
//! single JSON object, the same crash truncates the one array that holds
//! everything and the whole file becomes unparseable — a crash in turn 40 would
//! cost turns 1..39 as well. Append-only also means the common path never
//! rewrites bytes that are already safe on disk.
//!
//! ─── SCOPE ─────────────────────────────────────────────────────────────────
//!
//! This module STORES; it does not decide when to store. It is deliberately
//! ignorant of `tacet_engine::Turn` (see `Turn` below), of the shell's command
//! loop and of the terminal. The wiring — calling `append` at the end of a
//! turn, `latest` behind `--continue`, `list`/`purge_all` behind a command —
//! belongs to the shell phase, which owns `main.rs`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The subdirectory under the configuration directory.
const DIR: &str = "sessions";

/// The extension. One line per turn — see the module note.
const EXT: &str = "jsonl";

/// Turns kept in ONE session file. When the cap is reached the OLDEST turns are
/// dropped so the newest always survive.
///
/// WHY A NUMBER AT ALL: without one, a shell someone leaves open for a week
/// grows a file with no ceiling, and this file holds verbatim chat text.
///
/// WHY 400: the context budget is 4096 tokens, so a model can see roughly the
/// last ten to twenty turns; 400 is an order of magnitude more than anything
/// `--continue` can actually feed back into a prompt, which makes it a bound on
/// disk growth rather than a limit the user can feel. At the ~32 KiB-per-turn
/// ceiling below it also bounds a session file to a few megabytes in the worst
/// case, and in practice (a turn is a few hundred bytes) to well under one.
const MAX_TURNS_PER_SESSION: usize = 400;

/// Session FILES kept. The oldest is deleted when a new session starts.
///
/// WHY 50: enough that "the conversation from last week" is still there for a
/// daily user, small enough that the directory stays something a human can read
/// with `ls` and that the whole history of the tool cannot quietly become the
/// largest thing in the config directory. History that nobody will ever scroll
/// back to is not a feature, it is a liability — this is chat text.
const MAX_SESSIONS: usize = 50;

/// The ceiling on ONE turn's text.
///
/// A PASTE IS THE REAL CASE, not a chatty user: the input field accepts a
/// bracketed paste of arbitrary size, and a pasted logfile would otherwise land
/// in the transcript at full length. 32 KiB is already about eight times the
/// whole context budget, so nothing that the model could ever have read is
/// lost; what gets cut is the tail of something the model never saw either.
const MAX_TEXT_BYTES: usize = 32 * 1024;

/// Left in place of what was cut, so a reader is never shown a silent
/// half-sentence as if it were the whole thing.
const TRUNCATION_MARK: &str = "…[cut]";

/// How much of the first message `list` shows.
const PREVIEW_CHARS: usize = 60;

/// The text the shell shows when it tells the user their chat is on disk.
///
/// KEPT HERE, NEXT TO THE CODE THAT DOES IT. A privacy claim that lives in the
/// UI layer drifts from the behaviour it describes the first time this file
/// changes; the sentence and the write are in the same file on purpose.
pub const PRIVACY_NOTICE: &str = "\
Your conversation is written to disk: role, message text, the tools that ran, \
and the time. ANYTHING YOU PIPE IN IS PART OF THE MESSAGE and is stored with \
it, so `cat secrets.env | tacet -m ...` puts that file in the transcript. It \
stays on this machine and is never sent anywhere. It is kept \
in the 'sessions' folder of your config directory, readable only by you \
(0700/0600). Delete it any time — the folder is the only copy.";

// ── The record ──────────────────────────────────────────────────────────────

/// Who spoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    /// A tool result that was fed back to the model. Kept because a transcript
    /// without it reads as if the assistant knew the weather by magic.
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }

    /// An unknown role is `None`, NOT a default. A line whose role we cannot
    /// read is a line we cannot replay in the right order or attribute to the
    /// right speaker; guessing "user" would put words in the user's mouth.
    pub fn parse(text: &str) -> Option<Role> {
        match text {
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            "tool" => Some(Role::Tool),
            _ => None,
        }
    }
}

/// One stored turn.
///
/// DELIBERATELY NOT `tacet_engine::Turn`. That type is the prompt's shape and
/// it is free to change with the prompt (it carries no time, no tool names, and
/// a serde derive on it would make the on-disk format hostage to a prompt
/// refactor). This one is the FILE FORMAT: it changes only when the file
/// changes. The conversion between the two is the shell's, in `main.rs`, where
/// both types are already in scope — write `session::Turn` there to keep them
/// apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: Role,
    pub text: String,
    /// Tools that ran while producing this turn, by name. Names only: the
    /// ARGUMENTS of a tool call are not stored, because they are the part most
    /// likely to hold a path or a credential the user never meant to keep.
    pub tools: Vec<String>,
    /// Seconds since the Unix epoch, UTC.
    pub at: u64,
}

impl Turn {
    /// A turn stamped with the current time.
    pub fn new(role: Role, text: impl Into<String>) -> Turn {
        Turn {
            role,
            text: text.into(),
            tools: Vec::new(),
            at: now_secs(),
        }
    }

    pub fn with_tools(mut self, tools: Vec<String>) -> Turn {
        self.tools = tools;
        self
    }

    /// The one line that goes on disk.
    ///
    /// `to_string` on a `Value` NEVER emits a raw newline (a newline inside the
    /// text is escaped as `\n`), which is what makes "one turn = one line" hold
    /// for pasted multi-line input as well.
    fn to_line(&self) -> String {
        let mut object = json!({
            "at": self.at,
            "role": self.role.as_str(),
            "text": clip(&self.text, MAX_TEXT_BYTES),
        });
        // Omitted when empty: most turns have no tools and an empty array on
        // every line is pure weight in a file the user may well read by hand.
        if !self.tools.is_empty() {
            object["tools"] = Value::Array(
                self.tools
                    .iter()
                    .map(|t| Value::String(t.clone()))
                    .collect(),
            );
        }
        object.to_string()
    }

    /// `None` for anything we cannot trust: a half-written line, an unknown
    /// role, a line that is not an object.
    fn from_line(line: &str) -> Option<Turn> {
        let value: Value = serde_json::from_str(line).ok()?;
        let role = Role::parse(value.get("role")?.as_str()?)?;
        let text = value.get("text")?.as_str()?.to_string();
        let tools = value
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let at = value.get("at").and_then(Value::as_u64).unwrap_or(0);
        Some(Turn {
            role,
            text,
            tools,
            at,
        })
    }
}

/// One session as `list` describes it.
pub struct SessionInfo {
    /// The file stem — what a `/resume <id>` would take.
    pub id: String,
    /// When the session started (its first turn), seconds since the epoch, UTC.
    pub at: u64,
    /// The beginning of the first thing the user said, shortened.
    pub preview: String,
    pub turns: usize,
    pub path: PathBuf,
}

impl SessionInfo {
    /// The date as the user's own clock shows it. UTC is what is STORED (it
    /// sorts and never goes backwards over a DST change); local is what is
    /// SHOWN, because "14:35" has to match the clock the user was looking at.
    pub fn local_time(&self) -> String {
        local_stamp(self.at)
    }
}

// ── The handle ──────────────────────────────────────────────────────────────

/// An open session. Cheap: it holds a path and a count, no file handle — a
/// shell that sits idle for an hour must not pin a descriptor.
pub struct Session {
    /// `None` when the configuration directory cannot be resolved. NOT an error
    /// at construction time: chat must not fail to start because history cannot
    /// be kept. `append` is where that becomes visible, once, with a reason.
    file: Option<PathBuf>,
    /// Lines already written by THIS handle. Exact, because the file is fresh.
    turns: usize,
}

impl Session {
    /// Opens a new session. The FILE IS NOT CREATED YET — it appears on the
    /// first `append`, so a shell that is opened and closed without a word
    /// leaves nothing behind and `list` never shows empty rows.
    pub fn start() -> Session {
        match dir() {
            Some(d) => Session::start_in(&d),
            None => Session {
                file: None,
                turns: 0,
            },
        }
    }

    /// The real body, with the directory handed in.
    ///
    /// SPLIT OUT SO IT CAN BE MEASURED — the same reason `config::save_to`
    /// exists. The public entry reads a PROCESS-WIDE environment variable, and
    /// a test that sets it fights every other test in this binary; permissions
    /// and the retention limit are exactly the things that must be measured on
    /// the real write path rather than on a hand-rolled copy of it.
    fn start_in(dir: &Path) -> Session {
        // Pruning happens here, once, rather than on every append: it is the
        // one moment a session count actually changes. A file another shell is
        // appending to could in principle be pruned, and on Unix that writer's
        // descriptor stays valid (it loses the file, it does not corrupt one) —
        // at fifty sessions of headroom this cannot be reached in practice.
        prune_sessions(dir);
        Session {
            file: Some(dir.join(format!("{}.{EXT}", new_id()))),
            turns: 0,
        }
    }

    /// The session's identifier — its file stem. `None` when there is nowhere
    /// to write.
    pub fn id(&self) -> Option<String> {
        self.file
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
    }

    pub fn path(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Writes one turn.
    ///
    /// The error is returned rather than printed: whether a failure to persist
    /// is worth interrupting the user for is the shell's call, not this
    /// module's. It must NOT be worth ending the conversation over.
    pub fn append(&mut self, turn: &Turn) -> Result<(), String> {
        let path = self
            .file
            .clone()
            .ok_or_else(|| "the configuration directory cannot be resolved".to_string())?;
        if let Some(parent) = path.parent() {
            // 0700, not the umask: this directory holds chat text.
            tacet_kernel::fs::create_private_dir(parent).map_err(|e| e.to_string())?;
        }

        if self.turns >= MAX_TURNS_PER_SESSION {
            // The cap is reached: drop from the FRONT so the newest turns —
            // the ones `--continue` would actually feed to the model — survive.
            // This is the only path that rewrites the file, and it is rare.
            let existing = read_turns(&path);
            let dropped = existing.len().saturating_sub(MAX_TURNS_PER_SESSION - 1);
            let kept = &existing[dropped.min(existing.len())..];
            let mut body = String::new();
            for kept_turn in kept {
                body.push_str(&kept_turn.to_line());
                body.push('\n');
            }
            tacet_kernel::fs::write_private(&path, body.as_bytes()).map_err(|e| e.to_string())?;
            self.turns = kept.len();
        }

        append_line(&path, &turn.to_line()).map_err(|e| e.to_string())?;
        self.turns += 1;
        Ok(())
    }

    /// The most recent session's turns, for `--continue`.
    ///
    /// Walks BACKWARDS past a session that yields nothing readable: a file
    /// whose only line was lost to a crash is not the conversation the user
    /// meant to continue, and refusing to continue at all because of it would
    /// be a worse answer than the one before it.
    pub fn latest() -> Option<Vec<Turn>> {
        Session::latest_in(&dir()?)
    }

    fn latest_in(dir: &Path) -> Option<Vec<Turn>> {
        for path in session_files(dir).into_iter().rev() {
            let turns = read_turns(&path);
            if !turns.is_empty() {
                return Some(turns);
            }
        }
        None
    }

    /// The turns of ONE named session — what `--session <id>` loads.
    ///
    /// THE ID IS REDUCED TO A FILE STEM, not joined as given. An id reaches this
    /// function straight from the command line, and `Path::join` throws away
    /// everything to its left the moment the joined component is absolute — so
    /// `--session /etc/passwd` (or `../../.ssh/id_rsa`) would otherwise read a
    /// file outside the session folder and pour it into the model's context.
    /// `file_stem` on a rejected shape gives us nothing to open, which is the
    /// right answer: an id that is not one of ours matches no session.
    ///
    /// `None` means "no session by that name"; an EMPTY vector is impossible
    /// here for the same reason `list` hides such files — a file with nothing
    /// readable in it is not a conversation to continue.
    pub fn load(id: &str) -> Option<Vec<Turn>> {
        Session::load_in(&dir()?, id)
    }

    fn load_in(dir: &Path, id: &str) -> Option<Vec<Turn>> {
        // Two gates, not one: the components check refuses a separator or `..`
        // outright, and the stem comparison then insists the survivor is exactly
        // the name we were given (so `x.jsonl` cannot stand in for `x`).
        let stem = Path::new(id).file_stem()?.to_string_lossy().into_owned();
        if stem != id || id.is_empty() {
            return None;
        }
        let path = dir.join(format!("{id}.{EXT}"));
        let turns = read_turns(&path);
        (!turns.is_empty()).then_some(turns)
    }

    /// Every stored session, NEWEST FIRST.
    pub fn list() -> Vec<SessionInfo> {
        match dir() {
            Some(d) => Session::list_in(&d),
            None => Vec::new(),
        }
    }

    fn list_in(dir: &Path) -> Vec<SessionInfo> {
        let mut out: Vec<SessionInfo> = session_files(dir)
            .into_iter()
            .filter_map(|path| {
                let turns = read_turns(&path);
                if turns.is_empty() {
                    // A file with nothing readable in it is not a session to
                    // offer; showing "0 turns" invites the user to open it.
                    return None;
                }
                let id = path.file_stem()?.to_string_lossy().into_owned();
                let preview = turns
                    .iter()
                    .find(|t| t.role == Role::User)
                    .map(|t| preview_of(&t.text))
                    .unwrap_or_default();
                Some(SessionInfo {
                    id,
                    at: turns.first().map(|t| t.at).unwrap_or(0),
                    preview,
                    turns: turns.len(),
                    path,
                })
            })
            .collect();
        // By the stamp INSIDE the file, not by the name: the name is derived
        // from the same clock, but data beats a filename anyone can rename.
        out.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.id.cmp(&a.id)));
        out
    }

    /// Deletes every session. Returns how many files went, so the shell can say
    /// a number instead of "done".
    ///
    /// THE DIRECTORY ITSELF IS REMOVED TOO. Leaving an empty `sessions/` behind
    /// after the user asked for their chat to be gone is the kind of residue
    /// that makes people doubt the deletion happened at all.
    pub fn purge_all() -> Result<usize, String> {
        match dir() {
            Some(d) => Session::purge_all_in(&d),
            None => Err("the configuration directory cannot be resolved".to_string()),
        }
    }

    fn purge_all_in(dir: &Path) -> Result<usize, String> {
        if !dir.exists() {
            return Ok(0);
        }
        let count = session_files(dir).len();
        fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
        Ok(count)
    }
}

// ── Paths, time, plumbing ───────────────────────────────────────────────────

/// `<config dir>/sessions`.
pub fn dir() -> Option<PathBuf> {
    tacet_kernel::config_path(DIR)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `20260728T143512Z-41207`.
///
/// UTC AND FIXED WIDTH SO THE NAME SORTS: `session_files` relies on plain
/// lexicographic order being chronological order, and a local-time name breaks
/// that twice a year when the clock goes back. The trailing process id is what
/// keeps two shells started in the same second from claiming one file.
fn new_id() -> String {
    let at = tacet_tools::time::DateTime::from_epoch(now_secs() as i64);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z-{}",
        at.year,
        at.month,
        at.day,
        at.clock,
        at.minute,
        at.second,
        std::process::id()
    )
}

/// A stored (UTC) second as the user's own wall clock.
pub fn local_stamp(at: u64) -> String {
    let offset = tacet_tools::time::local_offset_minutes().unwrap_or(0);
    let local = tacet_tools::time::DateTime::from_epoch(at as i64 + offset * 60);
    format!("{} {}", local.iso_date(), local.iso_time())
}

/// The session files, OLDEST FIRST (see `new_id` for why the name sorts).
fn session_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == EXT))
        .collect();
    files.sort();
    files
}

/// Deletes the oldest files so that this session's own file still fits under
/// `MAX_SESSIONS`.
fn prune_sessions(dir: &Path) {
    let files = session_files(dir);
    let keep = MAX_SESSIONS.saturating_sub(1);
    if files.len() <= keep {
        return;
    }
    for old in &files[..files.len() - keep] {
        // Best effort: a file we cannot delete is not a reason to refuse the
        // user a new session.
        let _ = fs::remove_file(old);
    }
}

/// Reads the turns of one file, DROPPING any line that does not parse.
///
/// The line that a crash cuts in half is the LAST one, and dropping it costs
/// the turn that was being written when the power went — nothing earlier.
fn read_turns(path: &Path) -> Vec<Turn> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(Turn::from_line)
        .collect()
}

/// Appends one line, keeping the file 0600.
///
/// WHY NOT `write_private` EVERY TIME: it truncates, and rewriting the whole
/// transcript on every turn is both the slow path and the one where a crash
/// costs everything instead of one line. So the file is CREATED through the
/// kernel's private-write primitive (which is what stamps 0600 at creation, in
/// the same open as the first byte), and an existing file is narrowed through
/// `narrow_file` — the primitive that exists for exactly this, "a file we
/// deliberately do not overwrite" — before the descriptor is opened for append.
/// NO chmod IS WRITTEN HERE; both stamps are the kernel's.
fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    if path.exists() {
        // A file left by an older build (or by a user's own `cp`) can be 0644;
        // appending chat text to it as-is would leak it to every local account.
        tacet_kernel::fs::narrow_file(path);
    } else {
        tacet_kernel::fs::write_private(path, b"")?;
    }
    let mut file = fs::OpenOptions::new().append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")
}

/// Cuts text to a byte ceiling ON A CHARACTER BOUNDARY (a cut through the
/// middle of a UTF-8 sequence would panic on the slice and, if it reached disk,
/// make the line unreadable forever).
fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TRUNCATION_MARK}", &text[..end])
}

/// The first non-empty line, shortened — a list row is one line wide.
fn preview_of(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = line.chars().take(PREVIEW_CHARS).collect();
    if line.chars().count() > PREVIEW_CHARS {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own. The public entry points read a
    /// process-wide environment variable; every test here goes through the
    /// `*_in` bodies instead, so the tests cannot fight each other.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tacet-session-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn user(text: &str) -> Turn {
        Turn::new(Role::User, text)
    }

    /// THE POINT OF THE WHOLE MODULE: what was said is still there after the
    /// process is gone.
    #[test]
    fn a_conversation_written_is_the_conversation_read_back() {
        let dir = temp_dir("roundtrip");
        let mut session = Session::start_in(&dir);

        session
            .append(&user("what is the capital of Norway"))
            .unwrap();
        session
            .append(&Turn::new(Role::Assistant, "Oslo").with_tools(vec!["web_search".into()]))
            .unwrap();

        let read = Session::latest_in(&dir).expect("the session was not found");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].role, Role::User);
        assert_eq!(read[0].text, "what is the capital of Norway");
        assert_eq!(read[1].role, Role::Assistant);
        assert_eq!(read[1].text, "Oslo");
        assert_eq!(read[1].tools, vec!["web_search".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }

    /// `--session <id>` finds the named one, and a NAME THAT IS A PATH finds
    /// nothing. The second half is the point: the id comes off the command line
    /// and joins a directory, so an absolute or climbing value would otherwise
    /// read a file that is not a session at all.
    #[test]
    fn a_session_is_loaded_by_name_and_a_path_is_not_a_name() {
        let dir = temp_dir("byname");
        let mut session = Session::start_in(&dir);
        session.append(&user("the one I want")).unwrap();
        let id = session.id().unwrap();

        let read = Session::load_in(&dir, &id).expect("the named session was not found");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].text, "the one I want");

        // A neighbour file the loader must never reach.
        let outside = dir.parent().unwrap().join("tacet-session-outside.jsonl");
        fs::write(
            &outside,
            "{\"at\":1,\"role\":\"user\",\"text\":\"secret\"}\n",
        )
        .unwrap();
        for hostile in [
            "../tacet-session-outside",
            "/etc/passwd",
            "",
            "sub/dir",
            &format!("{id}.jsonl"),
        ] {
            assert!(
                Session::load_in(&dir, hostile).is_none(),
                "'{hostile}' was accepted as a session id"
            );
        }

        let _ = fs::remove_file(&outside);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A MULTI-LINE PASTE MUST NOT BECOME THREE TURNS. This is the reason the
    /// format can be line-oriented at all: the newline is escaped, not written.
    #[test]
    fn a_multi_line_message_stays_one_line_on_disk() {
        let dir = temp_dir("multiline");
        let mut session = Session::start_in(&dir);
        session.append(&user("first\nsecond\nthird")).unwrap();

        let path = session.path().unwrap();
        let raw = fs::read_to_string(path).unwrap();
        assert_eq!(raw.lines().count(), 1, "the paste was split across lines");
        assert_eq!(
            Session::latest_in(&dir).unwrap()[0].text,
            "first\nsecond\nthird"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// THE CRASH CASE, WHICH IS THE WHOLE ARGUMENT FOR JSONL. The last line is
    /// cut in half; everything written before it must still be readable. With a
    /// single JSON object on disk this test cannot be made to pass at all.
    #[test]
    fn a_half_written_last_line_costs_one_turn_not_the_file() {
        let dir = temp_dir("torn");
        let mut session = Session::start_in(&dir);
        session.append(&user("one")).unwrap();
        session.append(&Turn::new(Role::Assistant, "two")).unwrap();

        let path = session.path().unwrap().to_path_buf();
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"at\":1,\"role\":\"user\",\"te").unwrap();
        drop(file);

        let read = Session::latest_in(&dir).expect("the torn line took the file with it");
        assert_eq!(read.len(), 2, "a good turn was lost with the torn one");
        assert_eq!(read[0].text, "one");
        assert_eq!(read[1].text, "two");

        let _ = fs::remove_dir_all(&dir);
    }

    /// A line whose role we cannot read is dropped rather than guessed —
    /// attributing an unknown line to the user would put words in their mouth.
    #[test]
    fn a_line_with_an_unknown_role_is_dropped() {
        assert!(Turn::from_line("{\"role\":\"system\",\"text\":\"x\"}").is_none());
        assert!(Turn::from_line("{\"role\":\"user\"}").is_none());
        assert!(Turn::from_line("not json at all").is_none());
        assert!(Turn::from_line("{\"role\":\"user\",\"text\":\"x\"}").is_some());
    }

    /// THE PER-FILE CAP IS REAL AND IT DROPS FROM THE FRONT. The newest turns
    /// are the ones `--continue` feeds to the model, so they are the ones that
    /// must survive.
    #[test]
    fn the_turn_cap_drops_the_oldest_and_keeps_the_newest() {
        let dir = temp_dir("cap");
        let mut session = Session::start_in(&dir);
        for i in 0..MAX_TURNS_PER_SESSION + 5 {
            session.append(&user(&format!("turn {i}"))).unwrap();
        }

        let read = Session::latest_in(&dir).unwrap();
        assert_eq!(
            read.len(),
            MAX_TURNS_PER_SESSION,
            "the file grew past its cap"
        );
        assert_eq!(
            read.last().unwrap().text,
            format!("turn {}", MAX_TURNS_PER_SESSION + 4),
            "the newest turn was the one dropped"
        );
        assert!(
            !read.iter().any(|t| t.text == "turn 0"),
            "the oldest turn survived the cap"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// A PASTED LOGFILE MUST NOT LAND IN THE TRANSCRIPT AT FULL LENGTH, and the
    /// cut must not fall inside a UTF-8 sequence.
    #[test]
    fn an_oversized_message_is_cut_on_a_character_boundary() {
        let dir = temp_dir("clip");
        let mut session = Session::start_in(&dir);
        // Three-byte characters, so a byte-count cut lands mid-sequence unless
        // the boundary is respected.
        session.append(&user(&"ş".repeat(MAX_TEXT_BYTES))).unwrap();

        let read = Session::latest_in(&dir).unwrap();
        assert!(read[0].text.ends_with(TRUNCATION_MARK));
        assert!(read[0].text.len() <= MAX_TEXT_BYTES + TRUNCATION_MARK.len());

        let _ = fs::remove_dir_all(&dir);
    }

    /// THE SESSION COUNT IS BOUNDED TOO, and the oldest file is the one that
    /// goes. Written straight to disk rather than through fifty `start_in`
    /// calls, because two sessions started in the same second share a name.
    #[test]
    fn starting_a_session_drops_the_oldest_file_over_the_limit() {
        let dir = temp_dir("prune");
        tacet_kernel::fs::create_private_dir(&dir).unwrap();
        for i in 0..MAX_SESSIONS + 3 {
            let path = dir.join(format!("2026010{}T00000{}Z-{i:04}.{EXT}", i / 10, i % 10));
            fs::write(
                &path,
                format!("{{\"at\":1,\"role\":\"user\",\"text\":\"{i}\"}}\n"),
            )
            .unwrap();
        }
        assert_eq!(session_files(&dir).len(), MAX_SESSIONS + 3, "the premise");

        let mut session = Session::start_in(&dir);
        session.append(&user("the new one")).unwrap();

        let files = session_files(&dir);
        assert_eq!(files.len(), MAX_SESSIONS, "the directory grew past its cap");
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            !names.iter().any(|n| n.starts_with("20260100T000000Z")),
            "the oldest file survived the prune"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// `list` is what the user reads before choosing: newest first, with enough
    /// of the first message to recognise the conversation.
    #[test]
    fn list_shows_the_newest_session_first_with_its_opening_line() {
        let dir = temp_dir("list");
        tacet_kernel::fs::create_private_dir(&dir).unwrap();
        fs::write(
            dir.join(format!("20260101T000000Z-1.{EXT}")),
            "{\"at\":100,\"role\":\"user\",\"text\":\"the older question\"}\n",
        )
        .unwrap();
        fs::write(
            dir.join(format!("20260102T000000Z-1.{EXT}")),
            "{\"at\":200,\"role\":\"tool\",\"text\":\"a tool result\"}\n\
             {\"at\":201,\"role\":\"user\",\"text\":\"the newer question\"}\n",
        )
        .unwrap();

        let listed = Session::list_in(&dir);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "20260102T000000Z-1");
        assert_eq!(listed[0].turns, 2);
        assert_eq!(
            listed[0].preview, "the newer question",
            "the preview must be the USER's line, not a tool result"
        );
        assert_eq!(listed[1].preview, "the older question");

        let _ = fs::remove_dir_all(&dir);
    }

    /// A long first message is shortened, and the shortening is VISIBLE.
    #[test]
    fn a_long_opening_line_is_shortened_visibly() {
        let long = "x".repeat(PREVIEW_CHARS * 2);
        let preview = preview_of(&long);
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), PREVIEW_CHARS + 1);
        assert_eq!(preview_of("  \n  hello  \n world"), "hello");
    }

    /// DELETION MUST LEAVE NOTHING, not even the folder — residue makes people
    /// doubt the deletion happened.
    #[test]
    fn purge_removes_every_file_and_the_folder() {
        let dir = temp_dir("purge");
        let mut session = Session::start_in(&dir);
        session.append(&user("something private")).unwrap();
        assert!(dir.exists(), "the premise");

        let removed = Session::purge_all_in(&dir).unwrap();
        assert_eq!(removed, 1);
        assert!(!dir.exists(), "the sessions folder outlived the purge");
        assert!(Session::latest_in(&dir).is_none());
        // Purging twice is not an error: the user asked for nothing to be
        // there, and nothing is there.
        assert_eq!(Session::purge_all_in(&dir).unwrap(), 0);
    }

    /// A session with nowhere to write does not take the chat down with it.
    #[test]
    fn a_session_with_no_directory_reports_instead_of_panicking() {
        let mut session = Session {
            file: None,
            turns: 0,
        };
        assert!(session.append(&user("x")).is_err());
        assert!(session.id().is_none());
    }

    /// THIS IS CHAT TEXT AND THE UMASK DOES NOT GET TO DECIDE WHO READS IT.
    /// Measured against `fs::create_dir_all` + `fs::write` under umask 022 this
    /// is RED (0755/0644 — every second local account on the machine, and on
    /// macOS every account is in `staff`).
    ///
    /// THE SECOND APPEND IS PART OF THE TEST: the append path opens the file
    /// itself, so a mode that was only right at creation would not be enough.
    /// A 0644 file left by an earlier build is set up on purpose for the same
    /// reason.
    #[cfg(unix)]
    #[test]
    fn the_transcript_excludes_every_other_local_account() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("modes");
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;

        // An existing, wide-open directory: the upgrading user, not the fresh
        // install.
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        let mut session = Session::start_in(&dir);
        session.append(&user("a private thing")).unwrap();
        let path = session.path().unwrap().to_path_buf();

        assert_eq!(mode(&dir), 0o700, "the sessions directory is walkable");
        assert_eq!(mode(&path), 0o600, "the transcript is world-readable");

        session
            .append(&Turn::new(Role::Assistant, "and another"))
            .unwrap();
        assert_eq!(mode(&path), 0o600, "the append path widened the file");

        // A leftover 0644 file is narrowed BEFORE the next line lands in it.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        session.append(&user("a third")).unwrap();
        assert_eq!(mode(&path), 0o600, "chat text was appended to a 0644 file");
        assert_eq!(Session::latest_in(&dir).unwrap().len(), 3);

        let _ = fs::remove_dir_all(&dir);
    }

    /// The id has to sort chronologically as a plain string — `session_files`,
    /// and therefore `latest`, rests on that.
    #[test]
    fn the_id_is_a_fixed_width_utc_stamp() {
        let id = new_id();
        let (stamp, pid) = id.split_once('-').expect("the id carries no process id");
        assert_eq!(stamp.len(), 16, "the stamp is not fixed width: {stamp}");
        assert!(stamp.ends_with('Z'), "the stamp is not marked UTC: {stamp}");
        assert_eq!(&stamp[8..9], "T");
        assert!(pid.parse::<u32>().is_ok());
    }
}
