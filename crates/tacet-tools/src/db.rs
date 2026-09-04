//! `db` — a READ-ONLY window onto a SQLite database inside the working
//! directory.
//!
//! ===========================================================================
//! WHY THERE IS NO DRIVER, ONLY A BINARY
//! ===========================================================================
//!
//! This crate may not take a new dependency, so there is no `rusqlite`, no
//! `sqlx`, no `postgres`. The only honest way left is to call the `sqlite3`
//! command line tool through `std::process::Command` — the same shape `git.rs`
//! already uses for `git`. IF THE BINARY IS ABSENT THE TOOL DROPS OUT OF THE
//! CATALOG and `diagnose()` says why: a tool the model can see, call, and fail
//! at every single time is a trap that costs a turn (the reasoning is written
//! out in `run_code::discover`).
//!
//! ===========================================================================
//! WHY POSTGRES IS NOT HERE — A DELIBERATE ABSENCE, NOT AN OVERSIGHT
//! ===========================================================================
//!
//! `psql` was evaluated for this tool and REJECTED, because it cannot be opened
//! READ-ONLY at the tool level. What exists is `PGOPTIONS="-c
//! default_transaction_read_only=on"`, and it is not a lock: the SQL text — the
//! part the MODEL writes — can undo it with `SET default_transaction_read_only
//! = off` or simply `BEGIN READ WRITE`. The lock would sit in the same channel
//! as the thing it is supposed to constrain.
//!
//! The remaining option was to filter the query text — accept it if it "starts
//! with SELECT", reject it otherwise. That is exactly the shape this repository
//! refuses. A text filter over SQL loses to `WITH x AS (...) SELECT`, to a
//! leading comment, to `select/*x*/`, to a second statement after a semicolon,
//! to `CTE ... INSERT ... RETURNING`; the next bypass is always the one nobody
//! listed. And it would look safe on the screen while being nothing of the
//! kind. HALF SECURITY IS WORSE THAN NO SECURITY: it moves the user from "I
//! know this can write" to "I was told it cannot".
//!
//! So the tool covers SQLite only, and says so in its description. Adding
//! Postgres honestly needs one of: a driver crate (a dependency decision), or a
//! connection made as a role that holds no write privilege (a database-side
//! fact this tool cannot verify and therefore must not claim). Neither is
//! something to smuggle in behind a filter.
//!
//! A CONSEQUENCE WORTH STATING: with Postgres gone there is no connection
//! string, so there is no password anywhere near this file — nothing to keep at
//! 0600, nothing to keep out of a log, an error message or the model's window.
//! The narrowest form of the tool is also the one with no secret to leak.
//!
//! ===========================================================================
//! READ-ONLY IS A MEASURED PROPERTY OF THE BINARY, NOT A PROMISE IN A COMMENT
//! ===========================================================================
//!
//! Two flags carry it, and `discover()` MEASURES that this machine's `sqlite3`
//! really honours both before the tool is built:
//!
//!   * `-readonly` — the database file is OPENED read-only. No SQL can undo an
//!     open mode; a write attempt dies with "attempt to write a readonly
//!     database" inside the engine, below the level the query text can reach.
//!   * `-safe`     — SQLite's own mode for "the SQL comes from an untrusted
//!     source". It disables `.shell`, `.system`, `.import`, `.output`, `.read`,
//!     `.load`, `.open`, forbids ATTACH entirely, and forbids writing any file.
//!     Without it the dot-commands would be a straight command-execution hole:
//!     `sqlite3 db ".shell rm -rf ~"` runs a shell, and the argument is written
//!     by the model.
//!
//! MEASURED ON THIS MACHINE (sqlite3 3.51.0), each of these four:
//!   INSERT             -> `Error: stepping, attempt to write a readonly database (8)`
//!   `.shell echo X`    -> `cannot run .shell in safe mode`
//!   ATTACH + CREATE    -> `cannot run ATTACH in safe mode`
//!   `.output leak.txt` -> `cannot run .output in safe mode`, no file created
//!
//! `-safe` arrived in SQLite 3.34 (2020). On an older binary the option is
//! rejected outright, `verify_lock` fails, and the tool leaves the catalog with
//! the reason printed — fail-closed, the same shape as a missing network
//! shield.
//!
//! ===========================================================================
//! THE OTHER GATES
//! ===========================================================================
//!
//! NO SHELL. `Command` is given an ARRAY of arguments; there is no `sh -c` and
//! no string concatenation, so a database path or a query containing a space, a
//! quote or a `;` stays exactly one argument and no quoting rule can be got
//! wrong.
//!
//! THE FILE MUST BE INSIDE THE SANDBOX. The path goes through
//! `sandbox_path::resolve_existing_file` — the existing gate, CALLED, not
//! rewritten. That is what stops `../../../Library/.../Mail.sqlite` and, just
//! as importantly, a symlink planted inside the sandbox that points at it (the
//! attack that module was written for).
//!
//! BULK OUTPUT DOES NOT PASS THROUGH THE MODEL. A thousand-row answer would eat
//! the 4096-token window; the rows go into the `DataStore` as a typed `Table`
//! and the model gets the first few rows as valid markdown plus a `source_ref`.
//!
//! THE RESULT IS THE USER'S DATA, so the session is tainted and the next
//! outgoing call meets the approval gate.
//!
//! ===========================================================================
//! WHAT `-readonly` CANNOT OPEN — MEASURED, AND NOT FIXED HERE
//! ===========================================================================
//!
//! A database in WAL journal mode that has no `-shm` sidecar beside it CANNOT
//! BE OPENED READ-ONLY AT ALL. SQLite needs the shared-memory index to read a
//! WAL database and `-readonly` forbids creating it. Measured on this machine
//! (sqlite3 3.51.0): a file put into WAL mode and then closed cleanly answered
//!
//!   sqlite3 -readonly -safe -batch w.db "SELECT count(*) FROM t;"
//!   -> Error: in prepare, unable to open database file (14), exit 14
//!
//! while the same query without `-readonly` printed `3` — and after that
//! read-write open had left `w.db-shm` and `w.db-wal` behind, the read-only
//! form worked too. So this tool answers "the query could not be run" for a WAL
//! database at rest, which looks like a broken tool and is in fact the lock
//! doing exactly what it says.
//!
//! IT IS RECORDED RATHER THAN FIXED because every fix trades the lock away:
//! opening read-write to create the `-shm`, or passing `?immutable=1` (which
//! tells SQLite the file cannot change and returns WRONG ANSWERS if it does).
//! Neither is a trade this tool may make on its own. `db_write.rs` hit the same
//! wall from the other side and works around it WITHOUT touching the user's
//! file: it fingerprints its own scratch COPIES read-write and never opens the
//! original for reading at all.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;
use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult,
    TraceUpdate, boxed,
};

