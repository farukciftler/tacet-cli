//! `archive` — look inside a .zip, or unpack one, and refuse the whole archive
//! rather than skip an entry.
//!
//! WHY THIS TOOL EXISTS AT ALL: the hard part was already written and audited.
//! RFC-1951 inflate, CRC32, the EOCD scan and the bomb caps live in `tacet-zip`
//! and are exercised by a fuzz test — but the only way in was the OOXML path, so
//! a user who received a .zip had no tool at all. Nothing about decompression is
//! invented here; what is added is the part that has to exist before bytes from
//! somebody else's archive touch the user's disk.
//!
//! THE REFUSAL RULE, AND IT IS THE ONE DECISION THE REST FOLLOWS FROM: every
//! gate refuses the WHOLE ARCHIVE, never an entry. Skipping a bad entry and
//! extracting the rest reports success while silently withholding a file, which
//! is the same failure class `create_document`'s header already records — the
//! user carries around a result they believe is complete. A refusal the user can
//! read is the smaller harm.
//!
//! THE GATES RUN IN THIS ORDER, AND THE ORDER IS THE POINT:
//!   1. the .zip itself goes through `sandbox_path::resolve_existing_file` and a
//!      file-size cap, so a planted link cannot be read through;
//!   2. `tacet_zip::list` reads the central directory WITHOUT decoding a byte,
//!      and every declared-size gate is applied to that listing — a bomb is
//!      refused before any CPU is spent on it;
//!   3. only then is anything inflated, under a ceiling this file chooses;
//!   4. only then does a destination directory come into existence.
//!
//! Step 4 last is what makes "the destination was never created" an OBSERVABLE
//! proof that no inflate ran, which is exactly what the bomb test asserts.
//!
//! BOTH ACTIONS SHARE ONE VALIDATION PASS, deliberately. It would be possible to
//! let `list` show a hostile archive and only refuse at `extract`, and it was
//! rejected: the model would be handed a listing it can never act on, and the
//! entry names — `../../etc/passwd` is the interesting case — are the attacker's
//! text going into the model's window. One pass means the sizes `list` reports
//! are the sizes `extract` writes, or neither happens.
//!
//! WITH ONE EXCEPTION, AND IT IS THE FILESYSTEM'S, NOT THIS FILE'S: two entry
//! names that the DESTINATION considers equal while the duplicate gate does not
//! — macOS folds Unicode normalisation as well as case (measured; see
//! `extract`) — are listed as two entries and refused during the write, by name.
//! Everything a gate here can decide is still decided in the one pass.
//!
//! BULK OUTPUT GOES TO THE STORE. A listing of 400 entries is bulk data; it goes
//! in as a `Table` and the model gets a count plus a `source_ref`. Entry CONTENT
//! never reaches the model on either action — a user who wants a file read calls
//! `read_document` on it afterwards, which is the chain the architecture already
//! has.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult,
    TraceUpdate, boxed,
};

use crate::data_store::{SharedStore, Table, Value as StoredValue};

/// The upper bound on the .zip FILE, mirroring `read_document`'s `FILE_CAP`.
/// Above this it is not a document somebody sent, and the whole file is read
/// into memory before the directory can be walked.
const FILE_CAP: u64 = 32 * 1024 * 1024;

/// The most entries an archive may declare.
///
/// It is not a memory bound — the listing of a hundred thousand entries would
/// fit. It is a bound on how much of the user's directory tree one call may
/// create, and on how large the refusal is when something is wrong.
const MAX_ENTRIES: usize = 512;

/// The ceiling on ONE decoded entry.
///
/// NAMED `MAX_ENTRY_BYTES`, NOT `ENTRY_CAP`, and the collision is the reason:
/// `tacet_zip::ENTRY_CAP` is a public constant of 64 MiB and shadowing it with a
/// different number under the same name is how the two get confused. This one is
/// DELIBERATELY LOWER — the zip crate's cap is sized for an .xlsx part that the
/// document path needs, this one is sized for what a chat assistant should be
/// willing to write onto somebody's disk unattended. It is threaded into
/// `inflate` through `open_selected`, so it binds BEFORE the memory is
/// allocated, not after.
const MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;

/// The ceiling on the whole decoded archive. Held in memory in one piece before
/// anything is written (see `extract`), so this number is also the peak
/// allocation of the tool.
const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;

/// The declared expansion ceiling. 200:1 is far above what real data reaches
/// (text compresses around 3:1, an already-compressed payload around 1:1) and
/// far below the 1000:1 a zip bomb needs to be worth building — the fixture in
/// tacet-zip's own tests is 8144 bytes in, 8 MiB out, i.e. 1030:1.
const MAX_RATIO: usize = 200;

/// The longest entry name accepted, in characters. A name is a path component
/// the user will have to look at; a 4000-character one is not a file name.
const MAX_NAME_CHARS: usize = 200;

/// Above this many entries the listing goes to the DataStore instead of the
/// model. Twenty rows is a table a person reads at a glance and ~600 characters
/// in the window; beyond that the reference is cheaper than the rows.
const LIST_ROWS_TO_MODEL: usize = 20;

/// And above this many CHARACTERS, whatever the row count.
///
/// A ROW COUNT IS NOT A SIZE, which is the hole this closes. `safe_name` accepts
/// a 200-character entry name (`MAX_NAME_CHARS`), and twenty of those is a legal
/// archive that took the "small enough to pass through whole" branch: measured
/// with a 20-entry fixture of 200-character names, the model-facing text was
/// 4,439 characters — roughly 1,775 tokens by the router's own 2/5 estimator, on
/// a `CONTEXT_BUDGET` floor window of 4,096. The listing was not bulk by rows and
/// was bulk by every measure that matters.
///
/// THE ANSWER IS THE STORE, NOT A TRUNCATION. Cutting the text would hand the
/// model a listing missing entries it is not told about — the same silent
/// withholding this file refuses everywhere else. Over the cap the whole listing
/// goes in as a `Table` and the model gets the count plus a `source_ref`, which
/// is what the header already promised. The number is `db.rs`'s and
/// `http_call.rs`'s, so a tool result costs the same wherever it comes from.
const MODEL_CAP: usize = 1400;

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

pub struct ArchiveTool {
    /// The typed store, exactly as `ReadDocumentTool` holds it and for the same
    /// reason: the core contract's `put` only takes a `String` body, and a
    /// listing is a `Table`. Optional so the tool still works — falling back to
    /// text — when it is built without one.
    store: Option<Arc<SharedStore>>,
}

impl Default for ArchiveTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ArchiveTool {
    pub fn new() -> Self {
        Self { store: None }
    }

    pub fn with_store(store: Arc<SharedStore>) -> Self {
        Self { store: Some(store) }
    }
}

impl Tool for ArchiveTool {
    fn name(&self) -> &str {
        "archive"
    }

    /// THE WORDING IS A ROUTER SIGNAL AS WELL AS AN INSTRUCTION, and it was
    /// measured rather than guessed. `router::tool_score` prices a hint found in
    /// the NAME at four times one found in the description and CAPS the
    /// description's contribution at ten characters, so what this text can do
    /// for the tool is bounded — and what it can do TO another tool is the risk.
    /// It is kept short on purpose: `router::overlap`, the tie-break that orders
    /// every zero-scoring tool, matches word stems from the whole
    /// `name + description` string, so a long description is a long list of
    /// chances to outrank a tool on a sentence that has nothing to do with zips.
    fn description(&self) -> &str {
        "Opens a .zip archive on disk: 'list' names the entries inside it without unpacking, \
         'extract' unpacks them into a fresh subfolder of the working folder. Call this when \
         the user asks what is inside a zip, or asks to unzip or unpack one. It refuses the \
         whole archive if an entry would escape the destination, is a symlink, blows past the \
         size caps or fails its CRC."
    }

