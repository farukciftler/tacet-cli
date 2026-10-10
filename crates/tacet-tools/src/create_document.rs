//! Document production — the `create_document` tool, the `DocumentEngine`
//! contract and two engines (Excel/xlsx, plain text/markdown).
//!
//! A SINGLE WRITE GATE: the free function `write_document`. The trait only
//! defines `render`; preparing the folder, producing a unique name, the write
//! itself and the file permission (0600) live in one place, OUTSIDE the trait.
//! On the Swift side the same shape made it impossible to forget
//! FileProtection. This used to be a trait method named `write` with a default
//! body that relied on the comment "engines do not override this" — since
//! nothing in Rust stopped them, a new engine could silently drop the
//! permission stamp. There is nothing to override in a free function. An
//! engine never sees a path at all: it returns BYTES, so neither the naming
//! rule nor the way the file is opened can differ between two engines.
//!
//! BYPASS CHANNEL: if `source_ref` is given the data DOES NOT PASS through
//! the model; the body is read from the store. And the ref is BINDING: an
//! unresolvable ref does not quietly fall back to `content`. That was the most
//! dangerous error class in Swift — when the ref did not resolve, `content` was
//! empty too, so an EMPTY file was written and reported as "created"; the user
//! carried around a file they thought was full. Here the file is NEVER written.

use std::fs;
use std::path::{Path, PathBuf};

use tacet_kernel::{
    ArgSchema, Field, SourceRef, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult,
    ToolState, TraceUpdate, boxed,
};

use crate::data_store::Table;

/// THE PATH THE MODEL IS TOLD ABOUT — relative to the working directory, and
/// spelled with forward slashes on every platform.
///
/// WHY IT IS A FUNCTION AND NOT A `strip_prefix` AT THE CALL SITE. It was a
/// `strip_prefix` at the call site, and on Windows it produced a path no user
/// would recognise and the model could not round-trip:
///
/// ```text
/// file_created (markdown): \\?\C:\Users\...\Temp\tacet-...\notes\note.md
/// ```
///
/// AN INDENTED EXAMPLE WOULD HAVE BEEN A DOCTEST. Four spaces after `///` is a
/// Rust code block to rustdoc, and this line is a Windows path — it compiled
/// nowhere and turned the whole workspace red on all three platforms, having
/// been written to fix a failure on one. The fence is not decoration.
///
/// Two separate things went wrong there and only one of them was visible.
///
/// 1. THE STRIP SILENTLY FAILED. `ctx.working_dir` and the written path are
///    canonicalized at different moments, and on Windows canonicalization adds
///    the `\\?\` verbatim prefix — so one side carried it, the other did not,
///    `strip_prefix` returned `Err`, and the fallback handed over the FULL
///    absolute path. Nothing errored; the model simply got a different kind of
///    answer on one platform. Both sides are canonicalized here before the
///    comparison, which is the only form in which the question "is this file
///    inside the working directory" has one answer.
///
/// 2. THE SEPARATOR. Even a successful strip gives `notes\note.md` on Windows,
///    and this string goes into the model's context and comes back inside a
///    JSON argument, where a backslash is an ESCAPE character. The model then
///    has to write `"notes\\note.md"` to mean what it read — one more way for a
///    correct intention to produce an unreadable path. Forward slashes are
///    accepted by `resolve_existing_file` on every platform this ships to, so
///    the model is given the spelling that cannot be misread.
///
/// THE ABSOLUTE PATH REMAINS THE ANSWER OUTSIDE THE WORKING DIRECTORY, and
/// that is deliberate rather than a fallback: a file written into a registered
/// root that is not the working directory has no relative name the model could
/// use, and handing back the bare file name is what once made
/// `read_document` fail and the model invent the table.
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

/// The folder documents are written to: the sandbox ROOT itself.
///
/// IT USED TO BE A SUBFOLDER NAMED AFTER THE BRAND, AND THAT WAS WRONG: when
/// the user ran it in the home directory the files went under `~/<brand>/` and
/// were not visible when you ran `ls`
/// ("it should create these in the folder I am in"). The file is now written
/// straight into the working directory; the relative path returned to the model
/// (`meal_plan.xlsx`) is then exactly the path the user can really find.
///
/// THE SANDBOX BOUNDARY IS NOT LOST — two layers of defence remain:
///  1. `resolve_path(".")` validates the root inside the working directory (an
///     absolute path / `..` attempt is rejected there — in this call the target
///     is already the root, but the gate is still passed through),
///  2. `target_path` strips components such as `/`, `\` and `..` FROM THE FILE
///     NAME, so a name can never turn into a path and leave the root (see the tests).
pub fn output_folder(ctx: &ToolContext) -> ToolResult<PathBuf> {
    ctx.resolve_path(".")
}

/// How many lines at most the summary returned to the model carries.
const SUMMARY_LINES: usize = 12;

// ---------------------------------------------------------------------------
// Format
// ---------------------------------------------------------------------------

/// The file formats that can be produced.
///
/// A closed set, NOT free text. In Swift the format was resolved fuzzily with
/// `.contains` and every value that did not match silently fell back to plain
/// text: the user asked for "excel" and got a .txt. Because it stands in the
/// schema as a `Choice`, the grammar makes an invalid value UNPRODUCIBLE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentFormat {
    Excel,
    Markdown,
    Text,
}

impl DocumentFormat {
    /// Exactly the same order as the `Choice` values in the schema — so the
    /// grammar and the prompt come out bit-for-bit identical on every run.
    pub const ALL: [DocumentFormat; 3] = [
        DocumentFormat::Excel,
        DocumentFormat::Markdown,
        DocumentFormat::Text,
    ];

    pub fn key(&self) -> &'static str {
        match self {
            DocumentFormat::Excel => "excel",
            DocumentFormat::Markdown => "markdown",
            DocumentFormat::Text => "text",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            DocumentFormat::Excel => "xlsx",
            DocumentFormat::Markdown => "md",
            DocumentFormat::Text => "txt",
        }
    }

    /// The chip label shown to the user.
    pub fn tag(&self) -> &'static str {
        match self {
            DocumentFormat::Excel => "Excel",
            DocumentFormat::Markdown => "Markdown",
            DocumentFormat::Text => "Text",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            DocumentFormat::Excel => "table",
            DocumentFormat::Markdown => "document",
            DocumentFormat::Text => "document",
        }
    }

    /// Does this format expect a structured table? Only xlsx.
    pub fn table_shape(&self) -> bool {
        matches!(self, DocumentFormat::Excel)
    }

    pub fn resolve(raw: &str) -> Option<DocumentFormat> {
        DocumentFormat::ALL
            .into_iter()
            .find(|b| b.key().eq_ignore_ascii_case(raw.trim()))
    }
}

// ---------------------------------------------------------------------------
// The DocumentEngine contract
// ---------------------------------------------------------------------------

/// An engine responsible for a single format.
pub trait DocumentEngine: Send + Sync {
    fn format(&self) -> DocumentFormat;

    /// The file's bytes. The engine only produces the content; WHERE it goes
    /// and HOW the file is opened belong to `write_document`.
    ///
    /// IT USED TO BE `write_raw(target, ..)`, AND EACH ENGINE CALLED
    /// `fs::write(target)` (audit L15). That was a check-then-write: the gate
    /// proved `target` was not a link, then the engine opened it BY NAME,
    /// following whatever stood there by then. A link planted in between — a
    /// concurrent `run_code` is allowed to write in the sandbox, and a link is
    /// a write — was written through. Bytes cannot be written through anything.
    fn render(
        &self,
        title: Option<&str>,
        body: Option<&str>,
        table: Option<&Table>,
    ) -> ToolResult<Vec<u8>>;
}

/// THE SINGLE WRITE GATE.
///
/// WHY A FREE FUNCTION AND NOT A TRAIT METHOD: this used to be `DocumentEngine`'s
/// `write` method with a default body, and its comment said "engines do not
/// override this" — but nothing in Rust stopped them. If a new engine wrote its
/// own `fn write(...)`, `prepare_folder` and `finish_up` (the 0600 permission
/// stamp) would be SILENTLY skipped; neither the compiler nor a test would warn.
/// As a free function there is nothing left to override: the extension point is
/// `render` alone, which returns bytes and never sees a path, and the shared
/// steps — folder, exclusive create, permission stamp — run on every path.
pub fn write_document(
    engine: &dyn DocumentEngine,
    file_name: &str,
    title: Option<&str>,
    body: Option<&str>,
    table: Option<&Table>,
    folder: &Path,
) -> ToolResult<PathBuf> {
    // Rendered BEFORE a name is taken: a refused table (no rows, too many
    // columns) must not leave a file behind.
    let bytes = engine.render(title, body, table)?;
    prepare_folder(folder)?;
    write_new_file(|| target_path(file_name, engine.format(), folder), &bytes)
}

/// How many names are tried before giving up. Each miss is a name that came
/// into existence between the naming loop and the open — a race, not a
/// collision — so a handful means something is planting files on purpose.
const NAME_ATTEMPTS: usize = 16;

