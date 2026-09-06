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
            let table = parse_xlsx(&byte)?;
            Ok(self.table_outcome(ctx, &name, table).file_path(path))
        } else if TEXT_EXTENSIONS.contains(&extension.as_str()) {
            let text = to_text(&byte)?;
            Ok(self.text_outcome(ctx, &name, &text).file_path(path))
        } else {
            Err(ToolError::InvalidArgument(format!(
                "unsupported document format: .{extension}"
            )))
        }
    }

    fn table_outcome(&self, ctx: &ToolContext, name: &str, table: Table) -> ToolOutcome {
        if table.column_count() == 0 {
            return ToolOutcome::read_ok(
                format!("{name} read, the table is empty"),
                "document_empty (the spreadsheet has no rows)",
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
                "{name} read ({} rows x {} columns)",
                table.row_count(),
                table.column_count()
            ),
            format!("{body}{suffix}"),
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

/// Converts .xlsx bytes to a `Table`. The first row is taken as the header.
pub fn parse_xlsx(byte: &[u8]) -> ToolResult<Table> {
    let map = tacet_zip::open_map(byte).map_err(|h| ToolError::Other(h.to_string()))?;

    // sheet1.xml is the common name but not mandatory; if it is not found the
    // alphabetically first sheet is chosen (the choice is deterministic because BTreeMap is ordered).
    let sheet = map
        .get("xl/worksheets/sheet1.xml")
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

    let lines = parse_sheet(&String::from_utf8_lossy(sheet), &shared);
    let mut it = lines.into_iter();
    let Some(headers) = it.next() else {
        return Ok(Table::default());
    };
    Ok(Table::new(headers, it))
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
fn chunk(xml: &str) -> Vec<Chunk<'_>> {
    let mut parts = Vec::new();
    let mut i = 0usize;
    while i < xml.len() {
        if xml.as_bytes()[i] == b'<' {
            let Some(length) = xml[i..].find('>') else {
                break;
            };
            let inner = &xml[i + 1..i + length];
            i += length + 1;
            // A declaration (<?xml?>), a comment and a DOCTYPE do not concern us.
            if inner.starts_with('?') || inner.starts_with('!') {
                continue;
            }
            if let Some(name) = inner.strip_prefix('/') {
                parts.push(Chunk::Close(local_name(name.trim())));
            } else {
                let self_closing = inner.ends_with('/');
                let body = inner.trim_end_matches('/').trim();
                let (name, attributes) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
                parts.push(Chunk::Open {
                    name: local_name(name),
                    attributes,
                    self_closing,
                });
            }
        } else {
            let last = xml[i..].find('<').map(|p| i + p).unwrap_or(xml.len());
            parts.push(Chunk::Text(&xml[i..last]));
            i = last;
        }
    }
    parts
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

/// The row/cell values inside `sheet*.xml`.
fn parse_sheet(xml: &str, shared: &[String]) -> Vec<Vec<String>> {
    let mut lines: Vec<Vec<String>> = Vec::new();
    let mut active: Vec<String> = Vec::new();
    let mut body = String::new();
    let mut collecting = false;
    let mut is_shared = false;
    let mut column: Option<usize> = None;

    for part in chunk(xml) {
        match part {
            Chunk::Open { name: "row", .. } => active = Vec::new(),
            Chunk::Open {
                name: "c",
                attributes,
                self_closing,
            } => {
                is_shared = attribute(attributes, "t") == Some("s");
                column = attribute(attributes, "r").and_then(column_index);
                body.clear();
                if self_closing {
                    place_cell(&mut active, column, String::new());
                }
            }
            Chunk::Open {
                name: "t" | "v", ..
            } => collecting = true,
            Chunk::Text(m) if collecting => body.push_str(&resolve_entity(m)),
            Chunk::Close("t" | "v") => collecting = false,
            Chunk::Close("c") => {
                let value = if is_shared {
                    // A malformed index leaves the cell empty rather than
                    // panicking: the input may be a file somebody else produced.
                    body.trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| shared.get(i).cloned())
                        .unwrap_or_default()
                } else {
                    body.clone()
                };
                place_cell(&mut active, column, value);
                body.clear();
            }
            Chunk::Close("row") => lines.push(std::mem::take(&mut active)),
            _ => {}
        }
    }
    lines
}

/// Puts the cell in ITS OWN column.
///
/// The Swift version added the cells in order; since Excel does not write a
/// `<c>` for an empty cell, after one gap the whole row shifted one column
/// left. With the `r="C2"` reference available there is no reason to repeat that bug.
fn place_cell(line: &mut Vec<String>, column: Option<usize>, value: String) {
    match column {
        Some(i) => {
            while line.len() < i {
                line.push(String::new());
            }
            if line.len() == i {
                line.push(value);
            } else {
                line[i] = value;
            }
        }
        None => line.push(value),
    }
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
        let Some(last) = body.find(';').filter(|s| *s <= 12) else {
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
        let lines = parse_sheet(xml, &[]);
        assert_eq!(lines[1], vec!["x", "", "30"]);
    }

    #[test]
    fn the_shared_index_and_entities_are_resolved() {
        let shared = vec!["Company & Co".to_string(), "Second".to_string()];
        let xml = "<sheetData><row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c>\
                   <c r=\"B1\" t=\"s\"><v>1</v></c><c r=\"C1\" t=\"s\"><v>99</v></c></row></sheetData>";
        let lines = parse_sheet(xml, &shared);
        // An out-of-range index falls to an empty cell rather than panicking.
        assert_eq!(lines[0], vec!["Company & Co", "Second", ""]);
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
