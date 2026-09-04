//! `db_write` — CHANGES a SQLite database file the user named at install time.
//!
//! ===========================================================================
//! WHY THIS IS A SECOND TOOL AND NOT A FLAG ON `db`
//! ===========================================================================
//!
//! The requirement is "if write is off, the model must be UNABLE TO GENERATE a
//! write statement". A `write: true` flag on `db` cannot deliver that: the SQL
//! arrives in a free-text field, and a free-text field can always spell
//! `DROP TABLE`. The only gate that holds over free text is ABSENCE — the tool
//! is not in the catalog, so there is no name to call, no grammar branch to
//! emit and no runtime check anyone can forget. That is `shell.rs`'s own rule
//! ("absence is the strongest gate there is"), and it is why `db.rs` is
//! untouched by this file: its `-readonly` lock is intact, measured at its own
//! discovery, and nothing here can reach it.
//!
//! ===========================================================================
//! WHAT IS NOT GUARANTEED, SAID FIRST
//! ===========================================================================
//!
//! ONE STATEMENT ONLY IS NOT A CLAIM THIS FILE MAKES, and it must never become
//! one. Measured on this machine (/usr/bin/sqlite3 3.51.0):
//!
//!   sqlite3 -safe -batch -ascii -header t.db "SELECT 1; DROP TABLE t;"
//!   -> prints 1, exit 0, and `sqlite_master` is EMPTY afterwards
//!
//! The CLI takes a SQL *string* and re-parses it, so statement stacking cannot
//! be stopped at the argv layer at all — the equivalent boundary in the C API
//! (`sqlite3_prepare_v2`'s tail pointer) is not reachable through the binary.
//! A "single statement" check would be a text filter over SQL, which `db.rs`
//! refuses by name and by argument. So the design does not try to bound WHAT
//! the statement does; it bounds WHERE it can do it, and it MEASURES what it
//! did before the user is asked.
//!
//! ALLOWING A FILE ALLOWS EVERYTHING SQL CAN DO TO THAT FILE — the same
//! non-claim `shell.rs` makes about programs. `DROP`, `ALTER`, an `UPDATE` with
//! no `WHERE`, a trigger firing into another table of the same file: all of
//! them are inside the boundary. The boundary is the file.
//!
//! ===========================================================================
//! THE FIVE THINGS THAT BOUND IT, in the order they are hit
//! ===========================================================================
//!
//! 1. THE TOOL IS ABSENT unless the `db` addon is open AND its `writable` list
//!    holds at least one usable path. `with_files(vec![])` is `None`; there is
//!    no "empty means everything" state for anyone to patch later.
//! 2. THE FILE IS A CLOSED SET. `path` is an `ArgSchema::choice` over the
//!    user's own list, so the grammar turns it into a literal alternation and a
//!    file outside the list is not refused — it is UNGENERATABLE. Membership is
//!    checked a second time at run time for the paths where the grammar is off
//!    (eval, a direct call), exactly as `shell.rs` does.
//! 3. THE SANDBOX GATE IS CALLED, NOT REWRITTEN. The chosen path goes through
//!    `sandbox_path::resolve_existing_file`, so an allow-list entry that is a
//!    symlink pointing out of every workspace root is a `SandboxViolation` and
//!    not a write.
//! 4. `-safe` SURVIVES THE LOSS OF `-readonly`, and that is measured at
//!    discovery rather than assumed (`verify_write_lock`), so the statement
//!    cannot reach a SECOND file even though it can do anything to the one it
//!    was given. The four measurements are written out below.
//! 5. THE EFFECT IS MEASURED ON A COPY AND SHOWN TO THE USER, who has to say
//!    yes. `RefuseWrite` is the default sink, so every path that does not
//!    deliberately install a real one refuses.
//!
//! MEASURED ON THIS MACHINE (sqlite3 3.51.0), with `-safe` and NO `-readonly`:
//!
//! ```text
//! ATTACH DATABASE '<outside>' AS x  -> cannot run ATTACH in safe mode, exit 1, no file
//! VACUUM INTO '<outside>'           -> reported as ATTACH, exit 1, no file
//! SELECT writefile('<outside>','x') -> cannot use the writefile() function in safe mode
//! SELECT load_extension('…')        -> no such function: load_extension, in this build
//! ```
//!
//! ===========================================================================
//! WHY THE DRY RUN IS A FILE COPY AND NOT A TRANSACTION
//! ===========================================================================
//!
//! The obvious design is to wrap the model's text as
//! `BEGIN; <statement>; ROLLBACK;` and call the result a preview. THAT DESIGN
//! IS BROKEN BY THE VERY MEASUREMENT ABOVE: the model controls the text inside
//! the wrapper, so it can close the transaction itself. Measured on this
//! machine:
//!
//!   sqlite3 -safe -batch t.db "BEGIN; COMMIT; DROP TABLE t; ROLLBACK;"
//!   -> `cannot rollback - no transaction is active`, exit 1,
//!      and the table is PERMANENTLY GONE
//!
//! A textual wrapper around SQL is a text filter wearing a different hat. The
//! boundary used instead is the one `-safe` really enforces: THE DATABASE FILE
//! IS AN ARGV ARGUMENT. The statement runs, unwrapped, against a COPY; nothing
//! it can say reaches the original, because reaching a second file is the thing
//! `-safe` refuses.
//!
//! THE COPIES LIVE IN THE SYSTEM TEMP DIRECTORY, not in the working directory,
//! and that is deliberate: they hold the user's data, and anything inside the
//! working directory can be read straight back by `db` or `read_document`. The
//! temp directory is outside every sandbox root, so the model cannot open them.
//! They are removed by a `Drop` guard on every path, including the error ones.
//!
//! THE FINGERPRINT PROBES OPEN THE COPIES READ-WRITE. That looks wrong and is
//! not: a WAL database with no `-shm` sidecar CANNOT BE OPENED with `-readonly`
//! at all (measured — error 14; the note is written out at the top of `db.rs`),
//! so a read-only probe would fail on exactly the databases most likely to be
//! in daily use. The files being probed are our own scratch copies, so opening
//! them read-write costs nothing. THE ORIGINAL IS NEVER OPENED FOR READING —
//! `std::fs::copy` reads it as bytes, and the only `sqlite3` process ever
//! pointed at it is the approved commit.
//!
//! ===========================================================================
//! WHAT THE MEASUREMENT IS AND IS NOT
//! ===========================================================================
//!
//! THE DRY RUN AND THE COMMIT ARE TWO PROCESSES, so the shown effect is a
//! PREDICTION, not a guarantee. `INSERT INTO t VALUES(random())`,
//! `datetime('now')`, or another process writing to the file between the two
//! runs all make the committed effect differ from the approved one. It also
//! runs the statement TWICE, so a slow statement costs twice its time and both
//! runs share `TIMEOUT`.
//!
//! THE FINGERPRINT IS `type/name/sql` PER OBJECT, NOT NAMES ALONE, plus a row
//! count per table, plus `journal_mode` and `user_version`. Names alone would
//! MISS the one hostile input nothing else catches: measured here,
//! `PRAGMA writable_schema=ON; UPDATE sqlite_master SET sql='CREATE TABLE t(a,b)'`
//! exits 0 under `-safe` and rewrites the schema while every name stays the
//! same. `journal_mode` is in there for the same class of reason — measured,
//! `PRAGMA journal_mode=WAL` succeeds under `-safe` and permanently changes the
//! file's durability with no schema and no row count moving at all.
//!
//! AND THE FINGERPRINT DOES NOT SEE VALUES. That is the honest half of the same
//! sentence and it was missing: `UPDATE users SET pw='pwned'` over every row
//! moves no object, no count and no pragma, so the fingerprint's answer is
//! EMPTY — measured on this machine, `FINGERPRINT_SQL` and the count query
//! return byte-identical output before and after. A screen that said "nothing
//! measurable moved" there would be describing the tool's own primary use as a
//! no-op. So the empty case is split by a BYTE COMPARISON of the two scratch
//! copies (`same_bytes`), which answers exactly one weaker question — did the
//! statement write anything at all — and the sentence the user reads names
//! which of the two happened. Values themselves are still not diffed: doing
//! that in SQL means a per-column digest over every table, and the honest cheap
//! answer is to say what is not covered rather than to half-cover it.
//!
//! ===========================================================================
//! THE BACKUP, AND WHAT IT COSTS
//! ===========================================================================
//!
//! On approval the untouched copy taken for the dry run is placed beside the
//! database as `<name>.tacet-backup`. It is the EXACT image whose fingerprint
//! the user was shown. It is the only recovery from `writable_schema`, and it
//! is not free:
//!
//!   * it doubles the database on disk;
//!   * it lands INSIDE the working directory, where `db` and `read_document`
//!     can read it back — the copies used for the measurement are hidden from
//!     the model, this one is not, and it has to be, because the user must be
//!     able to find it;
//!   * a SECOND approved write OVERWRITES IT, so it is always "the state before
//!     the last write", never a history;
//!   * on a multi-gigabyte file it is a long silent pause.
//!
//! A WAL DATABASE WITH A NON-EMPTY `-wal` SIDECAR IS REFUSED OUTRIGHT. Its
//! committed data lives partly outside the main file, so `std::fs::copy` of the
//! main file alone is a torn image and the "backup" would be a lie. The two
//! consistent alternatives are both closed by the flags this tool keeps:
//! `.backup` is a dot-command (refused by `-safe`) and `VACUUM INTO` reports as
//! ATTACH (measured). An EMPTY `-wal` is accepted: it means the log was
//! checkpointed and the main file is complete.
//!
//! ===========================================================================
//! THE CONFIRMATION IS NOT THE APPROVAL GATE
//! ===========================================================================
//!
//! `executor.rs`'s gate 3 asks about OUTBOUND data: it fires only for
//! `EXTERNAL_TOOLS` and only in a TAINTED session, and it caches a denial per
//! tool for the rest of the session. Not one of those three properties fits a
//! local, destructive, per-call decision — the first turn of a clean session is
//! exactly when `DROP TABLE` is most likely, and "you said no once, so no
//! forever" is right for a remote server and wrong for a database. So this is a
//! SEPARATE concept, `WriteConfirm`, shaped after `tacet_mcp::InputAsk`: a
//! trait with a deny-by-default implementation, asked once per call, never
//! cached. `db_write` is deliberately NOT added to `EXTERNAL_TOOLS`: SQLite
//! opens no socket, and the reason `shell` IS on that list (a user's allow-list
//! can hold `curl`) has no equivalent here.
//!
//! ON REFUSAL the outcome is `ToolState::NeedsPermission`, built with
//! `ToolOutcome::new`. `outcome.rs` records that the `needs_permission(...)`
//! constructor was deleted because "the approval decision belongs to
//! `ToolExecutor`, not to the tool". That note is about the OUTBOUND gate and
//! it is respected: no constructor is added back, and this file does not
//! bypass, weaken or duplicate `ToolExecutor`'s gate — it answers a question
//! that gate cannot express. The executor already reads this state correctly
//! (`ExecutionReason::ApprovalDenied`, no taint, not an error).
//!
//! ===========================================================================
//! WHERE IT SITS IN THE CATALOG
//! ===========================================================================
//!
//! NOT IN `catalog::production_catalog`. Eval builds its catalog there
//! (`eval_cmd.rs`), and a measurement run must never hold a tool that can
//! change a file. It is added in `tacet-cli`'s `session_catalog` instead, which
//! is the only place that knows whether there is a human at the other end. That
//! also means it is added LAST, after `router::MAX_TOOLS` tools already exist,
//! so on a message with no trigger the router drops it first. That is accepted:
//! a tool that changes a database should never be reached by a message that did
//! not ask for a change.
//!
//! THE `Choice` SET IS THE USER'S OWN FILE PATHS, and they enter the tool
//! schema — therefore the system prompt and the grammar — on every turn while
//! the addon is open. They are RELATIVE paths (`data/app.db`), which is why
//! that cost is small: `Shape::DatabaseFile` refuses an absolute path, so a
//! home directory name and a machine's tree structure never reach the model
//! through this channel. The reasoning for relative-only lives on
//! `addon::WRITABLE_KEY`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult, ToolState,
    TraceUpdate, boxed,
};

