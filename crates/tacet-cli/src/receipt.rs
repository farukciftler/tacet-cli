//! The receipt chain — the storage behind `tacet log`.
//!
//! Every tool execution is recorded by PURE CODE: the model is never asked to
//! log anything and cannot reach this file through any tool (the sandbox sees
//! no files, the document tools live in the working directory, this lives in
//! the config directory). Each JSONL line carries the SHA-256 of
//! `previous hash + canonical payload`, so editing or deleting a line breaks
//! every hash after it, and a HEAD file beside the log records how many
//! receipts there are and the hash of the last one, so a cut tail is caught
//! too. `tacet log` verifies both on every run and says so out loud.
//!
//! WHAT THIS DOES NOT CLAIM (audit L26). The hash is not keyed: anybody who can
//! write the config directory can rewrite the log AND the head and recompute
//! every hash. The chain catches damage, a hand edit, a deleted line and a cut
//! tail made by something that did not also rewrite the head; it is not proof
//! against the owner of the directory, and `tacet log` does not say it is. A
//! key would have to live somewhere that person cannot read, and on a
//! single-user machine there is no such place. The audit measured the old
//! claim failing: two lines cut off the end, and `tacet log` answered
//! "chain intact · 22 receipts".
//!
//! WHAT A RECEIPT HOLDS (audit L27, RG-32): the time, the chip's KIND (its
//! icon: `shell`, `globe`, `~`…), its outcome and the LENGTH of its text — never
//! the text. A chip reads "Note taken · The user passport number is
//! U1234567" or "searching · <the query>", and the old receipt kept that line
//! verbatim in a file the privacy notice did not mention and `sessions --purge`
//! did not delete. A hash of the text was considered and rejected: an
//! unkeyed SHA-256 of a short chip is a lookup, not a secret — an eight-digit
//! number is 10^8 guesses, seconds of work. Lines written before this change
//! still carry their text; they verify as they were written, and
//! `tacet sessions --purge` now deletes the log with everything else.
//!
//! WRITE FAILURES ARE NON-FATAL. A read-only config directory must not take
//! the assistant down; the receipt is a witness, not a lock. The failure is
//! whispered once per process (the same restraint as the truncation notice).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tacet_kernel::ToolTrace;

use crate::ui::{BOLD, Color, DIM, YELLOW};

const FILE: &str = "receipts.jsonl";
/// `{"count": N, "hash": "<hash of entry N>"}` — what the tail must reach.
const HEAD: &str = "receipts.head";
/// Held while one process appends (audit L28).
const LOCK: &str = "receipts.lock";
/// The chain's anchor: the `prev` of the very first entry.
const GENESIS: &str = "genesis";
/// The line format that carries no chip text. Lines without `v` are the old
/// format, which carried it.
const FORMAT: u64 = 2;

/// How long an append waits for another session's append. An append is one
/// line and one small file; anything that holds the lock this long is dead.
const LOCK_WAIT: Duration = Duration::from_secs(2);
/// A lock older than this is a crashed writer's, and is taken over.
const LOCK_STALE: Duration = Duration::from_secs(5);
const LOCK_POLL: Duration = Duration::from_millis(5);

static WRITE_FAILED_REPORTED: AtomicBool = AtomicBool::new(false);

/// The config directory, where all three files live.
fn dir() -> Option<PathBuf> {
    tacet_kernel::config_path(FILE).and_then(|p| p.parent().map(Path::to_path_buf))
}

/// The files `tacet sessions --purge` removes, in the order they are removed:
/// the log first, so a crash half-way leaves a head that says the log was cut
/// rather than a log with no head to check it.
pub fn files_in(dir: &Path) -> [PathBuf; 3] {
    [dir.join(FILE), dir.join(HEAD), dir.join(LOCK)]
}

