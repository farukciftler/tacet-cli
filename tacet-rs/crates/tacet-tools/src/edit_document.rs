//! `edit_document` — rewrites an existing document as a new version.
//!
//! THE FLOW: the model first gets the content with `read_document`, then gives
//! the FULL edited content to this tool. The original is KEPT; a new file named
//! "<name> (edited)" is produced. A write → a green chip.
//!
//! A SWIFT LESSON — THE WORKABLE DOCUMENT (easy to skip, learned the expensive
//! way): the tool looks not at the "document ATTACHED to the chat" but at the
//! "document that can be WORKED ON". In Swift this was
//! `workableDocument = attached ?? lastProduced`. Looking only at the attached
//! one made the "make me an excel" → "now show that as a table" flow say "no
//! document is attached" on the second step: the user meant the file just
//! produced, and the tool could not see it.
//! THE RUST COUNTERPART — WHY NO FIELD WAS ADDED TO `ToolContext`: `ToolContext`
//! THE RUST COUNTERPART — WHY NO FIELD WAS ADDED TO `ToolContext`:
//! `ToolContext` lives in `tacet-core` and core is SINGLE-OWNER; besides, "the
//! last document" is not a contract concept but an APPLICATION intuition — put
//! into core, every tool would have to carry it. Instead there is a three-tier
//! resolution (see `workable_document`): an explicit `path` argument → the
//! session watcher (`DocumentWatcher`) → the MOST RECENTLY modified document in
//! the output folder. The third tier runs the Swift flow TODAY even if the
//! shell binds nothing; once the watcher is attached, fact replaces guess.
//! THE FORMAT IS NOT CONVERTED. The silent lie measured in Swift: the user says
//! THE FORMAT IS NOT CONVERTED. The silent lie measured in Swift: the user says
//! "convert this to word", the tool rewrites the .xlsx as an .xlsx, and the
//! model says "converted to Word". The fix is NOT to repeat the rule in the
//! prompt but to report the fact to the model: the returned text says plainly
//! that the format DID NOT CHANGE, and the model faces the lie in its own output.
//!
//! A SINGLE WRITE GATE: the write goes through the free function
//! `create_document::write_document`. The trait IS NOT OVERRIDDEN — preparing the
//! folder, the unique name and the 0600 permission stamp are shared steps this
//! tool cannot skip either.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tacet_core::{
    Field, Tool, ToolContext, ToolState, ToolFuture, ToolError, ToolResult, ToolOutcome, ArgSchema,
    TraceUpdate, SourceRef, boxed,
};

use crate::read_document::parse_xlsx;
use crate::create_document::{
    DocumentFormat, DocumentEngine, ExcelEngine, TextEngine, write_document, output_folder, markdown_dan,
};
use crate::data_store::Table;

/// The name suffix of the edited file. The original is NEVER OVERWRITTEN:
/// changing the data in the user's hands irreversibly means permanent loss on a
/// single request the model misunderstood.
const EDITED_SUFFIX: &str = " (edited)";

/// The upper bound of an editable file (32 MiB) — the same cap as `read_document`.
const FILE_CAP: u64 = 32 * 1024 * 1024;

/// The extensions looked at when searching for the "last document" in the output folder.
const DOCUMENT_EXTENSIONS: &[&str] = &["xlsx", "md", "markdown", "txt", "text"];

// ---------------------------------------------------------------------------
// The session watcher
// ---------------------------------------------------------------------------

/// Holds the document most recently produced/read in the session — the
/// counterpart of Swift's `lastProduced`.
///
/// WHY A SEPARATE TYPE: `ToolOutcome::file_path` is already filled on every
/// successful document operation, so the "last document" is NOT INVENTED, it is
/// COLLECTED from the flowing outcomes. When the shell calls `record(&outcome)`
/// at the end of every turn the watcher fills itself; no tool has to know another one's internal state.
///
/// If it is not attached the tool falls back to a guess (file time) and still
/// works — that is deliberate: killing the whole editing flow because of an
/// unattached watcher would be the costliest form of a silent integration bug.
#[derive(Default)]
pub struct DocumentWatcher {
    last: Mutex<Option<PathBuf>>,
}