use crate::data_store::{SharedStore, Value as StoredValue};
use crate::db::{MODEL_CAP, find_binary, parse_ascii, strip_control};

/// `db::SAFE_ARGS` MINUS `-readonly`, and nothing else changed.
///
/// It lives here rather than beside its read-only sibling on purpose: `db.rs`
/// opens with a hundred lines arguing that read-only is the whole point, and a
/// list with no `-readonly` in it underneath that argument is how the two get
/// mixed up at a call site. `the_read_tool_did_not_lose_its_lock` asserts the
/// exact relationship between the two constants.
pub(crate) const WRITE_ARGS: [&str; 4] = ["-safe", "-batch", "-ascii", "-header"];

/// Bytes kept from one run's stdout. Everything past it is READ AND DISCARDED
/// (see `drain_pipe`) rather than stopping the read: a `RETURNING *` over forty
/// thousand rows is 658 KB (measured), and stopping the read would block the
/// child on a full pipe — which for the commit run means blocking `sqlite3` in
/// the middle of a transaction.
const OUTPUT_CAP: usize = 256 * 1024;

/// Bytes kept from stderr. An engine message is a sentence.
const ERROR_CAP: usize = 4 * 1024;

/// The wall clock one `sqlite3` run gets.
///
/// LONGER THAN `db.rs`'s 15s and for a stated reason: a write does more work
/// than a read, and every statement here runs TWICE (once on the copy, once for
/// real), so a user's whole cost is up to twice this number plus the copying.
/// A statement slower than this is simply unavailable through this tool.
///
/// ON TIMEOUT THE CHILD IS KILLED. For the dry run that is free — it dies over
/// a scratch copy. For the commit it relies on SQLite's rollback journal to
/// leave the file consistent, which is what that journal is for; NOT MEASURED
/// here, because provoking a kill at a chosen point inside a commit is not
/// something this test suite can do deterministically.
const TIMEOUT: Duration = Duration::from_secs(60);

/// The wait loop's tick and the grace given to the pipe readers afterwards.
/// Both numbers are `shell.rs`'s, which already paid for them.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const JOIN_GRACE: Duration = Duration::from_secs(2);

/// What the pre-write copy is called, beside the database.
const BACKUP_SUFFIX: &str = ".tacet-backup";

/// Is there a sidecar with this suffix beside `database`, and does it hold
/// bytes?
///
/// A missing one and a 0-byte one are the same answer — no — because both mean
/// the main file is complete on its own. Only a NON-EMPTY sidecar says part of
/// the truth lives outside the bytes `std::fs::copy` would take.
fn non_empty_sidecar(database: &Path, suffix: &str) -> bool {
    let side = PathBuf::from(format!("{}{suffix}", database.display()));
    std::fs::metadata(&side)
        .map(|m| m.len() > 0)
        .unwrap_or(false)
}

/// Everything that identifies the state of a database, in one query.
///
/// `type||'/'||name||'/'||coalesce(sql,'')` AND NOT NAMES ALONE — see the
/// module note on `writable_schema`. `journal_mode` and `user_version` are the
/// two persistent per-file properties that no schema row and no row count would
/// show moving.
const FINGERPRINT_SQL: &str = "SELECT 'pragma/journal_mode/'||(SELECT journal_mode FROM pragma_journal_mode()) AS o \
     UNION ALL SELECT 'pragma/user_version/'||(SELECT user_version FROM pragma_user_version()) \
     UNION ALL SELECT type||'/'||name||'/'||coalesce(sql,'') FROM sqlite_master ORDER BY 1;";

// ---------------------------------------------------------------------------
// The confirmation — a SEPARATE concept from the outbound approval gate
// ---------------------------------------------------------------------------

/// What the user is being asked to approve. Every field is already measured;
/// nothing here is a guess about what the statement "probably" does.
pub struct WriteRequest<'a> {
    /// The database's file name, as the user wrote it in the allow-list.
    pub file: &'a str,
    /// The model's SQL, VERBATIM. It is data on this screen — the sink must
    /// sanitise it before printing (a `\r` in it would otherwise rewrite the
    /// sentence describing what is about to happen).
    pub statement: &'a str,
    /// The measured difference between the copy before and the copy after, one
    /// change per line. Empty-effect statements say so in words.
    pub effect: &'a str,
    /// Where the pre-write image will be placed if the answer is yes.
    pub backup: &'a str,
}

/// Asks the user whether a measured change may be applied.
///
/// SHAPED AFTER `tacet_mcp::InputAsk`/`DeclineInput`: a trait plus a
/// deny-by-default implementation, so a call site that forgot to install a real
/// sink REFUSES rather than writes. It is not `ApprovalGate` — see the module
/// note; that gate answers "may this data leave the machine", which is a
/// different question with different firing rules.
pub trait WriteConfirm: Send + Sync {
    fn confirm(&self, request: &WriteRequest<'_>) -> bool;
}

/// The default sink: NO, always.
///
/// This is what every non-interactive path gets — eval never sees the tool at
/// all, and `tacet why`/`tacet tools`/`tacet grammar` build a catalog with this
/// installed. A sink that read stdin in those paths would block a piped run
/// forever on a question nobody can see.
pub struct RefuseWrite;

impl WriteConfirm for RefuseWrite {
    fn confirm(&self, _request: &WriteRequest<'_>) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Running the binary
// ---------------------------------------------------------------------------

/// What a reader thread collected. Shared rather than returned so the main path
/// can abandon the thread and still keep what was read.
#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    truncated: bool,
}

/// Reads to EOF, STORES up to `cap`.
///
/// `shell.rs::read_pipe`'s discipline, written again rather than shared because
/// that one is welded to `shell`'s own 20 000-byte constant and this file needs
/// 256 KiB. The property that matters is the same and it is the reason
/// `db.rs::run_query` could NOT be reused for writes: that function stops
/// reading at the cap and KILLS the child, which for a commit means killing
/// `sqlite3` in the middle of a transaction and then reporting failure.
fn drain_pipe<R: Read>(pipe: Option<R>, sink: &Mutex<Sink>, cap: usize) {
    let Some(mut pipe) = pipe else { return };
    let mut buffer = [0u8; 8192];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut sink = sink.lock().expect("pipe sink lock");
                let slot = cap.saturating_sub(sink.bytes.len());
                if slot == 0 {
                    sink.truncated = true;
                    continue;
                }
                let taken = n.min(slot);
                sink.bytes.extend_from_slice(&buffer[..taken]);
                if taken < n {
                    sink.truncated = true;
                }
            }
        }
    }
}

