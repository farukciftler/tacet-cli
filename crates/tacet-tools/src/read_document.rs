//! `read_document` — a perception tool: reads an .xlsx table or a plain
//! text/markdown file and returns VALID markdown to the model.
//!
//! THE HEART OF THE CHAIN (learned the expensive way in the Swift version): for
//! a document with a table, the text going to the model is the output of
//! `Table::markdown_truncated` — piped, with an alignment row, and the full
//! column count on every row. The small model passes this block through almost
//! as it stands, and the upper layer parses it and draws the real table. It
//! used to return a plain `summary` (no pipes, cut at 5 lines); the model could
//! not rebuild the table from it and said "the table was shown" while skipping
//! the content. That is why truncation here ALWAYS goes through
//! markdown_truncated, and even the character cap is applied at a line boundary
//! (see `truncate_at_line_boundary`): a half-finished `| a | b` row would
//! invalidate the whole block.
//! BYPASS CHANNEL: if the document is large the raw table/text is put into the
//! DataStore and a short preview + `source_ref` goes back to the model.
//! Truncation therefore becomes not DATA LOSS but merely a window decision; the
//! next tool reaches the truncated part in full through the reference.
//! XML: the sheet/sharedStrings inside an xlsx are scanned by hand in this file.
//! We do not pull in a ready-made XML crate (the zero-dependency identity); the
//! subset we need -- tag name, attribute, text node, basic entity resolution -- is 150 lines.

use crate::data_store::{SharedStore, Table, Value as StoredValue};
use serde_json::Value;
use std::sync::Arc;
use tacet_kernel::{
    ArgSchema, Field, SourceRef, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult,
    TraceUpdate, boxed,
};

/// The number of table rows shown to the model when it is not put into the store.
///
/// The reason it is kept high: if the data cannot be fetched back, filling the
/// window is the only chance. With the store attached, the token cost of 30
/// rows no longer has a justification.
const PREVIEW_FULL: usize = 30;

/// The preview that suffices once it is in the store: it is enough for the
/// model to see the SHAPE of the table, the body is reached with source_ref.
const PREVIEW_WITH_REF: usize = 10;
/// The character cap of the body going to the model (~375 tokens).
const MODEL_CAP: usize = 1500;
/// The threshold for moving a text document into the store.
const TEXT_STORE_THRESHOLD: usize = 1500;
/// The upper bound of a file to be read (32 MiB). Anything above that is not a
/// document but a symptom of an error; the zip caps (tacet-zip) work separately behind this gate.
const FILE_CAP: u64 = 32 * 1024 * 1024;

/// The extensions allowed to be read as text.
///
/// A whitelist, not a blacklist: reading an unknown extension because "it is
/// probably text" means dumping a binary file to the model as broken UTF-8.
const TEXT_EXTENSIONS: &[&str] = &["txt", "md", "markdown", "text", "log", "csv", "tsv", "json"];

/// Reads the document and returns a markdown/text summary to the model.
pub struct ReadDocumentTool {
    /// The typed store. Optional, because the core contract (`ctx.store`) only
    /// takes a `String` body; storing a table AS A TABLE needs the concrete
    /// store. If it is not attached it still works, falling back to text.
    store: Option<Arc<SharedStore>>,
}

impl Default for ReadDocumentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadDocumentTool {
    pub fn new() -> Self {
        Self { store: None }
    }

    pub fn with_store(store: Arc<SharedStore>) -> Self {
        Self { store: Some(store) }
    }

    /// Puts the table into the store; without a typed store it falls back to the core contract.
    fn store_table(&self, ctx: &ToolContext, table: &Table) -> SourceRef {
        match &self.store {
            Some(d) => d.put_value("document", StoredValue::Table(table.clone())),
            None => ctx.store(
                "document",
                &table_summary(table),
                table.markdown_truncated(usize::MAX),
            ),
        }
    }

    fn store_text(&self, ctx: &ToolContext, text: &str) -> SourceRef {
        match &self.store {
            Some(d) => d.put_value("document", StoredValue::Text(text.to_string())),
            None => ctx.store(
                "document",
                &format!("{} lines of text", text.lines().count()),
                text.to_string(),
            ),
        }
    }
}

impl Tool for ReadDocumentTool {
    fn name(&self) -> &str {
        "read_document"
    }

    fn description(&self) -> &str {
        "Reads a document from disk (.xlsx spreadsheet, .md or plain text) and returns its \
         content. Call this IMMEDIATELY when the user asks about a file ('summarize it', \
         \"what's in it\", 'show it as a table', in any language); read before describing. \
         Spreadsheets come back as a markdown table you can pass through as-is."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new(
                "path",
                ArgSchema::text().description(
                    "Path to the document, relative to the working directory or an \
                     absolute path in a folder the user opened. \
                     Supported: .xlsx, .md, .txt, .csv, .log, .json",
                ),
            )
            .required(),
            Field::new(
                "focus",
                ArgSchema::text().description(
                    "Optional: the topic the user cares about. Leave empty to read the \
                     whole document.",
                ),
            ),
        ])
        .description("Read a document and return its content")
    }

    /// It is reading the user's own document: the content may be personal data,
    /// so the session becomes tainted.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let trace = ctx.start_chip("doc", "Reading document…");

            let outcome = self.read(&args, ctx);
            let (outcome, tainted) = match outcome {
                Ok(s) => (s, true),
                Err(h) => (ToolOutcome::failed(&h), false),
            };

            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(args.to_string())
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            // Tainting only when data is REALLY read: a failed path touched no
            // content, and tainting the session for it would push the user into
            // needless approval questions.
            if tainted {
                ctx.taint();
            }
            outcome
        })
    }
}

impl ReadDocumentTool {
    /// The synchronous body: `run` only holds the chip/taint shell, so the error
    /// path is collected in one place (`ToolOutcome::failed`).
    fn read(&self, args: &Value, ctx: &ToolContext) -> ToolResult<ToolOutcome> {
        let raw_path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::MissingField("path".into()))?;

        // SYMLINKS: `resolve_path` is LEXICAL, so a link planted in the sandbox
        // (`run_code` is allowed to write there, and creating a link IS a write)
        // passes it as an ordinary component and `metadata()`/`read()` FOLLOW it
        // straight out of the working directory — the file's bytes would land in
        // the model's window and could leave the device on a later web/mcp call.
        // `resolve_existing_file` canonicalizes EVERY component and redoes the
        // containment test on the real path. Do not replace it with a leaf-only
        // `symlink_metadata`: in `gate/Library/credentials.json` the leaf is a
        // real file and the escape happens at an intermediate component.
        let path = crate::sandbox_path::resolve_existing_file(ctx, raw_path)?;
        let upper = path
            .metadata()
            .map_err(|_| ToolError::FileNotFound(path.clone()))?;
        if upper.len() > FILE_CAP {
            return Err(ToolError::Other(format!(
                "file too large ({} bytes), cap {FILE_CAP}",
                upper.len()
            )));
        }