impl DocumentWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Learns the last document from a tool outcome. Only SUCCESSFUL outcomes
    /// that really produce/read a file count: the path carried by a failed call
    /// would present a non-existent file as "workable".
    pub fn save(&self, outcome: &ToolOutcome) {
        if matches!(outcome.state, ToolState::Failed(_)) {
            return;
        }
        if let Some(path) = &outcome.file_path
            && path.is_file()
        {
            *self.last.lock().expect("document watcher lock") = Some(path.clone());
        }
    }

    /// A direct notification — for tests and for paths the shell binds by hand.
    pub fn assign(&self, path: impl Into<PathBuf>) {
        *self.last.lock().expect("document watcher lock") = Some(path.into());
    }

    pub fn last(&self) -> Option<PathBuf> {
        self.last.lock().expect("document watcher lock").clone()
    }

    /// Called on a chat reset — whatever is session scoped ends here.
    pub fn clear(&self) {
        *self.last.lock().expect("document watcher lock") = None;
    }
}

// ---------------------------------------------------------------------------
// Workable document resolution
// ---------------------------------------------------------------------------

/// Where the document to work on came from — visible on the chip and in
/// diagnostics, and the tests verify which tier won.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentSource {
    /// The model gave `path` explicitly.
    Argument,
    /// The session watcher knew it.
    Watcher,
    /// The most recently modified document in the output folder (a guess).
    LastModified,
}

/// The three-tier resolution. THE ORDER MATTERS: an explicit request always beats a guess.
fn workable_document(
    ctx: &ToolContext,
    watcher: Option<&DocumentWatcher>,
    explicit_path: Option<&str>,
) -> ToolResult<(PathBuf, DocumentSource)> {
    // 1) The explicit path — it passes through the sandbox gate.
    if let Some(raw) = explicit_path {
        let path = ctx.resolve_path(raw)?;
        if !path.is_file() {
            return Err(ToolError::FileNotFound(path));
        }
        return Ok((path, DocumentSource::Argument));
    }

    // 2) The session watcher. The watcher's path is ALSO PUT through the gate:
    // if a path outside the sandbox somehow landed in the watcher, this is the last defence.
    if let Some(path) = watcher.and_then(DocumentWatcher::last)
        && let Ok(resolved) = ctx.resolve_path(&path)
        && resolved.is_file()
    {
        return Ok((resolved, DocumentSource::Watcher));
    }

    // 3) The most recently modified document in the output folder (which is now
    // the working directory ITSELF). Because create_document also writes to the
    // root, the "edit the document just produced" flow scans the same root (see `output_folder`).
    let folder = output_folder(ctx)?;
    if let Some(path) = last_modified_document(&folder) {
        return Ok((path, DocumentSource::LastModified));
    }

    Err(ToolError::FileNotFound(folder))
}