/// Takes what a reader collected. A poisoned lock counts as "nothing was read":
/// a helper thread must not take the turn down with it.
fn take(sink: &Mutex<Sink>) -> (String, bool) {
    let Ok(sink) = sink.lock() else {
        return (String::new(), false);
    };
    (
        String::from_utf8_lossy(&sink.bytes).into_owned(),
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

/// The outcome of one `sqlite3` run.
struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
    /// The output was longer than `OUTPUT_CAP`; the rest was read and dropped.
    truncated: bool,
    timed_out: bool,
}

/// Runs `sqlite3 -safe -batch -ascii -header <database> <sql>`.
///
/// NO SHELL: `Command` is given an argument ARRAY, so a statement holding a
/// space, a quote or a `;` stays exactly one argument. stdin is closed so the
/// binary can never sit waiting on a terminal a chat turn does not have.
fn run_sqlite(binary: &Path, database: &Path, sql: &str) -> std::io::Result<Run> {
    let mut child = Command::new(binary)
        .args(WRITE_ARGS)
        .arg(database)
        .arg(sql)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let out = child.stdout.take();
    let err = child.stderr.take();
    let out_sink = Arc::new(Mutex::new(Sink::default()));
    let err_sink = Arc::new(Mutex::new(Sink::default()));
    let out_arm = {
        let sink = Arc::clone(&out_sink);
        std::thread::spawn(move || drain_pipe(out, &sink, OUTPUT_CAP))
    };
    let err_arm = {
        let sink = Arc::clone(&err_sink);
        std::thread::spawn(move || drain_pipe(err, &sink, ERROR_CAP))
    };

    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {}
            // The poll failed, so we no longer know this process. Killing it
            // and leaving beats leaving something we cannot account for.
            Err(_) => {
                timed_out = true;
                break None;
            }
        }
        if start.elapsed() >= TIMEOUT {
            timed_out = true;
            break None;
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    if timed_out {
        let _ = child.kill();
        let _ = child.wait();
    }
    join_before(out_arm, JOIN_GRACE);
    join_before(err_arm, JOIN_GRACE);
    let (stdout, truncated) = take(&out_sink);
    let (stderr, _) = take(&err_sink);

    Ok(Run {
        ok: !timed_out && status.is_some_and(|s| s.success()),
        stdout,
        stderr,
        truncated,
        timed_out,
    })
}

/// The first line of the engine's message, control characters removed. A
/// multi-line error would break the one-line chip contract and a raw `ESC`
/// inside it would repaint the terminal.
fn first_line(text: &str) -> String {
    strip_control(text.lines().next().unwrap_or("").trim())
}

// ---------------------------------------------------------------------------
// Fingerprinting a file
// ---------------------------------------------------------------------------

/// Everything measured about one database image.
#[derive(Default, Clone, PartialEq, Eq)]
struct Fingerprint {
    /// `type/name/sql` for every schema object, plus two `pragma/...` rows.
    objects: Vec<String>,
    /// One `(table, rows)` pair per ordinary table.
    counts: Vec<(String, String)>,
    /// Did the count query run at all. A virtual table whose module is missing
    /// makes `count(*)` fail, and reporting "0 rows" for that would be a lie.
    counted: bool,
    /// Did `sqlite3` open the file and answer the schema query.
    ///
    /// `false` IS NOT "EMPTY DATABASE". A statement can leave a file that is no
    /// longer a database at all, and without this flag the empty `objects` of an
    /// unreadable image would be diffed against a full one and reported as "every
    /// table was REMOVED" — a specific, confident, wrong sentence on the one
    /// screen that must not carry one.
    readable: bool,
    /// Did both queries fit inside `OUTPUT_CAP`.
    ///
    /// SAME CLASS AS `readable`, and it was missing. `drain_pipe` stops STORING
    /// at 256 KiB and sets `Run::truncated`; `fingerprint` used to read only
    /// `ok` and `stdout`, so on a database whose `sqlite_master` text is larger
    /// than that the object list was silently SHORT. Two wrong sentences follow
    /// from that: an object past the cut can be dropped with `describe` saying
    /// nothing, and a single added object early in the `ORDER BY 1` sort shifts
    /// the cut and turns the tail into a run of `REMOVED:` lines for objects
    /// that are still there. The tool already surfaces the same condition for
    /// the statement's own output; there is no reason for the measurement to be
    /// quieter than the output.
    complete: bool,
}

/// Splits a `type/name/sql` row. `sql` may itself contain `/`, so only the
/// first two separators are honoured.
fn split_object(row: &str) -> (&str, &str, &str) {
    let mut parts = row.splitn(3, '/');
    (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    )
}

/// Reads the whole measurable state of one database image.
///
/// TWO PROCESSES: the schema/pragma query, then a count query built from the
/// table names it returned (the names have to be known before the counts can be
/// asked for, and the CLI has no dynamic SQL). Four processes per confirmation,
/// plus the trial run, plus the commit — six for an approved write, five for a
/// refused one, on top of the three `verify_write_lock` probes at discovery.
fn fingerprint(binary: &Path, file: &Path) -> Fingerprint {
    let Ok(schema) = run_sqlite(binary, file, FINGERPRINT_SQL) else {
        return Fingerprint::default();
    };
    if !schema.ok {
        return Fingerprint::default();
    }
    let objects: Vec<String> = parse_ascii(&schema.stdout)
        .rows
        .into_iter()
        .filter_map(|row| row.into_iter().next())
        .collect();

    // ORDINARY TABLES ONLY. `sqlite_*` names are the engine's own bookkeeping
    // and some of them cannot be counted at all; a view's count would run the
    // view's query, which is work the user did not ask for.
    let tables: Vec<String> = objects
        .iter()
        .filter_map(|o| {
            let (kind, name, _) = split_object(o);
            (kind == "table" && !name.starts_with("sqlite_")).then(|| name.to_string())
        })
        .collect();
    if tables.is_empty() {
        return Fingerprint {
            objects,
            counts: Vec::new(),
            counted: true,
            readable: true,
            complete: !schema.truncated,
        };
    }

    // THE NAMES COME FROM `sqlite_master`, NOT FROM THE MODEL — but they are
    // still quoted properly, because a table can legitimately be named with a
    // quote in it and an unquoted identifier would turn that into a syntax
    // error at best. `"` doubles inside an identifier, `'` doubles inside a
    // string literal.
    let counts_sql = tables
        .iter()
        .map(|t| {
            format!(
                "SELECT '{}' AS t, count(*) AS n FROM \"{}\"",
                t.replace('\'', "''"),
                t.replace('"', "\"\"")
            )
        })
        .collect::<Vec<_>>()
        .join(" UNION ALL ")
        + " ORDER BY 1;";
    let Ok(counted) = run_sqlite(binary, file, &counts_sql) else {
        return Fingerprint {
            objects,
            counts: Vec::new(),
            counted: false,
            readable: true,
            complete: !schema.truncated,
        };
    };
    if !counted.ok {
        return Fingerprint {
            objects,
            counts: Vec::new(),
            counted: false,
            readable: true,
            complete: !schema.truncated,
        };
    }
    let counts = parse_ascii(&counted.stdout)
        .rows
        .into_iter()
        .filter(|r| r.len() >= 2)
        .map(|r| (r[0].clone(), r[1].clone()))
        .collect();
    Fingerprint {
        objects,
        counts,
        counted: true,
        readable: true,
        complete: !schema.truncated && !counted.truncated,
    }
}

/// Do these two files hold the same bytes? `None` if either could not be read.
///
/// WHY A BYTE COMPARISON EXISTS BESIDE THE FINGERPRINT: the fingerprint is
/// blind to values (see `describe`), so it cannot tell "the statement did
/// nothing" from "the statement rewrote every row". The two scratch copies are
/// identical the instant before the trial run, so after it any difference in
/// their bytes is the statement's — SQLite writing a page it did not need to is
/// still the statement having written. It answers a WEAKER question than the
/// fingerprint (did anything change at all) and it answers it exactly, which is
/// the pair that makes the empty-effect sentence honest.
///
/// THE COST IS ONE MORE READ OF EACH COPY. That is the same order as the two
/// `std::fs::copy` calls already made, and it stops at the first differing
/// block, so the common case (a statement that wrote something early) reads far
/// less than the file.
fn same_bytes(a: &Path, b: &Path) -> Option<bool> {
    let (a_len, b_len) = (
        std::fs::metadata(a).ok()?.len(),
        std::fs::metadata(b).ok()?.len(),
    );
    if a_len != b_len {
        return Some(false);
    }
    let (mut a, mut b) = (std::fs::File::open(a).ok()?, std::fs::File::open(b).ok()?);
    let (mut left, mut right) = ([0u8; 64 * 1024], [0u8; 64 * 1024]);
    loop {
        let n = a.read(&mut left).ok()?;
        // Equal lengths, so the second read returning less is a file changing
        // under us; there is no honest answer then and `None` says so.
        let m = b.read(&mut right[..n]).ok()?;
        if m != n {
            return None;
        }
        if n == 0 {
            return Some(true);
        }
        if left[..n] != right[..n] {
            return Some(false);
        }
    }
}

/// The difference between two images, one change per line.
///
/// WHAT IT COVERS IS THE SCHEMA, THE ROW COUNTS AND THE DURABILITY PRAGMAS —
/// AND NOTHING ELSE. It does NOT look at the values inside existing rows, so
/// `UPDATE users SET pw='x'` over every row produces an EMPTY result. Measured
/// on this machine (/usr/bin/sqlite3 3.51.0) on `users(id, pw)` with two rows:
/// after that statement `FINGERPRINT_SQL`'s output and the count query's output
/// are byte-identical to before, while `SELECT * FROM users` shows both
/// passwords rewritten. An empty result is therefore NOT "the statement did
/// nothing" — `write` runs a byte comparison of the two copies for exactly that
/// distinction and the sentence the user is shown names which of the two it is.
fn describe(before: &Fingerprint, after: &Fingerprint) -> Vec<String> {
    let mut lines = Vec::new();
    // AN UNREADABLE IMAGE IS ITS OWN SENTENCE, NOT A DIFF. Falling through to
    // the comparison below would report an unopenable file as "every object
    // REMOVED", which is a confident wrong claim on the one screen that must not
    // carry one.
    if !after.readable {
        lines.push(
            "THE FILE COULD NOT BE READ BACK after the statement: sqlite3 no longer opens the \
             trial copy as a database. Nothing below could be measured."
                .to_string(),
        );
        return lines;
    }
    if !before.readable {
        lines.push(
            "the file could not be read before the statement either: sqlite3 does not open it as \
             a database, so there is nothing to compare against."
                .to_string(),
        );
        return lines;
    }
    let key = |row: &str| {
        let (kind, name, _) = split_object(row);
        format!("{kind} {name}")
    };
    let body = |row: &str| split_object(row).2.to_string();

    for old in &before.objects {
        let k = key(old);
        match after.objects.iter().find(|n| key(n) == k) {
            None => lines.push(format!("REMOVED: {k}")),
            Some(new) if body(new) != body(old) => {
                let (kind, name, _) = split_object(old);
                if kind == "pragma" {
                    // The durability class of change: shown as a value move
                    // rather than as "redefined", because `delete -> wal` is
                    // the whole message.
                    lines.push(format!("{name}: {} -> {}", body(old), body(new)));
                } else {
                    lines.push(format!(
                        "REDEFINED: {k} (its CREATE statement text changed)"
                    ));
                }
            }
            Some(_) => {}
        }
    }
    for new in &after.objects {
        let k = key(new);
        if !before.objects.iter().any(|o| key(o) == k) {
            lines.push(format!("added: {k}"));
        }
    }
    for (table, old) in &before.counts {
        if let Some((_, new)) = after.counts.iter().find(|(t, _)| t == table)
            && new != old
        {
            lines.push(format!("rows in {table}: {old} -> {new}"));
        }
    }
    if !before.counted || !after.counted {
        lines.push(
            "(the row counts could not be taken on one of the copies; only the schema was \
             compared)"
                .to_string(),
        );
    }
    // AN INCOMPLETE MEASUREMENT SAYS SO, and it says so LAST, where the eye
    // ends: everything above it may be missing entries, and a `REMOVED:` line
    // above it may be an artefact of the cut moving rather than an object going
    // away. NOT MEASURED: no test here builds a 256 KiB schema; the flag is
    // threaded from the same `truncated` the statement's own output already
    // reports.
    if !before.complete || !after.complete {
        lines.push(
            "(the schema of one of the copies is larger than the 256 KiB this reads back, so the \
             comparison above is INCOMPLETE — treat a line naming an object as a hint, not as the \
             whole list)"
                .to_string(),
        );
    }
    lines
}

// ---------------------------------------------------------------------------
// Discovery — `-safe` WITHOUT `-readonly` is measured, not assumed
// ---------------------------------------------------------------------------

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn scratch_name(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "tacet-db-write-{tag}-{}-{}",
        std::process::id(),
        nanos()
    ))
}