/// Writes `bytes` to the first name `next_name` offers that can be CREATED.
///
/// THE NAMING LOOP IS ADVICE; THE OPEN IS THE GATE. `target_path` rotates past
/// anything that exists — links included — but between its check and the write
/// a link can appear (audit L15). The open is therefore `create_new`: it
/// refuses a path that exists by then, dangling link or not, and the loop asks
/// for the next name. There is no step left where the path is checked and then
/// trusted.
fn write_new_file(mut next_name: impl FnMut() -> PathBuf, bytes: &[u8]) -> ToolResult<PathBuf> {
    for _ in 0..NAME_ATTEMPTS {
        let target = next_name();
        match create_exclusive(&target, bytes) {
            Ok(()) => return Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(ToolError::Other(format!(
        "no free file name after {NAME_ATTEMPTS} attempts: names are being taken as they are chosen"
    )))
}

/// Creates `target` — never opens anything that is already there — writes
/// `bytes`, and stamps 0600 THROUGH THE HANDLE.
///
/// `create_new` is `O_CREAT|O_EXCL`, which POSIX requires to fail with
/// `EEXIST` on an existing symlink, dangling or not — that is the belt that
/// holds on every platform. `O_NOFOLLOW` is a second belt where its value is
/// known (see `o_nofollow_for`), the same pair `run_code::create_script` uses
/// (see the measurement on Windows recorded there: `create_new` alone holds). The
/// permission used to be set with `fs::set_permissions(path)`, which resolves
/// the NAME again and follows a link that replaced the file — through the
/// handle there is no name left to swap. `mode(0o600)` closes the window in
/// which the file existed with the umask's permissions.
fn create_exclusive(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let nofollow = o_nofollow_for(std::env::consts::OS, std::env::consts::ARCH);
        options.custom_flags(nofollow).mode(0o600);
    }
    let mut file = options.open(target)?;
    let written = file.write_all(bytes).and_then(|()| finish_up(&file));
    if written.is_err() {
        // The name was created by this call a moment ago and holds a partial
        // document; leaving it would be a file reported nowhere. `remove_file`
        // removes a link itself, never its target.
        drop(file);
        let _ = fs::remove_file(target);
    }
    written
}

/// The `O_NOFOLLOW` bit for an (os, arch) pair, `0` where it is not known.
///
/// No libc crate here (zero-dependency identity), so the value is spelled out —
/// and on Linux it is spelled PER ARCHITECTURE, not per OS. The copy this
/// replaced said `0x20000` for every Linux, which is right on x86_64 and wrong
/// on aarch64, a published target: there `O_NOFOLLOW` is `0x8000` and `0x20000`
/// is `O_LARGEFILE`, so the second belt was silently absent (audit L15 review).
/// The values below are the ones the libc crate 0.2.190 declares per target
/// (`linux/gnu/b64/{x86_64,aarch64,riscv64,s390x,loongarch64,powerpc64}`,
/// `linux/gnu/b32/{x86,arm,powerpc}`, `android/b64/*`). An architecture not
/// listed gets `0` — no extra flag — because a guessed bit can be some other
/// flag; `create_new` still refuses an existing path there.
///
/// `pub(crate)` because `run_code::create_script` takes its bit from here too:
/// one table, so an architecture added once is right in both.
#[cfg(any(unix, test))]
pub(crate) fn o_nofollow_for(os: &str, arch: &str) -> i32 {
    match (os, arch) {
        ("macos" | "ios", _) => 0x0100,
        ("linux" | "android", "x86" | "x86_64" | "riscv64" | "s390x" | "loongarch64") => 0x20000,
        ("linux" | "android", "arm" | "aarch64" | "powerpc" | "powerpc64") => 0x8000,
        _ => 0,
    }
}

fn prepare_folder(folder: &Path) -> ToolResult<()> {
    fs::create_dir_all(folder)?;
    Ok(())
}

/// The shared post-write step: makes the file readable/writable by its owner only.
///
/// The counterpart of Swift's `applyProtection`. A produced document carries the
/// user's personal data; instead of trusting the default umask we stamp the
/// permission EXPLICITLY.
///
/// PLATFORM RECORD — THE 0600 STAMP EXISTS ONLY ON UNIX.
/// On Windows there is NO counterpart to this line and NONE HAS BEEN FAKED.
/// First, `set_permissions` on Windows only flips the "read only" flag; that is
/// accident protection, not access control. Stamping it would create the
/// illusion of "we stamped it" without providing any privacy — exactly the
/// silent lie this file avoids. Second, the real counterpart is narrowing the
/// file ACL (`SetNamedSecurityInfo`), and that wants a `winapi`/`windows-sys`
/// dependency, which contradicts the zero-dependency identity. Rather than add
/// a dependency, the ABSENCE is written down.
/// On Windows privacy therefore rests on the directory ACL: the document is
/// written into the user's profile tree, and that is closed to other users by
/// default. NOT MEASURED ON THIS MACHINE (no Windows); the basis is the
/// operating system default, not a stamp we applied.
fn finish_up(file: &fs::File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Through the handle (`fchmod`), not the name: see `create_exclusive`.
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = file;
    }
    Ok(())
}

/// Produces a unique, safe target path (appends `-2`, `-3` ... on a collision).
///
/// Name hygiene is strict: path separators and components like `..` never enter
/// the NAME. The sandbox gate is already there, but a file name must never be
/// defence.
fn target_path(file_name: &str, format: DocumentFormat, folder: &Path) -> PathBuf {
    let clean: String = file_name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let clean = clean.trim().trim_matches('.').trim().to_string();
    // THE STEM OF AN UNNAMED DOCUMENT IS VISIBLE ON THE USER'S DISK — it is the brand name,
    // not the crate name. This spot was missed during a brand change and files
    // carrying the old name were being created in users' folders.
    let mut base = if clean.is_empty() {
        "tacet-document".to_string()
    } else {
        clean
    };

    // Strip the extension the model BAKED INTO THE NAME — otherwise it doubles.
    // The schema asks for `file_name` "without extension", but the small model
    // still adds ".xlsx" most of the time; since we add the extension ourselves
    // the result was "name.xlsx.xlsx". Only THIS format's extension is stripped:
    // "notes.md" -> if excel is requested the stem "notes.md" is kept (the user
    // may have meant it), but "report.xlsx" -> excel becomes "report". Case insensitive.
    let extension_dot = format!(".{}", format.extension());
    if base.len() > extension_dot.len()
        && base[base.len() - extension_dot.len()..].eq_ignore_ascii_case(&extension_dot)
    {
        base.truncate(base.len() - extension_dot.len());
    }

    let mut candidate = folder.join(format!("{base}.{}", format.extension()));
    let mut i = 2;
    // NOT `exists()`: it FOLLOWS symlinks, so a DANGLING link (`report.md ->
    // ~/Library/Preferences/whatever.txt`, target not yet created) reports FALSE,
    // the loop never turns, and `fs::write` follows the link and creates the file
    // OUTSIDE the sandbox — while the model and the user are told "report.md was
    // written to the working folder". `symlink_metadata` sees the LINK ITSELF,
    // dangling or not, so the "never overwrite anything" contract now covers
    // links too.
    while candidate.symlink_metadata().is_ok() {
        candidate = folder.join(format!("{base}-{i}.{}", format.extension()));
        i += 1;
    }
    candidate
}

// ---------------------------------------------------------------------------
// MetinMotor
// ---------------------------------------------------------------------------

/// Writes plain text / markdown. One engine serves two formats: the only
/// difference is the extension, content production is identical.
#[derive(Debug, Clone, Copy)]
pub struct TextEngine {
    format: DocumentFormat,
}

impl TextEngine {
    pub fn new(format: DocumentFormat) -> Self {
        Self { format }
    }
}

/// Does the body already open with this title as its own first line?
///
/// The leading `#`s are stripped, so it catches both the markdown heading the
/// model usually writes and a bare repeat of the words. The comparison is exact
/// apart from ASCII case: folding beyond that would need Unicode casing rules,
/// and the case worth catching is the one where the two strings are literally
/// the same string.
fn opens_with(body: &str, title: &str) -> bool {
    let Some(first) = body.lines().find(|l| !l.trim().is_empty()) else {
        return false;
    };
    let head = first.trim().trim_start_matches('#').trim();
    !head.is_empty() && head.eq_ignore_ascii_case(title)
}

impl DocumentEngine for TextEngine {
    fn format(&self) -> DocumentFormat {
        self.format
    }