        let name = path
            .file_name()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_else(|| raw_path.to_string());
        let extension = path
            .extension()
            .map(|u| u.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let byte = std::fs::read(&path)?;

        if extension == "xlsx" {
            let workbook = parse_workbook(&byte)?;
            Ok(self.table_outcome(ctx, &name, workbook).file_path(path))
        } else if TEXT_EXTENSIONS.contains(&extension.as_str()) {
            let text = to_text(&byte)?;
            Ok(self.text_outcome(ctx, &name, &text).file_path(path))
        } else {
            Err(ToolError::InvalidArgument(format!(
                "unsupported document format: .{extension}"
            )))
        }
    }

    fn table_outcome(&self, ctx: &ToolContext, name: &str, workbook: Workbook) -> ToolOutcome {
        // The facts about what was NOT read go after the table, outside its
        // block, the way `markdown_truncated` writes its own row note.
        let mut notes = Vec::new();
        if workbook.rows_not_read > 0 {
            notes.push(format!(
                "{} more rows were not read: a sheet is read up to {CELL_BUDGET} cells",
                workbook.rows_not_read
            ));
        }
        if let Some(note) = workbook.other_sheets_note() {
            notes.push(note);
        }
        let notes: String = notes.iter().map(|n| format!("\n\n({n})")).collect();
        let sheets = match workbook.shown {
            Some(i) if workbook.sheets.len() > 1 => {
                format!(", sheet {} of {}", i + 1, workbook.sheets.len())
            }
            _ => String::new(),
        };
        let table = workbook.table;

        if table.column_count() == 0 {
            return ToolOutcome::read_ok(
                format!("{name} read, the table is empty{sheets}"),
                format!("document_empty (the spreadsheet has no rows){notes}"),
            );
        }

        // Only a table that does not fit in the preview goes into the store: a
        // small table already goes through in full, and producing a reference
        // would add a needless indirection for the model.
        let source = (table.row_count() > PREVIEW_FULL).then(|| self.store_table(ctx, &table));
        let preview = if source.is_some() {
            PREVIEW_WITH_REF
        } else {
            PREVIEW_FULL
        };

        let body = truncate_at_line_boundary(&table.markdown_truncated(preview), MODEL_CAP);
        let suffix = source
            .as_ref()
            .map(|r| tacet_kernel::source_ref_suffix(r.as_str()))
            .unwrap_or_default();

        ToolOutcome::read_ok(
            format!(
                "{name} read ({} rows x {} columns{sheets})",
                table.row_count(),
                table.column_count()
            ),
            format!("{body}{notes}{suffix}"),
        )
        // The chip detail carries the FULL table: the model's window is
        // truncated, what the user sees is not (transparency, second layer).
        .raw_output(table.markdown_truncated(usize::MAX))
    }

    fn text_outcome(&self, ctx: &ToolContext, name: &str, text: &str) -> ToolOutcome {
        if text.trim().is_empty() {
            return ToolOutcome::read_ok(
                format!("{name} read, the content is empty"),
                "document_empty (the file has no text)",
            );
        }

        let source = (text.len() > TEXT_STORE_THRESHOLD).then(|| self.store_text(ctx, text));
        let body = truncate_at_line_boundary(text, MODEL_CAP);
        let suffix = source
            .as_ref()
            .map(|r| tacet_kernel::source_ref_suffix(r.as_str()))
            .unwrap_or_default();

        ToolOutcome::read_ok(
            format!("{name} read ({} lines)", text.lines().count()),
            format!("{body}{suffix}"),
        )
        .raw_output(text.to_string())
    }
}

fn table_summary(table: &Table) -> String {
    format!(
        "{} rows x {} columns table",
        table.row_count(),
        table.column_count()
    )
}

/// Applies the character cap at a LINE boundary.
///
/// Cutting in the middle would invalidate the markdown table (a half `| a | b`);
/// an invalid table = the model skipping the content. Text over the cap is cut
/// at the last complete line, and the cut is stated OUTSIDE the table block it was cut from.
fn truncate_at_line_boundary(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let mut cut = 0usize;
    for (i, _) in text.match_indices('\n') {
        if i >= cap {
            break;
        }
        cut = i;
    }
    if cut == 0 {
        // A single enormous line: there is no table structure to break here.
        let mut last = cap.min(text.len());
        while last > 0 && !text.is_char_boundary(last) {
            last -= 1;
        }
        return format!("{}…", &text[..last]);
    }
    format!("{}\n\n(the rest was truncated)", &text[..cut])
}

/// Converts the bytes to text; rejects a binary file.
///
/// A NUL byte on its own is a sufficient signal: it does not occur in text
/// documents and occurs almost always in binary formats. The chance of a false
/// positive is cheaper than the risk of dumping a binary file to the model as a pile of broken characters.
fn to_text(byte: &[u8]) -> ToolResult<String> {
    if byte.contains(&0) {
        return Err(ToolError::InvalidArgument(
            "the file is not text (binary content)".into(),
        ));
    }
    Ok(String::from_utf8_lossy(byte).into_owned())
}

// ---------------------------------------------------------------------------
// xlsx resolution
// ---------------------------------------------------------------------------

/// The most cells — padding included — one sheet may turn into.
///
/// THE PADDING WAS THE BOMB, NOT THE DATA. Cells used to be placed by padding
/// every row with empty `String`s up to the cell's column, and Excel's column
/// cap is 16,384: a row reading `<row><c r="XFD1"/></row>` is 24 bytes of XML.
/// MEASURED (audit M6, 9 Oct 2026): a 1,582-byte sheet of 64 such rows became
/// 1,048,576 `String`s (~24 MiB of headers alone) and 3 MiB of markdown, and at
/// the 64 MiB zip entry cap the process ran out of memory. Rows are now sparse
/// (an empty cell costs nothing) and this budget caps what is left: the table
/// renders as `(rows + 1) x width` cells, so that product is what is counted.
/// 262,144 holds a 10,000-row export with columns A..Z; past it the remaining
/// rows are counted and reported, never silently dropped.
const CELL_BUDGET: usize = 262_144;

/// The most cell TEXT one sheet may materialise, in bytes.
///
/// The second amplifier: a `t="s"` cell is an index into sharedStrings, and
/// every reference CLONES the string. One 1 MiB shared string referenced from
/// 64 cells of ~20 bytes each is 64 MiB out of ~1.3 KiB of sheet XML. The cap
/// is the file cap — a table never holds more text than a whole file could.
const TEXT_BUDGET: usize = FILE_CAP as usize;