/// Proves that THIS binary still refuses to leave its file when `-readonly` is
/// gone.
///
/// `db.rs::verify_lock` is deliberately NOT extended with this probe and not
/// called from here. That function is what `DbTool::discover` gates on, so a
/// third probe inside it would make the READ-ONLY tool disappear whenever a
/// WRITE property could not be measured — the exact regression this feature
/// must not cause. The cost of the split is stated: four extra `:memory:`
/// processes per `DbWriteTool::discover`, and ZERO added to `DbTool::discover`.
///
/// A. `-safe -batch … "SELECT 1;"` must succeed and print 1. Proves the options
///    are UNDERSTOOD without `-readonly`; an unknown option makes sqlite3 exit
///    with "unknown option" and print nothing.
/// B. ATTACH must not create a file. This is the probe that matters: with
///    `-readonly` gone, ATTACH is the one construct that could reach a SECOND
///    file, and the whole design rests on `-safe` refusing it.
/// C. `VACUUM INTO` must not create a file. It reaches a second file WITHOUT
///    naming ATTACH, and this probe was missing while the README told the reader
///    all three constructs were "measured at startup". It was not: VACUUM INTO
///    had been measured once, here, on this author's sqlite3 3.51.0 — and the
///    whole reason this function exists is that the user's binary is not the
///    author's. Now it is measured on theirs. (On 3.51.0 it comes back as
///    `cannot run ATTACH in safe mode`, exit 1, no file.)
/// D. `writefile()` must not create a file. The other reach-a-second-file
///    route, and the one that needs no second database.
///
/// B ALONE WOULD BE WORTHLESS — a binary that rejects `-safe` outright also
/// fails to create the file. It is the PAIR with A that separates "understood
/// and enforced" from "not understood".
fn verify_write_lock(binary: &Path) -> bool {
    let works = Command::new(binary)
        .args(WRITE_ARGS)
        .arg(":memory:")
        .arg("SELECT 1;")
        .stdin(Stdio::null())
        .output();
    let Ok(works) = works else {
        return false;
    };
    if !works.status.success() || !String::from_utf8_lossy(&works.stdout).contains('1') {
        return false;
    }

    for tag in ["attach", "vacuum", "writefile"] {
        let target = scratch_name(tag);
        let Some(text) = target.to_str() else {
            return false;
        };
        // A `'` in the temp directory's own path would break out of the string
        // literal below. We cannot measure on such a machine, so there is no
        // tool on it — the fail-closed direction.
        if text.contains('\'') {
            return false;
        }
        let statement = match tag {
            "attach" => format!("ATTACH DATABASE '{text}' AS probe; CREATE TABLE probe.a(b);"),
            "vacuum" => format!("VACUUM INTO '{text}';"),
            _ => format!("SELECT writefile('{text}', 'x');"),
        };
        let ran = Command::new(binary)
            .args(WRITE_ARGS)
            .arg(":memory:")
            .arg(statement)
            .stdin(Stdio::null())
            .output();
        let escaped = target.exists();
        let _ = std::fs::remove_file(&target);
        if escaped || ran.is_err() {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// The scratch directory
// ---------------------------------------------------------------------------

/// A temporary directory that removes itself.
///
/// A `Drop` guard rather than a cleanup call at each `return`: this function has
/// nine early exits and the copies hold the user's data.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> std::io::Result<Scratch> {
        let path = scratch_name("copy");
        std::fs::create_dir_all(&path)?;
        Ok(Scratch { path })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// The tool
// ---------------------------------------------------------------------------

pub struct DbWriteTool {
    binary: PathBuf,
    /// Sorted, unique, never empty (see `with_files`).
    allowed: Vec<String>,
    /// Built once: it is asked for on every turn (prompt + grammar).
    schema_cache: Mutex<Option<ArgSchema>>,
    description: String,
    confirm: Arc<dyn WriteConfirm>,
    store: Option<Arc<SharedStore>>,
}

impl DbWriteTool {
    /// The production entry point: the tool EXISTS only if the `db` addon is
    /// installed, OPEN, and carries a usable writable list.
    ///
    /// ONE READ OF THE REGISTRY, the shape `shell.rs::discover` records: asking
    /// `addon::is_open` and then reading the settings separately would let the
    /// two answers come from two different versions of the file — a file list
    /// from before an edit, judged open by the state after it. A corrupt
    /// registry means no tool (`read().ok()?`).
    pub fn discover() -> Option<DbWriteTool> {
        use tacet_web::addon;
        let record = addon::read().ok()?;
        DbWriteTool::from_record(&record)
    }

    /// The gate itself, with the registry HANDED IN.
    ///
    /// SPLIT OUT SO ABSENCE CAN BE MEASURED. `discover` reads the machine's real
    /// `addons.json`, and `catalog.rs` records why a test must never do that: the
    /// result would depend on the developer's own registry, or the test would
    /// have to move the process-wide `TACET_HOME` and step on its neighbours.
    /// This function is the same three questions — installed, open, list — asked
    /// of a record a test can build.
    pub fn from_record(record: &tacet_web::addon::Record) -> Option<DbWriteTool> {
        use tacet_web::addon;
        let entry = record.find(addon::DB)?;
        if !entry.open {
            return None;
        }
        DbWriteTool::with_files(
            entry
                .values(addon::WRITABLE_KEY)
                .into_iter()
                .map(str::to_string)
                .collect(),
        )
    }

    /// The list given directly — the tests and any caller that already read the
    /// registry.
    ///
    /// `None` FOR AN EMPTY LIST, and this is the load-bearing line of the file:
    /// a tool with no list would be a tool that can write nothing, which sooner
    /// or later invites a patch making "empty" mean "any database". There is no
    /// such state to patch.
    ///
    /// THE SHAPE IS ASKED, NOT REIMPLEMENTED. `addon::Shape::DatabaseFile` owns
    /// what a writable entry looks like; the registry is a JSON file a human can
    /// edit after the installer has had its say, so the same rule is asked again
    /// here — an absolute path hand-written into `addons.json` never reaches the
    /// `Choice` set, and therefore is not something the model can name.
    pub fn with_files(files: Vec<String>) -> Option<DbWriteTool> {
        let mut allowed: Vec<String> = files
            .iter()
            .map(|f| f.trim().to_string())
            .filter(|f| tacet_web::addon::Shape::DatabaseFile.check(f).is_ok())
            .collect();
        allowed.sort();
        allowed.dedup();
        if allowed.is_empty() {
            return None;
        }
        // AND THE BINARY MUST BE THERE AND MEASURE CLEAN. The same fail-closed
        // direction `db.rs` takes: on a machine where `-safe` cannot be proved
        // to hold without `-readonly`, this tool does not exist.
        let binary = find_binary()?;
        if !verify_write_lock(&binary) {
            return None;
        }
        // THE FILE LIST IS IN THE DESCRIPTION. The model picks a tool from name
        // + description; without the list it cannot tell whether a change is
        // answerable here, and a call that fails the membership check has
        // already cost a turn.
        //
        // THE DESCRIPTION MUST NOT IMPLY "ONE STATEMENT". It says the opposite,
        // because the opposite is what was measured.
        let description = format!(
            "Changes data in one of these SQLite database files: {}. Use it ONLY when the user \
             asks for data to be inserted, updated or deleted. The whole `statement` text is run \
             — if it holds several statements separated by ';' they ALL run — so write exactly \
             what you mean. Every call is first tried on a copy, and the measured effect is shown \
             to the user, who must say yes before anything is written; if they say no, nothing \
             happened and you must not try a different wording. Only the files listed above can \
             be changed. Use `db` for reading.",
            allowed.join(", ")
        );
        Some(DbWriteTool {
            binary,
            allowed,
            schema_cache: Mutex::new(None),
            description,
            confirm: Arc::new(RefuseWrite),
            store: None,
        })
    }

    /// Why the tool is on or off — printed by the shell when the addon is open
    /// and no write tool appeared. A silent absence looks like a missing
    /// feature rather than an empty list or a missing package.
    pub fn diagnose() -> String {
        let Some(binary) = find_binary() else {
            return "db_write is off: no sqlite3 binary was found in the known locations."
                .to_string();
        };
        if !verify_write_lock(&binary) {
            return format!(
                "db_write is off: {} was found but safe mode could not be proved to hold without \
                 -readonly — ATTACH or writefile() reached a second file, or -safe is not \
                 supported (it needs SQLite 3.34 or newer). No statement is run without that \
                 measurement.",
                binary.display()
            );
        }
        "db_write is off: the `db` addon's writable list is empty or holds no usable path. \
         `tacet addon install db` records one path per line, RELATIVE to the project folder \
         (data/app.db). With an empty list the tool is not in the catalog at all — which is the \
         default, and the point."
            .to_string()
    }

    pub fn with_confirm(mut self, confirm: Arc<dyn WriteConfirm>) -> Self {
        self.confirm = confirm;
        self
    }

    pub fn with_store(mut self, store: Arc<SharedStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// The files this tool may change. The diagnostic path reads it.
    pub fn allowed(&self) -> &[String] {
        &self.allowed
    }

    /// EXACT MATCH, case included — `shell.rs::permits`'s rule and its reason: a
    /// case-insensitive compare has been a hole in this repository once, and
    /// forgiveness here would widen what the user's list means on a
    /// case-insensitive filesystem without buying anything.
    fn permits(&self, path: &str) -> bool {
        self.allowed.iter().any(|a| a == path)
    }
}

impl Tool for DbWriteTool {
    fn name(&self) -> &str {
        "db_write"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> ArgSchema {
        if let Some(s) = self.schema_cache.lock().expect("schema lock").clone() {
            return s;
        }
        // `path` IS A `Choice`, NOT A `Text`. This is the structural gate: the
        // grammar turns a closed set into a literal alternation, so a database
        // outside the user's list is not something the model is refused for
        // asking — it is something it CANNOT EMIT. A `Text` field guarded by a
        // check afterwards would be the same rule enforced one layer too late.
        let schema = ArgSchema::object(vec![
            Field::new(
                "path",
                ArgSchema::choice(self.allowed.iter().map(String::as_str))
                    .description("Which of the allowed database files to change."),
            )
            .required(),
            Field::new(
                "statement",
                ArgSchema::text().description(
                    "The SQL that changes the data, e.g. \"UPDATE orders SET status='paid' WHERE \
                     id=7\". It is run as written.",
                ),
            )
            .required(),
        ])
        .description("Change data in an allowed SQLite database");
        *self.schema_cache.lock().expect("schema lock") = Some(schema.clone());
        schema
    }

    /// TRUE. The measurement reads the user's own tables — row counts, table
    /// names, the schema text — and puts them in the window; once they are there
    /// a later outgoing call could carry them off the device.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let trace = ctx.start_chip("database", "Measuring the change…");
            let outcome = match self.write(&args, ctx) {
                Ok(outcome) => outcome,
                Err(e) => ToolOutcome::failed(&e),
            };
            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(args.to_string())
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            outcome
        })
    }
}

impl DbWriteTool {
    /// The synchronous body — free of the async wrapper so it is testable
    /// directly.
    fn write(&self, args: &Value, ctx: &ToolContext) -> ToolResult<ToolOutcome> {
        // THE MEMBERSHIP CHECK RUNS BEFORE THE SCHEMA VALIDATION, for
        // `shell.rs`'s reason: a schema failure returns the one fixed sentence
        // every tool error returns, and from that the model learns only "something
        // was wrong". The one thing it needs here is WHICH files it may change.
        let requested = args.get("path").and_then(Value::as_str).unwrap_or_default();
        if !requested.is_empty() && !self.permits(requested) {
            let shown = strip_control(requested);
            return Ok(ToolOutcome::new(
                format!("'{shown}' is not a writable database"),
                ToolState::Failed(format!("'{shown}' is not writable")),
                format!(
                    "error: '{shown}' is not one of the databases the user allowed to be changed. \
                     Only these can be: {}. Tell the user which file they would have to allow.",
                    self.allowed.join(", ")
                ),
            ));
        }

        self.schema().validate(args)?;
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| ToolError::MissingField("path".into()))?;
        let statement = args
            .get("statement")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::MissingField("statement".into()))?;

        // BELT TO THE `-safe` BRACES, AND LABELLED AS SUCH — the same line
        // `db.rs` carries and for the same reason: an argument beginning with
        // `.` goes to sqlite3's META-COMMAND parser rather than its SQL parser,
        // and there is no flag that says "this argument is SQL". Safe mode
        // already refuses every dangerous meta-command; this catches the
        // harmless remainder with a sentence the model can act on.
        if statement.starts_with('.') {
            return Err(ToolError::InvalidArgument(
                "the statement must be SQL, not a sqlite3 dot-command".into(),
            ));
        }

        // THE SANDBOX GATE — CALLED, NOT REWRITTEN. An allow-list entry that is
        // a symlink to a database outside every workspace root dies here, as a
        // `SandboxViolation` rather than as a write.
        let database = crate::sandbox_path::resolve_existing_file(ctx, path)?;
        let file = database
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "database".into());

        // A NON-EMPTY `-wal` MEANS COMMITTED DATA LIVES OUTSIDE THE MAIN FILE,
        // so a byte copy of the main file is a torn image and both the trial and
        // the backup would be lies. An EMPTY one means the log was checkpointed.
        if non_empty_sidecar(&database, "-wal") {
            return Err(ToolError::Other(format!(
                "'{file}' is in WAL mode and its write-ahead log is not empty, so part of its \
                 committed data is outside the file itself. A consistent copy of that is not \
                 reachable through `sqlite3 -safe` (.backup is a dot-command, VACUUM INTO reports \
                 as ATTACH), and without a copy neither the trial run nor the backup would be \
                 honest. Nothing was written."
            )));
        }
        // THE SAME ARGUMENT FROM THE OTHER SIDE, and it was missing while the
        // WAL one stood. A non-empty `<name>-journal` is what a killed writer
        // leaves behind: the MAIN FILE then holds uncommitted pages and only
        // that journal can roll them back. `std::fs::copy` takes the main file
        // alone, so the trial image AND the backup would both be the
        // unrolled-back state — a backup that cannot restore what sqlite3 would
        // have shown. Same refusal, different sentence, so the log says which.
        // A 0-byte journal is a clean exit's leftover and is accepted, exactly
        // as an empty `-wal` is.
        if non_empty_sidecar(&database, "-journal") {
            return Err(ToolError::Other(format!(
                "'{file}' has a rollback journal beside it ('{file}-journal') that is not empty, \
                 so a writer was interrupted and part of the file is uncommitted pages that only \
                 that journal can undo. A byte copy of the file alone would be that \
                 unrolled-back state, so neither the trial run nor the backup would be the \
                 database sqlite3 would open. Nothing was written."
            )));
        }

        let scratch = Scratch::new()
            .map_err(|_| ToolError::Other("A working copy could not be made.".into()))?;
        let before = scratch.path.join("before.db");
        let after = scratch.path.join("after.db");
        std::fs::copy(&database, &before).map_err(ToolError::Io)?;
        // COPIED FROM THE COPY, not from the original a second time: both halves
        // of the comparison must be the same bytes, or a file that changed under
        // us between two reads would show up as an effect of the statement.
        std::fs::copy(&before, &after).map_err(ToolError::Io)?;

        let dry = run_sqlite(&self.binary, &after, statement)
            .map_err(|_| ToolError::Other("The sqlite3 command could not be run.".into()))?;
        if dry.timed_out {
            return Err(ToolError::Timeout);
        }

        let before_fp = fingerprint(&self.binary, &before);
        let after_fp = fingerprint(&self.binary, &after);
        let changes = describe(&before_fp, &after_fp);

        // A FAILING TRIAL RUN DOES NOT SKIP THE QUESTION when it changed
        // something. This is the `BEGIN; COMMIT; DROP TABLE t; ROLLBACK;` case:
        // measured, it exits 1 AND drops the table. Refusing on the exit code
        // alone would hide the most dangerous shape there is — a statement whose
        // damage happens before its error — behind the words "it failed", and
        // teach the user that a failing statement is a harmless one. When
        // NOTHING moved there is nothing to approve, and the engine's own
        // sentence is the useful answer.
        let failure = (!dry.ok).then(|| first_line(&dry.stderr));
        if let Some(reason) = &failure
            && changes.is_empty()
        {
            return Err(ToolError::Other(if reason.is_empty() {
                "The statement could not be run, and nothing was changed.".into()
            } else {
                format!("The statement could not be run, and nothing was changed: {reason}")
            }));
        }

        // THE ONE SENTENCE THAT MUST NOT READ AS "THIS IS A NO-OP". It used to
        // say "nothing measurable moved ... (an UPDATE that writes the value a
        // column already held looks like this too)" — naming the harmless case
        // as the example for a screen that an `UPDATE users SET pw='x'` over
        // every row also lands on, which is this tool's own advertised primary
        // use. The byte comparison separates the two, so the sentence no longer
        // has to cover both.
        let mut effect = if changes.is_empty() {
            match same_bytes(&before, &after) {
                Some(true) => "the trial copy came back BYTE FOR BYTE identical to the file: \
                               this statement wrote nothing at all."
                    .to_string(),
                Some(false) => "the file's BYTES CHANGED, but nothing that is compared moved — no \
                                schema object, no row count, no journal mode. The comparison does \
                                not look inside existing rows, so an UPDATE that rewrites a \
                                column across the whole table looks exactly like this. Read the \
                                statement above."
                    .to_string(),
                None => "nothing that is compared moved: no schema object, no row count, no \
                         journal mode — and whether the file's bytes changed could not be read \
                         back, so a statement that rewrote every row cannot be told apart from \
                         one that did nothing. Read the statement above."
                    .to_string(),
            }
        } else {
            changes.join("\n")
        };
        if let Some(reason) = &failure {
            effect.push_str(&format!(
                "\n! the statement ALSO reported an error part-way: {reason}\n! what is listed \
                 above is what it managed to do before that, and the real file would end the same \
                 way"
            ));
        }
        if dry.truncated {
            effect.push_str(
                "\n! the statement returns more output than is captured; the rows it returns are \
                 not part of this measurement",
            );
        }

        let backup = PathBuf::from(format!("{}{BACKUP_SUFFIX}", database.display()));
        let backup_name = backup
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "backup".into());
        // THE BACKUP'S OWN NAME IS A DESTINATION AND IT WAS NOT GUARDED. The
        // database path is proven by `resolve_existing_file`; `<db>.tacet-backup`
        // is a STRING APPENDED to it and never went through anything.
        // `symlink_metadata`, NOT `exists` — the same line and the same reason as
        // `archive.rs`'s write loop: `exists` follows a link, so a DANGLING one
        // reports false. MEASURED ON THIS MACHINE: with `inside/app.db.tacet-backup`
        // a symlink to `outside/leak.db`, `std::fs::copy` returned `Ok(16)` and
        // wrote the source bytes THROUGH the link into `outside/leak.db` — the
        // whole pre-write database, outside every workspace root, from a link a
        // hostile checkout could carry. A pre-existing REGULAR file is still
        // overwritten; that is the documented behaviour ("always the state before
        // the last write, never a history").
        //
        // ASKED BEFORE THE USER IS ASKED: a question whose "yes" cannot be
        // honoured is worse than no question.
        if backup
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(ToolError::SandboxViolation(backup));
        }

        let approved = self.confirm.confirm(&WriteRequest {
            file: &file,
            statement,
            effect: &effect,
            backup: &backup_name,
        });
        if !approved {
            // NOT AN ERROR — a decision. `ToolExecutor` reads this state as
            // `ApprovalDenied`: no taint, no recovery loop, `is_error()` false.
            return Ok(ToolOutcome::new(
                format!("db_write · {file} · not run"),
                ToolState::NeedsPermission,
                "the user was shown this statement's measured effect and did not approve it. \
                 Nothing was written. Do not retry with a different wording of the same change — \
                 tell the user it was not applied and ask what they want instead.",
            )
            .raw_output(effect));
        }

        // THE BACKUP IS THE IMAGE THAT WAS MEASURED, not a fresh copy: it is the
        // exact bytes whose fingerprint the user was shown. If it cannot be
        // placed, the write does not happen — the recovery from
        // `PRAGMA writable_schema` is the only recovery there is.
        std::fs::copy(&before, &backup).map_err(|_| {
            ToolError::Other(format!(
                "The copy that would let this be undone could not be written next to the \
                 database, so nothing was changed. Expected: {backup_name}"
            ))
        })?;

        let run = run_sqlite(&self.binary, &database, statement)
            .map_err(|_| ToolError::Other("The sqlite3 command could not be run.".into()))?;
        if run.timed_out {
            // THE STATE IS `Written` EVEN THOUGH THE RUN WAS KILLED, and that is
            // the fail-closed direction: nothing here can know how far it got,
            // and `Written` is what stops the engine replaying the turn and
            // doing it a second time.
            return Ok(ToolOutcome::new(
                format!("db_write · {file} · timed out"),
                ToolState::Written,
                format!(
                    "error: the approved statement did not finish within {}s and was stopped. \
                     Part of it may have been applied; SQLite's journal should have rolled back \
                     an unfinished transaction, but that is not something this tool measured. The \
                     file as it was beforehand is beside it as {backup_name}. Tell the user; do \
                     not repeat the statement.",
                    TIMEOUT.as_secs()
                ),
            ));
        }
        if !run.ok {
            let reason = first_line(&run.stderr);
            return Ok(ToolOutcome::new(
                format!("db_write · {file} · failed part-way"),
                ToolState::Written,
                format!(
                    "error: the approved statement was run and the engine reported: {reason}. \
                     Part of it may have been applied — this was the outcome the trial predicted. \
                     The file as it was beforehand is beside it as {backup_name}. Tell the user; \
                     do not repeat the statement."
                ),
            )
            .raw_output(run.stderr.clone()));
        }

        // THE RETURNED ROWS DO NOT GO TO THE MODEL. `UPDATE … RETURNING *` over
        // forty thousand rows is 658 KB (measured); the model gets the COUNT and
        // a `source_ref`, and the rows themselves go in the store as a typed
        // `Table`. This is also why the commit run must never be killed at the
        // cap: the bound is on what we KEEP, not on how long the child may
        // write.
        let table = parse_ascii(&run.stdout);
        let returned = table.row_count();
        let mut summary = format!(
            "db_write: {file} was changed with the user's approval.\nmeasured effect (predicted \
             on a copy before the run):\n{effect}\nrows returned by the statement: {returned}{}\n\
             the file as it was beforehand is beside it as {backup_name}.",
            if run.truncated {
                " (the captured output was cut at the size limit, so more rows came back than \
                 were kept)"
            } else {
                ""
            }
        );
        let mut source = None;
        if returned > 0 {
            source = Some(match &self.store {
                Some(store) => store.put_value("db_write", StoredValue::Table(table)),
                None => ctx.store("db_write", &summary, run.stdout.clone()),
            });
        }
        summary = cut_lines(&summary, MODEL_CAP);
        if let Some(r) = &source {
            summary.push_str(&tacet_kernel::source_ref_suffix(r.as_str()));
        }

        Ok(ToolOutcome::written(
            format!(
                "db_write · {file} · {}",
                if returned > 0 {
                    format!("applied · {returned} rows returned")
                } else {
                    "applied".to_string()
                }
            ),
            summary,
        )
        // The chip detail carries the full captured output: what goes to the
        // model is a count, what the user can open is not.
        .raw_output(run.stdout))
    }
}