    fn render(
        &self,
        title: Option<&str>,
        body: Option<&str>,
        table: Option<&Table>,
    ) -> ToolResult<Vec<u8>> {
        // THE BODY IS BUILT FIRST, because whether the title gets written at all
        // depends on it.
        //
        // With no body the table is converted to markdown: changing the format
        // beats losing the data. If neither exists WE DO NOT WRITE AN EMPTY FILE —
        // an empty file reported as "created" is the costliest outcome for the user.
        let text = match (body.map(str::trim).filter(|g| !g.is_empty()), table) {
            (Some(g), _) => g.to_string(),
            (None, Some(t)) if !t.headers.is_empty() => t.markdown_truncated(usize::MAX),
            _ => {
                return Err(ToolError::InvalidArgument(
                    "no content for the document: give 'content' or a resolvable 'source_ref'"
                        .into(),
                ));
            }
        };

        // THE TITLE IS NOT WRITTEN TWICE.
        //
        // The model fills `title` AND opens `content` with the same heading —
        // which is the natural thing for it to do, since both fields describe the
        // document. The file then began:
        //
        //     # Rastgele İçerik
        //
        //     # Rastgele İçerik
        //
        // Suppressing the field rather than rewriting the body is deliberate: the
        // body is the user's text and this tool does not edit it. Dropping our own
        // added line is the change that removes the duplicate without touching
        // anything the model wrote.
        let mut content = String::new();
        if let Some(b) = title
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .filter(|b| !opens_with(&text, b))
        {
            // Mark the title in markdown; it does no harm in plain text either,
            // but it is noise, so we only print '#' in md.
            if self.format == DocumentFormat::Markdown {
                content.push_str("# ");
            }
            content.push_str(b);
            content.push_str("\n\n");
        }
        content.push_str(&text);
        if !content.ends_with('\n') {
            content.push('\n');
        }
        Ok(content.into_bytes())
    }
}

// ---------------------------------------------------------------------------
// ExcelMotor
// ---------------------------------------------------------------------------

/// Writes .xlsx (OOXML SpreadsheetML). Packaging goes through tacet-zip (STORE).
///
/// The cells are `inlineStr`; a sharedStrings table IS NOT USED. Rationale:
/// a shared string table means keeping a second index, and consistency between
/// the index and the cell is one more error surface for a hand-written producer.
/// For the small tables produced on device the size gain does not cover that risk.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExcelEngine;

impl ExcelEngine {
    pub fn new() -> Self {
        Self
    }
}

impl DocumentEngine for ExcelEngine {
    fn format(&self) -> DocumentFormat {
        DocumentFormat::Excel
    }

    fn render(
        &self,
        title: Option<&str>,
        body: Option<&str>,
        table: Option<&Table>,
    ) -> ToolResult<Vec<u8>> {
        let (headers, lines) = to_table(title, body, table)?;
        check_sheet_bounds(headers.len(), &lines)?;
        let sheet = sheet_xml(&headers, &lines);

        let entries = vec![
            tacet_zip::ZipEntry::new("[Content_Types].xml", CONTENT_TYPES_XML.as_bytes()),
            tacet_zip::ZipEntry::new("_rels/.rels", RELS_XML.as_bytes()),
            tacet_zip::ZipEntry::new("xl/workbook.xml", WORKBOOK_XML.as_bytes()),
            tacet_zip::ZipEntry::new("xl/_rels/workbook.xml.rels", WORKBOOK_RELS_XML.as_bytes()),
            tacet_zip::ZipEntry::new("xl/styles.xml", STYLES_XML.as_bytes()),
            tacet_zip::ZipEntry::new("xl/worksheets/sheet1.xml", sheet.into_bytes()),
        ];

        tacet_zip::pack(&entries)
            .map_err(|e| ToolError::Other(format!("the xlsx could not be packed: {e}")))
    }
}

/// The only input of an xlsx is a table. If there is no structured table, one is
/// attempted TO BE RECOVERED from the body; if it cannot be, an EXPLICIT ERROR is returned.
///
/// The silent fallback branch here was deliberately removed in Swift: the body
/// was being poured line by line into a SINGLE COLUMN. When the model wrote a
/// markdown table ("| Name | Age |") the result was a junk xlsx with every row
/// reported SUCCESS as "created". The rule:
/// - if the body is a parseable table -> it is converted to a real table,
/// - the body has pipes but no table comes out -> error (the model fixes it once),
/// - the body is a plain list with no pipes -> a single column is LEGITIMATE,
/// - neither a table nor a body -> error.
fn to_table(
    title: Option<&str>,
    body: Option<&str>,
    table: Option<&Table>,
) -> ToolResult<(Vec<String>, Vec<Vec<String>>)> {
    if let Some(t) = table.filter(|t| !t.headers.is_empty()) {
        if !t.rows.is_empty() {
            return Ok((t.headers.clone(), t.rows.clone()));
        }
        // There is a header but no rows: try to recover rows from the body.
        if let Some(a) = body.and_then(from_markdown).filter(|a| !a.rows.is_empty()) {
            return Ok((a.headers, a.rows));
        }
        return Ok((t.headers.clone(), Vec::new()));
    }

    let text = body.unwrap_or("").trim();
    if text.is_empty() {
        return Err(ToolError::InvalidArgument(
            "no content for the spreadsheet: give a table with column headers and rows".into(),
        ));
    }

    // 1) Is the body a markdown table? (the separator row is optional)
    if let Some(a) = from_markdown(text).filter(|a| !a.rows.is_empty()) {
        return Ok((a.headers, a.rows));
    }

    // 2) Pipes present but no table came out -> do not silently pour it into a
    //    single column, MAKE IT FIX THIS.
    if text.contains('|') {
        return Err(ToolError::InvalidArgument(
            "the table could not be parsed: give a table with a header row and data rows, one row \
             per line, cells separated by |"
                .into(),
        ));
    }

    // 3) A plain list with no pipes: a single column is a legitimate result.
    let slices: Vec<Vec<String>> = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| vec![s.to_string()])
        .collect();
    if slices.is_empty() {
        return Err(ToolError::InvalidArgument(
            "no content for the spreadsheet: give a table with column headers and rows".into(),
        ));
    }
    let emit = title
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .unwrap_or("Text");
    Ok((vec![emit.to_string()], slices))
}

/// Excel's grid: 1,048,576 rows and 16,384 columns (`XFD`).
const MAX_SHEET_ROWS: usize = 1_048_576;
const MAX_SHEET_COLUMNS: usize = 16_384;

/// Refuses a table the grid cannot hold, BEFORE any XML is built.
///
/// Nothing checked this (audit L13): a 16,385th column was written as `XFE1`
/// and a row past the last as `r="1048577"`, and Excel answers either with
/// "We found a problem with some content" and drops data while repairing. The
/// Total row is counted, because it is written whenever a column is numeric.
fn check_sheet_bounds(columns: usize, lines: &[Vec<String>]) -> ToolResult<()> {
    if columns > MAX_SHEET_COLUMNS {
        return Err(ToolError::InvalidArgument(format!(
            "the table has {columns} columns; a spreadsheet holds at most {MAX_SHEET_COLUMNS}"
        )));
    }
    // Header + data + the Total row when there may be one. The exact test for
    // a numeric column is in `sheet_xml`; counting the Total row whenever there
    // is data refuses at most one row more than strictly necessary.
    let total_row = usize::from(!lines.is_empty());
    let rows = 1 + lines.len() + total_row;
    if rows > MAX_SHEET_ROWS {
        return Err(ToolError::InvalidArgument(format!(
            "the table needs {rows} rows (header, data and Total); a spreadsheet holds at most \
             {MAX_SHEET_ROWS}"
        )));
    }
    Ok(())
}