/// Finds the most recently modified document in the folder.
///
/// Ties are broken BY NAME: when two files are written in the same millisecond
/// the directory traversal order (that is, randomness) must not decide — the
/// same session run twice must pick the same file, otherwise eval is not comparable.
fn last_modified_document(folder: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(folder).ok()?.flatten() {
        let path = entry.path();
        let Ok(upper) = path.symlink_metadata() else { continue };
        if !upper.is_file() || upper.file_type().is_symlink() {
            continue;
        }
        let extension = path
            .extension()
            .map(|u| u.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !DOCUMENT_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        let Ok(time) = upper.modified() else { continue };
        best = match best {
            Some((z, y)) if (z, &y) >= (time, &path) => Some((z, y)),
            _ => Some((time, path)),
        };
    }
    best.map(|(_, y)| y)
}

/// The format from the file extension. An unknown extension cannot be edited:
/// saying "I edited it" while pouring the content into another format is silent data loss.
fn from_format(path: &Path) -> Option<DocumentFormat> {
    match path.extension()?.to_string_lossy().to_ascii_lowercase().as_str() {
        "xlsx" => Some(DocumentFormat::Excel),
        "md" | "markdown" => Some(DocumentFormat::Markdown),
        "txt" | "text" => Some(DocumentFormat::Text),
        _ => None,
    }
}

fn engine(format: DocumentFormat) -> Box<dyn DocumentEngine> {
    match format {
        DocumentFormat::Excel => Box::new(ExcelEngine::new()),
        b => Box::new(TextEngine::new(b)),
    }
}

/// The stem of the edited file. If it already carries "(edited)" the suffix is
/// NOT ADDED AGAIN — three edits in a row would produce
/// "report (edited) (edited) (edited).xlsx".
fn edited_name(path: &Path) -> String {
    let base = path
        .file_stem()
        .map(|a| a.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".to_string());
    if base.ends_with(EDITED_SUFFIX) {
        base
    } else {
        format!("{base}{EDITED_SUFFIX}")
    }
}

// ---------------------------------------------------------------------------
// The tool
// ---------------------------------------------------------------------------

/// `edit_document` — rewrites the workable document as a new version.
#[derive(Default)]
pub struct EditDocumentTool {
    watcher: Option<std::sync::Arc<DocumentWatcher>>,
}

impl EditDocumentTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds it with the session watcher — the shell should prefer this;
    /// building without a watcher falls back to the guess tier.
    pub fn with_watcher(watcher: std::sync::Arc<DocumentWatcher>) -> Self {
        Self { watcher: Some(watcher) }
    }
}

impl Tool for EditDocumentTool {
    fn name(&self) -> &str {
        "edit_document"
    }

    fn description(&self) -> &str {
        "Edits the document in play — the file you just created or read — by writing a new \
         version of it. Call this when the user asks to change it ('add this row', 'delete \
         that line', 'change the title', in any language). First call read_document to get \
         the content, then pass the FULL edited content as 'new_content' (markdown; a \
         markdown table for Excel files). It never converts format: to change format call \
         create_document with the new format."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new(
                "new_content",
                ArgSchema::text().description(
                    "The FULL edited content as markdown — the WHOLE document, not just the \
                     changed part; anything you leave out is deleted. For an Excel document \
                     write a markdown table (| ... |).",
                ),
            ),
            // `new_content` IS NOT REQUIRED IN THE SCHEMA but IS REQUIRED in the
            // tool body (ONE OF THE TWO must arrive). Rationale: the grammar
            // forces required fields into production, so had it been marked
            // `required()` the model would HAVE TO write the body on every call
            // and the `source_ref` path — the bypass channel itself — would
            // become unusable. The same decision was made for the
            // `content`/`source_ref` pair in `create_document`; the two tools
            Field::new("title", ArgSchema::text().description("Optional new title.")),
            Field::new(
                "path",
                ArgSchema::text().description(
                    "Optional path of the document to edit, relative to the working directory. \
                     Leave empty to edit the document currently in play.",
                ),
            ),
            Field::new(
                "source_ref",
                ArgSchema::text().description(
                    "Data reference returned by another tool. If given, the new content is \
                     pulled from the store — leave 'new_content' empty.",
                ),
            ),
        ])
        .description("Rewrite the document in play with edited content")
    }

    /// The user's document is read and its new version written to disk: the session becomes tainted.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a mut ToolContext,
    ) -> ToolFuture<'a> {
        boxed(async move {
            if let Err(h) = self.schema().validate(&args) {
                return ToolOutcome::failed(&h);
            }
            let trace = ctx.start_chip("doc", "Editing document…");

            let (outcome, written) = match self.edit(&args, ctx) {
                Ok(s) => {
                    let written = s.state == ToolState::Written;
                    (s, written)
                }
                Err(h) => (ToolOutcome::failed(&h), false),
            };

            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(args.to_string())
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );

            if written {
                ctx.taint();
                // The new version is now the WORKABLE document: successive "add
                // one more row" requests must all build on the latest version.
                if let Some(trace) = &self.watcher {
                    trace.save(&outcome);
                }
            }
            outcome
        })
    }
}