/// Cuts at a LINE boundary. Half a line of a measured effect is worse than a
/// short one: the model reconstructs the missing half and invents a change that
/// was never measured.
fn cut_lines(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let mut kept = String::new();
    for line in text.lines() {
        if kept.chars().count() + line.chars().count() + 1 > cap {
            break;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    kept.push_str("(the rest of the report was cut)");
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tacet_kernel::{InMemoryDataStore, SilentReporter};

    // -----------------------------------------------------------------------
    // Harness
    // -----------------------------------------------------------------------

    fn temp_root(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-dbw-{tag}-{}-{}",
            std::process::id(),
            nanos()
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path.canonicalize().expect("resolved")
    }

    fn context(root: &Path) -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            root.to_path_buf(),
            Arc::new(SilentReporter),
        )
    }

    /// Builds a database with the SAME binary the tool measured, opened
    /// WRITABLE — possible only because this is the test's own invocation.
    ///
    /// THE SQL GOES DOWN STDIN, NOT INTO AN ARGUMENT, and that is not a style
    /// choice. Linux caps a SINGLE argv entry at `MAX_ARG_STRLEN` — 32 pages,
    /// 128 KiB — independently of the much larger total `ARG_MAX`, while macOS
    /// has no per-argument cap. `forty_thousand_returned_rows_do_not_reach_the_model`
    /// seeds ~1 MB of INSERT, so it passed here for two months and died on the
    /// first ubuntu CI run with `Os { code: 7, ArgumentListTooLong }` (run
    /// 33864401667, 2026-09-04). stdin has no such limit on either platform.
    fn seed(binary: &Path, file: &Path, sql: &str) {
        use std::io::Write as _;

        let mut child = Command::new(binary)
            .arg(file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("seed spawn");
        child
            .stdin
            .as_mut()
            .expect("seed stdin")
            .write_all(sql.as_bytes())
            .expect("seed write");
        let out = child.wait_with_output().expect("seed");
        assert!(
            out.status.success(),
            "seed failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Reads one scalar back with a plain (test-owned) invocation, so no
    /// assertion depends on the code under test.
    fn ask(binary: &Path, file: &Path, sql: &str) -> String {
        let out = Command::new(binary)
            .arg(file)
            .arg(sql)
            .stdin(Stdio::null())
            .output()
            .expect("ask");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A confirm sink that answers a fixed way and COUNTS the questions. The
    /// count is the point of one of the tests below: the outbound gate caches a
    /// denial per tool, and a database must be asked about every single time.
    struct Counting {
        answer: bool,
        asked: AtomicUsize,
        last_effect: Mutex<String>,
    }

    impl Counting {
        fn new(answer: bool) -> Arc<Counting> {
            Arc::new(Counting {
                answer,
                asked: AtomicUsize::new(0),
                last_effect: Mutex::new(String::new()),
            })
        }
        fn asked(&self) -> usize {
            self.asked.load(Ordering::SeqCst)
        }
        fn effect(&self) -> String {
            self.last_effect.lock().expect("effect lock").clone()
        }
    }

    impl WriteConfirm for Counting {
        fn confirm(&self, request: &WriteRequest<'_>) -> bool {
            self.asked.fetch_add(1, Ordering::SeqCst);
            *self.last_effect.lock().expect("effect lock") = request.effect.to_string();
            self.answer
        }
    }

    /// A tool over one allow-listed file, with the given sink. `None` when this
    /// machine has no usable `sqlite3` — every test below then returns, exactly
    /// as `db.rs`'s do.
    fn tool(files: &[&str], sink: Arc<dyn WriteConfirm>) -> Option<DbWriteTool> {
        DbWriteTool::with_files(files.iter().map(|f| f.to_string()).collect())
            .map(|t| t.with_confirm(sink))
    }

    // -----------------------------------------------------------------------
    // (1) The read tool did not lose its lock
    // -----------------------------------------------------------------------

    /// A REGRESSION HERE IS THE WHOLE FEATURE GOING WRONG. The two constants
    /// must differ in exactly one flag, and it must be that one.
    #[test]
    fn the_read_tool_did_not_lose_its_lock() {
        assert!(
            crate::db::SAFE_ARGS.contains(&"-readonly"),
            "the read tool's lock was removed"
        );
        assert!(
            !WRITE_ARGS.contains(&"-readonly"),
            "the write args carry -readonly; the write would be refused by the engine"
        );
        assert!(
            WRITE_ARGS.contains(&"-safe"),
            "-safe is the only thing keeping a write inside its own file"
        );
        // The difference is EXACTLY `-readonly`: everything else is shared.
        let shared: Vec<&str> = crate::db::SAFE_ARGS
            .iter()
            .copied()
            .filter(|a| *a != "-readonly")
            .collect();
        assert_eq!(shared, WRITE_ARGS.to_vec());
    }

    // -----------------------------------------------------------------------
    // (2) Absence — the gate that actually holds
    // -----------------------------------------------------------------------

    /// ABSENCE, THROUGH THE REAL GATE, WITH A REGISTRY THE TEST OWNS.
    ///
    /// This is the claim the whole design rests on, so it is measured against
    /// the same three questions `discover` asks — installed, open, list — rather
    /// than against a paraphrase of them. A CLOSED addon and an OPEN one with no
    /// writable list both produce NO TOOL: not a tool that refuses, no tool, so
    /// the model is never shown a name it could call.
    ///
    /// It does not read the machine's `addons.json`; `catalog.rs` records why
    /// that is forbidden here.
    #[test]
    fn a_closed_or_listless_addon_produces_no_write_tool() {
        use tacet_web::addon::{Addon, DB, Record, WRITABLE_KEY};

        // Not installed at all.
        assert!(DbWriteTool::from_record(&Record::empty()).is_none());

        // Installed and OPEN, but nothing named: the default after
        // `tacet addon install db` with an empty answer.
        let mut listless = Record::empty();
        listless.add(Addon::new(DB, DB));
        assert!(
            DbWriteTool::from_record(&listless).is_none(),
            "an open addon with no writable list produced a write tool"
        );

        // Named, but CLOSED — the state `tacet addon close db` leaves.
        let mut closed = Record::empty();
        let mut entry = Addon::new(DB, DB).with_setting(WRITABLE_KEY, "data/app.db");
        entry.open = false;
        closed.add(entry);
        assert!(
            DbWriteTool::from_record(&closed).is_none(),
            "a closed addon produced a write tool"
        );

        // Named AND open: the only combination that builds one — asserted so the
        // three refusals above cannot be passing because the gate refuses
        // everything. Skipped when this machine has no usable `sqlite3`, the same
        // condition every other test here checks.
        if DbWriteTool::with_files(vec!["data/app.db".into()]).is_some() {
            let mut open = Record::empty();
            open.add(Addon::new(DB, DB).with_setting(WRITABLE_KEY, "data/app.db"));
            let built = DbWriteTool::from_record(&open).expect("named and open");
            assert_eq!(built.allowed(), ["data/app.db"]);
        }
    }

    /// AN EMPTY OR UNUSABLE LIST IS NO TOOL AT ALL. Not a tool that refuses —
    /// no tool, so there is no name in the catalog, no grammar branch and no
    /// runtime check anyone can forget.
    #[test]
    fn an_unusable_list_produces_no_tool() {
        assert!(DbWriteTool::with_files(vec![]).is_none(), "empty list");
        assert!(
            DbWriteTool::with_files(vec!["   ".into()]).is_none(),
            "whitespace only"
        );
        // A hand-edited registry: every one of these is refused by
        // `Shape::DatabaseFile`, so none of them can reach the `Choice` set.
        for bad in [
            "/etc/passwd.db",
            "../secrets.db",
            "~/notes.db",
            "data/",
            "\\\\server\\share.db",
        ] {
            assert!(
                DbWriteTool::with_files(vec![bad.into()]).is_none(),
                "an unusable entry built a tool: {bad}"
            );
        }
    }

    /// THE ALLOWED SET IS THE SCHEMA. A path outside it fails validation, which
    /// is the same rule the grammar enforces one layer earlier: the model
    /// cannot EMIT the other path, so this is not a refusal it can retry past.
    #[test]
    fn a_path_outside_the_set_is_ungeneratable_and_then_refused() {
        let Some(tool) = tool(&["app.db"], Counting::new(true)) else {
            return;
        };
        let s = tool.schema();
        assert_eq!(tool.name(), "db_write");
        assert!(tool.taints_session());
        assert_eq!(
            s.fields()[0].schema.choices(),
            Some(&["app.db".to_string()][..])
        );
        assert!(
            s.validate(&json!({"path": "app.db", "statement": "DELETE FROM t"}))
                .is_ok()
        );
        assert!(
            s.validate(&json!({"path": "other.db", "statement": "DELETE FROM t"}))
                .is_err(),
            "a file outside the closed set validated"
        );
        assert!(s.validate(&json!({"path": "app.db"})).is_err());
    }

    /// THE SECOND CHECK, for the paths where the grammar is off (eval, a direct
    /// call). The file must be UNTOUCHED and the user must never have been
    /// asked — a refusal that still shows a confirmation screen has taught the
    /// user to click through one.
    #[test]
    fn a_file_outside_the_set_is_refused_without_asking() {
        let sink = Counting::new(true);
        let Some(tool) = tool(&["app.db"], sink.clone()) else {
            return;
        };
        let root = temp_root("outside-set");
        seed(
            &tool.binary,
            &root.join("other.db"),
            "CREATE TABLE t(a); INSERT INTO t VALUES(1);",
        );
        let ctx = context(&root);
        let outcome = tool
            .write(
                &json!({"path": "other.db", "statement": "DELETE FROM t"}),
                &ctx,
            )
            .expect("a refusal is an outcome, not an error");
        assert!(matches!(outcome.state, ToolState::Failed(_)), "{outcome:?}");
        assert!(outcome.to_model.contains("app.db"), "{}", outcome.to_model);
        assert_eq!(sink.asked(), 0, "the user was asked about a refused file");
        assert_eq!(
            ask(
                &tool.binary,
                &root.join("other.db"),
                "SELECT count(*) FROM t;"
            ),
            "1"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------------
    // (3) Statement stacking — SHOWN, because it cannot be refused
    // -----------------------------------------------------------------------

    /// THE HONEST CLAIM, MEASURED BOTH WAYS.
    ///
    /// `SELECT 1; DROP TABLE t` runs BOTH statements — that is measured at the
    /// top of this file and it is why "one statement only" is never claimed.
    /// What this test proves is that the stacking is SHOWN: the confirmation
    /// names `t` as removed, a refusal leaves it standing, and an approval
    /// really drops it.
    #[test]
    fn statement_stacking_is_shown_and_gated_not_hidden() {
        let refusing = Counting::new(false);
        let Some(tool) = tool(&["app.db"], refusing.clone()) else {
            return;
        };
        let root = temp_root("stack");
        let file = root.join("app.db");
        seed(
            &tool.binary,
            &file,
            "CREATE TABLE t(a); INSERT INTO t VALUES(1);",
        );
        let ctx = context(&root);
        let stacked = "SELECT 1; DROP TABLE t;";

        let refused = tool
            .write(&json!({"path": "app.db", "statement": stacked}), &ctx)
            .expect("a refusal is an outcome");
        assert!(matches!(refused.state, ToolState::NeedsPermission));
        assert_eq!(refusing.asked(), 1);
        assert!(
            refusing.effect().contains("REMOVED: table t"),
            "the second statement was not shown: {}",
            refusing.effect()
        );
        // AND THE TABLE IS STILL THERE — the claim, not the error text.
        assert_eq!(
            ask(&tool.binary, &file, "SELECT count(*) FROM sqlite_master;"),
            "1"
        );

        // Approved, the same statement really does both halves.
        let approving = Counting::new(true);
        let tool = tool.with_confirm(approving.clone());
        let done = tool
            .write(&json!({"path": "app.db", "statement": stacked}), &ctx)
            .expect("approved");
        assert!(matches!(done.state, ToolState::Written));
        assert_eq!(
            ask(&tool.binary, &file, "SELECT count(*) FROM sqlite_master;"),
            "0"
        );
        // The backup holds the table that was dropped.
        let backup = root.join("app.db.tacet-backup");
        assert!(backup.is_file(), "no backup was left");
        assert_eq!(ask(&tool.binary, &backup, "SELECT count(*) FROM t;"), "1");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// THE CRITIQUE'S CASE: a statement that CLOSES the transaction a textual
    /// `BEGIN … ROLLBACK` wrapper would have opened. Measured on this machine,
    /// `BEGIN; COMMIT; DROP TABLE t; ROLLBACK;` exits 1 AND drops the table —
    /// so a wrapper-based dry run would have destroyed the user's table while
    /// reporting a failure. Here the damage happens to a COPY, the original is
    /// untouched, and the user is still shown that `t` would go.
    #[test]
    fn a_statement_that_closes_the_transaction_cannot_reach_the_original() {
        let refusing = Counting::new(false);
        let Some(tool) = tool(&["app.db"], refusing.clone()) else {
            return;
        };
        let root = temp_root("commit-escape");
        let file = root.join("app.db");
        seed(
            &tool.binary,
            &file,
            "CREATE TABLE t(a); INSERT INTO t VALUES(1);",
        );
        let ctx = context(&root);

        let outcome = tool.write(
            &json!({"path": "app.db", "statement": "BEGIN; COMMIT; DROP TABLE t; ROLLBACK;"}),
            &ctx,
        );
        // Either shape is acceptable HERE — what is not acceptable is the table
        // being gone. The measured behaviour is that the effect is non-empty, so
        // the question IS asked and the answer decides.
        match outcome {
            Ok(o) => assert!(matches!(o.state, ToolState::NeedsPermission), "{o:?}"),
            Err(e) => panic!("the trial run must not turn into an error here: {e:?}"),
        }
        assert_eq!(refusing.asked(), 1);
        assert!(
            refusing.effect().contains("REMOVED: table t"),
            "{}",
            refusing.effect()
        );
        assert!(
            refusing.effect().contains("reported an error part-way"),
            "the engine's error was hidden: {}",
            refusing.effect()
        );
        assert_eq!(
            ask(&tool.binary, &file, "SELECT count(*) FROM sqlite_master;"),
            "1"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------------
    // (4) The file boundary survives the loss of `-readonly`
    // -----------------------------------------------------------------------

    /// ATTACH AND `VACUUM INTO` CANNOT REACH A SECOND FILE. This is the proof
    /// that `-safe` alone still holds the boundary, and it is the measurement
    /// the whole design rests on.
    #[test]
    fn attach_and_vacuum_into_cannot_escape_the_allowed_file() {
        let sink = Counting::new(true);
        let Some(tool) = tool(&["app.db"], sink.clone()) else {
            return;
        };
        let root = temp_root("attach");
        let outside = temp_root("attach-out");
        let file = root.join("app.db");
        seed(
            &tool.binary,
            &file,
            "CREATE TABLE t(a); INSERT INTO t VALUES(1);",
        );
        let ctx = context(&root);
        let leak = outside.join("leak.db");
        let escape = outside.join("x.db");

        for sql in [
            format!(
                "ATTACH DATABASE '{}' AS x; CREATE TABLE x.a(b);",
                escape.display()
            ),
            format!("VACUUM INTO '{}';", leak.display()),
            format!("SELECT writefile('{}', 'x');", leak.display()),
        ] {
            let outcome = tool.write(&json!({"path": "app.db", "statement": sql}), &ctx);
            assert!(outcome.is_err(), "not refused: {sql} -> {outcome:?}");
            assert!(!escape.exists(), "ATTACH created a second database: {sql}");
            assert!(!leak.exists(), "a second file was written: {sql}");
        }
        assert_eq!(sink.asked(), 0, "a refused escape still asked the user");
        assert_eq!(ask(&tool.binary, &file, "SELECT count(*) FROM t;"), "1");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// AN ALLOW-LISTED SYMLINK OUT OF THE SANDBOX IS A VIOLATION, NOT A WRITE.
    /// The list decides which NAME may be written; `sandbox_path` decides where
    /// that name is allowed to land, and it resolves every component.
    #[cfg(unix)]
    #[test]
    fn an_allow_listed_symlink_out_of_the_sandbox_is_refused() {
        let sink = Counting::new(true);
        let Some(tool) = tool(&["link.db"], sink.clone()) else {
            return;
        };
        let root = temp_root("link");
        let outside = temp_root("link-out");
        let secret = outside.join("secret.db");
        seed(
            &tool.binary,
            &secret,
            "CREATE TABLE s(v); INSERT INTO s VALUES('KEY');",
        );
        std::os::unix::fs::symlink(&secret, root.join("link.db")).expect("link");
        let ctx = context(&root);

        let outcome = tool.write(
            &json!({"path": "link.db", "statement": "DELETE FROM s"}),
            &ctx,
        );
        assert!(
            matches!(outcome, Err(ToolError::SandboxViolation(_))),
            "{outcome:?}"
        );
        assert_eq!(sink.asked(), 0);
        assert_eq!(ask(&tool.binary, &secret, "SELECT count(*) FROM s;"), "1");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// THE BACKUP'S NAME IS A DESTINATION TOO, and a planted link at it used to
    /// carry the whole pre-write database out of every workspace root.
    ///
    /// THE SCENARIO IS ORDINARY: a checkout from a hostile source (or one the
    /// `git` tool cloned) holding `data/app.db.tacet-backup -> ~/public/leak.db`.
    /// Every gate this file argues for held — the path was in the allow-list,
    /// `resolve_existing_file` proved it, `-safe` bounded the statement — and
    /// then the tool itself wrote the user's database through the link, because
    /// `<db>.tacet-backup` is a STRING APPENDED to a proven path and nothing
    /// looked at it.
    ///
    /// MEASURED BEFORE THE FIX by running exactly this test against it: the
    /// write was approved, `std::fs::copy` followed the link, and `leak.db`
    /// outside the root came back holding the seeded row. The mirror of
    /// `an_allow_listed_symlink_out_of_the_sandbox_is_refused`, one level down.
    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_backup_name_does_not_carry_the_database_out() {
        let sink = Counting::new(true);
        let Some(tool) = tool(&["app.db"], sink.clone()) else {
            return;
        };
        let root = temp_root("backup-link");
        let outside = temp_root("backup-link-out");
        let file = root.join("app.db");
        seed(
            &tool.binary,
            &file,
            "CREATE TABLE t(a); INSERT INTO t VALUES(1);",
        );
        let leak = outside.join("leak.db");
        // DANGLING on purpose: `exists()` reports false for it, which is why the
        // guard has to be `symlink_metadata`.
        std::os::unix::fs::symlink(&leak, root.join("app.db.tacet-backup")).expect("link");
        let ctx = context(&root);

        let outcome = tool.write(
            &json!({"path": "app.db", "statement": "INSERT INTO t VALUES(2)"}),
            &ctx,
        );
        assert!(
            matches!(outcome, Err(ToolError::SandboxViolation(_))),
            "{outcome:?}"
        );
        assert!(
            !leak.exists(),
            "the pre-write database was copied through the link to {}",
            leak.display()
        );
        assert_eq!(
            sink.asked(),
            0,
            "the user was asked a question whose yes could not be honoured"
        );
        // And the statement itself never ran: the refusal is before the commit.
        assert_eq!(ask(&tool.binary, &file, "SELECT count(*) FROM t;"), "1");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// AN UPDATE OVER EVERY ROW IS NOT A NO-OP AND MUST NOT READ LIKE ONE.
    ///
    /// The fingerprint is schema + row counts + durability pragmas, so this
    /// statement moves nothing in it — measured directly with this machine's
    /// sqlite3 3.51.0, `FINGERPRINT_SQL`'s output is byte-identical before and
    /// after. The screen used to say "nothing measurable moved ... (an UPDATE
    /// that writes the value a column already held looks like this too)", which
    /// is the sentence a user skims as "harmless" while approving a rewrite of
    /// every password in the table. This test fails on that sentence.
    #[test]
    fn an_update_that_rewrites_every_row_does_not_read_as_a_no_op() {
        let refusing = Counting::new(false);
        let Some(tool) = tool(&["app.db"], refusing.clone()) else {
            return;
        };
        let root = temp_root("values");
        let file = root.join("app.db");
        seed(
            &tool.binary,
            &file,
            "CREATE TABLE users(id, pw); INSERT INTO users VALUES(1,'a'),(2,'b');",
        );
        let ctx = context(&root);

        let refused = tool
            .write(
                &json!({"path": "app.db", "statement": "UPDATE users SET pw='pwned'"}),
                &ctx,
            )
            .expect("a refusal is an outcome");
        assert!(matches!(refused.state, ToolState::NeedsPermission));
        let effect = refusing.effect();
        assert!(
            effect.contains("BYTES CHANGED"),
            "the one screen the user decides on did not say the file changed: {effect}"
        );
        assert!(
            !effect.contains("wrote nothing at all"),
            "a statement that rewrote every row was described as writing nothing: {effect}"
        );
        // The complement, so the sentence is not simply always the same one: a
        // statement that really writes nothing must say so.
        let quiet = Counting::new(false);
        let tool = tool.with_confirm(quiet.clone());
        let _ = tool.write(
            &json!({"path": "app.db", "statement": "UPDATE users SET pw=pw WHERE id=999"}),
            &ctx,
        );
        assert!(
            quiet.effect().contains("wrote nothing at all"),
            "a statement that touched no row was not reported as one: {}",
            quiet.effect()
        );
        assert_eq!(
            ask(&tool.binary, &file, "SELECT pw FROM users WHERE id=1;"),
            "a"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A HOT ROLLBACK JOURNAL IS THE `-wal` CASE FROM THE OTHER SIDE. A killed
    /// writer leaves `<db>-journal` holding the pages that undo a half-finished
    /// transaction; the MAIN file then contains uncommitted state, so the byte
    /// copy this tool takes is neither what sqlite3 would open nor a backup that
    /// could restore it. The WAL check was there from the start and this one was
    /// not, though the argument is the same one.
    #[test]
    fn a_hot_rollback_journal_is_refused() {
        let sink = Counting::new(true);
        let Some(tool) = tool(&["app.db"], sink.clone()) else {
            return;
        };
        let root = temp_root("hot-journal");
        let file = root.join("app.db");
        seed(&tool.binary, &file, "CREATE TABLE t(a);");
        // Written directly for the same reason the `-wal` fixture is: sqlite3
        // deletes its journal on a clean exit, and the state under test is the
        // one only a crash leaves behind.
        std::fs::write(root.join("app.db-journal"), b"not empty").expect("journal");
        let ctx = context(&root);

        let outcome = tool.write(
            &json!({"path": "app.db", "statement": "INSERT INTO t VALUES(1)"}),
            &ctx,
        );
        match &outcome {
            Err(ToolError::Other(why)) => assert!(
                why.contains("rollback journal"),
                "refused for the wrong reason: {why}"
            ),
            other => panic!("a hot journal was not refused: {other:?}"),
        }
        assert_eq!(sink.asked(), 0);
        assert!(!root.join("app.db.tacet-backup").exists());
        // A 0-byte journal is a clean exit's leftover and must still be allowed.
        std::fs::write(root.join("app.db-journal"), b"").expect("journal");
        let allowed = tool.write(
            &json!({"path": "app.db", "statement": "INSERT INTO t VALUES(1)"}),
            &ctx,
        );
        assert!(allowed.is_ok(), "an empty journal was refused: {allowed:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------------
    // (5) The pragmas that change durability or rewrite the schema
    // -----------------------------------------------------------------------

    /// `PRAGMA writable_schema` was measured HERE as the one input nothing
    /// structurally refuses: on macOS with `/usr/bin/sqlite3 3.51.0` it exits 0
    /// under `-safe` and rewrites `sqlite_master` while every object NAME stays
    /// the same. That is the case this test was written for, and it fails the
    /// moment anyone weakens the fingerprint to names alone — it is the reason
    /// the backup exists at all.
    ///
    /// IT IS NOT UNIVERSAL, AND CI SAID SO. On ubuntu-latest the same statement
    /// is refused outright, in prepare: "table sqlite_master may not be modified"
    /// (run 33864401667, 2026-09-04). So the sentence "nothing structurally
    /// refuses it" was true of the machine it was measured on and false one
    /// platform over — the distro's sqlite3 is STRICTER than macOS's.
    ///
    /// BOTH OUTCOMES ARE ASSERTED, because both are correct and the weaker one
    /// is the one that needs the backup. A refusal must leave the schema intact;
    /// an acceptance must be DETECTED as a redefinition and be recoverable. What
    /// no platform may do is accept the rewrite silently, and that is what fails
    /// here either way.
    #[test]
    fn a_schema_rewrite_is_detected_and_recoverable() {
        let refusing = Counting::new(false);
        let Some(tool) = tool(&["app.db"], refusing.clone()) else {
            return;
        };
        let root = temp_root("wschema");
        let file = root.join("app.db");
        seed(&tool.binary, &file, "CREATE TABLE t(a);");
        let ctx = context(&root);
        let sql = "PRAGMA writable_schema=ON; UPDATE sqlite_master SET sql='CREATE TABLE t(a,b)' \
                   WHERE name='t';";

        let Ok(refused) = tool.write(&json!({"path": "app.db", "statement": sql}), &ctx) else {
            // The stricter build: sqlite3 would not prepare the statement, so the
            // trial run never touched the file. The guarantee holds by refusal,
            // which is strictly better than holding by detection — but the file
            // still has to be untouched, and that is the half worth asserting.
            assert_eq!(
                ask(&tool.binary, &file, "SELECT sql FROM sqlite_master;"),
                "CREATE TABLE t(a)",
                "the statement was refused, so nothing may have changed"
            );
            return;
        };
        assert!(matches!(refused.state, ToolState::NeedsPermission));
        assert!(
            refusing.effect().contains("REDEFINED: table t"),
            "a names-only fingerprint would miss this: {}",
            refusing.effect()
        );
        assert_eq!(
            ask(&tool.binary, &file, "SELECT sql FROM sqlite_master;"),
            "CREATE TABLE t(a)"
        );

        let tool = tool.with_confirm(Counting::new(true));
        let done = tool
            .write(&json!({"path": "app.db", "statement": sql}), &ctx)
            .expect("approved");
        assert!(matches!(done.state, ToolState::Written));
        assert_eq!(
            ask(&tool.binary, &file, "SELECT sql FROM sqlite_master;"),
            "CREATE TABLE t(a,b)"
        );
        // THE ONLY RECOVERY, and it holds the original text.
        assert_eq!(
            ask(
                &tool.binary,
                &root.join("app.db.tacet-backup"),
                "SELECT sql FROM sqlite_master;"
            ),
            "CREATE TABLE t(a)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A DURABILITY CHANGE IS A CHANGE. `PRAGMA journal_mode=WAL` moves no row
    /// and no schema object, so a fingerprint of `sqlite_master` alone would
    /// report "nothing moved" for a statement that permanently changes how the
    /// file is written and leaves `-wal`/`-shm` beside it. Refused, the original
    /// must still be in its old mode with no sidecars.
    #[test]
    fn a_journal_mode_change_is_shown_and_is_not_applied_when_refused() {
        let refusing = Counting::new(false);
        let Some(tool) = tool(&["app.db"], refusing.clone()) else {
            return;
        };
        let root = temp_root("wal");
        let file = root.join("app.db");
        seed(&tool.binary, &file, "CREATE TABLE t(a);");
        let ctx = context(&root);

        let refused = tool
            .write(
                &json!({"path": "app.db", "statement": "PRAGMA journal_mode=WAL;"}),
                &ctx,
            )
            .expect("a refusal is an outcome");
        assert!(matches!(refused.state, ToolState::NeedsPermission));
        assert!(
            refusing.effect().contains("journal_mode: delete -> wal"),
            "the durability change was invisible: {}",
            refusing.effect()
        );
        assert_eq!(ask(&tool.binary, &file, "PRAGMA journal_mode;"), "delete");
        assert!(!root.join("app.db-wal").exists(), "a -wal sidecar was left");
        assert!(!root.join("app.db-shm").exists(), "a -shm sidecar was left");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A DATABASE WHOSE WRITE-AHEAD LOG IS NOT EMPTY IS REFUSED, because a byte
    /// copy of the main file would be a torn image and the backup would be a
    /// lie. Measured: `.backup` is a dot-command and `VACUUM INTO` reports as
    /// ATTACH, so there is no consistent copy to make.
    #[test]
    fn a_wal_database_with_a_live_log_is_refused() {
        let sink = Counting::new(true);
        let Some(tool) = tool(&["app.db"], sink.clone()) else {
            return;
        };
        let root = temp_root("live-wal");
        let file = root.join("app.db");
        seed(&tool.binary, &file, "CREATE TABLE t(a);");
        // A non-empty `-wal` beside the file is the state this refuses. Written
        // directly rather than provoked out of sqlite3, because the CLI
        // checkpoints and truncates its log on a clean exit (measured: a WAL
        // database at rest has a 0-byte log), and a 0-byte log is exactly the
        // case that is ACCEPTED.
        std::fs::write(root.join("app.db-wal"), b"not empty").expect("wal");
        let ctx = context(&root);

        let outcome = tool.write(
            &json!({"path": "app.db", "statement": "INSERT INTO t VALUES(1)"}),
            &ctx,
        );
        assert!(matches!(outcome, Err(ToolError::Other(_))), "{outcome:?}");
        assert_eq!(sink.asked(), 0);
        assert!(!root.join("app.db.tacet-backup").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------------
    // (6) Bulk data does not reach the model
    // -----------------------------------------------------------------------

    /// FORTY THOUSAND ROWS. `UPDATE … RETURNING *` over 40 000 rows is 658 KB of
    /// output (measured). The model must get a COUNT, the store must get the
    /// rows, the process must NOT have been killed part-way — that last one is
    /// why `db.rs::run_query` could not be reused here.
    #[test]
    fn forty_thousand_returned_rows_do_not_reach_the_model() {
        let Some(tool) = tool(&["big.db"], Counting::new(true)) else {
            return;
        };
        let root = temp_root("bulk");
        let file = root.join("big.db");
        let values: String = (0..40_000)
            .map(|i| format!("({i},'name-{i}')"))
            .collect::<Vec<_>>()
            .join(",");
        seed(
            &tool.binary,
            &file,
            &format!("CREATE TABLE t(a INTEGER, b TEXT); INSERT INTO t VALUES {values};"),
        );
        let store = Arc::new(SharedStore::new());
        let tool = tool.with_store(Arc::clone(&store));
        let ctx = context(&root);

        let outcome = tool
            .write(
                &json!({"path": "big.db", "statement": "UPDATE t SET a=a+1 RETURNING *"}),
                &ctx,
            )
            .expect("approved");
        assert!(matches!(outcome.state, ToolState::Written));
        assert!(
            outcome.to_model.chars().count() <= MODEL_CAP + 120,
            "{} characters reached the model",
            outcome.to_model.chars().count()
        );
        for value in ["name-0", "name-39999", "name-1234"] {
            assert!(
                !outcome.to_model.contains(value),
                "a row value reached the model: {value}"
            );
        }
        assert!(
            outcome.to_model.contains("rows returned by the statement"),
            "{}",
            outcome.to_model
        );
        assert!(
            outcome.to_model.contains("source_ref"),
            "{}",
            outcome.to_model
        );
        // THE PROCESS WAS NOT KILLED PART-WAY: every one of the 40 000 rows
        // really was updated.
        assert_eq!(
            ask(
                &tool.binary,
                &file,
                "SELECT count(*), min(a), max(a) FROM t;"
            ),
            "40000|1|40000"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------------
    // (7) The gate is NOT the outbound gate
    // -----------------------------------------------------------------------

    /// A CLEAN SESSION IS EXACTLY WHERE `executor.rs`'s GATE 3 DOES NOT FIRE
    /// (it needs a tainted session AND an entry in `EXTERNAL_TOOLS`), so if
    /// anyone ever "reuses" that gate for this, this test goes red: an untainted
    /// session must still refuse, and the data must be untouched.
    ///
    /// AND THE SECOND CALL MUST STILL ASK. `ToolExecutor::ask_approval` caches a
    /// denial per tool for the rest of the session — right for a remote server,
    /// wrong for a database, where the next statement is a different question.
    #[test]
    fn the_confirmation_is_per_call_and_fires_in_a_clean_session() {
        let refusing = Counting::new(false);
        let Some(tool) = tool(&["app.db"], refusing.clone()) else {
            return;
        };
        let root = temp_root("clean");
        let file = root.join("app.db");
        seed(
            &tool.binary,
            &file,
            "CREATE TABLE t(a); INSERT INTO t VALUES(1),(2);",
        );
        let ctx = context(&root);
        assert!(!ctx.session_tainted(), "the session must start clean");

        for _ in 0..2 {
            let outcome = tool
                .write(
                    &json!({"path": "app.db", "statement": "DELETE FROM t"}),
                    &ctx,
                )
                .expect("a refusal is an outcome");
            assert!(matches!(outcome.state, ToolState::NeedsPermission));
        }
        assert_eq!(
            refusing.asked(),
            2,
            "the second call was answered from a cache; a database must be asked about every time"
        );
        assert_eq!(ask(&tool.binary, &file, "SELECT count(*) FROM t;"), "2");
        // A refusal leaves NO backup: nothing was about to change.
        assert!(!root.join("app.db.tacet-backup").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// THE DEFAULT SINK REFUSES. A tool built without `with_confirm` — the shape
    /// every non-interactive path gets — must not write.
    #[test]
    fn the_default_sink_refuses() {
        let Some(tool) = DbWriteTool::with_files(vec!["app.db".into()]) else {
            return;
        };
        let root = temp_root("default-sink");
        let file = root.join("app.db");
        seed(
            &tool.binary,
            &file,
            "CREATE TABLE t(a); INSERT INTO t VALUES(1);",
        );
        let ctx = context(&root);
        let outcome = tool
            .write(
                &json!({"path": "app.db", "statement": "DROP TABLE t"}),
                &ctx,
            )
            .expect("a refusal is an outcome");
        assert!(matches!(outcome.state, ToolState::NeedsPermission));
        assert_eq!(ask(&tool.binary, &file, "SELECT count(*) FROM t;"), "1");
        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------------
    // The happy path, and the scratch copies
    // -----------------------------------------------------------------------

    /// A GATE THAT ALSO REFUSES THE LEGITIMATE CASE IS THE ONE SOMEBODY DELETES
    /// LATER. An ordinary `UPDATE` must work, must report the row-count move it
    /// measured, and must leave nothing behind in the temp directory.
    #[test]
    fn an_ordinary_update_is_applied_and_leaves_no_scratch_behind() {
        let approving = Counting::new(true);
        let Some(tool) = tool(&["data/app.db"], approving.clone()) else {
            return;
        };
        let root = temp_root("happy");
        std::fs::create_dir_all(root.join("data")).expect("data dir");
        let file = root.join("data").join("app.db");
        seed(
            &tool.binary,
            &file,
            "CREATE TABLE t(a); INSERT INTO t VALUES(1),(2),(3);",
        );
        let ctx = context(&root);

        let outcome = tool
            .write(
                &json!({"path": "data/app.db", "statement": "DELETE FROM t WHERE a > 1"}),
                &ctx,
            )
            .expect("approved");
        assert!(matches!(outcome.state, ToolState::Written));
        assert!(
            approving.effect().contains("rows in t: 3 -> 1"),
            "{}",
            approving.effect()
        );
        assert_eq!(ask(&tool.binary, &file, "SELECT count(*) FROM t;"), "1");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// THE COPIES HOLD THE USER'S DATA, so the guard that removes them is a
    /// guarantee and not housekeeping. Measured directly rather than by
    /// counting leftovers in the temp directory after a write: the tests in this
    /// file run in parallel and each other's live scratch directories are
    /// indistinguishable from a leak, which is how a flaky assertion gets
    /// deleted a week later.
    #[test]
    fn the_scratch_directory_removes_itself() {
        let path = {
            let scratch = Scratch::new().expect("scratch");
            std::fs::write(scratch.path.join("before.db"), b"data").expect("write");
            assert!(scratch.path.is_dir());
            scratch.path.clone()
        };
        assert!(
            !path.exists(),
            "a scratch copy survived its guard: {path:?}"
        );
    }

    /// A DOT-COMMAND IS REFUSED BEFORE THE BINARY IS STARTED — the
    /// belt-to-the-braces layer, asserted so it does not rot into a no-op.
    #[test]
    fn a_dot_command_is_refused_up_front() {
        let sink = Counting::new(true);
        let Some(tool) = tool(&["app.db"], sink.clone()) else {
            return;
        };
        let root = temp_root("dot");
        seed(&tool.binary, &root.join("app.db"), "CREATE TABLE t(a);");
        let ctx = context(&root);
        assert!(matches!(
            tool.write(&json!({"path": "app.db", "statement": " .tables"}), &ctx),
            Err(ToolError::InvalidArgument(_))
        ));
        assert_eq!(sink.asked(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// DISCOVERY AND THE MEASUREMENT AGREE. If this machine's binary cannot be
    /// proved to keep `-safe` without `-readonly`, no tool is built; if it can,
    /// one is. A disagreement is the "the mechanism was built but never wired
    /// up" failure.
    #[test]
    fn discovery_and_the_write_lock_measurement_agree() {
        let found = find_binary();
        let locked = found.as_deref().map(verify_write_lock).unwrap_or(false);
        assert_eq!(
            DbWriteTool::with_files(vec!["app.db".into()]).is_some(),
            locked,
            "a write tool was built without the measurement passing"
        );
        let text = DbWriteTool::diagnose();
        assert!(text.starts_with("db_write is off"), "{text}");
    }

    /// THE FINGERPRINT COMPARISON ITSELF, with no database in sight. A
    /// `type/name/sql` row whose SQL text changed must read as REDEFINED, and a
    /// pragma row must read as a value move — the two are different sentences
    /// because they are different kinds of damage.
    #[test]
    fn the_diff_separates_a_redefinition_from_a_pragma_move() {
        let before = Fingerprint {
            objects: vec![
                "pragma/journal_mode/delete".into(),
                "table/t/CREATE TABLE t(a)".into(),
                "table/u/CREATE TABLE u(a)".into(),
            ],
            counts: vec![("t".into(), "3".into())],
            counted: true,
            readable: true,
            complete: true,
        };
        let after = Fingerprint {
            objects: vec![
                "pragma/journal_mode/wal".into(),
                "index/i/CREATE INDEX i ON t(a)".into(),
                "table/t/CREATE TABLE t(a,b)".into(),
            ],
            counts: vec![("t".into(), "0".into())],
            counted: true,
            readable: true,
            complete: true,
        };
        let lines = describe(&before, &after);
        assert!(
            lines.contains(&"journal_mode: delete -> wal".to_string()),
            "{lines:?}"
        );
        assert!(
            lines.contains(&"REDEFINED: table t (its CREATE statement text changed)".to_string()),
            "{lines:?}"
        );
        assert!(lines.contains(&"REMOVED: table u".to_string()), "{lines:?}");
        assert!(lines.contains(&"added: index i".to_string()), "{lines:?}");
        assert!(
            lines.contains(&"rows in t: 3 -> 0".to_string()),
            "{lines:?}"
        );
        // Identical images produce NO lines — that is what "nothing measurable
        // moved" is built on.
        assert!(describe(&before, &before).is_empty());

        // AND AN UNREADABLE IMAGE IS NOT A DIFF. Without the guard, an image
        // sqlite3 cannot open has zero objects, and the comparison would report
        // "REMOVED: table t / REMOVED: table u / REMOVED: pragma journal_mode" —
        // a precise, confident, wrong sentence on the approval screen.
        let broken = Fingerprint {
            readable: false,
            ..Fingerprint::default()
        };
        let lines = describe(&before, &broken);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("THE FILE COULD NOT BE READ BACK"),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("REMOVED: table t")),
            "an unopenable file was reported as a table drop: {lines:?}"
        );
    }

    /// A MEASUREMENT THAT DID NOT FIT SAYS SO. `drain_pipe` stops storing at
    /// `OUTPUT_CAP`, and a schema larger than that used to be diffed as though
    /// it were the whole schema — so an object past the cut could be dropped
    /// with the screen showing nothing, and a single added object could shift
    /// the cut and print a run of `REMOVED:` lines for objects that are still
    /// there. NOT MEASURED against a real 256 KiB schema: this drives the flag,
    /// which `fingerprint` sets from the same `Run::truncated` the statement's
    /// own output already reports.
    #[test]
    fn a_fingerprint_that_did_not_fit_is_not_presented_as_the_whole_schema() {
        let full = Fingerprint {
            objects: vec!["table/t/CREATE TABLE t(a)".into()],
            counts: vec![("t".into(), "1".into())],
            counted: true,
            readable: true,
            complete: true,
        };
        let cut = Fingerprint {
            complete: false,
            ..full.clone()
        };
        let lines = describe(&full, &cut);
        assert!(
            lines.iter().any(|l| l.contains("INCOMPLETE")),
            "a truncated read was presented as a complete comparison: {lines:?}"
        );
        // The complement: two complete, identical images still say nothing at
        // all, so the new line cannot become noise on every call.
        assert!(describe(&full, &full).is_empty());
    }
}