/// Excel's own column cap (`XFD`). A cell past it is malformed input; it is
/// dropped rather than allowed to widen the table.
const MAX_COLUMNS: usize = 16_384;

/// How many sheet names the model is told about, and how long each may be.
/// Excel caps a sheet name at 31 characters; a hand-made file need not, and
/// the names go into a 4096-token window.
const SHEET_NAMES_SHOWN: usize = 20;
const SHEET_NAME_CHARS: usize = 31;

/// One read of a workbook: the table of the sheet that was read, and the facts
/// that stop "one sheet" being mistaken for "the workbook".
///
/// WHY THE NAMES TRAVEL WITH THE TABLE (audit L17): the reader used to return
/// the first sheet's table and nothing else, so a three-sheet workbook was
/// reported to the model as if it were one table, and `edit_document` wrote an
/// "(edited)" copy holding one sheet without a word about the other two.
#[derive(Debug, Default)]
pub struct Workbook {
    pub table: Table,
    /// Every sheet name in workbook order. Empty when the file has no
    /// `xl/workbook.xml` to name them (a bare hand-packed sheet).
    pub sheets: Vec<String>,
    /// The index in `sheets` of the sheet `table` came from, when known.
    pub shown: Option<usize>,
    /// Data rows the budgets left out of `table`.
    pub rows_not_read: usize,
}

impl Workbook {
    /// The sentence that says which sheet this is and which ones were not read;
    /// `None` for a one-sheet workbook, where it would only be noise.
    pub fn other_sheets_note(&self) -> Option<String> {
        if self.sheets.len() < 2 {
            return None;
        }
        let names: Vec<String> = self
            .sheets
            .iter()
            .take(SHEET_NAMES_SHOWN)
            .enumerate()
            .map(|(i, n)| {
                let short: String = n.chars().take(SHEET_NAME_CHARS).collect();
                if Some(i) == self.shown {
                    format!("\"{short}\" (this one)")
                } else {
                    format!("\"{short}\"")
                }
            })
            .collect();
        let more = self.sheets.len().saturating_sub(SHEET_NAMES_SHOWN);
        let tail = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        Some(format!(
            "the workbook has {} sheets: {}{tail}; only one sheet is read",
            self.sheets.len(),
            names.join(", ")
        ))
    }

    /// The names of the sheets that were NOT read, for a caller that has to say
    /// what it is about to leave behind.
    pub fn unread_sheets(&self) -> Vec<String> {
        self.sheets
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != self.shown)
            .take(SHEET_NAMES_SHOWN)
            .map(|(_, n)| n.chars().take(SHEET_NAME_CHARS).collect())
            .collect()
    }
}

/// Converts .xlsx bytes to a `Table`. The first row is taken as the header.
pub fn parse_xlsx(byte: &[u8]) -> ToolResult<Table> {
    Ok(parse_workbook(byte)?.table)
}

/// Reads the first worksheet of an .xlsx and names every sheet in it.
pub fn parse_workbook(byte: &[u8]) -> ToolResult<Workbook> {
    let map = tacet_zip::open_map(byte).map_err(|h| ToolError::Other(h.to_string()))?;

    // THE WORKBOOK DECIDES WHICH SHEET IS FIRST, not the part name: Excel keeps
    // `sheet1.xml` for the first sheet ever created, and a sheet dragged to the
    // front keeps its own part name. Where the workbook or its rels are missing
    // the old rule stands: sheet1.xml, else the alphabetically first worksheet
    // part (deterministic because BTreeMap is ordered).
    let listed = sheet_list(&map);
    let shown = listed
        .iter()
        .position(|(_, part)| part.as_ref().is_some_and(|p| map.contains_key(p)));
    let sheet = shown
        .and_then(|i| listed[i].1.as_ref())
        .and_then(|p| map.get(p))
        .or_else(|| map.get("xl/worksheets/sheet1.xml"))
        .or_else(|| {
            map.iter()
                .find(|(name, _)| name.starts_with("xl/worksheets/") && name.ends_with(".xml"))
                .map(|(_, v)| v)
        })
        .ok_or_else(|| ToolError::Other("no worksheet found inside the xlsx".into()))?;

    let shared = map
        .get("xl/sharedStrings.xml")
        .map(|d| parse_shared_strings(&String::from_utf8_lossy(d)))
        .unwrap_or_default();

    let read = parse_sheet(&String::from_utf8_lossy(sheet), &shared);
    let mut it = read.rows.into_iter();
    let table = match it.next() {
        Some(headers) => Table::new(headers, it),
        None => Table::default(),
    };
    Ok(Workbook {
        table,
        sheets: listed.into_iter().map(|(name, _)| name).collect(),
        shown,
        rows_not_read: read.rows_not_read,
    })
}

/// `(sheet name, worksheet part path)` in workbook order. The part is `None`
/// when the relationship is missing or is not a worksheet (a chartsheet has a
/// name and no cells).
fn sheet_list(map: &std::collections::BTreeMap<String, Vec<u8>>) -> Vec<(String, Option<String>)> {
    let Some(workbook) = map.get("xl/workbook.xml") else {
        return Vec::new();
    };
    let mut targets = std::collections::HashMap::new();
    if let Some(rels) = map.get("xl/_rels/workbook.xml.rels") {
        let rels = String::from_utf8_lossy(rels);
        for part in chunk(&rels) {
            if let Chunk::Open {
                name: "Relationship",
                attributes,
                ..
            } = part
                && let (Some(id), Some(target)) =
                    (attribute(attributes, "Id"), attribute(attributes, "Target"))
                && attribute(attributes, "Type").is_some_and(|t| t.ends_with("/worksheet"))
            {
                let target = resolve_entity(target);
                // A target is relative to `xl/` unless it is package-absolute.
                let path = match target.strip_prefix('/') {
                    Some(absolute) => absolute.to_string(),
                    None => format!("xl/{target}"),
                };
                targets.insert(id.to_string(), path);
            }
        }
    }
    let workbook = String::from_utf8_lossy(workbook);
    chunk(&workbook)
        .filter_map(|part| match part {
            Chunk::Open {
                name: "sheet",
                attributes,
                ..
            } => {
                let name = resolve_entity(attribute(attributes, "name").unwrap_or(""));
                // `r:id` — `attribute` compares local names, and `sheetId` is
                // a different local name, so this cannot pick that one up.
                let part = attribute(attributes, "id").and_then(|id| targets.get(id).cloned());
                Some((name, part))
            }
            _ => None,
        })
        .collect()
}