/// The sheet XML. Writes a REAL `=SUM()` formula under fully numeric columns.
///
/// Writing a FORMULA rather than a computed constant is a deliberate decision:
/// when the user opens the file and changes a cell, the total must update
/// itself. A constant would carry a number that could quietly go wrong the
/// moment one cell changed.
///
/// IT IS THE ONLY FORMULA WRITTEN. There is no date arithmetic here — the
/// front page once showed a `=DATEDIF()` this code never produced (audit M21);
/// `the_countdown_demo_writes_a_sum_and_nothing_else` holds the claim to this.
fn sheet_xml(headers: &[String], lines: &[Vec<String>]) -> String {
    let column_count = headers.len();

    // Numeric column detection: in the data rows EVERY filled cell of that column is a number.
    let mut numeric_column = vec![false; column_count];
    for (k, flag) in numeric_column.iter_mut().enumerate() {
        let mut at_least_one = false;
        let mut all_numeric = true;
        for s in lines {
            let Some(h) = s.get(k).map(|h| h.trim()) else {
                continue;
            };
            if h.is_empty() {
                continue;
            }
            at_least_one = true;
            if !is_number(h) {
                all_numeric = false;
                break;
            }
        }
        *flag = at_least_one && all_numeric;
    }
    let has_numeric = numeric_column.iter().any(|b| *b);

    let mut body = String::new();

    // Row 1: the headers (all inlineStr).
    body.push_str("<row r=\"1\">");
    for (k, b) in headers.iter().enumerate() {
        body.push_str(&inline_cell(&format!("{}1", column_letter(k)), b));
    }
    body.push_str("</row>");

    // The data rows.
    for (idx, s) in lines.iter().enumerate() {
        let r = idx + 2;
        body.push_str(&format!("<row r=\"{r}\">"));
        for k in 0..column_count {
            let cell_ref = format!("{}{r}", column_letter(k));
            let empty = String::new();
            let value = s.get(k).unwrap_or(&empty);
            let truncate = value.trim();
            if !truncate.is_empty() && is_number(truncate) {
                body.push_str(&format!("<c r=\"{cell_ref}\"><v>{truncate}</v></c>"));
            } else {
                body.push_str(&inline_cell(&cell_ref, value));
            }
        }
        body.push_str("</row>");
    }

    // The summary (Total) row: only if there is at least one numeric column.
    //
    // THE LABEL GOES IN THE FIRST COLUMN THAT IS NOT SUMMED. It used to go in
    // column A unconditionally, and column A was then never summed — so a
    // one-column table of amounts (`| Amount |`, 100, 200) got a Total row
    // reading "Total" and no total at all (audit M21-b). When EVERY column is
    // numeric there is no free cell, and the row carries sums only: a label
    // outside the table would widen it for every reader, ours included —
    // `read_document` spans the header over the widest row, and
    // `edit_document` would then warn about a column the user never had.
    if has_numeric && !lines.is_empty() {
        let first_data = 2;
        let last_data = lines.len() + 1;
        let total_rows = last_data + 1;
        let label_column = numeric_column.iter().position(|numeric| !numeric);
        body.push_str(&format!("<row r=\"{total_rows}\">"));
        for (k, numeric) in numeric_column.iter().enumerate() {
            let cell_ref = format!("{}{total_rows}", column_letter(k));
            if Some(k) == label_column {
                body.push_str(&inline_cell(&cell_ref, "Total"));
            } else if *numeric {
                let letter = column_letter(k);
                body.push_str(&format!(
                    "<c r=\"{cell_ref}\"><f>SUM({letter}{first_data}:{letter}{last_data})</f></c>"
                ));
            } else {
                body.push_str(&inline_cell(&cell_ref, ""));
            }
        }
        body.push_str("</row>");
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <sheetData>{body}</sheetData></worksheet>"
    )
}

fn inline_cell(cell_ref: &str, text: &str) -> String {
    format!(
        "<c r=\"{cell_ref}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
        xml_escape(text)
    )
}

/// The number format Excel accepts inside `<v>`.
///
/// `f64::parse` is not enough: Rust also parses values like "inf", "NaN" and
/// "1e5", but written raw into the sheet those either become an invalid XML
/// value or look completely different in Excel. Writing them as text is righter.
fn is_number(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }
    let body = raw.strip_prefix('-').unwrap_or(raw);
    if body.is_empty() {
        return false;
    }
    let mut dot = false;
    let mut rakam = false;
    for c in body.chars() {
        match c {
            '0'..='9' => rakam = true,
            '.' if !dot => dot = true,
            _ => return false,
        }
    }
    rakam
}

/// Converts a column index (0 based) to a letter: 0->A, 25->Z, 26->AA.
fn column_letter(index: usize) -> String {
    let mut n = index as i64;
    let mut s = String::new();
    loop {
        let r = (n % 26) as u8;
        s.insert(0, (b'A' + r) as char);
        n = n / 26 - 1;
        if n < 0 {
            break;
        }
    }
    s
}

fn xml_escape(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\'' => s.push_str("&apos;"),
            // XML 1.0 accepts these with NO escape at all — not even as a
            // character reference; dropping is the only option, otherwise the
            // produced file cannot be opened.
            c if !is_xml_char(c) => {}
            c => s.push(c),
        }
    }
    s
}

/// XML 1.0's `Char` production: `#x9 | #xA | #xD | [#x20-#xD7FF] |
/// [#xE000-#xFFFD] | [#x10000-#x10FFFF]`.
///
/// Only the C0 controls were dropped before, and U+FFFE / U+FFFF went into
/// the sheet raw (audit L13, measured: the sheet XML held a raw U+FFFE, and
/// Excel opens such a file only through "repair"). A Rust `char` is never a
/// surrogate, so the D800–DFFF gap needs no arm of its own.
fn is_xml_char(c: char) -> bool {
    matches!(
        c as u32,
        0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
    )
}

// ---------------------------------------------------------------------------
// The fixed OOXML parts
// ---------------------------------------------------------------------------
//
// These five parts are identical in every xlsx. There is no template engine,
// needed: the only part that changes is sheet1.xml. Edit without breaking the
// link between the parts (Content_Types -> part path -> rels target); if one
// does not hold, Excel treats the file as "unrepairable".

const CONTENT_TYPES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>"#;

const RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;

const WORKBOOK_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
<fills count="1"><fill><patternFill patternType="none"/></fill></fills>
<borders count="1"><border/></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
</styleSheet>"#;

// ---------------------------------------------------------------------------
// Markdown table parsing
// ---------------------------------------------------------------------------

/// Converts a markdown table to a `Table`; `None` if no table comes out.
///
/// The separator row (`| --- |`) is OPTIONAL: small models skip it often, and
/// throwing the data away because it is missing gains the user nothing. On the
/// other hand, IF the separator row is there it does not count as data —
/// otherwise the first row of the table would be a bunch of dashes.
pub fn from_markdown(raw: &str) -> Option<Table> {
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| s.contains('|'))
        .collect();
    let (emit, remaining) = lines.split_first()?;

    let headers = split_into_cells(emit);
    if headers.is_empty() {
        return None;
    }

    let data: Vec<Vec<String>> = remaining
        .iter()
        .filter(|s| !is_separator_row(s))
        .map(|s| split_into_cells(s))
        .filter(|h| !h.is_empty())
        .collect();

    Some(Table::new(headers, data))
}

/// Is this an alignment row like `| --- | :---: |`?
fn is_separator_row(line: &str) -> bool {
    let cells = split_into_cells(line);
    !cells.is_empty()
        && cells.iter().all(|h| {
            let h = h.trim();
            !h.is_empty() && h.chars().all(|c| c == '-' || c == ':')
        })
}