    fn schema(&self) -> ArgSchema {
        // THE SCHEMA IS THE BOUNDARY. `action` is a `Choice`, so the grammar
        // turns it into a literal alternation and a third action is
        // UNGENERATABLE — "this tool only lists and extracts" is a property of
        // the shape, not of a filter somebody has to keep updated.
        //
        // THERE IS NO `into` FIELD, AND ITS ABSENCE IS THE GUARANTEE. A
        // destination the model can name is a destination the model can aim: the
        // path would then have to be validated, and a validated path is a check
        // that can be got wrong. Instead the destination is derived here — a
        // NEW directory whose name is rotated until it is free — so "extract
        // never overwrites anything and never leaves the working directory" is
        // true because there is no argument that could make it false.
        ArgSchema::object(vec![
            Field::new(
                "path",
                ArgSchema::text().description(
                    "Path to the .zip file, relative to the working directory or an \
                     absolute path in a folder the user opened. Example: backup.zip",
                ),
            )
            .required(),
            Field::new(
                "action",
                ArgSchema::choice(["list", "extract"])
                    .description("list = what is inside, extract = unpack it into a new subfolder"),
            )
            .required(),
        ])
        .description("List or extract a .zip archive")
    }

    /// Entry names and entry content are the user's own data — the same
    /// reasoning as `read_document`. Once they are in the window a later web/mcp
    /// call could carry them off the device, so the session is tainted and the
    /// next external tool meets the approval gate.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let trace = ctx.start_chip("archive", "Reading archive…");

            let (outcome, tainted) = match self.work(&args, ctx) {
                Ok(o) => (o, true),
                Err(e) => (ToolOutcome::failed(&e), false),
            };

            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(args.to_string())
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            // Only a run that REALLY read the archive taints. A refusal touched
            // no content of the user's, and tainting for it would push them into
            // approval prompts that buy nothing.
            if tainted {
                ctx.taint();
            }
            outcome
        })
    }
}

impl ArchiveTool {
    /// The synchronous body; `run` holds only the chip/taint shell so the error
    /// path is collected in one place.
    fn work(&self, args: &Value, ctx: &ToolContext) -> ToolResult<ToolOutcome> {
        self.schema().validate(args)?;

        let raw_path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::MissingField("path".into()))?;
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .ok_or_else(|| ToolError::MissingField("action".into()))?;