use crate::data_store::{SharedStore, Table, Value as StoredValue};

/// Where `sqlite3` is looked for. FIXED PATHS, NOT `PATH`.
///
/// `PATH` comes from the calling process's environment and can be poisoned;
/// finding something called `sqlite3` on it and trusting it to honour `-safe`
/// would make the measurement below meaningless — the same reasoning
/// `run_code::discover_interpreters` writes down for interpreters, and it binds
/// harder here because these two flags ARE the entire security story.
const SQLITE_PATHS: &[&str] = &[
    "/usr/bin/sqlite3",
    "/opt/homebrew/bin/sqlite3",
    "/usr/local/bin/sqlite3",
    "/bin/sqlite3",
];

/// The flags that make the run read-only. Kept as one constant so no call site
/// can build an invocation that is missing one of them.
///
/// `-batch` closes interactive prompting: there is no terminal at the other end
/// of a chat turn, and a binary waiting on one would hang until the watchdog.
/// `-ascii` sets the field/record separators to `0x1F`/`0x1E`, which no SQL
/// value realistically contains, so the output parses without a quoting rule.
/// `-header` puts the column names in the first record.
///
/// `pub(crate)` SO IT CAN BE COMPARED, NOT SO IT CAN BE REUSED. `db_write.rs`
/// keeps its own `WRITE_ARGS` (the same list MINUS `-readonly`) and a test
/// there asserts the two constants differ in exactly that flag. The write
/// constant deliberately does NOT live in this file: this module's doc comment
/// is a hundred-line argument that read-only is the whole point, and a list
/// with no `-readonly` in it sitting underneath that argument is how the two
/// get mixed up at a call site.
pub(crate) const SAFE_ARGS: [&str; 5] = ["-readonly", "-safe", "-batch", "-ascii", "-header"];

/// ASCII unit separator — between fields.
const FIELD_SEPARATOR: char = '\u{1f}';
/// ASCII record separator — between rows.
const RECORD_SEPARATOR: char = '\u{1e}';

/// The cap on the bytes kept from the query output (256 KiB) — the same number
/// as `git.rs`, for the same reason: a `DataStore` record and a chip detail have
/// to stay bounded, and an answer larger than this is a dump rather than an
/// answer.
const OUTPUT_CAP: usize = 256 * 1024;

/// The cap on the error text read back. An error message is a sentence; more
/// than this is a symptom.
const ERROR_CAP: usize = 4 * 1024;