/// The parts produced by the XML scanner.
#[derive(Debug, PartialEq)]
enum Chunk<'a> {
    Open {
        name: &'a str,
        attributes: &'a str,
        self_closing: bool,
    },
    Close(&'a str),
    Text(&'a str),
}

/// A very small XML scanner.
///
/// ITS LIMIT IS EXPLICIT: if an attribute value contains an unescaped '>' the
/// tag ends early. In OOXML production that character is always escaped as
/// `&gt;`, so it is an unreachable path in practice; the cost of pulling in a
/// full XML parser is too high for this single limit.
///
/// AN ITERATOR, NOT A `Vec`: a `Chunk` is ~40 bytes and `<a/>` is 4, so
/// collecting the whole part first cost ten times the XML before a single cell
/// was read — 640 MiB for a sheet at the 64 MiB zip entry cap.
fn chunk(xml: &str) -> impl Iterator<Item = Chunk<'_>> {
    let mut i = 0usize;
    std::iter::from_fn(move || {
        while i < xml.len() {
            if xml.as_bytes()[i] == b'<' {
                let length = xml[i..].find('>')?;
                let inner = &xml[i + 1..i + length];
                i += length + 1;
                // A declaration (<?xml?>), a comment and a DOCTYPE do not concern us.
                if inner.starts_with('?') || inner.starts_with('!') {
                    continue;
                }
                if let Some(name) = inner.strip_prefix('/') {
                    return Some(Chunk::Close(local_name(name.trim())));
                }
                let self_closing = inner.ends_with('/');
                let body = inner.trim_end_matches('/').trim();
                let (name, attributes) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
                return Some(Chunk::Open {
                    name: local_name(name),
                    attributes,
                    self_closing,
                });
            }
            let start = i;
            i = xml[i..].find('<').map(|p| i + p).unwrap_or(xml.len());
            return Some(Chunk::Text(&xml[start..i]));
        }
        None
    })
}

/// `x:t` -> `t`. Namespace prefixes carry no meaning, and in a single-sheet
/// OOXML there are no two `t`s to confuse.
fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Extracts a single value out of a tag's attribute text.
fn attribute<'a>(attributes: &'a str, needle: &str) -> Option<&'a str> {
    let b = attributes.as_bytes();
    let n = b.len();
    let mut i = 0usize;
    while i < n {
        while i < n && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let emit = i;
        while i < n && b[i] != b'=' && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i == emit {
            break;
        }
        let key = &attributes[emit..i];
        while i < n && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n || b[i] != b'=' {
            continue;
        }
        i += 1;
        while i < n && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n || (b[i] != b'"' && b[i] != b'\'') {
            break;
        }
        let quote = b[i];
        i += 1;
        let deger_bas = i;
        while i < n && b[i] != quote {
            i += 1;
        }
        let value = &attributes[deger_bas..i.min(n)];
        i += 1;
        if local_name(key) == needle {
            return Some(value);
        }
    }
    None
}

/// The `<si>` values inside `sharedStrings.xml`.
fn parse_shared_strings(xml: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut body = String::new();
    let mut inside_si = false;
    let mut collecting = false;

    for part in chunk(xml) {
        match part {
            Chunk::Open { name: "si", .. } => {
                inside_si = true;
                body.clear();
            }
            // In rich text a single <si> carries more than one <t>; they are all
            // joined, otherwise a formatted cell is read half.
            Chunk::Open { name: "t", .. } if inside_si => collecting = true,
            Chunk::Text(m) if collecting => body.push_str(&resolve_entity(m)),
            Chunk::Close("t") => collecting = false,
            Chunk::Close("si") => {
                values.push(std::mem::take(&mut body));
                inside_si = false;
            }
            _ => {}
        }
    }
    values
}

/// The rows of one sheet, and how many rows the budgets left out.
#[derive(Debug, Default)]
struct Sheet {
    /// Row 0 is the header, padded to the widest row kept; data rows end at
    /// their last non-empty cell (the markdown writer pads them on output).
    rows: Vec<Vec<String>>,
    rows_not_read: usize,
}

/// What one sheet may cost; a parameter so the tests can reach the edge with
/// a fixture of a few kilobytes.
#[derive(Debug, Clone, Copy)]
struct Budget {
    cells: usize,
    text: usize,
}

const BUDGET: Budget = Budget {
    cells: CELL_BUDGET,
    text: TEXT_BUDGET,
};

/// The row/cell values inside `sheet*.xml`.
fn parse_sheet(xml: &str, shared: &[String]) -> Sheet {
    parse_sheet_within(xml, shared, BUDGET)
}

fn parse_sheet_within(xml: &str, shared: &[String], budget: Budget) -> Sheet {
    let mut sheet = Sheet::default();
    // SPARSE: `(column, value)` for the non-empty cells only. An empty cell —
    // the self-closing `<c r="XFD1"/>` the audit's bomb was built from — moves
    // the next free column and allocates nothing.
    let mut active: Vec<(usize, String)> = Vec::new();
    let mut next = 0usize;
    let mut body = String::new();
    let mut collecting = false;
    let mut is_shared = false;
    let mut column = 0usize;
    let mut width = 0usize;
    let mut text_used = 0usize;
    let mut full = false;

    for part in chunk(xml) {
        match part {
            Chunk::Open { name: "row", .. } => {
                active.clear();
                next = 0;
            }
            Chunk::Open {
                name: "c",
                attributes,
                ..
            } => {
                is_shared = attribute(attributes, "t") == Some("s");
                // Puts the cell in ITS OWN column. The Swift version added the
                // cells in order; since Excel does not write a `<c>` for an
                // empty cell, after one gap the whole row shifted one column
                // left. With the `r="C2"` reference available there is no
                // reason to repeat that bug. Without one, the cell follows the
                // furthest column the row has reached.
                column = attribute(attributes, "r")
                    .and_then(column_index)
                    .unwrap_or(next);
                next = next.max(column.saturating_add(1));
                body.clear();
            }
            // Once the budget is spent the remaining rows are only COUNTED:
            // collecting their text would be the very work the budget refuses.
            Chunk::Open {
                name: "t" | "v", ..
            } => collecting = !full,
            Chunk::Text(m) if collecting => body.push_str(&resolve_entity(m)),
            Chunk::Close("t" | "v") => collecting = false,
            Chunk::Close("c") => {
                let value = if full || column >= MAX_COLUMNS {
                    String::new()
                } else if is_shared {
                    // A malformed index leaves the cell empty rather than
                    // panicking: the input may be a file somebody else
                    // produced. The length is checked BEFORE the clone — the
                    // clone is the amplifier.
                    match body
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| shared.get(i))
                    {
                        Some(s) if text_used.saturating_add(s.len()) > budget.text => {
                            full = true;
                            String::new()
                        }
                        Some(s) => s.clone(),
                        None => String::new(),
                    }
                } else {
                    std::mem::take(&mut body)
                };
                if !value.is_empty() {
                    text_used = text_used.saturating_add(value.len());
                    if text_used > budget.text {
                        full = true;
                    } else {
                        active.push((column, value));
                    }
                }
                body.clear();
            }
            Chunk::Close("row") => {
                if full {
                    sheet.rows_not_read += 1;
                    continue;
                }
                let row_width = active.iter().map(|(c, _)| c + 1).max().unwrap_or(0);
                let wider = width.max(row_width);
                // The table renders as (rows + 1) x width; an empty row still
                // costs one, or a million `<row></row>` would be free.
                let cost = (sheet.rows.len() + 1).saturating_mul(wider.max(1));
                if cost > budget.cells {
                    full = true;
                    sheet.rows_not_read += 1;
                    continue;
                }
                width = wider;
                let mut dense = vec![String::new(); row_width];
                for (c, v) in active.drain(..) {
                    dense[c] = v;
                }
                sheet.rows.push(dense);
            }
            _ => {}
        }
    }
    // THE HEADER SPANS EVERY COLUMN THAT HOLDS DATA. Trailing empty cells are
    // no longer stored, so a header whose last cells are blank would otherwise
    // be narrower than its data, and `Table` shows exactly `headers.len()`
    // columns — the data under a blank header would vanish from the read.
    if let Some(header) = sheet.rows.first_mut() {
        header.resize(width, String::new());
    }
    sheet
}