        // GATE 1. The canonicalizing gate, not the lexical one: the file exists,
        // so a link planted inside the sandbox would otherwise be followed
        // straight out of it. See `sandbox_path`'s header for the full attack.
        let path = crate::sandbox_path::resolve_existing_file(ctx, raw_path)?;
        let size = path
            .metadata()
            .map_err(|_| ToolError::FileNotFound(path.clone()))?
            .len();
        if size > FILE_CAP {
            return Err(ToolError::Other(format!(
                "the archive is {size} bytes, over the {FILE_CAP} byte limit"
            )));
        }
        let bytes = fs::read(&path)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| raw_path.to_string());

        // GATE 2. The central directory only — nothing is decoded yet.
        let listing = tacet_zip::list(&bytes).map_err(zip_error)?;
        let kept = inspect(&listing)?;

        match action {
            "list" => Ok(self.listing_outcome(ctx, &name, &kept).file_path(path)),
            "extract" => self.extract(ctx, &name, &bytes, &kept),
            other => Err(ToolError::InvalidArgument(format!(
                "unknown action '{other}'"
            ))),
        }
    }

    /// Puts the listing into the store; without a typed store it falls back to
    /// the core contract's `String` body, exactly as `read_document` does.
    fn store_listing(&self, ctx: &ToolContext, table: &Table) -> tacet_kernel::SourceRef {
        match &self.store {
            Some(store) => store.put_value("archive", StoredValue::Table(table.clone())),
            None => ctx.store(
                "archive",
                &format!("{} entries", table.row_count()),
                table.markdown_truncated(usize::MAX),
            ),
        }
    }

    fn listing_outcome(&self, ctx: &ToolContext, name: &str, kept: &[Kept]) -> ToolOutcome {
        let rows: Vec<Vec<String>> = kept
            .iter()
            .map(|k| {
                vec![
                    k.name.clone(),
                    k.declared.to_string(),
                    k.compressed.to_string(),
                ]
            })
            .collect();
        // "declared", NOT "size", AND THE WORD IS THE POINT. Nothing has been
        // decoded at this stage — every number in this table is the central
        // directory's own claim, which `tacet_zip::ZipListing` says outright is
        // unproven. `extract` proves them and refuses an archive that lied;
        // `list` cannot, because proving them is exactly the work `list` exists
        // to avoid. Measured while writing this: an archive declaring 3 bytes
        // for a 100-byte entry is LISTED as 3 and REFUSED on extract
        // (`a_listing_reports_the_declared_size_even_when_it_is_a_lie`). A column
        // headed `size` would have been the tool asserting a fact it does not
        // have.
        let table = Table::new(["path", "declared size", "compressed"], rows);
        let total: usize = kept.iter().map(|k| k.declared).sum();
        let summary = format!(
            "{} entries in {name}, {total} bytes declared when unpacked (the archive's own \
             figures — they are proven only by an extract)",
            kept.len()
        );

        // BULK GOES TO THE STORE. Only a listing that does not fit the preview
        // is put there: a five-entry table already passes through whole, and a
        // reference for it would make the model resolve an indirection for
        // nothing — the same rule `read_document` follows.
        //
        // BOTH MEASURES, because either one alone lets bulk through: 400 short
        // names are too many rows, and 20 two-hundred-character names are few
        // enough rows and 4,439 characters (see `MODEL_CAP`).
        let rendered = format!("{summary}\n{}", table.markdown_truncated(usize::MAX));
        if kept.len() > LIST_ROWS_TO_MODEL || rendered.chars().count() > MODEL_CAP {
            let source = self.store_listing(ctx, &table);
            return ToolOutcome::summarize(
                format!("{name} · {} entries", kept.len()),
                &summary,
                source.as_str(),
            )
            // The chip detail carries the FULL listing: the model's window is
            // truncated, what the user sees is not (transparency, second layer).
            .raw_output(table.markdown_truncated(usize::MAX));
        }

        ToolOutcome::read_ok(format!("{name} · {} entries", kept.len()), rendered)
            .raw_output(table.markdown_truncated(usize::MAX))
    }

    /// GATES 3 AND 4. Everything is decoded and proven BEFORE the destination
    /// exists, so a refusal leaves no directory behind and the absence of one is
    /// the proof that no inflate ran.
    fn extract(
        &self,
        ctx: &ToolContext,
        name: &str,
        bytes: &[u8],
        kept: &[Kept],
    ) -> ToolResult<ToolOutcome> {
        let wanted: Vec<String> = kept
            .iter()
            .filter(|k| !k.is_dir)
            .map(|k| k.name.clone())
            .collect();
        let decoded = tacet_zip::open_selected(bytes, &wanted, MAX_ENTRY_BYTES, MAX_ARCHIVE_BYTES)
            .map_err(zip_error)?;

        // THE DECLARED SIZE MUST BE THE TRUE SIZE. For a DEFLATE entry the
        // reader ignores `raw_size` entirely and a zero CRC is skipped — MEASURED
        // in tacet-zip's `list_reports_the_directory_without_decoding_anything`,
        // where an archive declaring 4 GiB opens without complaint and yields
        // 8 MiB. So the declared numbers are checked against reality HERE, and
        // an archive that lied is refused.
        //
        // IT DOES NOT MAKE `list`'S OUTPUT TRUE — the sentence that used to
        // stand here claimed it did, and that claim was bigger than the code.
        // `list` decodes nothing, so its sizes stay the archive's own claim; all
        // it is protected from is a claim large enough for `inspect` to refuse
        // on. A small lie is listed as written and only caught here, which is
        // why the listing now says "declared" in both the column and the
        // sentence.
        //
        // THE PRICE, written down rather than discovered: an archive written as a
        // STREAM (the local header carries zeros and the real sizes live in a
        // data descriptor) is refused instead of extracted. That is the
        // conservative direction and the error says so.
        for entry in &decoded {
            let Some(k) = kept.iter().find(|k| k.name == entry.name) else {
                return Err(ToolError::Other(
                    "the archive returned an entry that was not asked for".into(),
                ));
            };
            if entry.data.len() != k.declared {
                return Err(ToolError::Other(format!(
                    "'{}' declared {} bytes and decoded to {} — the archive's own \
                     directory does not describe its contents",
                    k.name,
                    k.declared,
                    entry.data.len()
                )));
            }
        }

        // GATE 4. `resolve_path` is the LEXICAL gate and the right one here: the
        // directory does not exist yet, so `canonicalize` could not be asked.
        let folder = ctx.resolve_path(".")?;
        let destination = free_directory(&folder, name);
        fs::create_dir(&destination)?;
        // BELT AND BRACES, exactly as `write_document` does after its own naming
        // loop: the directory was just created and rotated past anything that
        // existed, but the write gate must not depend on the naming rule staying
        // correct forever. Re-proving it canonically is what stops a link that
        // won a race from being written through.
        let destination = crate::sandbox_path::resolve_existing_dir(ctx, &destination)?;

        // FROM HERE ON A FAILURE CAN LEAVE A PARTIAL DIRECTORY, and that is a
        // decision rather than an oversight. Everything that could be refused
        // has been refused above; what is left is the disk itself — no space, a
        // permission, a device gone. Rolling back would mean a recursive delete
        // running on an error path, and a recursive delete is the more dangerous
        // of the two outcomes to get wrong. What remains is a directory the tool
        // created, under a name nothing else had, and the chip says the run
        // failed. NOT MEASURED: no test here fills a disk.
        //
        // AND ONE CASE THAT IS NOT THE DISK: a name pair the DESTINATION
        // FILESYSTEM considers equal while `inspect`'s fold does not. The fold
        // lowercases, which covers `A.txt` vs `a.txt`; it does not normalise
        // Unicode, and macOS's APFS is normalisation-insensitive as well as
        // case-insensitive. MEASURED ON THIS MACHINE (APFS, /private/tmp):
        // writing `é.txt` in NFD and then `é.txt` in NFC leaves ONE file holding
        // the second write. Folding that pair away without a Unicode
        // normalisation table is not something this workspace can do honestly —
        // the table IS the algorithm — so the pair is caught HERE instead, by
        // the filesystem itself, and refused by name. `list` therefore reports
        // two entries where `extract` refuses: the one place the "one pass, so
        // list and extract agree" rule does not hold, and it is written down
        // rather than implied.
        for k in kept.iter().filter(|k| k.is_dir) {
            fs::create_dir_all(destination.join(&k.relative))?;
        }
        let mut written = 0usize;
        let mut bytes_written = 0usize;
        for entry in &decoded {
            let Some(k) = kept.iter().find(|k| k.name == entry.name) else {
                continue;
            };
            let target = destination.join(&k.relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            // `symlink_metadata`, NOT `exists`: `exists` follows a link, so a
            // DANGLING one reports false and `fs::write` would create the file
            // at the link's target — outside the directory we just proved.
            //
            // AND THE TWO READINGS ARE DIFFERENT SENTENCES. A LINK here is
            // something outside this tool put in a directory it created moments
            // ago — an escape, and named as one. A REGULAR FILE here is not an
            // escape at all: the destination was empty, so it can only be an
            // entry this same run already wrote under a name this filesystem
            // considers the same (case, or Unicode normalisation — see the note
            // above). Calling that a `SandboxViolation` was an error message
            // claiming an escape that did not happen.
            if let Ok(meta) = target.symlink_metadata() {
                if meta.file_type().is_symlink() {
                    return Err(ToolError::SandboxViolation(target));
                }
                return Err(ToolError::Other(format!(
                    "'{}' is the same file as an entry already unpacked — this filesystem \
                     treats the two names as one (letter case, or a different Unicode spelling \
                     of the same letters). One would silently replace the other, so the rest of \
                     the archive was not written.",
                    short(&k.name)
                )));
            }
            fs::write(&target, &entry.data)?;
            stamp_owner_only(&target)?;
            written += 1;
            bytes_written += entry.data.len();
        }

        let shown = path_for_model(&destination, &ctx.working_dir);
        Ok(ToolOutcome::written(
            format!("{name} · {written} files unpacked"),
            // COUNTS AND A PATH, NEVER CONTENT. What was inside the files is not
            // the model's business here; if the user wants one read, that is a
            // `read_document` call on a path it now has.
            format!("extracted {written} files ({bytes_written} bytes) into {shown}"),
        )
        .raw_output(format!("{shown}: {written} files, {bytes_written} bytes"))
        .file_path(destination))
    }
}

// ---------------------------------------------------------------------------
// The gates
// ---------------------------------------------------------------------------

/// One entry that survived every gate. `relative` is the name already proven to
/// be a containable relative path — nothing downstream re-parses `name`.
struct Kept {
    name: String,
    relative: PathBuf,
    is_dir: bool,
    declared: usize,
    compressed: usize,
}

/// Every declared-size and name gate, applied to the listing alone.
///
/// NOTHING HERE DECODES. That is what lets the caller refuse a bomb before it
/// costs anything, and it is why the numbers used are the archive's own claims:
/// a claim large enough to refuse on is a refusal that is free.
fn inspect(listing: &[tacet_zip::ZipListing]) -> ToolResult<Vec<Kept>> {
    if listing.len() > MAX_ENTRIES {
        return Err(ToolError::Other(format!(
            "the archive declares {} entries, over the {MAX_ENTRIES} limit",
            listing.len()
        )));
    }

    let mut kept: Vec<Kept> = Vec::with_capacity(listing.len());
    let mut total = 0usize;
    for item in listing {
        // A SYMLINK ENTRY IS NOT A FILE. Its body is the link TARGET, so writing
        // it as a file would be wrong, and creating it as a link would hand the
        // archive's author a pointer out of the sandbox that every later tool
        // would follow. Neither is acceptable, so the archive is refused.
        if item.is_symlink() {
            return Err(ToolError::Other(format!(
                "'{}' inside the archive is a symbolic link; links are not unpacked",
                short(&item.name)
            )));
        }

        let Some((relative, is_dir)) = safe_name(&item.name) else {
            return Err(ToolError::SandboxViolation(PathBuf::from(short(
                &item.name,
            ))));
        };

        // ZIP SEMANTICS ARE "THE LAST ONE WINS", so two entries with the same
        // name mean one of them is silently shadowed — a file the user never
        // learns did not arrive. Folded to lowercase because the destination may
        // sit on a case-insensitive filesystem (the default on macOS), where
        // `A.txt` and `a.txt` collide even though the archive keeps them apart.
        let folded = relative.to_string_lossy().to_lowercase();
        if kept
            .iter()
            .any(|k| k.relative.to_string_lossy().to_lowercase() == folded)
        {
            return Err(ToolError::Other(format!(
                "'{}' occurs twice in the archive; one copy would silently replace the other",
                short(&item.name)
            )));
        }

        if item.declared_size > MAX_ENTRY_BYTES {
            return Err(ToolError::Other(format!(
                "'{}' declares {} bytes, over the {MAX_ENTRY_BYTES} byte per-entry limit",
                short(&item.name),
                item.declared_size
            )));
        }
        // THE RATIO GATE, ON THE DECLARED NUMBERS AND THEREFORE FREE. A body of
        // zero bytes that claims to decode to something is not a ratio at all —
        // it is division by zero, and refusing it is the only answer that does
        // not need a special case further down.
        if item.compressed_size == 0 {
            if item.declared_size > 0 {
                return Err(ToolError::Other(format!(
                    "'{}' claims {} bytes from nothing",
                    short(&item.name),
                    item.declared_size
                )));
            }
        } else if item.declared_size / item.compressed_size > MAX_RATIO {
            return Err(ToolError::Other(format!(
                "'{}' expands {}:1, over the {MAX_RATIO}:1 limit",
                short(&item.name),
                item.declared_size / item.compressed_size
            )));
        }

        total = total
            .checked_add(item.declared_size)
            .ok_or_else(|| ToolError::Other("the declared total size overflowed".into()))?;
        if total > MAX_ARCHIVE_BYTES {
            return Err(ToolError::Other(format!(
                "the archive declares more than the {MAX_ARCHIVE_BYTES} byte total limit"
            )));
        }

        kept.push(Kept {
            name: item.name.clone(),
            relative,
            is_dir,
            declared: item.declared_size,
            compressed: item.compressed_size,
        });
    }
    Ok(kept)
}