/// The wall clock the query gets.
///
/// UNLIKE `git.rs`, THERE IS A TIMEOUT HERE, and the difference is not
/// inconsistency. `git status` walks a tree whose size the user knows; a SQL
/// query is written by the MODEL, and `SELECT * FROM a, b, c` on three modest
/// tables is a cross join that never finishes. The bounded read below already
/// stops memory from growing — the process blocks on a full pipe — but blocking
/// forever on a full pipe is exactly the hang this bound exists to cut.
const TIMEOUT: Duration = Duration::from_secs(15);

/// How many rows the model sees. The rest stay in the store behind the
/// `source_ref`.
///
/// The same reasoning as `git::PREVIEW_LINES`: the model needs the SHAPE of the
/// answer (which columns, what the values look like) to write a sentence about
/// it, not the whole table.
const PREVIEW_ROWS: usize = 12;

/// The cap on the text handed to the model (~500 tokens). A wide table can blow
/// a budget with far fewer rows than `PREVIEW_ROWS`, so the character cap is
/// enforced as well as the row cap.
pub(crate) const MODEL_CAP: usize = 1400;

// ---------------------------------------------------------------------------
// Discovery — the read-only lock is MEASURED, not assumed
// ---------------------------------------------------------------------------

/// The marker the safe-mode probe tries, and must fail, to print.
const UNSAFE_MARKER: &str = "TACET_SAFE_MODE_IS_OFF";

/// Finds the binary. `None` = it is not in any of the known locations.
pub(crate) fn find_binary() -> Option<PathBuf> {
    SQLITE_PATHS
        .iter()
        .map(Path::new)
        .find(|p| p.is_file())
        .map(Path::to_path_buf)
}