/// Deletes the receipt log, its head and a leftover lock. Returns how many
/// receipts went, so the caller can say a number.
pub fn purge_in(dir: &Path) -> Result<usize, String> {
    let count = std::fs::read_to_string(dir.join(FILE))
        .map(|raw| raw.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    // Then what a crash can leave beside them: a take-over guard
    // (`receipts.lock.break.*`) or a head that never got renamed into place
    // (`receipts.head.<pid>.<n>`). Neither holds a word, but a purge that
    // leaves files named "receipts" behind does not look like one.
    let leftovers: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        n.starts_with(&format!("{LOCK}.")) || n.starts_with(&format!("{HEAD}."))
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    for path in files_in(dir).into_iter().chain(leftovers) {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(count)
}

pub fn purge() -> Result<usize, String> {
    match dir() {
        Some(d) => purge_in(&d),
        None => Err("the configuration directory cannot be resolved".to_string()),
    }
}

/// The canonical payload text the hash covers, in the CURRENT format. serde_json's
/// default map is ordered (BTree), so serialising the same fields always yields
/// the same bytes — the property the verifier depends on.
fn canonical(at: u64, kind: &str, chars: u64, state: &str) -> String {
    serde_json::json!({ "at": at, "chars": chars, "state": state, "tool": kind, "v": FORMAT })
        .to_string()
}

/// The payload of a line written before format 2, which carried the chip text.
/// Kept only so those lines still verify.
fn canonical_v1(at: u64, tool: &str, text: &str, state: &str) -> String {
    serde_json::json!({ "at": at, "state": state, "text": text, "tool": tool }).to_string()
}

/// Records one finished chip. Called from the shell after a turn, once per
/// tool trace; the model is not in the loop.
///
/// IT TAKES THE TRACE AND KEEPS ITS SHAPE, NOT ITS WORDS: the text is counted
/// and dropped here, so no caller can hand the log a sentence by passing the
/// wrong field.
pub fn record(at: u64, trace: &ToolTrace) {
    let Some(d) = dir() else { return };
    if record_in(&d, at, trace).is_err() && !WRITE_FAILED_REPORTED.swap(true, Ordering::Relaxed) {
        eprintln!("(the receipt log could not be written — receipts are off for this session)");
    }
}

pub fn record_in(dir: &Path, at: u64, trace: &ToolTrace) -> std::io::Result<()> {
    append_in(
        dir,
        at,
        &crate::ui::one_line(&trace.icon),
        trace.text.chars().count() as u64,
        &format!("{:?}", trace.state),
    )
}

/// One line, under the lock, followed by the head.
///
/// THE LOCK (audit L28). `prev` is read from the file and the line is written
/// after; two sessions finishing a turn at the same moment both read the same
/// last hash and both append, and the log forks into a permanent "CHAIN BROKEN"
/// that nothing ever wrote by hand. The audit saw no fork in 24 parallel runs —
/// the window is the time between one read and one write — but a fork is not
/// repairable, so the window is closed rather than estimated. The lock is a
/// `create_new` file, as in `tacet-memory`'s store, because `File::lock` needs
/// Rust 1.89 and this workspace's floor is 1.85.
fn append_in(dir: &Path, at: u64, kind: &str, chars: u64, state: &str) -> std::io::Result<()> {
    tacet_kernel::fs::create_private_dir(dir)?;
    let _lock = DirLock::acquire(&dir.join(LOCK))?;
    let path = dir.join(FILE);

    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let (count, prev) = tail(&lines);
    // THE HEAD ON DISK IS EVIDENCE, NOT A CACHE (audit L26, found in review).
    // Rebuilding it from the log alone meant the first append after a cut
    // rewrote the head to fit the shortened log: five receipts, two cut off
    // the end, one more tool run, and `tacet log` answered "chain intact, tail
    // matches its head". So the head moves forward only from a log that still
    // reaches it. Otherwise the receipt is still written and the head is left
    // where it was. From then on the log is either shorter than the head
    // (TailCut) or holds a different entry where the head points (Diverged),
    // and that stays true however many receipts follow, until
    // `sessions --purge` starts a new log. A head file that exists but cannot
    // be parsed is kept for the same reason: overwriting it would turn
    // "something is wrong here" into "intact".
    let head_path = dir.join(HEAD);
    let advance = match read_head(dir) {
        Some(head) => {
            judge(lines.len() as u64, |i| line_hash(lines[i]), &head) == HeadVerdict::Reached
        }
        None => !head_path.exists(),
    };
    let payload = canonical(at, kind, chars, state);
    let hash = tacet_web::download::sha256_hex(format!("{prev}{payload}").as_bytes());
    let line = serde_json::json!({
        "at": at, "chars": chars, "state": state, "tool": kind, "v": FORMAT,
        "prev": prev, "hash": hash,
    })
    .to_string();

    let fresh = !path.exists();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    if fresh {
        tacet_kernel::fs::narrow_file(&path);
    }
    writeln!(f, "{line}")?;
    drop(f);

    // AFTER the line, never before: a crash between the two leaves a head one
    // entry behind the log, which `check_head` reads as "the head lags" — the
    // log still reaches it. The other order would leave a head pointing past
    // the end, which is exactly what a cut tail looks like.
    if advance {
        write_head(dir, count + 1, &hash)?;
    }
    Ok(())
}

/// How many entries the log has and the hash of the last one. A log that does
/// not end in a readable line counts what it can and chains from what it last
/// read; the verifier will name the break, and that is its job, not this one's.
fn tail(lines: &[&str]) -> (u64, String) {
    let last = lines
        .last()
        .and_then(|l| line_hash(l))
        .unwrap_or_else(|| GENESIS.to_string());
    (lines.len() as u64, last)
}

/// The `hash` a line claims. Unverified: `append_in` only needs to know where
/// the head points, and `verify` is what checks that the claim is true.
fn line_hash(line: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()?
        .get("hash")?
        .as_str()
        .map(str::to_string)
}

/// Written to a temp name and renamed, so a reader never sees half a head.
/// The name is unique per write, not per process: two threads of one process
/// sharing a temp file would rename it out from under each other.
fn write_head(dir: &Path, count: u64, hash: &str) -> std::io::Result<()> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let body = serde_json::json!({ "count": count, "hash": hash }).to_string();
    let temp = dir.join(format!(
        "{HEAD}.{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    tacet_kernel::fs::write_private(&temp, body.as_bytes())?;
    std::fs::rename(&temp, dir.join(HEAD)).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

fn read_head(dir: &Path) -> Option<(u64, String)> {
    let raw = std::fs::read_to_string(dir.join(HEAD)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some((
        v.get("count")?.as_u64()?,
        v.get("hash")?.as_str()?.to_string(),
    ))
}

/// The append lock: a file created with `create_new`, holding a token that is
/// unique to this acquisition, removed on drop.
///
/// TAKING OVER A STALE LOCK IS WHERE THE RACE WAS (audit L28, found in review).
/// The first version removed any lock older than `LOCK_STALE`. Two waiters
/// could both see a crashed writer's lock as stale; A removed it and created
/// its own, and B, still acting on what it had seen, removed A's fresh lock and
/// created a third. Both then held "the" lock and the chain could fork, for
/// good. Renaming the stale lock away does not close this: B's rename moves A's
/// fresh lock just as easily as its remove did.
///
/// What closes it: the token. A waiter that judges the lock stale remembers
/// the token it read, and before removing anything it takes a second
/// `create_new` file named after THAT token (`take_over`). Under that guard it
/// removes the lock only if the lock still carries the token. Tokens are never
/// reused, so a lock that still carries it is the stale one, and only the
/// guard holder can remove it in the meantime. A waiter that arrives late finds
/// the guard taken, or finds a different token, and leaves the new lock alone.
/// Release goes through the same guard (`Drop`), so a holder that outlived
/// `LOCK_STALE` (stopped, swapped out) does not remove the lock of the session
/// that took over from it.
///
/// WHAT IS LEFT: a guard can go stale too, if a process dies in the
/// microseconds between taking it and removing it. A stale guard is removed
/// plainly, and two waiters racing over THAT is the old race again, one level
/// down. It needs a crash inside a take-over that itself followed a crash.
struct DirLock {
    path: PathBuf,
    token: String,
}

impl DirLock {
    fn acquire(path: &Path) -> std::io::Result<Self> {
        let token = fresh_token();
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match create_exclusive(path) {
                Ok(mut f) => {
                    // A fresh lock cannot be judged stale for LOCK_STALE, so
                    // nobody else can have removed it before this cleanup.
                    if let Err(e) = f.write_all(token.as_bytes()) {
                        drop(f);
                        let _ = std::fs::remove_file(path);
                        return Err(e);
                    }
                    return Ok(Self {
                        path: path.to_path_buf(),
                        token,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if let Some(seen) = stale_token(path) {
                        if take_over(path, &seen) {
                            continue;
                        }
                    }
                    if Instant::now() >= deadline {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "the receipt log is locked by another session",
                        ));
                    }
                    std::thread::sleep(LOCK_POLL);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        // No guard means a waiter is taking this lock over as stale: the lock
        // file is the waiter's to remove now, and it may already be its own.
        let guard = guard_for(&self.path, &self.token);
        if create_exclusive(&guard).is_ok() {
            if std::fs::read_to_string(&self.path).is_ok_and(|t| t == self.token) {
                let _ = std::fs::remove_file(&self.path);
            }
            let _ = std::fs::remove_file(&guard);
        }
    }
}

fn create_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Unique per acquisition, not per process: threads of one process take the
/// lock too, and a token two holders share would let each release the other.
fn fresh_token() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn is_stale(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > LOCK_STALE)
}

/// The token of a lock old enough to be a crashed writer's. A lock left by a
/// version before the token, or by a crash between create and write, is empty,
/// and the empty string is as good a token as any.
fn stale_token(path: &Path) -> Option<String> {
    if !is_stale(path) {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The guard for one token. Named by a hash so that whatever is in a lock file
/// cannot steer the name outside the directory.
fn guard_for(lock: &Path, token: &str) -> PathBuf {
    let mut name = lock.as_os_str().to_owned();
    name.push(".break.");
    name.push(&tacet_web::download::sha256_hex(token.as_bytes())[..16]);
    PathBuf::from(name)
}

/// Removes the lock if, and only if, it still carries `seen`. Returns whether
/// it was removed. See `DirLock` for why the guard is what makes this safe.
fn take_over(lock: &Path, seen: &str) -> bool {
    let guard = guard_for(lock, seen);
    match create_exclusive(&guard) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if is_stale(&guard) {
                let _ = std::fs::remove_file(&guard);
            }
            return false;
        }
        Err(_) => return false,
    }
    // BOTH, not just the token: the empty token is not unique. A lock is empty
    // for the instant between its create and its write, and a waiter that saw
    // an old empty lock would otherwise take that instant for the stale one.
    let still_stale = std::fs::read_to_string(lock).is_ok_and(|t| t == seen) && is_stale(lock);
    if still_stale {
        let _ = std::fs::remove_file(lock);
    }
    let _ = std::fs::remove_file(&guard);
    still_stale
}

/// One verified entry, parsed for display.
struct Entry {
    at: u64,
    tool: String,
    /// The chip text — only on lines written before format 2.
    text: Option<String>,
    /// The chip text's length — only on format-2 lines.
    chars: Option<u64>,
    state: String,
    hash: String,
}

/// Walks the file front to back, recomputing every hash. `Err(k)` = the chain
/// is broken at 1-based entry `k` (edited, deleted in the middle, or written
/// by something that is not this code).
fn verify(raw: &str) -> Result<Vec<Entry>, usize> {
    let mut prev = GENESIS.to_string();
    let mut entries = Vec::new();
    for (i, line) in raw.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let v: serde_json::Value = serde_json::from_str(line).map_err(|_| i + 1)?;
        let field = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let at = v.get("at").and_then(|x| x.as_u64()).ok_or(i + 1)?;
        let (Some(tool), Some(state), Some(line_prev), Some(line_hash)) =
            (field("tool"), field("state"), field("prev"), field("hash"))
        else {
            return Err(i + 1);
        };
        let (payload, text, chars) = match v.get("v").and_then(|x| x.as_u64()) {
            Some(FORMAT) => {
                let chars = v.get("chars").and_then(|x| x.as_u64()).ok_or(i + 1)?;
                // A format-2 line that carries text was not written by this
                // code — and it would be exactly the leak format 2 exists to
                // stop, so it is not shown as a valid receipt.
                if v.get("text").is_some() {
                    return Err(i + 1);
                }
                (canonical(at, &tool, chars, &state), None, Some(chars))
            }
            None => {
                let text = field("text").ok_or(i + 1)?;
                (canonical_v1(at, &tool, &text, &state), Some(text), None)
            }
            Some(_) => return Err(i + 1),
        };
        let expected = tacet_web::download::sha256_hex(format!("{prev}{payload}").as_bytes());
        if line_prev != prev || line_hash != expected {
            return Err(i + 1);
        }
        prev = line_hash.clone();
        entries.push(Entry {
            at,
            tool,
            text,
            chars,
            state,
            hash: line_hash,
        });
    }
    Ok(entries)
}

/// What the head says about the verified entries.
#[derive(Debug, PartialEq, Eq)]
enum HeadVerdict {
    /// The log reaches the head exactly, or one past it (a crash between the
    /// line and the head — see `append_in`).
    Reached,
    /// No head on disk: a log from before the head existed. Its tail cannot be
    /// checked, and the report says so instead of saying "intact".
    Unrecorded,
    /// The head names more entries than the log holds — the tail was cut.
    TailCut { expected: u64, found: u64 },
    /// The entry the head names is not the one in the log.
    Diverged { at: u64 },
}

fn check_head(entries: &[Entry], head: Option<&(u64, String)>) -> HeadVerdict {
    let Some(head) = head else {
        return HeadVerdict::Unrecorded;
    };
    judge(
        entries.len() as u64,
        |i| Some(entries[i].hash.clone()),
        head,
    )
}

/// The one rule for "does this log reach this head", shared by `tacet log`
/// and by `append_in` deciding whether it may move the head. Two copies of it
/// would drift, and the drift would be a log that the writer calls fine and
/// the reader calls cut. `hash_at(i)` is the hash of 0-based entry `i`, asked
/// only for `i < found`.
fn judge(
    found: u64,
    hash_at: impl Fn(usize) -> Option<String>,
    head: &(u64, String),
) -> HeadVerdict {
    let (count, hash) = head;
    if found < *count {
        return HeadVerdict::TailCut {
            expected: *count,
            found,
        };
    }
    if *count == 0 {
        return HeadVerdict::Reached;
    }
    // One past the head is the crash window; more than that means the head
    // stopped being written, which no code path here does without failing.
    if found > count + 1 {
        return HeadVerdict::Diverged { at: *count };
    }
    match hash_at(*count as usize - 1) {
        Some(h) if &h == hash => HeadVerdict::Reached,
        _ => HeadVerdict::Diverged { at: *count },
    }
}

/// `tacet log` — the tail of the chain plus the verification verdict.
pub fn log(json: bool, limit: usize) -> ExitCode {
    let color = Color::setup();
    let Some(d) = dir() else {
        eprintln!("error: the config directory cannot be resolved");
        return ExitCode::FAILURE;
    };
    log_in(&d, json, limit, &color)
}

fn log_in(dir: &Path, json: bool, limit: usize, color: &Color) -> ExitCode {
    let p = dir.join(FILE);
    let raw = std::fs::read_to_string(&p).unwrap_or_default();
    let head = read_head(dir);
    // AN EMPTY LOG WITH A HEAD IS THE WHOLE TAIL CUT, not "no receipts yet".
    if raw.trim().is_empty() && head.as_ref().is_none_or(|(n, _)| *n == 0) {
        println!(
            "{}",
            color.paint(DIM, "no receipts yet — they appear as tools run.")
        );
        return ExitCode::SUCCESS;
    }

    let broken = |what: String, k: Option<usize>| {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "intact": false, "broken_at": k, "reason": what,
                    "path": p.display().to_string(),
                })
            );
        } else {
            eprintln!(
                "{}",
                color.paint(YELLOW, &format!("CHAIN BROKEN — {what} ({})", p.display()))
            );
        }
        ExitCode::FAILURE
    };

    let entries = match verify(&raw) {
        Err(k) => return broken(format!("entry {k} was edited or damaged"), Some(k)),
        Ok(entries) => entries,
    };
    let verdict = check_head(&entries, head.as_ref());
    match verdict {
        HeadVerdict::TailCut { expected, found } => {
            return broken(
                format!(
                    "the log ends after {found} receipts and its head records {expected}: \
                     the last {} were cut",
                    expected - found
                ),
                Some(found as usize + 1),
            );
        }
        HeadVerdict::Diverged { at } => {
            return broken(
                format!("receipt {at} is not the one the head recorded"),
                Some(at as usize),
            );
        }
        HeadVerdict::Reached | HeadVerdict::Unrecorded => {}
    }
    let tail_checked = verdict == HeadVerdict::Reached;

    if json {
        let rows: Vec<serde_json::Value> = entries
            .iter()
            .rev()
            .take(limit)
            .map(|e| {
                let mut row = serde_json::json!({ "at": e.at, "tool": e.tool, "state": e.state });
                if let Some(map) = row.as_object_mut() {
                    if let Some(t) = &e.text {
                        map.insert("text".into(), t.clone().into());
                    }
                    if let Some(c) = e.chars {
                        map.insert("chars".into(), c.into());
                    }
                }
                row
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "intact": true, "tail_checked": tail_checked,
                "total": entries.len(), "receipts": rows,
            })
        );
        return ExitCode::SUCCESS;
    }
    let shown: Vec<&Entry> = entries.iter().rev().take(limit).collect();
    for e in shown.iter().rev() {
        let what = match (&e.text, e.chars) {
            (Some(t), _) => format!("{} · {}", crate::ui::one_line(t), e.state),
            (None, Some(c)) => format!("{} · {c} chars", e.state),
            (None, None) => e.state.clone(),
        };
        println!(
            "  {} {}  {}",
            color.paint(DIM, &format!("{}", e.at)),
            color.paint(BOLD, &crate::ui::one_line(&e.tool)),
            color.paint(DIM, &what)
        );
    }
    // THE VERDICT SAYS WHAT WAS CHECKED. "intact" over a log with no head would
    // claim a tail check that did not happen.
    let verdict_line = if tail_checked {
        format!(
            "  chain intact, tail matches its head · {} receipts · {}",
            entries.len(),
            p.display()
        )
    } else {
        format!(
            "  chain intact · {} receipts · no head recorded yet, so a cut tail would not show · {}",
            entries.len(),
            p.display()
        )
    };
    println!("{}", color.paint(DIM, &verdict_line));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use tacet_kernel::{ToolState, TraceId};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tacet-receipt-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn trace(icon: &str, text: &str) -> ToolTrace {
        ToolTrace {
            id: TraceId(1),
            icon: icon.to_string(),
            text: text.to_string(),
            state: ToolState::Read,
            raw_input: None,
            raw_output: None,
            file_path: None,
        }
    }

    fn verdict(dir: &Path) -> Result<HeadVerdict, usize> {
        let raw = std::fs::read_to_string(dir.join(FILE)).unwrap_or_default();
        let entries = verify(&raw)?;
        Ok(check_head(&entries, read_head(dir).as_ref()))
    }

    #[test]
    fn a_valid_chain_verifies_and_a_tampered_one_breaks() {
        let prev = GENESIS.to_string();
        let payload = canonical_v1(7, "calculate", "12*34 = 408", "Read");
        let hash = tacet_web::download::sha256_hex(format!("{prev}{payload}").as_bytes());
        let line1 = serde_json::json!({
            "at": 7, "state": "Read", "text": "12*34 = 408", "tool": "calculate",
            "prev": prev, "hash": hash,
        })
        .to_string();

        let payload2 = canonical_v1(9, "time", "clock", "Read");
        let hash2 = tacet_web::download::sha256_hex(format!("{hash}{payload2}").as_bytes());
        let line2 = serde_json::json!({
            "at": 9, "state": "Read", "text": "clock", "tool": "time",
            "prev": hash, "hash": hash2,
        })
        .to_string();

        let good = format!("{line1}\n{line2}\n");
        assert_eq!(verify(&good).map(|e| e.len()), Ok(2));

        // Editing entry 1's text breaks the chain AT entry 1.
        let tampered = good.replace("408", "409");
        assert!(matches!(verify(&tampered), Err(1)));
    }

    /// AUDIT L26: two lines cut off the end. The chain alone cannot see it —
    /// every remaining line still hashes to the next — and `tacet log` said
    /// "chain intact · 22 receipts". The head is what remembers the end.
    #[test]
    fn a_cut_tail_is_caught_by_the_head() {
        let dir = temp_dir("tail");
        for i in 0..5 {
            record_in(&dir, i, &trace("calc", "1+1 = 2")).unwrap();
        }
        assert_eq!(verdict(&dir), Ok(HeadVerdict::Reached), "the premise");

        let path = dir.join(FILE);
        let raw = std::fs::read_to_string(&path).unwrap();
        let kept: Vec<&str> = raw.lines().take(3).collect();
        std::fs::write(&path, kept.join("\n") + "\n").unwrap();

        assert_eq!(
            verdict(&dir),
            Ok(HeadVerdict::TailCut {
                expected: 5,
                found: 3
            }),
            "two receipts were cut off the end and nothing noticed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole log emptied is the whole tail cut, not "no receipts yet".
    #[test]
    fn an_emptied_log_with_a_head_is_reported_broken() {
        let dir = temp_dir("emptied");
        record_in(&dir, 1, &trace("calc", "x")).unwrap();
        std::fs::write(dir.join(FILE), "").unwrap();
        let code = log_in(&dir, true, 5, &Color::plain());
        assert_eq!(
            code,
            ExitCode::FAILURE,
            "an emptied log read as a fresh one"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The crash window: the line landed, the head did not. That is not a cut
    /// and must not be reported as one.
    #[test]
    fn a_head_one_entry_behind_is_a_crash_not_a_cut() {
        let dir = temp_dir("lag");
        record_in(&dir, 1, &trace("calc", "a")).unwrap();
        let head = read_head(&dir).unwrap();
        record_in(&dir, 2, &trace("calc", "b")).unwrap();
        write_head(&dir, head.0, &head.1).unwrap();
        assert_eq!(verdict(&dir), Ok(HeadVerdict::Reached));
        // And the next append catches the head up: the crash window is not
        // evidence to keep, it is the one lag the head is allowed.
        record_in(&dir, 3, &trace("calc", "c")).unwrap();
        assert_eq!(read_head(&dir).map(|h| h.0), Some(3));
        assert_eq!(verdict(&dir), Ok(HeadVerdict::Reached));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AUDIT L26, the hole a review found: the next append used to rebuild the
    /// head from the log, so one normal tool run after a cut rewrote the head
    /// to fit and `tacet log` said "chain intact, tail matches its head". The
    /// cut has to stay visible however many receipts follow it.
    #[test]
    fn a_cut_tail_stays_caught_after_more_receipts() {
        let dir = temp_dir("cut-then-append");
        for i in 0..5 {
            record_in(&dir, i, &trace("calc", "1+1 = 2")).unwrap();
        }
        let path = dir.join(FILE);
        let raw = std::fs::read_to_string(&path).unwrap();
        let kept: Vec<&str> = raw.lines().take(3).collect();
        std::fs::write(&path, kept.join("\n") + "\n").unwrap();

        record_in(&dir, 10, &trace("calc", "one more turn")).unwrap();
        assert_eq!(
            verdict(&dir),
            Ok(HeadVerdict::TailCut {
                expected: 5,
                found: 4
            }),
            "one append after the cut wiped the evidence of it"
        );
        for i in 11..14 {
            record_in(&dir, i, &trace("calc", "and more")).unwrap();
        }
        assert_eq!(
            verdict(&dir),
            Ok(HeadVerdict::Diverged { at: 5 }),
            "the cut was forgotten once the log grew back past the head"
        );
        assert_eq!(log_in(&dir, true, 5, &Color::plain()), ExitCode::FAILURE);
        // The receipts themselves were still written: the head is frozen, the
        // log is not.
        let entries = verify(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(entries.len(), 7);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same for a head that is there but cannot be read: rewriting it
    /// would turn "something is wrong" into "intact".
    #[test]
    fn a_head_that_cannot_be_read_is_not_overwritten() {
        let dir = temp_dir("garbled-head");
        record_in(&dir, 1, &trace("calc", "a")).unwrap();
        std::fs::write(dir.join(HEAD), "not a head").unwrap();
        record_in(&dir, 2, &trace("calc", "b")).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(HEAD)).unwrap(),
            "not a head",
            "an unreadable head was replaced by one the log agrees with"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn age(path: &Path) {
        let old = std::time::SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    /// AUDIT L28, the take-over race a review found. Two waiters both saw the
    /// crashed writer's lock as stale; the first removed it and took the lock.
    /// The second, acting on what it saw before, must not remove the FIRST
    /// one's lock — that is two holders and a forked chain.
    #[test]
    fn a_late_taker_leaves_the_new_lock_alone() {
        let dir = temp_dir("takeover");
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join(LOCK);
        std::fs::write(&lock, "a crashed writer").unwrap();
        age(&lock);

        // Both waiters look before either acts.
        let seen_by_a = stale_token(&lock).expect("the premise: the lock is stale");
        let seen_by_b = stale_token(&lock).expect("the premise: the lock is stale");

        assert!(
            take_over(&lock, &seen_by_a),
            "A could not take a stale lock"
        );
        let held_by_a = DirLock::acquire(&lock).unwrap();

        assert!(!take_over(&lock, &seen_by_b), "B took over a lock A holds");
        assert_eq!(
            std::fs::read_to_string(&lock).ok().as_deref(),
            Some(held_by_a.token.as_str()),
            "B removed the lock A had just taken"
        );
        drop(held_by_a);
        assert!(!lock.exists(), "A's release left its lock behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A lock is empty for the instant between its create and its write, and
    /// an old empty lock (a crash in that instant, or a version before tokens)
    /// has the same empty token. A waiter that saw the old one must not take
    /// the new one's instant for it.
    #[test]
    fn an_old_empty_lock_does_not_license_removing_a_fresh_empty_one() {
        let dir = temp_dir("empty-token");
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join(LOCK);
        std::fs::write(&lock, "").unwrap();
        age(&lock);
        let seen = stale_token(&lock).unwrap();
        assert_eq!(seen, "");

        // Someone else took it over and is between create and write.
        std::fs::remove_file(&lock).unwrap();
        std::fs::write(&lock, "").unwrap();

        assert!(!take_over(&lock, &seen), "a fresh lock was taken as stale");
        assert!(lock.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A holder that outlived LOCK_STALE (stopped, swapped out) has been taken
    /// over; when it finally lets go it must not remove its successor's lock.
    #[test]
    fn a_holder_that_outlived_its_lock_leaves_the_successor_alone() {
        let dir = temp_dir("outlived");
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join(LOCK);
        let slow = DirLock::acquire(&lock).unwrap();
        age(&lock);
        let successor = DirLock::acquire(&lock).expect("a stale lock is taken over");
        assert_ne!(slow.token, successor.token);

        drop(slow);
        assert_eq!(
            std::fs::read_to_string(&lock).ok().as_deref(),
            Some(successor.token.as_str()),
            "the slow holder's release removed the successor's lock"
        );
        drop(successor);
        assert!(!lock.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AUDIT L27 / RG-32: a chip's words never reach the receipt log — not a
    /// remembered note, not a search query, not a line of an imported session
    /// transcript. Its kind, outcome and length do.
    #[test]
    fn a_receipt_keeps_the_shape_of_a_chip_and_none_of_its_words() {
        let dir = temp_dir("words");
        let secrets = [
            ("~", "Note taken · The user passport number is U1234567."),
            ("globe", "searching · flights for Ayşe Yılmaz to Ankara"),
            (
                "sessions",
                "imported session 11111111 · \"deploy with AWS_SECRET=abc123 to prod\"",
            ),
        ];
        for (i, (icon, text)) in secrets.iter().enumerate() {
            record_in(&dir, i as u64, &trace(icon, text)).unwrap();
        }
        let raw = std::fs::read_to_string(dir.join(FILE)).unwrap();
        for needle in [
            "U1234567",
            "passport",
            "Ayşe",
            "AWS_SECRET",
            "abc123",
            "deploy",
        ] {
            assert!(
                !raw.contains(needle),
                "the receipt log carries chip text ({needle}):\n{raw}"
            );
        }
        let entries = verify(&raw).expect("the chain verifies");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].tool, "~");
        assert_eq!(entries[0].chars, Some(secrets[0].1.chars().count() as u64));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A log written before format 2 still verifies, and a new receipt chains
    /// onto it — an upgrade must not turn every old log into "CHAIN BROKEN".
    #[test]
    fn an_old_log_still_verifies_and_new_receipts_chain_onto_it() {
        let dir = temp_dir("upgrade");
        std::fs::create_dir_all(&dir).unwrap();
        let payload = canonical_v1(7, "calculate", "12*34 = 408", "Read");
        let hash = tacet_web::download::sha256_hex(format!("{GENESIS}{payload}").as_bytes());
        let old = serde_json::json!({
            "at": 7, "state": "Read", "text": "12*34 = 408", "tool": "calculate",
            "prev": GENESIS, "hash": hash,
        });
        std::fs::write(dir.join(FILE), format!("{old}\n")).unwrap();
        assert_eq!(verdict(&dir), Ok(HeadVerdict::Unrecorded));

        record_in(&dir, 8, &trace("calc", "1+1")).unwrap();
        assert_eq!(verdict(&dir), Ok(HeadVerdict::Reached));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AUDIT L28: appends from several sessions at once do not fork the chain.
    /// Without the lock, each thread reads the same last hash and the log
    /// breaks at the second of them.
    #[test]
    fn appends_from_many_sessions_at_once_keep_one_chain() {
        let dir = temp_dir("race");
        std::fs::create_dir_all(&dir).unwrap();
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    for i in 0..15 {
                        record_in(&dir, t * 100 + i, &trace("calc", "x")).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let raw = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let entries = verify(&raw).unwrap_or_else(|k| panic!("the chain forked at entry {k}"));
        assert_eq!(entries.len(), 120);
        assert_eq!(verdict(&dir), Ok(HeadVerdict::Reached));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `sessions --purge` takes the receipts too, and leaves no head behind to
    /// call the next, fresh log "cut".
    #[test]
    fn purge_removes_the_log_its_head_and_its_lock() {
        let dir = temp_dir("purge");
        record_in(&dir, 1, &trace("calc", "x")).unwrap();
        record_in(&dir, 2, &trace("calc", "y")).unwrap();
        std::fs::write(dir.join(LOCK), "").unwrap();
        // What a crash inside a take-over or a head write leaves behind.
        let guard = guard_for(&dir.join(LOCK), "dead");
        let half_head = dir.join(format!("{HEAD}.1.0"));
        std::fs::write(&guard, "").unwrap();
        std::fs::write(&half_head, "{}").unwrap();
        assert_eq!(purge_in(&dir), Ok(2));
        for f in files_in(&dir).iter().chain([&guard, &half_head]) {
            assert!(!f.exists(), "{} survived the purge", f.display());
        }
        record_in(&dir, 3, &trace("calc", "z")).unwrap();
        assert_eq!(verdict(&dir), Ok(HeadVerdict::Reached));
        assert_eq!(purge_in(&dir), Ok(1));
        assert_eq!(purge_in(&dir), Ok(0), "purging nothing is not an error");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