/// Turns an entry name into a relative path, or refuses it.
///
/// THE CHECKS ARE PLATFORM-INDEPENDENT ON PURPOSE, and that is the whole reason
/// the byte-level rules sit alongside the component walk. On unix a backslash is
/// an ordinary character, so `..\..\x.txt` is ONE `Component::Normal` and the
/// component walk waves it through — while on Windows the same archive escapes
/// two directories. The same is true of `C:\Windows\x`, which is a `Prefix`
/// there and a plain name here. A refusal that only holds on the platform the
/// test ran on is the gap this class of check exists to close, so `\` and `:`
/// are refused everywhere.
///
/// `.` COMPONENTS ARE DROPPED RATHER THAN REFUSED: `zip -r` writes `./name`
/// routinely and removing a no-op component changes nothing about containment.
/// `..`, an absolute root and a Windows prefix are refused outright — a path
/// with two readings must never have one of them accepted.
fn safe_name(raw: &str) -> Option<(PathBuf, bool)> {
    if raw.is_empty() || raw.chars().count() > MAX_NAME_CHARS {
        return None;
    }
    // A control character (NUL included) in a file name is never legitimate and
    // is how a name is made to read as one thing and mean another.
    if raw.contains('\\') || raw.contains(':') || raw.chars().any(char::is_control) {
        return None;
    }
    let is_dir = raw.ends_with('/');
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let mut out = PathBuf::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if out.as_os_str().is_empty() {
        return None;
    }
    Some((out, is_dir))
}

/// The destination: a NEW directory under `folder`, rotated until the name is
/// free.
///
/// `symlink_metadata`, NOT `exists` — the same lesson `create_document`'s
/// `target_path` records. `exists` follows a link, so a DANGLING link named
/// `backup` reports false, the loop never turns, and the extraction would land
/// wherever the link points. `symlink_metadata` sees the link itself.
fn free_directory(folder: &Path, archive_name: &str) -> PathBuf {
    let stem: String = archive_name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(archive_name)
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let base = {
        let cleaned = stem.trim().trim_matches('.').trim().to_string();
        if cleaned.is_empty() {
            "archive".to_string()
        } else {
            cleaned
        }
    };
    let mut candidate = folder.join(&base);
    let mut i = 2;
    while candidate.symlink_metadata().is_ok() {
        candidate = folder.join(format!("{base}-{i}"));
        i += 1;
    }
    candidate
}

/// The owner-only stamp on an extracted file.
///
/// THE SAME PLATFORM RECORD AS `create_document::finish_up`, RESTATED HERE
/// RATHER THAN ASSUMED KNOWN, because this tool materialises somebody else's
/// archive on the user's disk and that is the moment the absence matters. On
/// Windows there is NO counterpart and none has been faked: `set_permissions`
/// there only flips the read-only flag, which is accident protection and not
/// access control, and the real counterpart is narrowing the file ACL — a
/// dependency this workspace does not take. On Windows the privacy of an
/// extracted file therefore rests on the directory ACL of the user's profile
/// tree. NOT MEASURED ON THIS MACHINE (no Windows here); the basis is the
/// operating system default, not a stamp we applied.
fn stamp_owner_only(target: &Path) -> ToolResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(target, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = target;
    }
    Ok(())
}

/// The path the model is told about: relative to the working directory and
/// spelled with forward slashes on every platform. The reasoning — a Windows
/// backslash is a JSON escape character when the model hands the path back — is
/// written out in full above `create_document::path_for_model`.
fn path_for_model(path: &Path, working_dir: &Path) -> String {
    let base = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());
    let full = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    match full.strip_prefix(&base) {
        Ok(relative) => relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => path.display().to_string(),
    }
}

/// An entry name inside an error message is ATTACKER TEXT. It is cut so a
/// crafted name cannot turn a one-line chip into a wall, and the user still sees
/// enough to recognise which entry was refused.
fn short(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .take(60)
        .collect();
    clean
}