/// Proves that THIS binary really honours both flags.
///
/// Checking "does the file exist" is not enough and the gap is not theoretical:
/// `-safe` did not exist before SQLite 3.34, and a binary that does not know the
/// option REFUSES TO START rather than ignoring it — which is the safe
/// direction, but only if somebody actually looks. It is looked at here, once,
/// with two real processes:
///
///   A. `-readonly -safe … "SELECT 1;"` must SUCCEED and print `1`. This is the
///      half that proves the options are UNDERSTOOD: an unknown option makes
///      sqlite3 exit with "unknown option" and print nothing.
///   B. `-readonly -safe … ".shell echo <marker>"` must NOT print the marker.
///      This is the half that proves safe mode is ENFORCED rather than merely
///      accepted.
///
/// NEITHER PROBE TOUCHES A FILE: both run against `:memory:`, so discovery
/// creates nothing on disk and cannot be made to write anywhere.
///
/// B ALONE WOULD BE WORTHLESS — a binary that rejects `-safe` outright also
/// fails to print the marker. It is the PAIR that separates "understood and
/// enforced" from "not understood". Written down because a later reader deleting
/// probe A would leave a test that passes on a binary with no safe mode at all.
fn verify_lock(binary: &Path) -> bool {
    let works = Command::new(binary)
        .args(SAFE_ARGS)
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

    let blocked = Command::new(binary)
        .args(SAFE_ARGS)
        .arg(":memory:")
        .arg(format!(".shell echo {UNSAFE_MARKER}"))
        .stdin(Stdio::null())
        .output();
    match blocked {
        // The marker on EITHER stream means the shell really ran.
        Ok(out) => {
            !String::from_utf8_lossy(&out.stdout).contains(UNSAFE_MARKER)
                && !String::from_utf8_lossy(&out.stderr).contains(UNSAFE_MARKER)
        }
        // The probe could not be run at all: refuse. On uncertainty the tool
        // does not exist — the same direction `verify_shield` takes.
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Running the binary
// ---------------------------------------------------------------------------

/// The outcome of one `sqlite3` run.
struct QueryRun {
    ok: bool,
    stdout: String,
    /// The engine's message. Kept because "no such table: orders" is the single
    /// most useful thing this tool can tell a model that guessed a schema; it
    /// reaches the model through a NAMED, fenced field, never as an instruction.
    stderr: String,
    truncated: bool,
    timed_out: bool,
}

/// Runs the query and returns bounded output.
///
/// THE READING HAPPENS IN A WORKER THREAD AND THE WAITING IN THIS ONE, on
/// purpose. The obvious shape — share the `Child` behind a `Mutex` and let a
/// watchdog thread kill it — deadlocks the moment the main thread calls `wait()`
/// while holding that lock: the watchdog blocks on the mutex, the child never
/// exits, and the "timeout" hangs forever. Splitting it the other way round
/// means this thread OWNS the child and can kill it outright, with no lock in
/// the picture at all.
///
/// STDERR IS READ AFTER STDOUT AND ONLY WHEN STDOUT FIT. If the output hit the
/// cap the child is still writing, so draining stderr first would block; in that
/// case the error text is skipped, which costs nothing — a query that produced
/// 256 KiB of rows did not fail.
fn run_query(binary: &Path, database: &Path, query: &str) -> std::io::Result<QueryRun> {
    let mut child = Command::new(binary)
        .args(SAFE_ARGS)
        .arg(database)
        .arg(query)
        // stdin is CLOSED: sqlite3 must never be able to sit waiting on a
        // terminal that a chat turn does not have.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut out = child.stdout.take().expect("stdout was piped");
    let mut err = child.stderr.take().expect("stderr was piped");
    let (sender, receiver) = mpsc::channel::<(Vec<u8>, Vec<u8>, bool)>();

    std::thread::spawn(move || {
        let mut stdout = Vec::new();
        let _ = out
            .by_ref()
            .take(OUTPUT_CAP as u64)
            .read_to_end(&mut stdout);
        let truncated = stdout.len() >= OUTPUT_CAP;
        let mut stderr = Vec::new();
        if !truncated {
            let _ = err.by_ref().take(ERROR_CAP as u64).read_to_end(&mut stderr);
        }
        // The receiver is gone on the timeout path; that is not an error.
        let _ = sender.send((stdout, stderr, truncated));
    });

    match receiver.recv_timeout(TIMEOUT) {
        Ok((stdout, stderr, truncated)) => {
            if truncated {
                // We stopped reading, so the child is blocked on a full pipe and
                // would never exit on its own.
                let _ = child.kill();
            }
            let status = child.wait()?;
            Ok(QueryRun {
                ok: status.success() && !truncated,
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                truncated,
                timed_out: false,
            })
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Ok(QueryRun {
                ok: false,
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
                timed_out: true,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Turns `-ascii -header` output into a table.
///
/// The shape is `col1 US col2 RS row1a US row1b RS`, with a trailing record
/// separator after the last row. An EMPTY output is zero rows AND zero columns:
/// sqlite3 prints no header at all when a `SELECT` matches nothing, so "which
/// columns were there" is genuinely unknown and must not be invented.
///
/// A MULTI-STATEMENT QUERY prints a header per statement, so the second
/// statement's header arrives as a data row. That is a display quirk, not a
/// safety one — `-readonly` and `-safe` hold whatever the statement count — and
/// it is left visible rather than papered over by dropping rows that "look like"
/// headers, which would silently delete real data.
///
/// EVERY CELL IS STRIPPED OF CONTROL CHARACTERS. A value read out of the user's
/// database lands in the chip detail, which the reporter's `single_line` funnel
/// deliberately does NOT sanitise (the raw record must stay faithful for
/// diagnosis). A row holding `ESC[2J` would then repaint the terminal from
/// inside a table — the same class of hole the chip text funnel was written to
/// close. The separators are split on FIRST and stripped SECOND, so the
/// stripping cannot eat the structure.
pub(crate) fn parse_ascii(raw: &str) -> Table {
    let mut records = raw
        .split(RECORD_SEPARATOR)
        .filter(|r| !r.is_empty())
        .map(|record| {
            record
                .split(FIELD_SEPARATOR)
                .map(strip_control)
                .collect::<Vec<String>>()
        });
    let Some(headers) = records.next() else {
        return Table::default();
    };
    Table::new(headers, records.collect::<Vec<_>>())
}

/// Replaces control characters with a visible dot — the same substitution
/// `tacet_kernel::reporter::single_line` makes, and for the same reason.
/// `is_control()` covers C0, DEL and the C1 range (U+009B is a single-character
/// CSI on a UTF-8 terminal, as good as `ESC [`).
pub(crate) fn strip_control(cell: &str) -> String {
    cell.chars()
        .map(|c| if c.is_control() { '·' } else { c })
        .collect()
}

// ---------------------------------------------------------------------------
// The tool
// ---------------------------------------------------------------------------

pub struct DbTool {
    binary: PathBuf,
    store: Option<Arc<SharedStore>>,
}

impl DbTool {
    /// Builds the tool ONLY if a `sqlite3` that really locks read-only was
    /// found. `None` is not a malfunction — see the catalog note at the top.
    pub fn discover() -> Option<DbTool> {
        let binary = find_binary()?;
        if !verify_lock(&binary) {
            return None;
        }
        Some(DbTool {
            binary,
            store: None,
        })
    }

    /// Why the tool is on or off — printed by the shell. A silent absence would
    /// make a machine without sqlite3 look like a missing feature rather than a
    /// missing package.
    pub fn diagnose() -> String {
        let Some(binary) = find_binary() else {
            return format!(
                "db is off: no sqlite3 binary was found. Paths searched: {}. PATH is deliberately \
                 not searched — the -readonly/-safe guarantee is only worth as much as the \
                 binary it is measured on.",
                SQLITE_PATHS.join(" | ")
            );
        };
        if !verify_lock(&binary) {
            return format!(
                "db is off: {} was found but the read-only measurement did not pass — either \
                 -safe is not supported (it needs SQLite 3.34 or newer) or safe mode did not \
                 block a .shell command. Queries are not run without the lock.",
                binary.display()
            );
        }
        format!(
            "db is on: {} (-readonly -safe; writes, ATTACH, dot-commands and file access all \
             refused by the engine). SQLite only — Postgres has no equivalent tool-level lock, \
             see the note at the top of db.rs.",
            binary.display()
        )
    }

    pub fn with_store(mut self, store: Arc<SharedStore>) -> Self {
        self.store = Some(store);
        self
    }
}

impl Tool for DbTool {
    fn name(&self) -> &str {
        "db"
    }

    fn description(&self) -> &str {
        "Runs a read-only SQL query against a SQLite database file in the working \
         directory and returns the rows. Use when the user asks about data held in a \
         .db/.sqlite file — counts, lookups, listing records. READ-ONLY: it cannot \
         insert, update, delete or alter anything. SQLite only."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new(
                "path",
                ArgSchema::text().description(
                    "Path of the SQLite file inside the working directory, e.g. 'data/app.db'.",
                ),
            )
            .required(),
            Field::new(
                "query",
                ArgSchema::text()
                    .description("The SQL to run, e.g. 'SELECT name, total FROM orders LIMIT 20'."),
            )
            .required(),
        ])
        .description("Query a SQLite database (read-only)")
    }

    /// TRUE. The rows, the column names and the file name are the user's own
    /// data; once they are in the window a later outgoing call could carry them
    /// off the device, so the next external tool must meet the approval gate.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let trace = ctx.start_chip("database", "Querying the database…");

            let outcome = match self.query(&args, ctx) {
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

impl DbTool {
    /// The synchronous body — free of the async wrapper so it is testable
    /// directly.
    fn query(&self, args: &Value, ctx: &ToolContext) -> ToolResult<ToolOutcome> {
        self.schema().validate(args)?;
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| ToolError::MissingField("path".into()))?;
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| ToolError::MissingField("query".into()))?;

        // BELT TO THE `-safe` BRACES, AND LABELLED AS SUCH. The read-only
        // guarantee is `-readonly` + `-safe`, both measured at discovery; this
        // line is not that guarantee. It exists because an argument beginning
        // with `.` is handed to sqlite3's META-COMMAND parser rather than its
        // SQL parser, and there is no flag that says "this argument is SQL". Safe
        // mode already refuses every dangerous meta-command, so what this catches
        // is the harmless remainder — but a call that was never going to be SQL
        // is better refused with a sentence the model can act on than answered
        // with a parser error.
        if query.starts_with('.') {
            return Err(ToolError::InvalidArgument(
                "the query must be SQL, not a sqlite3 dot-command".into(),
            ));
        }

        // THE SANDBOX GATE — CALLED, NOT REWRITTEN. This is what stops both
        // `../../Library/Mail/Envelope Index` and a symlink planted inside the
        // sandbox that points at it; the leaf-only check that misses the second
        // case is the failure `sandbox_path` was written for.
        let database = crate::sandbox_path::resolve_existing_file(ctx, path)?;

        let run = run_query(&self.binary, &database, query)
            .map_err(|_| ToolError::Other("The sqlite3 command could not be run.".into()))?;

        if run.timed_out {
            return Err(ToolError::Timeout);
        }
        if !run.ok {
            // A FAILING QUERY IS AN ANSWER, NOT A MALFUNCTION — but it is
            // returned as a `Failed` outcome all the same, because the model
            // must not read a schema mistake as a result. The engine's sentence
            // goes to the CHIP so the user can see it; `ToolOutcome::failed`
            // gives the model the fixed text, so no database detail (a table
            // name, a column name) leaks into the window on a path the user
            // never approved.
            let reason = first_line(&run.stderr);
            return Err(ToolError::Other(if reason.is_empty() {
                "The query could not be run.".into()
            } else {
                format!("The query could not be run: {reason}")
            }));
        }

        let table = parse_ascii(&run.stdout);
        let rows = table.row_count();
        let file = database
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "database".into());

        if rows == 0 {
            // NOT AN ERROR, A FACT. Reporting "no rows" as a failure tells the
            // model "the tool is broken, answer from memory" — the same reasoning
            // as `web_search`'s `no_results`.
            return Ok(ToolOutcome::read_ok(
                format!("db · {file} · no rows"),
                "no_rows: the query ran and matched nothing",
            )
            .raw_output(run.stdout));
        }

        let headline = format!(
            "db: {rows} rows x {} columns from {file}{}",
            table.column_count(),
            if run.truncated {
                " (the output was cut at the size limit)"
            } else {
                ""
            }
        );
        let preview = table.markdown_truncated(PREVIEW_ROWS);
        // THE CHARACTER CAP AS WELL AS THE ROW CAP: twelve rows of a
        // forty-column table is still a blown window. Truncated at a line
        // boundary so the markdown that survives is still a valid table.
        let preview = cut_at_line(&preview, MODEL_CAP.saturating_sub(headline.chars().count()));
        let summary = format!("{headline}\n{preview}");

        // THE BYPASS CHANNEL. The typed `Table` — not a flattened string — goes
        // into the store, so a later step can truncate it as valid markdown
        // rather than re-parsing text (that is why `data_store::Value` carries
        // the distinction at all).
        let source_ref = match &self.store {
            Some(store) => store.put_value("db", StoredValue::Table(table)),
            None => ctx.store("db", &headline, run.stdout.clone()),
        };
        Ok(ToolOutcome::summarize(
            format!("db · {file} · {rows} rows"),
            summary,
            source_ref.as_str(),
        )
        // The chip detail carries the FULL result: what goes to the
        // model is truncated, what the user can open is not.
        .raw_output(run.stdout))
    }
}

/// The first line of the engine's message, with control characters removed. A
/// multi-line error would break the one-line chip contract, and a raw `ESC`
/// inside it would repaint the terminal.
fn first_line(text: &str) -> String {
    strip_control(text.lines().next().unwrap_or("").trim())
}

/// Cuts at a LINE boundary. A half-written markdown row is worse than a short
/// table: the model tries to reconstruct it and invents the missing cells.
fn cut_at_line(text: &str, cap: usize) -> String {
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
    kept.push_str("(the rest of the rows are behind the source_ref)\n");
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tacet_kernel::{InMemoryDataStore, SilentReporter, ToolState};

    fn block_on<F: std::future::Future>(mut f: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn empty(_: *const ()) {}
        fn clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(clone, empty, empty, empty);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = unsafe { std::pin::Pin::new_unchecked(&mut f) };
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-db-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
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

    /// Builds a small database with the SAME binary the tool measured, opened
    /// WRITABLE — which is only possible because this is the test's own
    /// invocation and not the tool's.
    fn seed(binary: &Path, file: &Path, sql: &str) {
        let out = Command::new(binary)
            .arg(file)
            .arg(sql)
            .stdin(Stdio::null())
            .output()
            .expect("seed");
        assert!(
            out.status.success(),
            "seed failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // -----------------------------------------------------------------------
    // Parsing — no binary needed
    // -----------------------------------------------------------------------

    #[test]
    fn ascii_output_parses_into_a_table() {
        let raw = "id\u{1f}name\u{1e}1\u{1f}Ada\u{1e}2\u{1f}Bob\u{1e}";
        let t = parse_ascii(raw);
        assert_eq!(t.headers, vec!["id", "name"]);
        assert_eq!(t.row_count(), 2);
        assert_eq!(t.rows[1], vec!["2", "Bob"]);
    }

    /// An empty result is zero rows AND zero columns — sqlite3 prints no header
    /// when nothing matched, so the columns are genuinely unknown and must not
    /// be invented.
    #[test]
    fn an_empty_result_is_an_empty_table() {
        let t = parse_ascii("");
        assert_eq!(t.row_count(), 0);
        assert_eq!(t.column_count(), 0);
    }

    /// A CELL CANNOT REPAINT THE TERMINAL. The chip detail carries raw output
    /// and the reporter deliberately does not sanitise that field, so the
    /// stripping has to happen here — otherwise a row of the user's own database
    /// is an escape-sequence injection into their screen.
    #[test]
    fn a_control_character_inside_a_cell_is_neutralised() {
        let raw = "id\u{1f}name\u{1e}1\u{1f}a\u{1b}[2K\rb\u{1e}";
        let t = parse_ascii(raw);
        let cell = &t.rows[0][1];
        for bad in ['\u{1b}', '\r', '\n', '\u{9b}', '\u{7}'] {
            assert!(!cell.contains(bad), "{bad:?} survived: {cell:?}");
        }
        // Neutralising must not hide the value — that would defeat the
        // transparency surface from the other side.
        assert!(cell.contains('a') && cell.contains('b'), "{cell:?}");
    }

    #[test]
    fn the_model_text_is_cut_at_a_line_boundary() {
        let table = Table::new(
            ["a", "b"],
            (0..200)
                .map(|i| vec![format!("value-{i}"), "x".repeat(60)])
                .collect::<Vec<_>>(),
        );
        let cut = cut_at_line(&table.markdown_truncated(PREVIEW_ROWS), 300);
        assert!(cut.chars().count() <= 300 + 60, "{}", cut.chars().count());
        // Every kept line is a whole markdown row.
        for line in cut.lines().filter(|l| l.starts_with('|')) {
            assert!(line.ends_with('|'), "half a row survived: {line}");
        }
        assert!(cut.contains("source_ref"));
    }

    #[test]
    fn the_schema_demands_both_fields() {
        let Some(tool) = DbTool::discover() else {
            return; // no sqlite3 here; the schema test below is covered anyway
        };
        let s = tool.schema();
        assert_eq!(tool.name(), "db");
        assert!(tool.taints_session());
        assert!(
            s.validate(&json!({"path": "a.db", "query": "SELECT 1"}))
                .is_ok()
        );
        assert!(s.validate(&json!({"path": "a.db"})).is_err());
        assert!(s.validate(&json!({"query": "SELECT 1"})).is_err());
    }

    // -----------------------------------------------------------------------
    // The read-only lock — the whole reason this tool is allowed to exist
    // -----------------------------------------------------------------------

    /// THE MEASUREMENT ITSELF. If this machine has a sqlite3 that does not lock,
    /// the tool must NOT be built; if it has one that does, it must be. Either
    /// way `discover()` and `verify_lock()` have to agree — a disagreement is
    /// exactly the "the mechanism was built but never wired up" failure.
    #[test]
    fn discovery_and_the_lock_measurement_agree() {
        let found = find_binary();
        let locked = found.as_deref().map(verify_lock).unwrap_or(false);
        assert_eq!(
            DbTool::discover().is_some(),
            locked,
            "the tool was built without the lock measurement passing"
        );
        // The diagnosis always says which of the two states we are in.
        let text = DbTool::diagnose();
        assert!(
            text.starts_with(if locked { "db is on" } else { "db is off" }),
            "{text}"
        );
    }

    /// A WRITE REALLY FAILS — measured through the tool's own invocation, not
    /// through a hand-built command line. This is the claim `description()`
    /// makes to the user, so it is the claim that has to be measured.
    #[test]
    fn a_write_is_refused_by_the_engine() {
        let Some(tool) = DbTool::discover() else {
            return;
        };
        let root = temp_root("write");
        let file = root.join("app.db");
        seed(
            &tool.binary,
            &file,
            "CREATE TABLE t(a); INSERT INTO t VALUES(1);",
        );
        let ctx = context(&root);

        for sql in [
            "INSERT INTO t VALUES(2)",
            "UPDATE t SET a = 9",
            "DELETE FROM t",
            "DROP TABLE t",
            "CREATE TABLE u(b)",
            // A CTE in front of the write is exactly what a "starts with SELECT"
            // filter would have waved through.
            "WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x",
        ] {
            let outcome = tool.query(&json!({"path": "app.db", "query": sql}), &ctx);
            assert!(outcome.is_err(), "the write was not refused: {sql}");
        }

        // AND THE DATA IS UNCHANGED — the real claim, not just the error text.
        let check = Command::new(&tool.binary)
            .arg(&file)
            .arg("SELECT count(*), sum(a) FROM t;")
            .output()
            .expect("check");
        assert_eq!(String::from_utf8_lossy(&check.stdout).trim(), "1|1");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// SAFE MODE BLOCKS THE COMMAND-EXECUTION HOLE. `.shell` is the reason this
    /// tool needs `-safe` and not merely `-readonly`.
    #[test]
    fn a_dot_command_cannot_run_a_shell() {
        let Some(tool) = DbTool::discover() else {
            return;
        };
        let root = temp_root("shell");
        let file = root.join("app.db");
        seed(&tool.binary, &file, "CREATE TABLE t(a);");
        let ctx = context(&root);
        let witness = root.join("pwned.txt");

        for sql in [
            &format!(".shell touch {}", witness.display()),
            &format!(".system touch {}", witness.display()),
            &format!(".output {}", witness.display()),
            &format!(".once {}", witness.display()),
        ] {
            let _ = tool.query(&json!({"path": "app.db", "query": sql}), &ctx);
            assert!(!witness.exists(), "a dot-command escaped: {sql}");
        }
        // ATTACH cannot create a second, writable database either.
        let other = root.join("other.db");
        let _ = tool.query(
            &json!({"path": "app.db",
                    "query": format!("ATTACH DATABASE '{}' AS x; CREATE TABLE x.a(b);", other.display())}),
            &ctx,
        );
        assert!(!other.exists(), "ATTACH created a writable database");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// THE SANDBOX GATE IS REALLY CALLED. A database outside the working
    /// directory is unreachable both directly and through a planted symlink —
    /// the second is the case a leaf-only check misses.
    #[test]
    fn a_database_outside_the_sandbox_cannot_be_read() {
        let Some(tool) = DbTool::discover() else {
            return;
        };
        let root = temp_root("inside");
        let outside = temp_root("outside");
        let secret = outside.join("secret.db");
        seed(
            &tool.binary,
            &secret,
            "CREATE TABLE s(v); INSERT INTO s VALUES('KEY');",
        );
        let ctx = context(&root);

        let direct = tool.query(
            &json!({"path": secret.display().to_string(), "query": "SELECT * FROM s"}),
            &ctx,
        );
        assert!(matches!(
            direct,
            Err(ToolError::SandboxViolation(_)) | Err(ToolError::FileNotFound(_))
        ));

        assert!(
            tool.query(
                &json!({"path": "../outside/secret.db", "query": "SELECT * FROM s"}),
                &ctx
            )
            .is_err()
        );

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret, root.join("link.db")).expect("link");
            assert!(matches!(
                tool.query(
                    &json!({"path": "link.db", "query": "SELECT * FROM s"}),
                    &ctx
                ),
                Err(ToolError::SandboxViolation(_))
            ));
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// THE HAPPY PATH — a security gate that also refuses the legitimate case is
    /// the one somebody deletes later. Also the proof of the bypass channel: the
    /// model gets a window, the store gets the table.
    #[test]
    fn a_select_returns_rows_through_the_bypass_channel() {
        let Some(tool) = DbTool::discover() else {
            return;
        };
        let root = temp_root("select");
        let file = root.join("app.db");
        let rows: String = (0..200)
            .map(|i| format!("({i},'name-{i}')"))
            .collect::<Vec<_>>()
            .join(",");
        seed(
            &tool.binary,
            &file,
            &format!(
                "CREATE TABLE people(id INTEGER, name TEXT); INSERT INTO people VALUES {rows};"
            ),
        );
        let store = Arc::new(SharedStore::new());
        let tool = tool.with_store(Arc::clone(&store));
        let ctx = context(&root);

        let outcome = tool
            .query(
                &json!({"path": "app.db", "query": "SELECT id, name FROM people"}),
                &ctx,
            )
            .expect("the query must succeed");
        assert!(matches!(outcome.state, ToolState::Read));
        assert!(
            outcome.to_model.contains("200 rows x 2 columns"),
            "{}",
            outcome.to_model
        );
        assert!(outcome.to_model.contains("source_ref"));
        assert!(outcome.to_model.contains("name-0"));
        assert!(
            !outcome.to_model.contains("name-199"),
            "the whole table must not reach the model"
        );
        // The full table really is in the store.
        let refs = store.value(&tacet_kernel::SourceRef("db#1".into()));
        match refs {
            Some(StoredValue::Table(t)) => assert_eq!(t.row_count(), 200),
            other => panic!("a table was expected in the store: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// NO ROWS IS A FACT, NOT A FAILURE. Reported as an error the model would be
    /// told "the tool is broken" and would answer from memory.
    #[test]
    fn an_empty_result_is_reported_as_a_fact() {
        let Some(tool) = DbTool::discover() else {
            return;
        };
        let root = temp_root("empty");
        seed(&tool.binary, &root.join("app.db"), "CREATE TABLE t(a);");
        let ctx = context(&root);
        let outcome = tool
            .query(&json!({"path": "app.db", "query": "SELECT * FROM t"}), &ctx)
            .expect("an empty result is not an error");
        assert!(
            outcome.to_model.starts_with("no_rows"),
            "{}",
            outcome.to_model
        );
        assert!(matches!(outcome.state, ToolState::Read));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A BAD QUERY REACHES THE USER AS A SENTENCE AND THE MODEL AS THE FIXED
    /// TEXT — the schema detail must not enter the window on a path nobody
    /// approved.
    #[test]
    fn a_bad_query_names_the_problem_on_the_chip_but_not_to_the_model() {
        let Some(tool) = DbTool::discover() else {
            return;
        };
        let root = temp_root("badsql");
        seed(&tool.binary, &root.join("app.db"), "CREATE TABLE t(a);");
        let mut ctx = context(&root);
        let outcome = block_on(tool.run(
            json!({"path": "app.db", "query": "SELECT * FROM missing_table"}),
            &mut ctx,
        ));
        assert_eq!(outcome.to_model, tacet_kernel::ERROR_MODEL_TEXT);
        assert!(
            outcome.chip_text.contains("missing_table"),
            "{}",
            outcome.chip_text
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A DOT-COMMAND IS REFUSED BEFORE THE BINARY IS EVEN STARTED — the
    /// belt-to-the-braces layer, asserted so it does not rot into a no-op.
    #[test]
    fn a_dot_command_is_refused_up_front() {
        let Some(tool) = DbTool::discover() else {
            return;
        };
        let root = temp_root("dot");
        seed(&tool.binary, &root.join("app.db"), "CREATE TABLE t(a);");
        let ctx = context(&root);
        assert!(matches!(
            tool.query(&json!({"path": "app.db", "query": " .tables"}), &ctx),
            Err(ToolError::InvalidArgument(_)) | Err(ToolError::Other(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }
}