/// Splits a markdown row into cells. An escaped pipe (`\|`) DOES NOT COUNT as a
/// separator — `Table::markdown_truncated` produces exactly that escape, so
/// that cell boundaries survive when a stored table summary is read back.
fn split_into_cells(line: &str) -> Vec<String> {
    let body = line.trim().trim_start_matches('|').trim_end_matches('|');
    let mut cells = Vec::new();
    let mut active = String::new();
    let mut escape = false;
    for c in body.chars() {
        if escape {
            // Interpret only the pipe as an escape; for anything else a
            // backslash is the text itself (Windows paths are common in cells).
            if c != '|' {
                active.push('\\');
            }
            active.push(c);
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else if c == '|' {
            cells.push(active.trim().to_string());
            active = String::new();
        } else {
            active.push(c);
        }
    }
    if escape {
        active.push('\\');
    }
    cells.push(active.trim().to_string());
    if cells.iter().all(|h| h.is_empty()) {
        return Vec::new();
    }
    cells
}

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

/// `create_document` — produces a file from chat data or from a source in the store.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateDocumentTool;

impl CreateDocumentTool {
    pub fn new() -> Self {
        Self
    }

    fn engine(format: DocumentFormat) -> Box<dyn DocumentEngine> {
        match format {
            DocumentFormat::Excel => Box::new(ExcelEngine::new()),
            b => Box::new(TextEngine::new(b)),
        }
    }
}

impl Tool for CreateDocumentTool {
    fn name(&self) -> &str {
        "create_document"
    }

    fn description(&self) -> &str {
        "Creates an Excel, Markdown or plain text file. Call this IMMEDIATELY when the user asks \
         for a file, table, list or report ('make an excel/table/report', in any language) — do \
         not ask or narrate. Write markdown into 'content'; for a table write a markdown table \
         (| ... |). For bulk device data pass 'source_ref' instead of 'content'."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new(
                "format",
                ArgSchema::choice(DocumentFormat::ALL.map(|b| b.key()))
                    .description("File format: 'excel' (spreadsheet), 'markdown' or 'text' (plain text)."),
            )
            .required(),
            Field::new(
                "file_name",
                ArgSchema::text()
                    .description("File name WITHOUT extension, e.g. 'july-meetings'."),
            )
            .required(),
            Field::new("title", ArgSchema::text().description("Document title (optional).")),
            // THE FIELD THAT DID NOT EXIST, and its absence was silent. The
            // transcript: the user opened `~/Desktop` with the workspace addon,
            // said "create a md file in this folder /Users/…/Desktop", and the
            // file was written to the working directory — while the model
            // reported success. There was no way to express the destination, so
            // the destination in the request was simply dropped.
            Field::new(
                "folder",
                ArgSchema::text().description(
                    "Destination directory (optional). Leave it out and the file goes to the \
                     working directory. Give it only when the user named a directory; write \
                     the path exactly as they wrote it, e.g. '/Users/me/Desktop'.",
                ),
            ),
            Field::new(
                "content",
                ArgSchema::text().description(
                    "Document body as MARKDOWN. For a table write a header row (| Col1 | Col2 |), \
                     then | --- | --- |, then the data rows. Excel files are built from that table.",
                ),
            ),
            Field::new(
                "source_ref",
                ArgSchema::text().description(
                    "Data reference returned by another tool. If given, the full data job pulled \
                     from the store — leave 'content' empty.",
                ),
            ),
        ])
    }

    /// The produced file carries the user's personal data, and the `source_ref`
    /// path reads device data: the session becomes tainted.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let schema = self.schema();
            if let Err(e) = schema.validate(&args) {
                return ToolOutcome::failed(&e);
            }

            let text_field = |name: &str| -> Option<String> {
                args.get(name)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };

            let Some(format) = text_field("format")
                .as_deref()
                .and_then(DocumentFormat::resolve)
            else {
                return ToolOutcome::failed(&ToolError::MissingField("format".into()));
            };
            let Some(file_name) = text_field("file_name") else {
                return ToolOutcome::failed(&ToolError::MissingField("file_name".into()));
            };
            let title = text_field("title");
            let mut body = text_field("content");
            let source_ref = text_field("source_ref");

            let chip = ctx.start_chip(format.icon(), &format!("creating {}", format.tag()));

            // --- The bypass channel: if a ref EXISTS it is BINDING. ---
            let mut table: Option<Table> = None;
            if let Some(r) = source_ref.as_deref() {
                let Some(record) = ctx.from_store(&SourceRef(r.to_string())) else {
                    // Unresolvable ref: the file is NEVER written. Only the fact
                    // goes back to the model — which ref was asked for, what did
                    // not happen. No imperative directive; a retry directive is
                    let error = ToolError::Other(format!("unknown source_ref: {r}"));
                    ctx.update_chip(
                        chip,
                        TraceUpdate::state(ToolState::Failed("Source data not found".into()))
                            .text("Source data not found".to_string())
                            .raw_output(format!("source_ref={r}")),
                    );
                    let mut outcome = ToolOutcome::failed(&error);
                    outcome.chip_text = "Source data not found".to_string();
                    outcome.to_model = format!("unknown_data_ref: \"{r}\"; no file was created");
                    return outcome;
                };
                // The body in the store is valid markdown for a `Table`; if Excel
                // is requested it is converted back to a structured table.
                if format.table_shape()
                    && let Some(t) = from_markdown(&record.body).filter(|t| !t.rows.is_empty())
                {
                    table = Some(t);
                    body = None;
                } else {
                    body = Some(record.body);
                }
            } else if format.table_shape()
                && let Some(t) = body
                    .as_deref()
                    .and_then(from_markdown)
                    .filter(|t| !t.rows.is_empty())
            {
                table = Some(t);
                body = None;
            }

            // The sandbox gate. With no `folder` the output goes to the working
            // directory ITSELF (see `output_folder` — files go to the root, not a
            // subfolder); that is the unchanged path and the common one.
            //
            // WITH a `folder` THE GATE IS THE SAME ONE THE READ PATH USES.
            // `resolve_existing_dir` resolves every component, retests the
            // link-free result against the workspace roots and refuses a
            // destination outside them. So a directory the user opened works, and
            // one they did not is a `SandboxViolation` — not a quiet write
            // somewhere else, which is exactly the failure this field exists to
            // end. A directory that does not exist is `FileNotFound`: the tool
            // DOES NOT CREATE the destination, because "make me a file in
            // /Uzers/me/Desktop" must read as a typo, not as a new tree.
            let folder = match text_field("folder") {
                Some(named) => crate::sandbox_path::resolve_existing_dir(ctx, &named),
                None => output_folder(ctx),
            };
            let folder = match folder {
                Ok(k) => k,
                Err(e) => {
                    ctx.update_chip(chip, TraceUpdate::state(ToolState::Failed(e.short_error())));
                    return ToolOutcome::failed(&e);
                }
            };

            let engine = Self::engine(format);
            let path = match write_document(
                engine.as_ref(),
                &file_name,
                title.as_deref(),
                body.as_deref(),
                table.as_ref(),
                &folder,
            ) {
                Ok(y) => y,
                Err(e) => {
                    ctx.update_chip(chip, TraceUpdate::state(ToolState::Failed(e.short_error())));
                    return ToolOutcome::failed(&e);
                }
            };

            let name = path
                .file_name()
                .map(|a| a.to_string_lossy().into_owned())
                .unwrap_or_else(|| file_name.clone());

            // THE PATH RETURNED TO THE MODEL IS RELATIVE TO THE WORKING DIRECTORY.
            // Since the file now goes straight into the root, this is THE SAME as
            // the bare file name ("meal_plan.xlsx").
            // WHY RELATIVE: when the model is told "show that as a table" in the
            // same session it calls read_document with this name; `resolve_path`
            // looks for it at the root and finds it exactly there. (The file used
            // to be written into a subfolder named after the brand while the bare
            // name still went to the model; the name and the real path diverged,
            // read_document returned "File not found" and the model MADE THE TABLE
            // UP FROM ITS OWN MEMORY. Writing to the root removes that divergence.)
            // OUTSIDE THE WORKING DIRECTORY THE ABSOLUTE PATH IS THE ONE THAT
            // ROUND-TRIPS. This fallback used to hand back the bare name, which
            // was harmless only because `folder` did not exist and the branch was
            // unreachable. Now it is reachable, and a bare name would recreate the
            // very divergence the paragraph above describes: the model would call
            // `read_document("note.md")`, the working directory would not hold it,
            // and the table would get made up. `resolve_existing_file` accepts an
            // absolute path inside a registered root, so the absolute form is what
            // comes back.
            let relative = path_for_model(&path, &ctx.working_dir);
            let chip_text = format!("{} created: {name}", format.tag());

            ctx.update_chip(
                chip,
                TraceUpdate::state(ToolState::Written)
                    .text(chip_text.clone())
                    .raw_output(path.display().to_string())
                    .file_path(path.clone()),
            );
            ctx.taint();

            // Only the fact goes to the model; do not write UI directives such as
            // "preview/share" — the model parrots them and describes a button to
            // the user that does not exist.
            ToolOutcome::written(
                chip_text,
                format!("file_created ({}): {relative}", format.key()),
            )
            .raw_output(path.display().to_string())
            .file_path(path)
        })
    }
}

/// A shortcut that puts a table into the store and returns a short summary to the
/// model — the meeting point between `create_document` and the tools that use
pub fn store_table(ctx: &ToolContext, kind: &str, table: &Table) -> SourceRef {
    ctx.store(
        kind,
        &format!(
            "{} rows x {} columns table",
            table.row_count(),
            table.column_count()
        ),
        table.markdown_truncated(usize::MAX),
    )
}