/// The zip layer's error, translated into a sentence the user can act on.
///
/// `ZipError`'s own `Display` is written for diagnostics and reads as
/// "the zip content exceeds the limit (...)"; it is fine on a chip, and the
/// model still gets the fixed `ERROR_MODEL_TEXT` either way (see
/// `ToolOutcome::failed`). What matters is that a LIMIT refusal says so, because
/// the alternative — a legitimate large archive refused with "the file is
/// broken" — sends the user looking for the wrong problem.
fn zip_error(error: tacet_zip::ZipError) -> ToolError {
    ToolError::Other(format!("{error}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tacet_kernel::{SilentReporter, SourceRef, ToolState};

    /// Builds a zip container around ALREADY-ENCODED bodies.
    ///
    /// `tacet_zip::pack` cannot produce a single one of the archives below: it
    /// always writes method 0 with `compressed_size == raw_size` and a correct
    /// CRC (writer.rs says so, and says why). Every hostile case here is exactly
    /// an archive that does NOT look like that — a name that escapes, a size the
    /// header lies about, a mode bit claiming a symlink, a CRC that does not
    /// match. So the container is assembled field by field. It validates
    /// nothing, on purpose: a fixture that refuses bad input cannot test a gate.
    struct RawEntry {
        name: String,
        body: Vec<u8>,
        method: u16,
        crc: u32,
        declared_size: u32,
        external_attributes: u32,
    }

    impl RawEntry {
        /// A well-formed STORE entry — the shape everything else deviates from.
        fn stored(name: &str, body: &[u8]) -> Self {
            Self {
                name: name.to_string(),
                body: body.to_vec(),
                method: 0,
                crc: tacet_zip::crc32(body),
                declared_size: body.len() as u32,
                external_attributes: 0o100_644 << 16,
            }
        }
    }

    fn raw_zip(entries: &[RawEntry]) -> Vec<u8> {
        fn w16(out: &mut Vec<u8>, v: u16) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        fn w32(out: &mut Vec<u8>, v: u32) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let mut body: Vec<u8> = Vec::new();
        let mut central: Vec<u8> = Vec::new();
        for e in entries {
            let offset = body.len() as u32;
            let compressed = e.body.len() as u32;
            w32(&mut body, 0x0403_4b50);
            w16(&mut body, 20);
            w16(&mut body, 0);
            w16(&mut body, e.method);
            w16(&mut body, 0);
            w16(&mut body, 0);
            w32(&mut body, e.crc);
            w32(&mut body, compressed);
            w32(&mut body, e.declared_size);
            w16(&mut body, e.name.len() as u16);
            w16(&mut body, 0);
            body.extend_from_slice(e.name.as_bytes());
            body.extend_from_slice(&e.body);

            w32(&mut central, 0x0201_4b50);
            w16(&mut central, 20);
            w16(&mut central, 20);
            w16(&mut central, 0);
            w16(&mut central, e.method);
            w16(&mut central, 0);
            w16(&mut central, 0);
            w32(&mut central, e.crc);
            w32(&mut central, compressed);
            w32(&mut central, e.declared_size);
            w16(&mut central, e.name.len() as u16);
            w16(&mut central, 0);
            w16(&mut central, 0);
            w16(&mut central, 0);
            w16(&mut central, 0);
            w32(&mut central, e.external_attributes);
            w32(&mut central, offset);
            central.extend_from_slice(e.name.as_bytes());
        }
        let central_offset = body.len() as u32;
        let central_size = central.len() as u32;
        let count = entries.len() as u16;
        let mut out = body;
        out.extend_from_slice(&central);
        w32(&mut out, 0x0605_4b50);
        w16(&mut out, 0);
        w16(&mut out, 0);
        w16(&mut out, count);
        w16(&mut out, count);
        w32(&mut out, central_size);
        w32(&mut out, central_offset);
        w16(&mut out, 0);
        out
    }

    /// A DEFLATE stream carrying `data` in a single STORED block (RFC 1951,
    /// 3.2.4). It is real deflate — the reader's `inflate` decodes it through
    /// the ordinary path — and it is the smallest way to build a method-8 entry
    /// whose decoded length is under our control, which is what the
    /// lying-header case needs.
    fn deflate_stored(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x01u8];
        let length = data.len() as u16;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    fn temp_root(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-archive-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&path).expect("temp dir");
        path.canonicalize().expect("resolved")
    }

    /// The root list is process-wide; every test that resolves a path takes the
    /// same lock `sandbox_path`'s tests take and starts from an empty list. A
    /// test that inherited another test's roots would be measuring a scope
    /// nobody wrote down.
    fn isolated() -> std::sync::MutexGuard<'static, ()> {
        let guard = crate::workspace::test_lock();
        crate::workspace::clear_roots();
        guard
    }

    fn context(root: &Path, store: &Arc<SharedStore>) -> ToolContext {
        ToolContext::new(
            Arc::clone(store) as Arc<dyn tacet_kernel::DataStore>,
            root.to_path_buf(),
            Arc::new(SilentReporter),
        )
    }

    fn write_zip(root: &Path, name: &str, entries: &[RawEntry]) -> PathBuf {
        let path = root.join(name);
        fs::write(&path, raw_zip(entries)).expect("fixture written");
        path
    }

    /// The refusal, as the error the gate produced. `run` would flatten every
    /// variant into `ToolState::Failed`, and which gate refused is exactly what
    /// these tests are about.
    fn refusal(tool: &ArchiveTool, ctx: &ToolContext, args: Value) -> ToolError {
        tool.work(&args, ctx)
            .expect_err("this archive must be refused")
    }

    fn extract_args(name: &str) -> Value {
        json!({"path": name, "action": "extract"})
    }

    /// Nothing was unpacked: the working directory holds the archive and
    /// nothing else. Stronger than "the destination is absent", because it also
    /// catches a stray file written somewhere else under the root.
    fn only_the_archive_is_there(root: &Path, archive: &str) {
        let names: Vec<String> = fs::read_dir(root)
            .expect("readable")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![archive.to_string()],
            "the refusal left something behind"
        );
    }

    /// Core has no tokio; the minimal executor used everywhere else in this
    /// crate's tests.
    fn execute<F: std::future::Future>(mut f: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn noop(_: *const ()) {}
        fn clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = unsafe { std::pin::Pin::new_unchecked(&mut f) };
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    // --- the happy path, so a refusal below means something ---

    #[test]
    fn an_ordinary_archive_is_listed_and_unpacked() {
        let _guard = isolated();
        let root = temp_root("happy");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::with_store(Arc::clone(&store));
        write_zip(
            &root,
            "backup.zip",
            &[
                RawEntry::stored("notes/", b""),
                RawEntry::stored("notes/a.txt", b"alpha"),
                RawEntry::stored("b.txt", b"beta"),
            ],
        );

        let listed = tool
            .work(&json!({"path": "backup.zip", "action": "list"}), &ctx)
            .expect("listing");
        assert!(listed.to_model.contains("notes/a.txt"));
        assert!(
            !listed.to_model.contains("alpha"),
            "entry CONTENT must never reach the model from a listing"
        );

        let done = tool
            .work(&extract_args("backup.zip"), &ctx)
            .expect("extract");
        assert!(
            done.state.changed_world(),
            "extraction must close off retrying"
        );
        assert_eq!(fs::read(root.join("backup/notes/a.txt")).unwrap(), b"alpha");
        assert_eq!(fs::read(root.join("backup/b.txt")).unwrap(), b"beta");
        assert!(root.join("backup/notes").is_dir());
        assert!(
            !done.to_model.contains("alpha"),
            "file content must not reach the model on extraction either"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(root.join("backup/b.txt"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "extracted files carry the owner stamp");
        }
    }

    // --- name gates ---

    #[test]
    fn zip_slip_is_refused_and_writes_nothing_outside() {
        let _guard = isolated();
        let root = temp_root("slip");
        let outside = temp_root("slip-victim");
        let victim = outside.join("escape.txt");
        fs::write(&victim, b"ORIGINAL").expect("victim");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        write_zip(
            &root,
            "evil.zip",
            &[RawEntry::stored("../../escape.txt", b"OWNED")],
        );

        assert!(matches!(
            refusal(&tool, &ctx, extract_args("evil.zip")),
            ToolError::SandboxViolation(_)
        ));
        // The refusal is not the whole claim: the file it aimed at is untouched.
        assert_eq!(fs::read(&victim).unwrap(), b"ORIGINAL");
        only_the_archive_is_there(&root, "evil.zip");
        // AND `list` REFUSES IT TOO. One validation pass means the model is
        // never shown a listing it could not act on.
        assert!(
            tool.work(&json!({"path": "evil.zip", "action": "list"}), &ctx)
                .is_err()
        );
    }

    #[test]
    fn an_absolute_entry_name_is_refused() {
        let _guard = isolated();
        let root = temp_root("absolute");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        for (file, name) in [("unix.zip", "/etc/passwd"), ("win.zip", "C:\\Windows\\x")] {
            write_zip(&root, file, &[RawEntry::stored(name, b"x")]);
            assert!(
                matches!(
                    refusal(&tool, &ctx, extract_args(file)),
                    ToolError::SandboxViolation(_)
                ),
                "{name} was not refused"
            );
            assert!(!root.join("unix").exists() && !root.join("win").exists());
        }
    }

    /// THE PLATFORM-INDEPENDENCE CLAIM, and it is the reason `safe_name` checks
    /// bytes as well as components. On unix `\` is an ordinary character, so
    /// `..\..\x.txt` is ONE `Component::Normal` and a component-only check waves
    /// it through — while the same archive escapes two directories on Windows.
    /// A refusal that only holds where the test runs is the gap this test exists
    /// to close.
    #[test]
    fn a_backslash_entry_name_is_refused_on_every_platform() {
        let _guard = isolated();
        let root = temp_root("backslash");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        write_zip(&root, "win.zip", &[RawEntry::stored("..\\..\\x.txt", b"x")]);
        assert!(matches!(
            refusal(&tool, &ctx, extract_args("win.zip")),
            ToolError::SandboxViolation(_)
        ));
        only_the_archive_is_there(&root, "win.zip");
    }

    #[test]
    fn a_control_character_or_overlong_name_is_refused() {
        let _guard = isolated();
        let root = temp_root("names");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        let long = "a".repeat(MAX_NAME_CHARS + 1);
        for (file, name) in [
            ("nul.zip", "a\u{0}b.txt".to_string()),
            ("newline.zip", "a\nb.txt".to_string()),
            ("long.zip", long),
        ] {
            write_zip(&root, file, &[RawEntry::stored(&name, b"x")]);
            assert!(
                matches!(
                    refusal(&tool, &ctx, extract_args(file)),
                    ToolError::SandboxViolation(_)
                ),
                "{file} was not refused"
            );
        }
    }

    /// ZIP SEMANTICS ARE "THE LAST ONE WINS", so a duplicate name means one file
    /// silently does not arrive — a success report that withholds a file.
    #[test]
    fn a_duplicate_entry_name_is_refused() {
        let _guard = isolated();
        let root = temp_root("dup");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        write_zip(
            &root,
            "dup.zip",
            &[
                RawEntry::stored("a.txt", b"first"),
                RawEntry::stored("a.txt", b"second"),
            ],
        );
        assert!(
            refusal(&tool, &ctx, extract_args("dup.zip"))
                .short_error()
                .contains("twice")
        );
        only_the_archive_is_there(&root, "dup.zip");

        // AND CASE-FOLDED, because the destination may be a case-insensitive
        // filesystem (the macOS default), where these two collide even though
        // the archive keeps them apart.
        write_zip(
            &root,
            "case.zip",
            &[
                RawEntry::stored("A.txt", b"first"),
                RawEntry::stored("a.txt", b"second"),
            ],
        );
        assert!(tool.work(&extract_args("case.zip"), &ctx).is_err());
    }

    #[test]
    fn a_symlink_entry_is_refused() {
        let _guard = isolated();
        let root = temp_root("symentry");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        let mut link = RawEntry::stored("gate", b"/etc/passwd");
        // Exactly what `zip` writes for a symbolic link: S_IFLNK in the high
        // half of the external attributes. The BODY is the link target.
        link.external_attributes = 0o120_777 << 16;
        write_zip(&root, "link.zip", &[link]);
        assert!(
            refusal(&tool, &ctx, extract_args("link.zip"))
                .short_error()
                .contains("link"),
            "the refusal must say it was a link"
        );
        only_the_archive_is_there(&root, "link.zip");
    }

    // --- size gates ---

    /// WHAT PROVES NO INFLATE RAN IS THE ERROR STRING, not the missing
    /// directory. The comment here used to name the absent destination as the
    /// receipt, and that is weaker than it sounds: the directory is created
    /// after decoding, so its absence only says the decode did not FINISH.
    /// "per-entry limit" and "expands" are written in exactly one place —
    /// `inspect`, which is handed the central-directory listing and nothing
    /// else — so reading one of them back is the proof that the refusal
    /// happened before a byte was decompressed. The absent directory is kept as
    /// the second half: refused, AND nothing on disk.
    #[test]
    fn a_declared_bomb_is_refused_before_anything_is_inflated() {
        let _guard = isolated();
        let root = temp_root("bomb");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();

        // (a) the per-entry ceiling: 4 GiB declared.
        let mut huge = RawEntry::stored("big.bin", &[0u8; 16]);
        huge.declared_size = u32::MAX;
        huge.method = 8;
        write_zip(&root, "huge.zip", &[huge]);
        assert!(
            refusal(&tool, &ctx, extract_args("huge.zip"))
                .short_error()
                .contains("per-entry limit")
        );

        // (b) the RATIO ceiling, under the per-entry ceiling: 2 MiB declared out
        //     of 4 KiB compressed is 512:1, and 200:1 is the line.
        let mut ratio = RawEntry::stored("ratio.bin", &[0u8; 4096]);
        ratio.declared_size = 2 * 1024 * 1024;
        ratio.method = 8;
        write_zip(&root, "ratio.zip", &[ratio]);
        assert!(
            refusal(&tool, &ctx, extract_args("ratio.zip"))
                .short_error()
                .contains("expands")
        );

        // (c) a body of nothing that claims to become something. NAMED, not
        // merely `is_err()`: a bare "it failed" would still pass if the zero
        // divisor made `inspect` panic-free by accident somewhere else, and the
        // division-by-zero branch is the one being measured.
        let mut nothing = RawEntry::stored("nothing.bin", b"");
        nothing.declared_size = 1024;
        nothing.method = 8;
        write_zip(&root, "nothing.zip", &[nothing]);
        assert!(
            refusal(&tool, &ctx, extract_args("nothing.zip"))
                .short_error()
                .contains("from nothing")
        );

        assert!(!root.join("huge").exists());
        assert!(!root.join("ratio").exists());
        assert!(!root.join("nothing").exists());
    }

    /// A HEADER THAT LIES ABOUT ITS SIZE. For a DEFLATE entry the zip reader
    /// ignores `raw_size` entirely and skips a zero CRC — measured in tacet-zip's
    /// own `list_reports_the_directory_without_decoding_anything`. So the
    /// declared numbers are proven against reality here, and an archive whose
    /// directory does not describe its contents is refused rather than unpacked.
    #[test]
    fn a_header_that_lies_about_its_size_is_refused() {
        let _guard = isolated();
        let root = temp_root("liar");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        let real = vec![7u8; 100];
        let stream = deflate_stored(&real);
        let liar = RawEntry {
            name: "small.bin".into(),
            body: stream,
            method: 8,
            crc: 0, // a zero CRC is skipped by the reader — the other half of the lie
            declared_size: 3,
            external_attributes: 0o100_644 << 16,
        };
        write_zip(&root, "liar.zip", &[liar]);
        assert!(
            refusal(&tool, &ctx, extract_args("liar.zip"))
                .short_error()
                .contains("does not describe its contents")
        );
        only_the_archive_is_there(&root, "liar.zip");

        // THE SAME FIXTURE WITH AN HONEST HEADER UNPACKS, so the refusal above
        // is the rule working rather than the deflate fixture being broken.
        let honest = RawEntry {
            name: "small.bin".into(),
            body: deflate_stored(&real),
            method: 8,
            crc: tacet_zip::crc32(&real),
            declared_size: 100,
            external_attributes: 0o100_644 << 16,
        };
        write_zip(&root, "honest.zip", &[honest]);
        tool.work(&extract_args("honest.zip"), &ctx)
            .expect("an honest archive unpacks");
        assert_eq!(fs::read(root.join("honest/small.bin")).unwrap(), real);
    }

    /// THE NON-GUARANTEE, PINNED DOWN. `list` decodes nothing — that is the
    /// whole reason it can answer a bomb for free — so the sizes it reports are
    /// the central directory's own claim and NOT a fact. This test states that
    /// limitation as a measurement rather than leaving it to a comment: the same
    /// archive whose header lies is LISTED as declaring 3 bytes and is REFUSED
    /// the moment an extract proves the number against the 100 bytes really
    /// there.
    ///
    /// IT IS ALSO WHAT KEEPS THE WORDING HONEST. If somebody re-heads the column
    /// `size` or drops "declared" from the sentence, the tool starts asserting a
    /// fact it does not have and this test fails on the string.
    #[test]
    fn a_listing_reports_the_declared_size_even_when_it_is_a_lie() {
        let _guard = isolated();
        let root = temp_root("liar-list");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::with_store(Arc::clone(&store));
        let real = vec![7u8; 100];
        let liar = RawEntry {
            name: "small.bin".into(),
            body: deflate_stored(&real),
            method: 8,
            crc: 0,
            declared_size: 3,
            external_attributes: 0o100_644 << 16,
        };
        write_zip(&root, "liar.zip", &[liar]);

        let listed = tool
            .work(&json!({"path": "liar.zip", "action": "list"}), &ctx)
            .expect("a listing decodes nothing, so it cannot notice the lie");
        assert!(
            listed.to_model.contains("declared"),
            "the listing presents unproven numbers as fact: {}",
            listed.to_model
        );
        assert!(
            listed.to_model.contains("3 bytes declared"),
            "the archive's own figure is not what was reported: {}",
            listed.to_model
        );
        // AND THE EXTRACT IS WHERE IT IS CAUGHT — so the pair says exactly where
        // the proof lives.
        assert!(
            refusal(&tool, &ctx, extract_args("liar.zip"))
                .short_error()
                .contains("does not describe its contents")
        );
        only_the_archive_is_there(&root, "liar.zip");
    }

    #[test]
    fn an_entry_count_explosion_is_refused() {
        let _guard = isolated();
        let root = temp_root("count");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        let entries: Vec<RawEntry> = (0..MAX_ENTRIES + 1)
            .map(|i| RawEntry::stored(&format!("f{i}.txt"), b""))
            .collect();
        write_zip(&root, "many.zip", &entries);
        assert!(
            refusal(&tool, &ctx, extract_args("many.zip"))
                .short_error()
                .contains("over the")
        );
        only_the_archive_is_there(&root, "many.zip");
    }

    #[test]
    fn a_crc_mismatch_leaves_the_destination_absent() {
        let _guard = isolated();
        let root = temp_root("crc");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        let mut bad = RawEntry::stored("a.txt", b"alpha");
        bad.crc ^= 0xFFFF_FFFF;
        write_zip(&root, "corrupt.zip", &[bad]);
        assert!(tool.work(&extract_args("corrupt.zip"), &ctx).is_err());
        only_the_archive_is_there(&root, "corrupt.zip");
    }

    #[test]
    fn a_truncated_archive_is_refused() {
        let _guard = isolated();
        let root = temp_root("cut");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        let whole = raw_zip(&[RawEntry::stored("a.txt", b"alpha")]);
        // Every truncation, not one: the failure this guards against is a
        // decoder that reads past the end at some particular length.
        for cut in 0..whole.len() {
            fs::write(root.join("cut.zip"), &whole[..cut]).expect("fixture");
            assert!(
                tool.work(&extract_args("cut.zip"), &ctx).is_err(),
                "a {cut}-byte archive was accepted"
            );
        }
        only_the_archive_is_there(&root, "cut.zip");
    }

    // --- the destination ---

    /// NEVER OVERWRITE, AND NEVER FOLLOW A LINK OUT. The rotation uses
    /// `symlink_metadata`, so a DANGLING link named like the destination — which
    /// `exists()` reports as false — is stepped over rather than written
    /// through.
    #[cfg(unix)]
    #[test]
    fn a_planted_link_at_the_destination_is_stepped_over_not_written_through() {
        let _guard = isolated();
        let root = temp_root("planted");
        let outside = temp_root("planted-victim");
        let victim = outside.join("backup");
        fs::create_dir_all(victim.join("notes")).expect("victim tree");
        fs::write(victim.join("notes/a.txt"), b"ORIGINAL").expect("victim file");
        std::os::unix::fs::symlink(&victim, root.join("backup")).expect("plant");

        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        write_zip(
            &root,
            "backup.zip",
            &[RawEntry::stored("notes/a.txt", b"OWNED")],
        );
        tool.work(&extract_args("backup.zip"), &ctx)
            .expect("extraction rotates past the link");

        assert_eq!(
            fs::read(victim.join("notes/a.txt")).unwrap(),
            b"ORIGINAL",
            "the file behind the planted link was written through"
        );
        assert_eq!(
            fs::read(root.join("backup-2/notes/a.txt")).unwrap(),
            b"OWNED"
        );
    }

    #[test]
    fn an_existing_directory_is_never_overwritten() {
        let _guard = isolated();
        let root = temp_root("rotate");
        fs::create_dir(root.join("backup")).expect("existing dir");
        fs::write(root.join("backup/keep.txt"), b"KEEP").expect("existing file");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        write_zip(&root, "backup.zip", &[RawEntry::stored("keep.txt", b"NEW")]);

        tool.work(&extract_args("backup.zip"), &ctx)
            .expect("extract");
        assert_eq!(
            fs::read(root.join("backup/keep.txt")).unwrap(),
            b"KEEP",
            "the existing file was overwritten"
        );
        assert_eq!(fs::read(root.join("backup-2/keep.txt")).unwrap(), b"NEW");
    }

    #[test]
    fn the_archive_path_cannot_leave_the_sandbox() {
        let _guard = isolated();
        let root = temp_root("escape");
        let outside = temp_root("escape-outside");
        fs::write(outside.join("secret.zip"), raw_zip(&[])).expect("outside archive");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();

        assert!(matches!(
            refusal(&tool, &ctx, extract_args("../../secret.zip")),
            ToolError::SandboxViolation(_)
        ));
        assert!(matches!(
            refusal(
                &tool,
                &ctx,
                extract_args(&outside.join("secret.zip").to_string_lossy())
            ),
            ToolError::SandboxViolation(_)
        ));
        // AND THROUGH A PLANTED DIRECTORY LINK — the case a lexical check waves
        // through and the reason `resolve_existing_file` is the gate here.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("gate")).expect("plant");
            assert!(matches!(
                refusal(&tool, &ctx, extract_args("gate/secret.zip")),
                ToolError::SandboxViolation(_)
            ));
        }
    }

    // --- the model's window ---

    #[test]
    fn a_large_listing_goes_to_the_store_and_not_to_the_model() {
        let _guard = isolated();
        let root = temp_root("bulk");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::with_store(Arc::clone(&store));
        let entries: Vec<RawEntry> = (0..400)
            .map(|i| RawEntry::stored(&format!("part/{i:04}-invoice.txt"), b"x"))
            .collect();
        write_zip(&root, "many.zip", &entries);

        let out = tool
            .work(&json!({"path": "many.zip", "action": "list"}), &ctx)
            .expect("listing");
        assert!(
            out.to_model.len() < 1500,
            "400 rows reached the model: {} chars",
            out.to_model.len()
        );
        assert!(out.to_model.contains("source_ref="));
        assert!(out.to_model.contains("400 entries"));
        // The rows are not lost — they are in the store, in full.
        let reference = out
            .to_model
            .split("source_ref=")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("a reference");
        let Some(crate::data_store::Value::Table(table)) =
            store.value(&SourceRef(reference.to_string()))
        else {
            panic!("the listing is not in the store as a table");
        };
        assert_eq!(table.row_count(), 400);
    }

    /// TWENTY ENTRIES IS NOT A SIZE. The store bypass used to be gated on the
    /// ROW COUNT alone, so an archive with 20 legal 200-character names — under
    /// `LIST_ROWS_TO_MODEL`, under `MAX_NAME_CHARS`, nothing hostile about it —
    /// went to the model whole. Measured before the fix by removing the
    /// character half of the gate and re-running exactly this test: 4,439
    /// characters of `to_model`, about 1,775 tokens by the router's 2/5
    /// estimator, on a 4,096
    /// window that also has to hold the tool block and the conversation.
    ///
    /// The rows are not dropped, and the assertion below says so: they go to the
    /// store in full, which is what the every-other-tool rule already was.
    #[test]
    fn a_listing_of_long_names_goes_to_the_store_even_though_the_rows_are_few() {
        let _guard = isolated();
        let root = temp_root("longnames");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::with_store(Arc::clone(&store));
        // 200 characters is the longest name `safe_name` accepts, and 20 rows is
        // not MORE than `LIST_ROWS_TO_MODEL` — the exact corner between the two.
        let entries: Vec<RawEntry> = (0..20)
            .map(|i| {
                let name = format!("{i:02}-{}", "n".repeat(197));
                assert_eq!(name.chars().count(), MAX_NAME_CHARS);
                RawEntry::stored(&name, b"x")
            })
            .collect();
        write_zip(&root, "long.zip", &entries);

        let out = tool
            .work(&json!({"path": "long.zip", "action": "list"}), &ctx)
            .expect("listing");
        assert!(
            out.to_model.chars().count() <= MODEL_CAP,
            "a 20-entry listing put {} characters in the model's window",
            out.to_model.chars().count()
        );
        assert!(out.to_model.contains("source_ref="));
        let reference = out
            .to_model
            .split("source_ref=")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .expect("a reference");
        let Some(crate::data_store::Value::Table(table)) =
            store.value(&SourceRef(reference.to_string()))
        else {
            panic!("the listing is not in the store as a table");
        };
        assert_eq!(
            table.row_count(),
            20,
            "rows were dropped rather than stored"
        );
        // AND THE SHORT CASE STILL PASSES THROUGH WHOLE, so the cap has not
        // simply turned every listing into an indirection.
        write_zip(
            &root,
            "short.zip",
            &[
                RawEntry::stored("a.txt", b"x"),
                RawEntry::stored("b.txt", b"y"),
            ],
        );
        let small = tool
            .work(&json!({"path": "short.zip", "action": "list"}), &ctx)
            .expect("listing");
        assert!(small.to_model.contains("a.txt"), "{}", small.to_model);
        assert!(
            !small.to_model.contains("source_ref="),
            "{}",
            small.to_model
        );
    }

    /// A NORMALISATION COLLISION IS REFUSED BY NAME, NOT AS AN ESCAPE.
    ///
    /// `inspect`'s duplicate gate folds case, which is enough for `A.txt` vs
    /// `a.txt`. It does not fold Unicode normalisation, and this filesystem
    /// does: MEASURED HERE (APFS, /private/tmp) writing `é.txt` in NFD and then
    /// in NFC leaves ONE file holding the second write. So the pair passes every
    /// gate, `list` reports two entries, and `extract` meets the first one again
    /// on disk. It used to die there as `SandboxViolation` — an error naming an
    /// escape that never happened, on a path inside the destination the tool had
    /// just created. It must name the collision instead.
    ///
    /// macOS ONLY, and not for convenience: on a normalisation-SENSITIVE
    /// filesystem (ext4) the two names really are two files and unpacking both
    /// is the correct outcome, so there is nothing to assert there.
    #[cfg(target_os = "macos")]
    #[test]
    fn two_spellings_of_one_name_are_refused_as_a_collision_not_as_an_escape() {
        let _guard = isolated();
        let root = temp_root("nfc");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::with_store(Arc::clone(&store));
        // "e" + COMBINING ACUTE, then the single precomposed code point. Two
        // distinct strings, one file on APFS.
        write_zip(
            &root,
            "norm.zip",
            &[
                RawEntry::stored("e\u{301}.txt", b"first"),
                RawEntry::stored("\u{e9}.txt", b"second"),
            ],
        );

        // The listing passes — that is the honest half of the story and the
        // reason this case is documented rather than claimed away.
        let listed = tool
            .work(&json!({"path": "norm.zip", "action": "list"}), &ctx)
            .expect("the listing decodes nothing and refuses nothing");
        assert!(listed.to_model.contains("2 entries"), "{}", listed.to_model);

        match refusal(
            &tool,
            &ctx,
            json!({"path": "norm.zip", "action": "extract"}),
        ) {
            ToolError::Other(why) => assert!(
                why.contains("same file as an entry already unpacked"),
                "refused for the wrong reason: {why}"
            ),
            other => panic!("a name collision was reported as {other:?}"),
        }
    }

    // --- the contract ---

    #[test]
    fn a_refusal_does_not_taint_the_session_and_a_success_does() {
        let _guard = isolated();
        let root = temp_root("taint");
        let store = Arc::new(SharedStore::new());
        let mut ctx = context(&root, &store);
        let tool = ArchiveTool::with_store(Arc::clone(&store));
        write_zip(&root, "evil.zip", &[RawEntry::stored("../x.txt", b"x")]);
        write_zip(&root, "good.zip", &[RawEntry::stored("a.txt", b"alpha")]);

        let refused = execute(tool.run(extract_args("evil.zip"), &mut ctx));
        assert!(matches!(refused.state, ToolState::Failed(_)));
        assert_eq!(refused.to_model, tacet_kernel::ERROR_MODEL_TEXT);
        assert!(
            !ctx.session_tainted(),
            "a refusal read nothing and must not tighten the approval gate"
        );

        let ok = execute(tool.run(json!({"path": "good.zip", "action": "list"}), &mut ctx));
        assert_eq!(ok.state, ToolState::Read);
        assert!(ctx.session_tainted(), "reading the user's archive taints");
        assert!(tool.taints_session());
    }

    #[test]
    fn the_schema_is_the_boundary() {
        let js = ArchiveTool::new().schema().json_schema();
        assert_eq!(js["required"], json!(["path", "action"]));
        assert_eq!(
            js["additionalProperties"],
            json!(false),
            "an invented key must not be an escape hatch"
        );
        // The closed set is what makes "this tool only lists and extracts" a
        // property of the SHAPE — the grammar cannot emit a third value.
        assert_eq!(
            js["properties"]["action"]["enum"],
            json!(["list", "extract"])
        );
        // There is no destination field at all: see the note on `schema`.
        assert!(js["properties"]["into"].is_null());
        // And the tool's own gate refuses what the grammar would never produce,
        // because the grammar can be disabled and eval/the CLI call directly.
        let _guard = isolated();
        let root = temp_root("schema");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        write_zip(&root, "a.zip", &[RawEntry::stored("a.txt", b"x")]);
        let tool = ArchiveTool::new();
        assert!(
            tool.work(&json!({"path": "a.zip", "action": "delete"}), &ctx)
                .is_err()
        );
        assert!(tool.work(&json!({"path": "a.zip"}), &ctx).is_err());
        assert!(tool.work(&json!({"action": "list"}), &ctx).is_err());
    }

    #[test]
    fn a_directory_traversal_that_stays_inside_is_still_accepted() {
        // The gate must not be so blunt that it refuses legitimate archives:
        // `zip -r` writes `./name` routinely, and a nested path is the normal
        // case. A gate that breaks the ordinary flow is a gate somebody deletes.
        let _guard = isolated();
        let root = temp_root("normal");
        let store = Arc::new(SharedStore::new());
        let ctx = context(&root, &store);
        let tool = ArchiveTool::new();
        write_zip(
            &root,
            "ok.zip",
            &[
                RawEntry::stored("./a.txt", b"alpha"),
                RawEntry::stored("deep/nested/b.txt", b"beta"),
            ],
        );
        tool.work(&extract_args("ok.zip"), &ctx).expect("accepted");
        assert_eq!(fs::read(root.join("ok/a.txt")).unwrap(), b"alpha");
        assert_eq!(
            fs::read(root.join("ok/deep/nested/b.txt")).unwrap(),
            b"beta"
        );
    }
}