impl EditDocumentTool {
    /// The synchronous body — the error path is collected in a single place.
    fn edit(
        &self,
        args: &serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult<ToolOutcome> {
        let text_field = |name: &str| -> Option<String> {
            args.get(name)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };

        // --- The workable document ---
        let explicit_path = text_field("path");
        let resolution = workable_document(ctx, self.watcher.as_deref(), explicit_path.as_deref());
        let (path, source) = match resolution {
            Ok(v) => v,
            Err(ToolError::FileNotFound(_)) if explicit_path.is_none() => {
                // NO DOCUMENT is NOT AN ERROR, it is a FACT. Had we turned it
                // into the fixed `tool_failed` text the model could not tell
                // "the tool broke" from "there is no document" and would tell the user the wrong thing.
                return Ok(ToolOutcome::read_ok(
                    "There is no document to edit",
                    "no_document_in_play (ask the user which file to edit, or create one first)",
                ));
            }
            Err(h) => return Err(h),
        };

        let Some(format) = from_format(&path) else {
            return Err(ToolError::InvalidArgument(format!(
                "format that cannot be edited: {}",
                path.extension().map(|u| u.to_string_lossy().into_owned()).unwrap_or_default()
            )));
        };

        let upper = path.metadata().map_err(|_| ToolError::FileNotFound(path.clone()))?;
        if upper.len() > FILE_CAP {
            return Err(ToolError::Other(format!(
                "document too large ({} bytes), cap {FILE_CAP}",
                upper.len()
            )));
        }

        // --- New content: the bypass channel or a direct argument ---
        //
        // IF `source_ref` EXISTS IT IS BINDING (the same rule as create_document):
        // an unresolvable ref DOES NOT silently fall back to `new_content`. That
        // was the costliest error class in Swift — when the ref did not resolve
        // the file was written with an empty body and reported as "edited", i.e. the user's document WAS EMPTIED.
        let new_content = match text_field("source_ref") {
            Some(r) => {
                let Some(record) = ctx.from_store(&SourceRef(r.clone())) else {
                    let mut outcome =
                        ToolOutcome::failed(&ToolError::Other(format!("unknown source_ref: {r}")));
                    outcome.chip_text = "Source data not found".to_string();
                    outcome.to_model = format!("unknown_data_ref: \"{r}\"; nothing was edited");
                    return Ok(outcome);
                };
                record.body
            }
            None => text_field("new_content")
                .ok_or_else(|| ToolError::MissingField("new_content".into()))?,
        };

        // "Editing" with empty content is DELETING the document; the model does that by accident.
        if new_content.trim().is_empty() {
            return Err(ToolError::InvalidArgument(
                "the new content is empty: an edit cannot empty the document".into(),
            ));
        }

        // In Excel the ORIGINAL COLUMN COUNT is a correctness signal: if the
        // model forgets half the table and sends a single-column list the file
        // looks "edited" but data is lost. Reading is cheap (we already have the
        // file) and the warning goes to the model as a fact.
        let mut warning = String::new();
        let table: Option<Table> = if format.table_shape() {
            let previous = fs::read(&path).ok().and_then(|b| parse_xlsx(&b).ok());
            let new = markdown_dan(&new_content).filter(|t| !t.rows.is_empty());
            if let (Some(o), Some(y)) = (&previous, &new)
                && o.column_count() > 0
                && y.column_count() < o.column_count()
            {
                warning = format!(
                    "; warning: the new table has {} columns, the original had {}",
                    y.column_count(),
                    o.column_count()
                );
            }
            new
        } else {
            None
        };

        // If Excel is requested but no table came out, the `to_table` gate
        // inside `write_document` takes over: a body with pipes that cannot be
        // parsed is NOT SILENTLY poured into one column, an explicit error is returned.
        let body = if table.is_some() { None } else { Some(new_content.as_str()) };

        let folder = output_folder(ctx)?;
        let name = edited_name(&path);
        let new_path = write_document(
            engine(format).as_ref(),
            &name,
            text_field("title").as_deref(),
            body,
            table.as_ref(),
            &folder,
        )?;

        let new_name = new_path
            .file_name()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());
        let chip = format!("{} edited: {new_name}", format.tag());

        // THE FORMAT IS STATED PLAINLY: instead of repeating the rule in the
        // prompt we return the FACT, so the model cannot say "I converted it to word".
        Ok(ToolOutcome::written(
            chip,
            format!(
                "file_edited: {new_name} (source: {}, format unchanged: {}; this tool never \
                 converts format — to change format call create_document with the new format){warning}",
                match source {
                    DocumentSource::Argument => "given path",
                    DocumentSource::Watcher => "document in play",
                    DocumentSource::LastModified => "most recent document",
                },
                format.extension(),
            ),
        )
        .raw_output(new_path.display().to_string())
        .file_path(new_path))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_store::SharedStore;
    use serde_json::json;
    use tacet_core::{SilentReporter, DataStore as CoreDataStore};
    use std::sync::Arc;

    fn hold<F: std::future::Future>(gelecek: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut gelecek = pin!(gelecek);
        loop {
            if let Poll::Ready(output) = gelecek.as_mut().poll(&mut cx) {
                return output;
            }
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-edit-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn context(root: &Path, store: Arc<SharedStore>) -> ToolContext {
        ToolContext::new(store, root, Arc::new(SilentReporter))
    }

    /// Produces an xlsx in the output folder (which is now the root ITSELF).
    fn xlsx_uret(root: &Path, name: &str, table: Table) -> PathBuf {
        write_document(&ExcelEngine::new(), name, None, None, Some(&table), root).expect("xlsx")
    }

    fn md_uret(root: &Path, name: &str, body: &str) -> PathBuf {
        write_document(&TextEngine::new(DocumentFormat::Markdown), name, None, Some(body), None, root)
            .expect("md")
    }

    /// 1) THE SWIFT LESSON ITSELF: the user gives no path and there is no
    ///    watcher; the tool does NOT say "no document", it finds the last document in the output folder.
    #[test]
    fn without_a_watcher_the_flow_finds_the_last_produced_document() {
        let root = temp_dir("last-produced");
        let previous = xlsx_uret(
            &root,
            "monthly-report",
            Table::new(["Month", "Amount"], vec![vec!["January".into(), "100".into()]]),
        );
        assert!(previous.exists());

        let mut ctx = context(&root, Arc::new(SharedStore::new()));
        let outcome = hold(EditDocumentTool::new().run(
            json!({"new_content": "| Month | Amount |\n| --- | --- |\n| January | 100 |\n| February | 250 |"}),
            &mut ctx,
        ));

        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.chip_text);
        assert!(outcome.to_model.contains("file_edited"), "{}", outcome.to_model);
        let new = outcome.file_path.expect("file path");
        assert_eq!(new.extension().unwrap(), "xlsx", "the format must be kept");
        assert!(new.file_name().unwrap().to_string_lossy().contains("(edited)"));
        // The original IS STILL THERE: an edit must not be irreversible loss.
        assert!(previous.exists(), "the original was deleted");

        let byte = fs::read(&new).unwrap();
        let entries = tacet_zip::open_map(&byte).expect("zip");
        let sheet = String::from_utf8(entries["xl/worksheets/sheet1.xml"].clone()).unwrap();
        assert!(sheet.contains("February"), "the new row did not reach the file");
        fs::remove_dir_all(&root).ok();
    }

    /// 2) With the watcher attached, FACT beats the GUESS: even if there is a
    ///    newer document in the folder, the one the watcher names is edited.
    #[test]
    fn the_watcher_beats_the_guess() {
        let root = temp_dir("watcher");
        let target = md_uret(&root, "target", "old body");
        // Another document written LATER — the guess tier would pick this one.
        let _newer = md_uret(&root, "zzz-newer", "unrelated");

        let watcher = Arc::new(DocumentWatcher::new());
        watcher.assign(&target);
        let tool = EditDocumentTool::with_watcher(watcher.clone());
        let mut ctx = context(&root, Arc::new(SharedStore::new()));

        let outcome = hold(tool.run(json!({"new_content": "new body"}), &mut ctx));
        assert_eq!(outcome.state, ToolState::Written);
        let new = outcome.file_path.clone().expect("path");
        assert!(
            new.file_name().unwrap().to_string_lossy().starts_with("target"),
            "the guess won instead of the watcher: {}",
            new.display()
        );
        assert!(outcome.to_model.contains("document in play"));
        // The new version must now be the workable document (chained editing).
        assert_eq!(watcher.last().as_deref(), Some(new.as_path()));
        fs::remove_dir_all(&root).ok();
    }

    /// 3) THE FORMAT IS NOT CONVERTED and this is stated PLAINLY to the model
    ///    (the lock on the silent lie from Swift).
    #[test]
    fn the_format_is_not_converted_and_the_model_is_told_so() {
        let root = temp_dir("format");
        let target = xlsx_uret(&root, "table", Table::new(["A"], vec![vec!["1".into()]]));
        let watcher = Arc::new(DocumentWatcher::new());
        watcher.assign(&target);
        let tool = EditDocumentTool::with_watcher(watcher);
        let mut ctx = context(&root, Arc::new(SharedStore::new()));

        let outcome = hold(tool.run(
            json!({"new_content": "| A |\n| --- |\n| 1 |\n| 2 |"}),
            &mut ctx,
        ));
        assert_eq!(outcome.file_path.as_ref().unwrap().extension().unwrap(), "xlsx");
        assert!(
            outcome.to_model.contains("format unchanged: xlsx"),
            "{}",
            outcome.to_model
        );
        assert!(outcome.to_model.contains("never converts format"));
        fs::remove_dir_all(&root).ok();
    }

    /// 4) If there is no document that is a FACT, not an error — the fixed
    ///    `tool_failed` must not go to the model, otherwise "the tool is broken" and "there is no document" cannot be told apart.
    #[test]
    fn when_there_is_no_document_a_fact_is_returned_not_an_error() {
        let root = temp_dir("empty");
        let mut ctx = context(&root, Arc::new(SharedStore::new()));
        let outcome =
            hold(EditDocumentTool::new().run(json!({"new_content": "x"}), &mut ctx));

        assert_eq!(outcome.state, ToolState::Read);
        assert!(outcome.to_model.starts_with("no_document_in_play"));
        assert_ne!(outcome.to_model, tacet_core::ERROR_MODEL_TEXT);
        assert!(!ctx.session_tainted(), "no document was read");
        fs::remove_dir_all(&root).ok();
    }

    /// 5) The sandbox: an explicit `path` argument CANNOT get out; an
    ///    unresolvable `source_ref` WRITES NO file (emptying the document is the costliest error).
    #[test]
    fn the_sandbox_and_unresolvable_ref_gates() {
        let root = temp_dir("gates");
        let target = md_uret(&root, "notes", "body");
        let watcher = Arc::new(DocumentWatcher::new());
        watcher.assign(&target);
        let tool = EditDocumentTool::with_watcher(watcher);
        let mut ctx = context(&root, Arc::new(SharedStore::new()));

        for escape in ["../outside.md", "/etc/hosts", "sub/../../escape.md"] {
            let s = hold(tool.run(json!({"new_content": "x", "path": escape}), &mut ctx));
            assert!(matches!(s.state, ToolState::Failed(_)), "the escape passed: {escape}");
            assert_eq!(s.to_model, tacet_core::ERROR_MODEL_TEXT);
        }

        let previous_count = fs::read_dir(&root).unwrap().count();
        let s = hold(tool.run(json!({"source_ref": "document#404"}), &mut ctx));
        assert!(matches!(s.state, ToolState::Failed(_)));
        assert!(s.to_model.contains("unknown_data_ref"));
        assert!(s.to_model.contains("nothing was edited"));
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            previous_count,
            "an unresolvable ref wrote a file"
        );
        fs::remove_dir_all(&root).ok();
    }

    /// 6) The bypass channel: the new content comes from the store, the model NEVER carries the body.
    #[test]
    fn editing_with_a_source_ref_goes_through_the_bypass_channel() {
        let root = temp_dir("bypass");
        let target = xlsx_uret(&root, "large", Table::new(["Name", "Amount"], vec![vec!["x".into(), "1".into()]]));
        let watcher = Arc::new(DocumentWatcher::new());
        watcher.assign(&target);
        let tool = EditDocumentTool::with_watcher(watcher);

        let store = Arc::new(SharedStore::new());
        let large = Table::new(
            ["Name", "Amount"],
            (0..400).map(|i| vec![format!("person{i}"), format!("{i}")]).collect::<Vec<_>>(),
        );
        let r = CoreDataStore::put(&*store, "document", "400 rows", large.markdown_truncated(usize::MAX));
        let mut ctx = context(&root, store);

        let outcome = hold(tool.run(json!({"source_ref": r.as_str()}), &mut ctx));
        assert_eq!(outcome.state, ToolState::Written, "{}", outcome.chip_text);
        // The 400 rows DID NOT PASS through the model.
        assert!(outcome.to_model.len() < 260, "{}", outcome.to_model);
        assert!(!outcome.to_model.contains("person399"));
        // But the file IS FULL.
        let byte = fs::read(outcome.file_path.unwrap()).unwrap();
        let sheet = String::from_utf8(
            tacet_zip::open_map(&byte).unwrap()["xl/worksheets/sheet1.xml"].clone(),
        )
        .unwrap();
        assert!(sheet.contains("person399"), "the data did not reach the file");
        fs::remove_dir_all(&root).ok();
    }

    /// 7) An edit with empty content CANNOT EMPTY the document; column loss is
    ///    reported to the model as a WARNING (no silent data loss).
    #[test]
    fn empty_content_is_rejected_and_column_loss_is_reported() {
        let root = temp_dir("loss");
        let target = xlsx_uret(
            &root,
            "wide",
            Table::new(["Name", "Amount", "Note"], vec![vec!["a".into(), "1".into(), "n".into()]]),
        );
        let watcher = Arc::new(DocumentWatcher::new());
        watcher.assign(&target);
        let tool = EditDocumentTool::with_watcher(watcher);
        let mut ctx = context(&root, Arc::new(SharedStore::new()));

        let empty = hold(tool.run(json!({"new_content": "   "}), &mut ctx));
        assert!(matches!(empty.state, ToolState::Failed(_)), "empty content passed");

        let narrow = hold(tool.run(json!({"new_content": "| Ad |\n| --- |\n| a |"}), &mut ctx));
        assert_eq!(narrow.state, ToolState::Written);
        assert!(narrow.to_model.contains("warning"), "{}", narrow.to_model);
        assert!(narrow.to_model.contains("3"), "{}", narrow.to_model);
        fs::remove_dir_all(&root).ok();
    }

    /// 8) The name suffix DOES NOT PILE UP and the file carries the 0600
    ///    permission stamp (the single write gate was not overridden).
    #[test]
    fn the_name_suffix_does_not_pile_up_and_the_permission_stamp_stays() {
        assert_eq!(edited_name(Path::new("/a/report.xlsx")), "report (edited)");
        assert_eq!(
            edited_name(Path::new("/a/report (edited).xlsx")),
            "report (edited)",
            "the suffix must not pile up"
        );

        let root = temp_dir("permission");
        let target = md_uret(&root, "note", "old");
        let watcher = Arc::new(DocumentWatcher::new());
        watcher.assign(&target);
        let tool = EditDocumentTool::with_watcher(watcher);
        let mut ctx = context(&root, Arc::new(SharedStore::new()));
        let outcome = hold(tool.run(json!({"new_content": "new"}), &mut ctx));
        let new = outcome.file_path.expect("path");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mod_ = fs::metadata(&new).unwrap().permissions().mode() & 0o777;
            assert_eq!(mod_, 0o600, "the permission stamp was dropped: {mod_:o}");
        }
        assert_eq!(fs::read_to_string(&new).unwrap().trim(), "new");
        fs::remove_dir_all(&root).ok();
    }