/// `C12` -> `Some(2)`. Bijective base 26, converted to a 0-based index.
fn column_index(cell_ref: &str) -> Option<usize> {
    let mut n = 0usize;
    let mut has_letter = false;
    for c in cell_ref.chars() {
        if c.is_ascii_alphabetic() {
            has_letter = true;
            n = n
                .checked_mul(26)?
                .checked_add((c.to_ascii_uppercase() as usize) - ('A' as usize) + 1)?;
        } else {
            break;
        }
    }
    // Excel's column cap is 16384; above that the reference is malformed or
    // malicious and would turn straight into allocating an enormous Vec.
    if !has_letter || n == 0 || n > 16_384 {
        return None;
    }
    Some(n - 1)
}

/// XML entity resolution — the set OOXML produces.
fn resolve_entity(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut output = String::with_capacity(raw.len());
    let mut remaining = raw;
    while let Some(emit) = remaining.find('&') {
        output.push_str(&remaining[..emit]);
        let body = &remaining[emit..];
        // THE `;` IS LOOKED FOR IN THE FIRST 13 BYTES ONLY. The longest name
        // accepted is 12 bytes past the `&`, and the filter used to run AFTER
        // a `find` over the whole rest of the text — once per `&`, so a run of
        // ampersands was quadratic. MEASURED (audit M7): 128 KiB 0.25 s,
        // 256 KiB 0.86 s, 512 KiB 3.40 s, ~15 hours projected at the 64 MiB
        // entry cap. `;` is ASCII, so the byte index is a char boundary.
        let window = &body.as_bytes()[..body.len().min(13)];
        let Some(last) = window.iter().position(|b| *b == b';') else {
            output.push('&');
            remaining = &body[1..];
            continue;
        };
        let name = &body[1..last];
        match name {
            "amp" => output.push('&'),
            "lt" => output.push('<'),
            "gt" => output.push('>'),
            "quot" => output.push('"'),
            "apos" => output.push('\''),
            _ => {
                let numeric = name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse::<u32>().ok()))
                    .and_then(char::from_u32);
                match numeric {
                    Some(c) => output.push(c),
                    // Leaving an entity we do not recognise AS IT IS beats
                    // silently swallowing it: no data is lost and it is visible to the eye.
                    None => output.push_str(&body[..=last]),
                }
            }
        }
        remaining = &body[last + 1..];
    }
    output.push_str(remaining);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use tacet_kernel::{ERROR_MODEL_TEXT, InMemoryDataStore, SilentReporter, ToolState};
    use tacet_zip::{ZipEntry, pack};

    /// no tokio dependency; the minimum executor that suffices for a test.
    fn no_futures<F: std::future::Future>(mut f: F) -> F::Output {
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

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "tacet-read-document-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn context(dir: &std::path::Path) -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            dir,
            Arc::new(SilentReporter),
        )
    }

    /// Headers as inlineStr, data rows as <v>: the mixture real Excel output has.
    fn xlsx_uret(headers: &[&str], lines: &[Vec<&str>]) -> Vec<u8> {
        let mut sheet = String::from("<?xml version=\"1.0\"?><worksheet><sheetData>");
        sheet.push_str("<row r=\"1\">");
        for (k, b) in headers.iter().enumerate() {
            sheet.push_str(&format!(
                "<c r=\"{}1\" t=\"inlineStr\"><is><t>{b}</t></is></c>",
                column_letter(k)
            ));
        }
        sheet.push_str("</row>");
        for (i, line) in lines.iter().enumerate() {
            sheet.push_str(&format!("<row r=\"{}\">", i + 2));
            for (k, h) in line.iter().enumerate() {
                sheet.push_str(&format!(
                    "<c r=\"{}{}\"><v>{h}</v></c>",
                    column_letter(k),
                    i + 2
                ));
            }
            sheet.push_str("</row>");
        }
        sheet.push_str("</sheetData></worksheet>");
        pack(&[ZipEntry::new(
            "xl/worksheets/sheet1.xml",
            sheet.into_bytes(),
        )])
        .unwrap()
    }

    fn column_letter(i: usize) -> char {
        (b'A' + i as u8) as char
    }

    #[test]
    fn shared_strings_join_rich_text() {
        let xml = "<sst><si><t>Name</t></si><si><r><t>Sur</t></r><r><t>name</t></r></si>\
                   <si><t>Ag&amp;e</t></si></sst>";
        assert_eq!(parse_shared_strings(xml), vec!["Name", "Surname", "Ag&e"]);
    }

    #[test]
    fn an_empty_cell_does_not_shift_the_columns() {
        // Column B is missing: without r="C2" the "30" would shift into B.
        let xml = "<sheetData><row r=\"1\"><c r=\"A1\"><v>a</v></c><c r=\"B1\"><v>b</v></c>\
                   <c r=\"C1\"><v>c</v></c></row>\
                   <row r=\"2\"><c r=\"A2\"><v>x</v></c><c r=\"C2\"><v>30</v></c></row>\
                   </sheetData>";
        let lines = parse_sheet(xml, &[]).rows;
        assert_eq!(lines[1], vec!["x", "", "30"]);
    }

    /// The out-of-range index sits in the MIDDLE column. It used to be the last
    /// one, and a trailing empty cell is no longer stored (audit M6: trailing
    /// padding is what the XFD bomb was made of), so there it would prove
    /// nothing — in the middle, "empty and not panicking" is still visible.
    #[test]
    fn the_shared_index_and_entities_are_resolved() {
        let shared = vec!["Company & Co".to_string(), "Second".to_string()];
        let xml = "<sheetData><row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c>\
                   <c r=\"B1\" t=\"s\"><v>99</v></c><c r=\"C1\" t=\"s\"><v>1</v></c></row></sheetData>";
        let lines = parse_sheet(xml, &shared).rows;
        // An out-of-range index falls to an empty cell rather than panicking.
        assert_eq!(lines[0], vec!["Company & Co", "", "Second"]);
    }

    /// Everything a sheet turned into: the cells and the bytes of their text.
    fn cells_and_text(sheet: &Sheet) -> (usize, usize) {
        let cells = sheet.rows.iter().map(Vec::len).sum();
        let text = sheet.rows.iter().flatten().map(String::len).sum();
        (cells, text)
    }

    /// THE SHEET THE AUDIT MEASURED (M6): a header and 64 rows, each one an
    /// empty cell in column XFD — 1,582 bytes of XML. Padding every row out to
    /// its cell made that 1,048,576 `String`s and 3 MiB of markdown.
    #[test]
    fn an_empty_cell_in_column_xfd_allocates_nothing() {
        let mut xml = String::from("<worksheet><sheetData>");
        for _ in 0..65 {
            xml.push_str("<row><c r=\"XFD1\"/></row>");
        }
        xml.push_str("</sheetData></worksheet>");
        assert!(
            xml.len() < 1_700,
            "the fixture is the small one: {}",
            xml.len()
        );

        let sheet = parse_sheet(&xml, &[]);
        let (cells, _) = cells_and_text(&sheet);
        assert!(
            cells < 100,
            "{cells} cells were allocated for a sheet with no data in it"
        );
        let workbook =
            pack(&[ZipEntry::new("xl/worksheets/sheet1.xml", xml.into_bytes())]).unwrap();
        let table = parse_xlsx(&workbook).unwrap();
        assert!(
            table.markdown_truncated(usize::MAX).len() < 1_000,
            "the markdown is padded out to column XFD"
        );
    }

    /// …and when the XFD cells DO hold data the width is real, so the budget is
    /// what stops it: no more than CELL_BUDGET cells, and the rows left out are
    /// counted, not silently lost.
    #[test]
    fn a_sheet_wider_than_the_budget_stops_at_the_budget_and_says_so() {
        let mut xml = String::from("<worksheet><sheetData>");
        for i in 1..=65 {
            xml.push_str(&format!(
                "<row><c r=\"A{i}\"><v>{i}</v></c><c r=\"XFD{i}\"><v>x</v></c></row>"
            ));
        }
        xml.push_str("</sheetData></worksheet>");

        let sheet = parse_sheet(&xml, &[]);
        let (cells, _) = cells_and_text(&sheet);
        assert!(cells <= CELL_BUDGET, "{cells} cells, budget {CELL_BUDGET}");
        assert!(
            sheet.rows_not_read > 0,
            "the rows past the budget were not counted"
        );
        assert_eq!(sheet.rows.len() + sheet.rows_not_read, 65);
        assert_eq!(
            sheet.rows[1][0], "2",
            "the rows that fit are read as they are"
        );

        // The model is TOLD: a quiet truncation would read as the whole sheet.
        let dir = temp_dir("budget");
        let bytes = pack(&[ZipEntry::new("xl/worksheets/sheet1.xml", xml.into_bytes())]).unwrap();
        std::fs::write(dir.join("wide.xlsx"), bytes).unwrap();
        let mut ctx = context(&dir);
        let outcome = no_futures(
            ReadDocumentTool::new().run(serde_json::json!({ "path": "wide.xlsx" }), &mut ctx),
        );
        assert_eq!(outcome.state, ToolState::Read, "{}", outcome.to_model);
        assert!(
            outcome
                .to_model
                .contains(&format!("{} more rows were not read", sheet.rows_not_read)),
            "{}",
            outcome.to_model
        );
    }

    /// The second amplifier: every `t="s"` reference clones the shared string.
    #[test]
    fn shared_string_references_cannot_multiply_past_the_text_budget() {
        let shared = vec!["y".repeat(1024)];
        let mut xml = String::from("<sheetData>");
        for i in 1..=100 {
            xml.push_str(&format!("<row><c r=\"A{i}\" t=\"s\"><v>0</v></c></row>"));
        }
        xml.push_str("</sheetData>");
        let budget = Budget {
            cells: CELL_BUDGET,
            text: 10 * 1024,
        };
        let sheet = parse_sheet_within(&xml, &shared, budget);
        let (_, text) = cells_and_text(&sheet);
        assert!(
            text <= budget.text,
            "{text} bytes of text, budget {}",
            budget.text
        );
        assert_eq!(sheet.rows.len(), 10);
        assert_eq!(sheet.rows_not_read, 90);
    }

    /// THE QUADRATIC ENTITY SCAN (audit M7). Each `&` searched the whole rest
    /// of the text for a `;`, and only afterwards checked it was within 12
    /// bytes. Measured before the fix: 512 KiB of `&` took 3.4 s (audit), and
    /// 4.1 s in this test's debug build with the fix reverted (10 Oct 2026,
    /// Apple Silicon). Fixed, the whole module's 16 tests finish in 0.07 s.
    /// The bound is generous so a loaded machine does not flake it — the gap
    /// between the two behaviours is two orders of magnitude.
    #[test]
    fn a_run_of_ampersands_resolves_in_linear_time() {
        let raw = "&".repeat(512 * 1024);
        let started = std::time::Instant::now();
        let out = resolve_entity(&raw);
        let took = started.elapsed();
        assert_eq!(out.len(), raw.len(), "a bare & is kept as it is");
        assert!(
            took < std::time::Duration::from_millis(500),
            "512 KiB of '&' took {took:?}"
        );
        // The bound did not cost the real entities.
        assert_eq!(resolve_entity("a &amp; b &#x41;&#66; &lt;"), "a & b AB <");
        assert_eq!(
            resolve_entity("&unknown; &#1114111111;"),
            "&unknown; &#1114111111;"
        );
    }

    /// A three-sheet workbook as Excel packs it: names in `workbook.xml`,
    /// parts through the rels, and the first sheet NOT in `sheet1.xml`.
    fn three_sheet_workbook() -> Vec<u8> {
        let workbook = "<workbook xmlns:r=\"r\"><sheets>\
             <sheet name=\"Sales &amp; Costs\" sheetId=\"3\" r:id=\"rId3\"/>\
             <sheet name=\"Notes\" sheetId=\"1\" r:id=\"rId1\"/>\
             <sheet name=\"Archive\" sheetId=\"2\" r:id=\"rId2\"/>\
             </sheets></workbook>";
        let rels = "<Relationships>\
             <Relationship Id=\"rId1\" Type=\"http://x/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
             <Relationship Id=\"rId2\" Type=\"http://x/relationships/worksheet\" Target=\"/xl/worksheets/sheet2.xml\"/>\
             <Relationship Id=\"rId3\" Type=\"http://x/relationships/worksheet\" Target=\"worksheets/sheet3.xml\"/>\
             </Relationships>";
        let sheet = |a: &str, b: &str| {
            format!(
                "<worksheet><sheetData><row><c r=\"A1\" t=\"inlineStr\"><is><t>{a}</t></is></c></row>\
                 <row><c r=\"A2\" t=\"inlineStr\"><is><t>{b}</t></is></c></row></sheetData></worksheet>"
            )
        };
        pack(&[
            ZipEntry::new("xl/workbook.xml", workbook.as_bytes()),
            ZipEntry::new("xl/_rels/workbook.xml.rels", rels.as_bytes()),
            ZipEntry::new("xl/worksheets/sheet1.xml", sheet("Note", "n1").into_bytes()),
            ZipEntry::new("xl/worksheets/sheet2.xml", sheet("Old", "o1").into_bytes()),
            ZipEntry::new("xl/worksheets/sheet3.xml", sheet("Item", "i1").into_bytes()),
        ])
        .unwrap()
    }

    /// AUDIT L17: a workbook is not silently one sheet. Every name is reported,
    /// and the sheet read is the one the WORKBOOK puts first.
    #[test]
    fn every_sheet_name_is_reported_and_the_first_sheet_is_read() {
        let book = parse_workbook(&three_sheet_workbook()).unwrap();
        assert_eq!(book.sheets, vec!["Sales & Costs", "Notes", "Archive"]);
        assert_eq!(book.shown, Some(0));
        assert_eq!(
            book.table.headers,
            vec!["Item"],
            "the workbook's first sheet is read"
        );
        assert_eq!(book.unread_sheets(), vec!["Notes", "Archive"]);

        let dir = temp_dir("sheets");
        std::fs::write(dir.join("book.xlsx"), three_sheet_workbook()).unwrap();
        let mut ctx = context(&dir);
        let outcome = no_futures(
            ReadDocumentTool::new().run(serde_json::json!({ "path": "book.xlsx" }), &mut ctx),
        );
        let m = &outcome.to_model;
        assert!(m.starts_with("| Item |"), "the table still leads: {m}");
        for name in ["Sales & Costs", "Notes", "Archive"] {
            assert!(m.contains(name), "sheet {name} is not named: {m}");
        }
        assert!(m.contains("only one sheet is read"), "{m}");
        assert!(
            outcome.chip_text.contains("sheet 1 of 3"),
            "{}",
            outcome.chip_text
        );

        // A one-sheet workbook — what `create_document` writes — says nothing extra.
        let single = parse_workbook(&xlsx_uret(&["A"], &[vec!["1"]])).unwrap();
        assert!(single.other_sheets_note().is_none());
    }

    #[test]
    fn the_attribute_and_the_column_index_are_read_correctly() {
        assert_eq!(attribute("r=\"C2\" s=\"1\" t=\"s\"", "t"), Some("s"));
        assert_eq!(attribute("r='A1'", "r"), Some("A1"));
        assert_eq!(attribute("r=\"A1\"", "t"), None);
        assert_eq!(column_index("A1"), Some(0));
        assert_eq!(column_index("Z9"), Some(25));
        assert_eq!(column_index("AA1"), Some(26));
        assert_eq!(column_index("1"), None);
        assert_eq!(column_index("ZZZZZ1"), None);
    }

    #[test]
    fn the_xlsx_is_read_and_valid_markdown_is_returned_to_the_model() {
        let dir = temp_dir("xlsx");
        let byte = xlsx_uret(
            &["Month", "Revenue"],
            &[vec!["January", "100"], vec!["February", "200"]],
        );
        std::fs::write(dir.join("report.xlsx"), byte).unwrap();

        let tool = ReadDocumentTool::new();
        let mut ctx = context(&dir);
        let outcome = no_futures(tool.run(serde_json::json!({ "path": "report.xlsx" }), &mut ctx));

        assert_eq!(outcome.state, ToolState::Read);
        let m = &outcome.to_model;
        assert!(
            m.starts_with("| Month | Revenue |"),
            "no markdown header: {m}"
        );
        assert!(m.contains("| --- | --- |"), "no alignment row: {m}");
        assert!(m.contains("| January | 100 |"), "no data row: {m}");
        // Every row must carry the full column count — invalid markdown breaks the model.
        for line in m.lines().filter(|s| s.starts_with('|')) {
            assert_eq!(
                line.matches('|').count(),
                3,
                "the column count is broken: {line}"
            );
        }
        assert!(
            ctx.session_tainted(),
            "a document was read, the session must be tainted"
        );
    }

    #[test]
    fn a_large_table_goes_to_the_store_and_a_source_ref_is_returned() {
        let dir = temp_dir("large");
        let lines: Vec<Vec<String>> = (0..80)
            .map(|i| vec![format!("s{i}"), format!("{}", i * 3)])
            .collect();
        let view: Vec<Vec<&str>> = lines
            .iter()
            .map(|s| s.iter().map(String::as_str).collect())
            .collect();
        std::fs::write(dir.join("large.xlsx"), xlsx_uret(&["Name", "Value"], &view)).unwrap();

        let store = Arc::new(SharedStore::new());
        let tool = ReadDocumentTool::with_store(store.clone());
        let mut ctx = context(&dir);
        let outcome = no_futures(tool.run(serde_json::json!({ "path": "large.xlsx" }), &mut ctx));

        let m = &outcome.to_model;
        assert!(m.contains("source_ref="), "no reference returned: {m}");
        // Truncated but still a table: the model must be able to see the format.
        assert!(m.starts_with("| Name | Value |"));
        assert!(m.contains("more rows not shown"));

        let source = m
            .split("source_ref=")
            .nth(1)
            .unwrap()
            .trim_end_matches(')')
            .trim()
            .to_string();
        let Some(StoredValue::Table(t)) = store.value(&SourceRef(source)) else {
            panic!("no table in the store");
        };
        assert_eq!(t.row_count(), 80, "the raw data must be stored in full");
        // The chip detail is not truncated.
        assert!(outcome.raw_output.unwrap().contains("| s79 | 237 |"));
    }

    #[test]
    fn plain_text_and_markdown_are_read() {
        let dir = temp_dir("text");
        std::fs::write(dir.join("note.md"), "# Heading\n\nContent goes here.\n").unwrap();

        let tool = ReadDocumentTool::new();
        let mut ctx = context(&dir);
        let outcome = no_futures(tool.run(serde_json::json!({ "path": "note.md" }), &mut ctx));

        assert_eq!(outcome.state, ToolState::Read);
        assert!(outcome.to_model.contains("# Heading"));
        assert!(outcome.to_model.contains("Content goes here."));
        assert!(
            !outcome.to_model.contains("source_ref"),
            "small text must not go into the store"
        );
        assert_eq!(outcome.file_path.unwrap().file_name().unwrap(), "note.md");
    }

    #[test]
    fn long_text_goes_to_the_store() {
        let dir = temp_dir("longtext");
        let content: String = (0..400).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join("journal.txt"), &content).unwrap();

        let store = Arc::new(SharedStore::new());
        let tool = ReadDocumentTool::with_store(store.clone());
        let mut ctx = context(&dir);
        let outcome = no_futures(tool.run(serde_json::json!({ "path": "journal.txt" }), &mut ctx));

        assert!(outcome.to_model.contains("source_ref="));
        assert!(outcome.to_model.len() < MODEL_CAP + 120);
        // Truncation happens at a line boundary: the last line must not be left half.
        assert!(outcome.to_model.contains("(the rest was truncated)"));
        assert_eq!(outcome.raw_output.unwrap().lines().count(), 400);
    }

    #[test]
    fn the_errors_pass_through_a_single_gate() {
        let dir = temp_dir("error");
        std::fs::write(dir.join("image.png"), b"\x89PNG\r\n").unwrap();
        let tool = ReadDocumentTool::new();
        let mut ctx = context(&dir);

        // A file that does not exist.
        let none = no_futures(tool.run(serde_json::json!({ "path": "missing.md" }), &mut ctx));
        assert!(matches!(none.state, ToolState::Failed(_)));
        assert_eq!(none.to_model, ERROR_MODEL_TEXT);

        // An unsupported format.
        let png = no_futures(tool.run(serde_json::json!({ "path": "image.png" }), &mut ctx));
        assert_eq!(png.to_model, ERROR_MODEL_TEXT);

        // A missing field.
        let empty = no_futures(tool.run(serde_json::json!({}), &mut ctx));
        assert_eq!(empty.to_model, ERROR_MODEL_TEXT);

        // A sandbox escape.
        let escape =
            no_futures(tool.run(serde_json::json!({ "path": "../../etc/hosts" }), &mut ctx));
        assert_eq!(escape.to_model, ERROR_MODEL_TEXT);

        // No failing path may taint the session.
        assert!(!ctx.session_tainted());
    }

    /// A SYMLINK PLANTED INSIDE THE SANDBOX IS NOT A DOOR OUT OF IT.
    ///
    /// `run_code` may legitimately write into the sandbox, and creating a
    /// symlink IS a write — so a poisoned prompt can make the model plant
    /// `gate -> /Users/<victim>` and then ask for
    /// `gate/Library/.../credentials.json`. `resolve_path` is only lexical, so
    /// `gate` looked like an ordinary component and `metadata()`/`read()`
    /// followed it: measured before the fix, the model was handed
    /// `{"api_key":"SK-TOPSECRET"}` verbatim. Once in the 4096-token window that
    /// data can leave the device on the next web/mcp call.
    ///
    /// BOTH SHAPES ARE MEASURED: an INTERMEDIATE directory link (the one a
    /// leaf-only `symlink_metadata` check would miss, because the leaf really is
    /// a file) and a direct file link.
    #[cfg(unix)]
    #[test]
    fn a_planted_symlink_cannot_read_outside_the_sandbox() {
        let dir = temp_dir("symlink");
        let outside = temp_dir("symlink-victim");
        std::fs::create_dir_all(outside.join("Library")).expect("victim tree");
        std::fs::write(
            outside.join("Library").join("credentials.json"),
            "{\"api_key\":\"SK-TOPSECRET\"}",
        )
        .expect("victim file");
        std::fs::write(outside.join("id.txt"), "PRIVATE KEY MATERIAL").expect("victim file");

        std::os::unix::fs::symlink(&outside, dir.join("gate")).expect("plant the directory link");
        std::os::unix::fs::symlink(outside.join("id.txt"), dir.join("notes.txt"))
            .expect("plant the file link");

        let tool = ReadDocumentTool::new();
        let mut ctx = context(&dir);

        for path in ["gate/Library/credentials.json", "notes.txt"] {
            let outcome = no_futures(tool.run(serde_json::json!({ "path": path }), &mut ctx));
            assert!(
                matches!(outcome.state, ToolState::Failed(_)),
                "the link was followed for {path}: {}",
                outcome.to_model
            );
            assert_eq!(outcome.to_model, ERROR_MODEL_TEXT);
            assert!(
                !outcome.to_model.contains("SK-TOPSECRET")
                    && !outcome.to_model.contains("PRIVATE KEY"),
                "the secret reached the model: {}",
                outcome.to_model
            );
        }
        // A refused read touched nothing; it must not tighten the approval gate.
        assert!(!ctx.session_tainted());

        // REGRESSION GUARD: an ordinary file in the sandbox still reads fine.
        std::fs::write(dir.join("plain.md"), "hello").expect("write");
        let fine = no_futures(tool.run(serde_json::json!({ "path": "plain.md" }), &mut ctx));
        assert_eq!(fine.state, ToolState::Read, "{}", fine.to_model);

        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn the_schema_forces_the_model_into_the_right_call() {
        let schema = ReadDocumentTool::new().schema();
        let fields = schema.fields();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "path");
        assert!(fields[0].required, "path must be required");
        assert_eq!(fields[1].name, "focus");
        assert!(!fields[1].required);
        assert!(
            schema
                .validate(&serde_json::json!({ "path": "a.md" }))
                .is_ok()
        );
        assert!(
            schema
                .validate(&serde_json::json!({ "focus": "revenue" }))
                .is_err()
        );
        assert!(ReadDocumentTool::new().taints_session());
    }
}