/// The length of the truncated summary shown to the model — public so that eval
/// constant.
pub const fn summary_line_limit() -> usize {
    SUMMARY_LINES
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_store::{SharedStore, Value};
    use std::future::Future;
    use std::sync::Arc;
    use tacet_kernel::{DataStore as CoreDataStore, SilentReporter};

    /// THE SEPARATOR IS THE PRODUCT DECISION, not the operating system's.
    ///
    /// This asserts the property directly rather than through a tool run,
    /// because the tool run can only observe it on the platform it happens to
    /// be running on — and the platform where it was wrong is the one nobody
    /// develops on. `join("/")` over the components is what makes the answer
    /// the same everywhere; a `to_string_lossy()` on the whole relative path
    /// would read identically here and produce `notes\note.md` on Windows.
    #[test]
    fn the_path_given_to_the_model_uses_forward_slashes() {
        let root = std::env::temp_dir().join(format!(
            "tacet-path-for-model-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let nested = root.join("notes").join("deep");
        fs::create_dir_all(&nested).expect("the fixture directories are created");
        let file = nested.join("note.md");
        fs::write(&file, "hello").expect("the fixture file is written");

        let shown = path_for_model(&file, &root);
        assert_eq!(shown, "notes/deep/note.md", "{shown}");
        assert!(
            !shown.contains('\\'),
            "a backslash reaching the model has to be escaped again inside a \
             JSON argument: {shown}"
        );

        // OUTSIDE the working directory there is no relative name, and the
        // absolute path is the only thing that round-trips.
        let elsewhere = root.join("other");
        fs::create_dir_all(&elsewhere).expect("the sibling directory is created");
        let outside = path_for_model(&file, &elsewhere);
        assert!(outside.contains("note.md"), "{outside}");
        assert_ne!(outside, "notes/deep/note.md");

        fs::remove_dir_all(&root).ok();
    }

    /// There is no tokio in core; this crate must not pick an executor either.
    /// a minimal ~15 line poll loop that only ever returns `Ready` is enough (core made the same call 3 times).
    fn hold<F: Future>(gelecek: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut ctx = Context::from_waker(&waker);
        let mut gelecek = pin!(gelecek);
        loop {
            if let Poll::Ready(output) = gelecek.as_mut().poll(&mut ctx) {
                return output;
            }
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-document-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn context(root: &Path, store: Arc<SharedStore>) -> ToolContext {
        ToolContext::new(store, root, Arc::new(SilentReporter))
    }

    /// AN UNNAMED DOCUMENT IS STEMMED FROM THE BRAND NAME and that is visible on
    /// the user's disk. This spot was missed during a brand change; the test
    /// prevents a regression.
    /// There used to be an "the old name must not leak" claim alongside it; once
    /// the old name was gone from the code base entirely it measured nothing and
    /// was removed. What protects us is ALREADY the equality claim: the stem is
    /// EXACTLY `tacet-document`.
    #[test]
    fn an_unnamed_document_is_stemmed_from_the_brand_name() {
        let root = temp_dir("unnamed");
        let path = target_path("   ", DocumentFormat::Excel, &root);
        let name = path.file_name().unwrap().to_string_lossy();
        assert_eq!(
            name, "tacet-document.xlsx",
            "the unnamed document stem changed"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_column_letter_goes_past_26() {
        assert_eq!(column_letter(0), "A");
        assert_eq!(column_letter(25), "Z");
        assert_eq!(column_letter(26), "AA");
        assert_eq!(column_letter(27), "AB");
        assert_eq!(column_letter(51), "AZ");
        assert_eq!(column_letter(52), "BA");
    }

    /// When truncated markdown output is handed back to the parser, cell boundaries
    /// must survive — an Excel round trip of a stored table depends on this.
    #[test]
    fn a_markdown_round_trip_is_preserved() {
        let table = Table::new(
            ["Name", "Amount"],
            vec![
                vec!["Alice | Bob".to_string(), "120".to_string()],
                vec!["Carol".to_string(), "80".to_string()],
            ],
        );
        let md = table.markdown_truncated(usize::MAX);
        let back = from_markdown(&md).expect("the table must resolve");
        assert_eq!(back.headers, vec!["Name", "Amount"]);
        assert_eq!(
            back.rows.len(),
            2,
            "the separator row must not count as data"
        );
        assert_eq!(back.rows[0][0], "Alice | Bob");
        assert_eq!(back.rows[1][1], "80");
    }

    /// Under a numeric column a REAL formula is written, not a computed constant.
    #[test]
    fn a_real_sum_formula_is_written_for_a_numeric_column() {
        let headers = vec!["Name".to_string(), "Amount".to_string()];
        let lines = vec![
            vec!["Ali".to_string(), "120".to_string()],
            vec!["Carol".to_string(), "80".to_string()],
        ];
        let xml = sheet_xml(&headers, &lines);
        assert!(xml.contains("<f>SUM(B2:B3)</f>"), "no formula: {xml}");
        assert!(
            !xml.contains("<v>200</v>"),
            "a constant total was written: {xml}"
        );
        // A text column is not summed.
        assert!(!xml.contains("SUM(A2"), "the text column was summed: {xml}");
        // A numeric cell must be a raw value, not inlineStr (so Excel sees a number).
        assert!(xml.contains("<c r=\"B2\"><v>120</v></c>"), "{xml}");
    }

    /// The produced .xlsx must be a real zip and carry the OOXML parts.
    #[test]
    fn the_xlsx_contains_a_real_zip_and_real_ooxml_parts() {
        let root = temp_dir("xlsx");
        let table = Table::new(["Name", "Amount"], vec![vec!["Ali".into(), "12".into()]]);
        let path = write_document(
            &ExcelEngine::new(),
            "report",
            None,
            None,
            Some(&table),
            &root,
        )
        .expect("must be written");
        assert_eq!(path.extension().unwrap(), "xlsx");

        let byte = fs::read(&path).unwrap();
        assert_eq!(&byte[..2], b"PK", "no zip signature");
        let entries = tacet_zip::open_map(&byte).expect("the zip must open");
        for part in [
            "[Content_Types].xml",
            "_rels/.rels",
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
            "xl/styles.xml",
            "xl/worksheets/sheet1.xml",
        ] {
            assert!(entries.contains_key(part), "missing part: {part}");
        }
        let sheet = String::from_utf8(entries["xl/worksheets/sheet1.xml"].clone()).unwrap();
        assert!(sheet.contains("Ali"));
        fs::remove_dir_all(&root).ok();
    }

    /// A body with pipes that cannot be parsed is NOT SILENTLY poured into one column.
    #[test]
    fn an_unparseable_pipe_table_body_errors() {
        let error = to_table(None, Some("| Name Amount"), None);
        assert!(matches!(error, Err(ToolError::InvalidArgument(_))));
        // A plain list with no pipes makes a single column legitimate.
        let (headers, lines) =
            to_table(Some("Notes"), Some("one\ntwo\nthree"), None).expect("legitimate");
        assert_eq!(headers, vec!["Notes"]);
        assert_eq!(lines.len(), 3);
    }

    /// When the same name is asked for twice nothing is OVERWRITTEN.
    #[test]
    fn the_same_name_is_not_overwritten() {
        let root = temp_dir("collision");
        let engine = TextEngine::new(DocumentFormat::Markdown);
        let one = write_document(&engine, "notes", None, Some("first"), None, &root).unwrap();
        let two = write_document(&engine, "notes", None, Some("second"), None, &root).unwrap();
        assert_ne!(one, two);
        assert_eq!(fs::read_to_string(&one).unwrap().trim(), "first");
        assert_eq!(fs::read_to_string(&two).unwrap().trim(), "second");
        // A name cannot form a path.
        let third = write_document(&engine, "../escape", None, Some("x"), None, &root).unwrap();
        assert_eq!(third.parent().unwrap(), root);
        fs::remove_dir_all(&root).ok();
    }

    /// A DANGLING SYMLINK IS NOT A FREE NAME.
    ///
    /// The attack: leave `report.md -> ~/Library/Preferences/whatever.txt` in the
    /// working folder with the target NOT YET EXISTING, then get the model to
    /// call `create_document(file_name="report")`. The collision loop used
    /// `exists()`, which FOLLOWS the link — a dangling link reports false, so the
    /// loop never turned, the candidate WAS the link, and `fs::write` created the
    /// file outside the sandbox while the model was told "report.md was written
    /// to the working folder". `finish_up` then stamped 0600 on the foreign file
    /// too. Measured before the fix: `state=Written`, victim exists.
    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_is_not_written_through() {
        let root = temp_dir("dangling");
        let outside = temp_dir("dangling-victim");
        let victim = outside.join("planted.md");
        let _ = fs::remove_file(&victim);
        std::os::unix::fs::symlink(&victim, root.join("report.md")).expect("plant the link");

        let engine = TextEngine::new(DocumentFormat::Markdown);
        let written = write_document(&engine, "report", None, Some("body"), None, &root).unwrap();

        assert_eq!(
            written.file_name().unwrap(),
            "report-2.md",
            "the loop did not rotate past the link"
        );
        assert!(
            !victim.exists(),
            "the file OUTSIDE the sandbox was created: {}",
            victim.display()
        );
        assert!(
            root.join("report.md")
                .symlink_metadata()
                .expect("the link")
                .file_type()
                .is_symlink(),
            "the link itself was replaced"
        );
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&outside).ok();
    }

    /// If the model bakes an extension INTO THE NAME no double extension MUST be
    /// produced: even though the schema says "without extension" the small model
    /// "gunluk_menu.xlsx" -> "gunluk_menu.xlsx.xlsx".
    #[test]
    fn an_extension_already_in_the_name_is_not_doubled() {
        let root = temp_dir("doubleextension");
        let engine = TextEngine::new(DocumentFormat::Markdown);
        // ".md" is baked into the name -> stem "notes", result "notes.md" (not doubled).
        let path = write_document(&engine, "notes.md", None, Some("x"), None, &root).unwrap();
        assert_eq!(path.file_name().unwrap(), "notes.md");
        // Case insensitive.
        let path2 = write_document(&engine, "REPORT.MD", None, Some("y"), None, &root).unwrap();
        assert_eq!(path2.file_name().unwrap(), "REPORT.md");
        // ANOTHER format's extension IS KEPT: the user may have meant it.
        let path3 = write_document(&engine, "veri.xlsx", None, Some("z"), None, &root).unwrap();
        assert_eq!(path3.file_name().unwrap(), "veri.xlsx.md");
        fs::remove_dir_all(&root).ok();
    }

    /// An unresolvable source_ref: the file must NEVER be written (the costliest error class).
    #[test]
    fn an_unresolvable_ref_writes_no_file() {
        let root = temp_dir("noref");
        let store = Arc::new(SharedStore::new());
        let mut ctx = context(&root, store);
        let tool = CreateDocumentTool::new();
        let outcome = hold(tool.run(
            serde_json::json!({
                "format": "excel",
                "file_name": "report",
                "source_ref": "table#99"
            }),
            &mut ctx,
        ));
        assert!(matches!(outcome.state, ToolState::Failed(_)));
        assert!(outcome.to_model.contains("unknown_data_ref"));
        // The file would now be written to the root (not a subfolder); it must still NEVER be written.
        assert!(!root.join("report.xlsx").exists());
        fs::remove_dir_all(&root).ok();
    }

    /// THE TITLE IS NOT PRINTED TWICE. From a real file: the model passed
    /// `title` and opened `content` with the same heading, and both were written.
    #[test]
    fn a_title_the_body_already_carries_is_not_repeated() {
        let root = temp_dir("title-dup");
        let store = Arc::new(SharedStore::new());
        let mut ctx = context(&root, store);

        let tool = CreateDocumentTool::new();
        let outcome = hold(tool.run(
            serde_json::json!({
                "format": "markdown",
                "file_name": "rastgele_dosya",
                "title": "Rastgele İçerik",
                "content": "# Rastgele İçerik\n\nBu bir rastgele oluşturulan dosyadır.\n"
            }),
            &mut ctx,
        ));
        assert_eq!(outcome.state, ToolState::Written);
        let written = fs::read_to_string(root.join("rastgele_dosya.md")).unwrap();
        assert_eq!(
            written.matches("Rastgele İçerik").count(),
            1,
            "the heading was written twice:\n{written}"
        );
        fs::remove_dir_all(&root).ok();
    }

    /// …and a title the body does NOT carry is still written. The fix must not
    /// have quietly dropped the field.
    #[test]
    fn a_title_the_body_does_not_carry_is_still_written() {
        let root = temp_dir("title-kept");
        let store = Arc::new(SharedStore::new());
        let mut ctx = context(&root, store);

        let tool = CreateDocumentTool::new();
        let outcome = hold(tool.run(
            serde_json::json!({
                "format": "markdown",
                "file_name": "note",
                "title": "Meeting Notes",
                "content": "We agreed on the plan.\n"
            }),
            &mut ctx,
        ));
        assert_eq!(outcome.state, ToolState::Written);
        let written = fs::read_to_string(root.join("note.md")).unwrap();
        assert!(written.starts_with("# Meeting Notes"), "{written}");
        fs::remove_dir_all(&root).ok();
    }

    /// A NAMED DESTINATION IS HONOURED. Without the `folder` field the file
    /// went to the working directory while the model reported success — the
    /// user asked for `~/Desktop` and found the file in their home directory.
    #[test]
    fn a_named_folder_is_where_the_file_lands() {
        let root = temp_dir("folder-named");
        let sub = root.join("notes");
        fs::create_dir_all(&sub).unwrap();
        let store = Arc::new(SharedStore::new());
        let mut ctx = context(&root, store);

        let tool = CreateDocumentTool::new();
        let outcome = hold(tool.run(
            serde_json::json!({
                "format": "markdown",
                "file_name": "note",
                "content": "Hello",
                "folder": "notes"
            }),
            &mut ctx,
        ));
        assert_eq!(outcome.state, ToolState::Written);
        assert!(sub.join("note.md").exists(), "not in the named folder");
        assert!(
            !root.join("note.md").exists(),
            "it also landed in the working directory"
        );
        // The path handed to the model must be the one `read_document` will
        // find — the divergence that made the model invent a table.
        //
        // FORWARD SLASHES ON EVERY PLATFORM, and this assertion is the reason
        // the spelling is not left to the operating system: on Windows the old
        // code produced `\\?\C:\Users\...\notes\note.md` — an absolute
        // path with a verbatim prefix, in a string that then comes back through
        // a JSON argument where the backslash is an escape. See
        // `path_for_model`.
        assert!(
            outcome.to_model.contains("notes/note.md"),
            "{}",
            outcome.to_model
        );
        fs::remove_dir_all(&root).ok();
    }

    /// A DESTINATION THAT DOES NOT EXIST IS A TYPO, NOT A NEW TREE — and no
    /// file is written anywhere, least of all in the working directory.
    #[test]
    fn a_missing_folder_writes_no_file() {
        let root = temp_dir("folder-missing");
        let store = Arc::new(SharedStore::new());
        let mut ctx = context(&root, store);

        let tool = CreateDocumentTool::new();
        let outcome = hold(tool.run(
            serde_json::json!({
                "format": "markdown",
                "file_name": "note",
                "content": "Hello",
                "folder": "nowhere"
            }),
            &mut ctx,
        ));
        assert!(matches!(outcome.state, ToolState::Failed(_)));
        assert!(!root.join("note.md").exists());
        fs::remove_dir_all(&root).ok();
    }

    /// THE SANDBOX STILL DECIDES. A destination the user never opened is
    /// refused; the field opens a way to NAME a directory, not a way out.
    #[test]
    fn a_folder_outside_the_sandbox_is_refused() {
        let root = temp_dir("folder-escape");
        let store = Arc::new(SharedStore::new());
        let mut ctx = context(&root, store);

        let tool = CreateDocumentTool::new();
        let outcome = hold(tool.run(
            serde_json::json!({
                "format": "markdown",
                "file_name": "note",
                "content": "Hello",
                "folder": "/tmp"
            }),
            &mut ctx,
        ));
        assert!(matches!(outcome.state, ToolState::Failed(_)));
        assert!(!std::path::Path::new("/tmp/note.md").exists());
        fs::remove_dir_all(&root).ok();
    }

    /// The bypass channel end to end: the table is in the store, the model carries only the ref, the file is full.
    #[test]
    fn a_file_produced_with_source_ref_is_not_empty() {
        let root = temp_dir("bypass");
        let store = Arc::new(SharedStore::new());
        let table = Table::new(
            ["Name", "Amount"],
            (0..500)
                .map(|i| vec![format!("person{i}"), format!("{i}")])
                .collect::<Vec<_>>(),
        );
        let source_ref = store.put_value("calendar", Value::Table(table));
        let mut ctx = context(&root, store.clone());

        let tool = CreateDocumentTool::new();
        let outcome = hold(tool.run(
            serde_json::json!({
                "format": "excel",
                "file_name": "calendar",
                "source_ref": source_ref.as_str()
            }),
            &mut ctx,
        ));
        assert_eq!(outcome.state, ToolState::Written);
        // The text returned to the model is SHORT: 500 rows of data did not pass through here.
        assert!(outcome.to_model.len() < 80, "{}", outcome.to_model);
        assert!(!outcome.to_model.contains("person499"));
        assert!(
            ctx.session_tainted(),
            "device data was written, the session must be tainted"
        );

        let path = outcome.file_path.expect("file path");
        let byte = fs::read(&path).unwrap();
        let entries = tacet_zip::open_map(&byte).unwrap();
        let sheet = String::from_utf8(entries["xl/worksheets/sheet1.xml"].clone()).unwrap();
        assert!(
            sheet.contains("person499"),
            "the data did not reach the file"
        );
        assert!(sheet.contains("<f>SUM(B2:B501)</f>"), "no formula");
        fs::remove_dir_all(&root).ok();
    }

    /// PLAIN TEXT from the store is preserved as-is when it goes to the markdown format.
    #[test]
    fn the_text_format_writes_the_title_and_the_body() {
        let root = temp_dir("text");
        let store = Arc::new(SharedStore::new());
        let r = CoreDataStore::put(&*store, "note", "3 lines", "alpha\nbeta\ngamma".to_string());
        let mut ctx = context(&root, store);
        let tool = CreateDocumentTool::new();
        let outcome = hold(tool.run(
            serde_json::json!({
                "format": "markdown",
                "file_name": "notes",
                "title": "My Notes",
                "source_ref": r.as_str()
            }),
            &mut ctx,
        ));
        assert_eq!(outcome.state, ToolState::Written);
        let text = fs::read_to_string(outcome.file_path.unwrap()).unwrap();
        assert!(text.starts_with("# My Notes\n\n"), "{text}");
        assert!(text.contains("beta"));
        fs::remove_dir_all(&root).ok();
    }

    /// An invalid format value produces nothing even if it escapes the grammar.
    #[test]
    fn an_invalid_format_is_rejected() {
        let root = temp_dir("format");
        let store = Arc::new(SharedStore::new());
        let mut ctx = context(&root, store);
        let tool = CreateDocumentTool::new();
        let outcome = hold(tool.run(
            serde_json::json!({"format": "powerpoint", "file_name": "x", "content": "y"}),
            &mut ctx,
        ));
        assert!(matches!(outcome.state, ToolState::Failed(_)));
        assert_eq!(outcome.to_model, tacet_kernel::ERROR_MODEL_TEXT);
        fs::remove_dir_all(&root).ok();
    }

    /// THE RACE ITSELF (audit L15). The naming loop saw a free name; before the
    /// write, a dangling link `report.md -> <outside>` landed on that name. The
    /// closure stands in for the moment between the two: it hands out the name
    /// and plants the link in the same breath. The old gate checked the name
    /// and then `fs::write`d it, which follows the link.
    #[cfg(unix)]
    #[test]
    fn a_link_planted_after_the_name_was_chosen_is_not_written_through() {
        let root = temp_dir("race");
        let outside = temp_dir("race-victim");
        let victim = outside.join("planted.md");

        let mut calls = 0;
        let written = write_new_file(
            || {
                calls += 1;
                if calls == 1 {
                    let name = root.join("report.md");
                    std::os::unix::fs::symlink(&victim, &name).expect("plant the link");
                    name
                } else {
                    target_path("report", DocumentFormat::Markdown, &root)
                }
            },
            b"body",
        )
        .expect("the next free name is used");

        assert!(
            !victim.exists(),
            "the document was written through the link to {}",
            victim.display()
        );
        assert_eq!(written.file_name().unwrap(), "report-2.md");
        assert_eq!(fs::read_to_string(&written).unwrap(), "body");
        assert!(
            root.join("report.md")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink(),
            "the planted link itself was replaced"
        );
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&outside).ok();
    }

    /// The same gate for a REAL file that appears in the window: it is not
    /// overwritten, and the permission stamp lands on the file that was made.
    #[test]
    fn a_file_that_appears_after_the_name_was_chosen_is_not_overwritten() {
        let root = temp_dir("race-file");
        let mut calls = 0;
        let written = write_new_file(
            || {
                calls += 1;
                let name = root.join(format!("notes-{calls}.md"));
                if calls == 1 {
                    fs::write(&name, "the user's own file").unwrap();
                }
                name
            },
            b"new",
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(root.join("notes-1.md")).unwrap(),
            "the user's own file"
        );
        assert_eq!(written, root.join("notes-2.md"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&written).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{mode:o}");
        }
        fs::remove_dir_all(&root).ok();
    }

    /// AUDIT L13: U+FFFE and U+FFFF are not XML characters, even escaped. They
    /// reached the sheet raw; the C0 controls were the only ones dropped.
    #[test]
    fn characters_xml_cannot_carry_are_dropped() {
        let xml = sheet_xml(
            &["Na\u{FFFE}me".to_string()],
            &[
                vec!["a\u{FFFF}b\u{1}c\td".to_string()],
                vec!["😀 ü".to_string()],
            ],
        );
        assert!(!xml.contains('\u{FFFE}'), "U+FFFE reached the sheet");
        assert!(!xml.contains('\u{FFFF}'), "U+FFFF reached the sheet");
        assert!(!xml.contains('\u{1}'));
        assert!(xml.contains(">Name<"), "{xml}");
        assert!(xml.contains(">abc\td<"), "the tab is legal and kept: {xml}");
        assert!(xml.contains("😀 ü"), "legal text outside the BMP is kept");
    }

    /// AUDIT L13: the grid has edges. A 16,385th column would be written as
    /// `XFE`, a row past 1,048,576 as `r="1048577"` — both are a "repair" in
    /// Excel. They are refused before any XML is built, and no file is left.
    #[test]
    fn a_table_past_the_grid_is_refused_and_writes_no_file() {
        let root = temp_dir("grid");
        let wide = Table::new(
            (0..MAX_SHEET_COLUMNS + 1).map(|i| format!("c{i}")),
            vec![vec!["1".to_string()]],
        );
        let refused = write_document(&ExcelEngine::new(), "wide", None, None, Some(&wide), &root);
        assert!(
            matches!(refused, Err(ToolError::InvalidArgument(ref m)) if m.contains("16384")),
            "{refused:?}"
        );

        // Header + 1,048,575 data rows + Total = one row past the grid. Empty
        // rows allocate nothing, so the fixture is ~25 MiB of `Vec` headers.
        let tall = Table::new(["A"], vec![Vec::new(); MAX_SHEET_ROWS - 1]);
        let refused = write_document(&ExcelEngine::new(), "tall", None, None, Some(&tall), &root);
        assert!(
            matches!(refused, Err(ToolError::InvalidArgument(ref m)) if m.contains("1048576")),
            "{:?}",
            refused.map(|p| p.display().to_string())
        );
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            0,
            "a file was left behind"
        );

        // The edges themselves fit.
        assert!(check_sheet_bounds(MAX_SHEET_COLUMNS, &[vec![]]).is_ok());
        assert!(check_sheet_bounds(1, &vec![Vec::new(); MAX_SHEET_ROWS - 2]).is_ok());
        fs::remove_dir_all(&root).ok();
    }

    /// AUDIT M21-b: `| Amount |` with 100 and 200 got a Total row whose only
    /// cell was the word "Total" — column A was labelled and never summed.
    #[test]
    fn a_single_numeric_column_still_gets_its_total() {
        let xml = sheet_xml(
            &["Amount".to_string()],
            &[vec!["100".to_string()], vec!["200".to_string()]],
        );
        assert!(
            xml.contains("<c r=\"A4\"><f>SUM(A2:A3)</f></c>"),
            "no total under the only column: {xml}"
        );
        assert!(
            !xml.contains("r=\"B4\""),
            "the Total row must not widen the table: {xml}"
        );

        // A numeric FIRST column next to a text one: the sum stays in A and the
        // label moves to the first column that is not summed.
        let xml = sheet_xml(
            &["Amount".to_string(), "Note".to_string()],
            &[vec!["5".to_string(), "x".to_string()]],
        );
        assert!(xml.contains("<c r=\"A3\"><f>SUM(A2:A2)</f></c>"), "{xml}");
        assert!(
            xml.contains("<c r=\"B3\" t=\"inlineStr\"><is><t xml:space=\"preserve\">Total</t>"),
            "{xml}"
        );
    }

    /// AUDIT L15 REVIEW: the `O_NOFOLLOW` bit was `0x20000` on every Linux. On
    /// aarch64 that bit is `O_LARGEFILE`, so the second belt was missing on a
    /// published target. Expected values are the libc crate's, per target.
    #[test]
    fn the_nofollow_bit_is_the_one_each_architecture_defines() {
        assert_eq!(o_nofollow_for("linux", "aarch64"), 0x8000);
        assert_eq!(o_nofollow_for("android", "aarch64"), 0x8000);
        assert_eq!(o_nofollow_for("linux", "arm"), 0x8000);
        assert_eq!(o_nofollow_for("linux", "x86_64"), 0x20000);
        assert_eq!(o_nofollow_for("linux", "riscv64"), 0x20000);
        assert_eq!(o_nofollow_for("macos", "aarch64"), 0x0100);
        assert_eq!(o_nofollow_for("macos", "x86_64"), 0x0100);
        // An architecture nobody spelled out gets no extra flag, not a guess.
        assert_eq!(o_nofollow_for("linux", "mips64"), 0);
        assert_eq!(o_nofollow_for("freebsd", "x86_64"), 0);
    }

    /// AUDIT M21, RE-DERIVED: the front page's demo ("put that in a spreadsheet
    /// called countdown") claimed a live `=DATEDIF()`. This runs the tool on the
    /// table that request produces and records what is really written: one data
    /// row, and a `=SUM()` under the numeric column — the only formula there is.
    #[test]
    fn the_countdown_demo_writes_a_sum_and_nothing_else() {
        let root = temp_dir("countdown");
        let mut ctx = context(&root, Arc::new(SharedStore::new()));
        let outcome = hold(CreateDocumentTool::new().run(
            serde_json::json!({
                "format": "excel",
                "file_name": "countdown",
                "content": "| Event | Date | Days left |\n| --- | --- | --- |\n| 14 March | 2027-03-14 | 229 |"
            }),
            &mut ctx,
        ));
        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.to_model);
        assert_eq!(outcome.to_model, "file_created (excel): countdown.xlsx");

        let bytes = fs::read(root.join("countdown.xlsx")).unwrap();
        let sheet = String::from_utf8(
            tacet_zip::open_map(&bytes).unwrap()["xl/worksheets/sheet1.xml"].clone(),
        )
        .unwrap();
        let formulas: Vec<&str> = sheet
            .split("<f>")
            .skip(1)
            .filter_map(|f| f.split("</f>").next())
            .collect();
        assert_eq!(formulas, vec!["SUM(C2:C2)"], "{sheet}");
        assert!(!sheet.contains("DATEDIF"));
        // The date is text: nothing here does date arithmetic.
        assert!(sheet.contains(">2027-03-14<"), "{sheet}");
        fs::remove_dir_all(&root).ok();
    }

    /// AUDIT M21: both READMEs quote the countdown demo, and the formula they
    /// name has to be the one `the_countdown_demo_writes_a_sum_and_nothing_else`
    /// shows the tool writing. The page once said `=DATEDIF()`, which no line of
    /// this crate produces.
    #[test]
    fn the_readme_demo_names_the_formula_the_tool_writes() {
        let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for page in ["../../README.md", "../tacet-cli/README.md"] {
            let text = fs::read_to_string(crate_dir.join(page)).unwrap();
            let line = text
                .lines()
                .find(|l| l.contains("countdown.xlsx, one row"))
                .unwrap_or_else(|| panic!("{page}: the countdown demo line is gone"));
            assert!(!line.contains("DATEDIF"), "{page}: {line}");
            assert!(line.contains("=SUM()"), "{page}: {line}");
        }
    }

    /// Without XML escaping the produced sheet becomes unopenable.
    #[test]
    fn xml_special_chars_are_escaped() {
        let xml = sheet_xml(
            &["A&B".to_string()],
            &[vec!["<script>".to_string()], vec!["\"quote\"".to_string()]],
        );
        assert!(xml.contains("A&amp;B"));
        assert!(xml.contains("&lt;script&gt;"));
        assert!(!xml.contains("<script>"));
    }
}