    /// 9) The watcher records only SUCCESSFUL outcomes with files that really exist.
    #[test]
    fn the_watcher_does_not_record_a_failed_outcome() {
        let root = temp_dir("watcher-gate");
        let real = md_uret(&root, "real", "x");
        let trace = DocumentWatcher::new();

        trace.save(&ToolOutcome::failed(&ToolError::FileNotFound(real.clone())));
        assert!(trace.last().is_none(), "a failed outcome was recorded");

        trace.save(&ToolOutcome::written("x", "y").file_path(root.join("missing.md")));
        assert!(trace.last().is_none(), "a non-existent file was recorded");

        trace.save(&ToolOutcome::written("x", "y").file_path(real.clone()));
        assert_eq!(trace.last().as_deref(), Some(real.as_path()));

        trace.clear();
        assert!(trace.last().is_none());
        fs::remove_dir_all(&root).ok();
    }

    /// 10) The schema contract and the router triggers.
    #[test]
    fn the_schema_and_router_contract() {
        let js = EditDocumentTool::new().schema().json_schema();
        // `new_content` IS NOT required IN THE SCHEMA: were it required the
        // grammar would force the model to write the body on every call and the
        // `source_ref` (bypass channel) path would be gone. The requirement is in the tool body, as a "one of the two" rule.
        assert_eq!(js["required"], json!([]));
        assert_eq!(js["additionalProperties"], json!(false));

        let schema = EditDocumentTool::new().schema();
        assert!(schema.validate(&json!({"new_content": "x"})).is_ok());
        assert!(schema.validate(&json!({"source_ref": "document#1"})).is_ok());
        assert!(schema.validate(&json!({"new_content": 1})).is_err());

        // If neither is there the TOOL rejects it (not the schema): where the
        // gate lives is written down in the test so that if it moves into the schema later this test warns.
        let root = temp_dir("schema-gate");
        md_uret(&root, "x", "body");
        let mut ctx = context(&root, Arc::new(SharedStore::new()));
        let empty = hold(EditDocumentTool::new().run(json!({}), &mut ctx));
        assert!(matches!(empty.state, ToolState::Failed(_)));
        fs::remove_dir_all(&root).ok();

        // The router must be able to see this tool from the Document profile: the
        // name carries both the "document" and the "edit" hint.
        use crate::router::{Router, simplify};
        let text = simplify(EditDocumentTool::new().name());
        assert!(text.contains("document") && text.contains("edit"));

        let mut catalog = tacet_core::ToolCatalog::new();
        catalog.add(Arc::new(crate::calc::CalcTool));
        catalog.add(Arc::new(EditDocumentTool::new()));
        let chosen = Router::new().max(1).select("add a row to the table", &catalog);
        assert_eq!(chosen[0].name(), "edit_document");
    }
}
