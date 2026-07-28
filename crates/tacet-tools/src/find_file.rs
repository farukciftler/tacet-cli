//! `find_file` — file name and content search inside the sandbox.
//!
//! Its Swift counterpart is `SearchNotesTool` (CoreSpotlight). Spotlight has no
//! Rust equivalent and MUST NOT have one: querying the device's own index is a
//! platform capability, whereas this crate DOES NOT SEE outside the sandbox. So
//! there is no index here, just direct walking. The real lesson inherited from
//! Swift is not the search algorithm but the NAMING: in Swift the tool was first
//! called `arama` ("search") and when the user said "search the internet" the
//! model called it and looked at device notes. Since the name is a stronger signal
//! than the description, it was renamed to `search_notes`. The name here is
//! deliberately `find_file` too: the name itself says WHERE it searches, so it
//! cannot be confused with the web.
//!
//! ZERO DEPENDENCIES: the walking and the matching are written with `std`. Pulling
//! in a `walkdir` or a `regex` would mean taking an unaudited body of code inside
//! for the sake of a substring search; the subset we need (iterative walk +
//! case-folded substring) is ~100 lines.
//!
//! THE BYPASS CHANNEL: many results DO NOT PASS through the model. The full list
//! goes into the `DataStore`, and the model gets the first few lines plus a
//! `source_ref`. Dumping a 400-file match list into the context means finishing
//! the 4096 window with a single search.

use std::fs;
use std::path::{Path, PathBuf};

use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult,
    TraceUpdate, boxed,
};

/// The most matches shown directly to the model. Anything above falls into the
/// bypass channel.
const MODEL_LINES: usize = 12;

/// The default and maximum number of results.
const DEFAULT_COUNT: usize = 20;
const MAX_COUNT: usize = 100;

/// The most entries to walk. If the sandbox is large the search must not run
/// forever; when the cap is hit the result is not "nothing" but "partial", and
/// that IS TOLD to the model — silently returning a short list leads the model to
/// say "there is no such file".
const MAX_ENTRIES: usize = 20_000;

/// The upper bound on a file whose content is scanned (1 MiB). Above that it is
/// not a text document; reading it wastes memory and time.
const CONTENT_CAP: u64 = 1024 * 1024;

/// The directory nesting limit. The walk is iterative (stack based) so there is no
/// overflow risk; the limit is for link loops and pathological trees.
const MAX_DEPTH: usize = 24;

/// The extensions whose content is scanned — an ALLOW list.
///
/// With a deny list, every unknown extension would be read on the assumption "it
/// is probably text"; on a binary file that means trying to match broken UTF-8.
/// The same rule as `read_document`, deliberately the same vocabulary.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "text", "log", "csv", "tsv", "json", "yaml", "yml", "toml", "rs",
    "swift", "py", "js", "ts", "html", "css", "sh", "conf", "ini",
];

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A single match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The path relative to the sandbox ROOT. An absolute path is never handed
    /// out: in the text going to the model, the user's home directory is personal
    /// data.
    pub path: String,
    /// Whether it matched in the name or in the content — so the model knows which
    /// signal to trust.
    pub search_in: SearchIn,
    /// The truncated form of the matching line, for a content match.
    pub line: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchIn {
    Name,
    Content,
}

impl SearchIn {
    pub fn label(&self) -> &'static str {
        match self {
            SearchIn::Name => "name",
            SearchIn::Content => "content",
        }
    }
}

/// The complete outcome of one search.
#[derive(Debug, Clone, Default)]
pub struct SearchOutcome {
    pub matches: Vec<Match>,
    /// Was the cap hit — if so the list is INCOMPLETE and that is not kept secret.
    pub partial: bool,
    pub visited: usize,
}

impl SearchOutcome {
    /// Turns the matches into the list of lines going to the model/the store.
    pub fn lines(&self, max: usize) -> String {
        let mut s = String::new();
        for (i, m) in self.matches.iter().take(max).enumerate() {
            s.push_str(&format!("{}. {} [{}]", i + 1, m.path, m.search_in.label()));
            if let Some(line) = &m.line {
                s.push_str(&format!(" — {line}"));
            }
            s.push('\n');
        }
        let hidden = self.matches.len().saturating_sub(max);
        if hidden > 0 {
            s.push_str(&format!("(+{hidden} more results not shown)\n"));
        }
        s
    }
}

// ---------------------------------------------------------------------------
// The search core
// ---------------------------------------------------------------------------

/// Searches the sandbox. Free-standing so it can also be called from outside the
/// tool (test, eval).
///
/// `root` must be ALREADY RESOLVED: this function does not set up the sandbox gate
/// itself, the caller passes it through `ToolContext::resolve_path`. Resolving in
/// two places means two different gates.
pub fn search(
    root: &Path,
    pattern: &str,
    search_content: bool,
    max_results: usize,
) -> ToolResult<SearchOutcome> {
    let needle = fold(pattern);
    if needle.is_empty() {
        return Err(ToolError::InvalidArgument("empty search pattern".into()));
    }

    let mut outcome = SearchOutcome::default();
    // An iterative walk: recursion overflows the stack on a deep tree, and that is
    // not a catchable error but an outright crash.
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        // An unreadable directory DOES NOT END THE SEARCH: losing the entire result
        // over one permission-denied subfolder gains the user nothing.
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        // The sort is for DETERMINISM: `read_dir` gives file system order, i.e. the
        // same sandbox could produce a different list on two runs and eval would
        // become incomparable.
        let mut candidates: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        candidates.sort();

        for path in candidates {
            if outcome.visited >= MAX_ENTRIES {
                outcome.partial = true;
                return Ok(outcome);
            }
            outcome.visited += 1;

            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Hidden entries are skipped: trees like `.git` eat the entire search
            // budget and are not what the user calls "my files".
            if name.starts_with('.') {
                continue;
            }

            // `symlink_metadata`: links ARE NOT FOLLOWED. Followed, a link inside
            // the sandbox would read outside it — exactly what would invalidate
            // `resolve_path` from the start.
            let Ok(meta) = path.symlink_metadata() else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }

            if meta.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            if !meta.is_file() {
                continue;
            }

            let relative = relative_path(root, &path);

            if fold(&name).contains(&needle) {
                outcome.matches.push(Match {
                    path: relative,
                    search_in: SearchIn::Name,
                    line: None,
                });
            } else if search_content
                && meta.len() <= CONTENT_CAP
                && is_text_extension(&path)
                && let Some(line) = search_in_content(&path, &needle)
            {
                outcome.matches.push(Match {
                    path: relative,
                    search_in: SearchIn::Content,
                    line: Some(line),
                });
            }

            if outcome.matches.len() >= max_results {
                // The requested count is full: this is NOT "partial", it is the
                // user's own cap. Confusing the two would give the model wrong
                // information.
                return Ok(outcome);
            }
        }
    }
    Ok(outcome)
}

/// Searches inside a file; returns the truncated form of the first matching line.
fn search_in_content(path: &Path, needle: &str) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    // A NUL byte: does not occur in text documents, occurs in almost every binary
    // format.
    if bytes.contains(&0) {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes);
    for line in text.lines() {
        if fold(line).contains(needle) {
            return Some(truncate_line(line));
        }
    }
    None
}

/// Shortens the matching line so the chip and the model text stay on one line.
fn truncate_line(line: &str) -> String {
    const CAP: usize = 100;
    let clean: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() <= CAP {
        return clean;
    }
    let head: String = clean.chars().take(CAP - 1).collect();
    format!("{head}…")
}

/// The comparison form of the text: lowercase + Turkish character folding.
///
/// It applies THE SAME rule as `router::simplify` but was deliberately NOT REUSED:
/// that function exists for intent triggers, and search behaviour must not change
/// silently when that one changes. Two separate purposes, two separate
/// definitions.
fn fold(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        let d = match c {
            'ı' | 'İ' | 'I' => 'i',
            'ş' | 'Ş' => 's',
            'ğ' | 'Ğ' => 'g',
            'ü' | 'Ü' => 'u',
            'ö' | 'Ö' => 'o',
            'ç' | 'Ç' => 'c',
            _ => c,
        };
        s.extend(d.to_lowercase());
    }
    s.trim().to_string()
}

fn is_text_extension(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| TEXT_EXTENSIONS.contains(&e.as_str()))
}

/// The path text relative to the root. If it is outside the root (it should not
/// be) it falls back to the file name — under no condition is an absolute path
/// returned.
fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

/// `find_file` — name + content search in the sandbox.
#[derive(Debug, Clone, Copy, Default)]
pub struct FindFileTool;

impl FindFileTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for FindFileTool {
    fn name(&self) -> &str {
        "find_file"
    }

    fn description(&self) -> &str {
        // THE NAME AND THE FIRST SENTENCE SAY WHERE IT SEARCHES (the Swift lesson):
        // the model must not call this tool for a "search the internet" request.
        // The warning in the description is not enough on its own, the name carries
        // it — but the two together are better.
        "Searches the user's OWN files inside the working directory by file name and by \
         text content. Call this when the user asks to find a file or a note on this device \
         ('find the file about X', 'which file mentions Y', in any language). It cannot see \
         the internet and it cannot leave the working directory — never use it for weather, \
         news or general knowledge."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new(
                "pattern",
                ArgSchema::text().description(
                    "Text to look for in file names and file contents, e.g. 'meeting'.",
                ),
            )
            .required(),
            Field::new(
                "folder",
                ArgSchema::text().description(
                    "Optional subfolder to search in, relative to the working directory. \
                     Leave empty to search everything.",
                ),
            ),
            Field::new(
                "search_content",
                ArgSchema::bool().description(
                    "Also search inside text files (default true). false = names only.",
                ),
            ),
            Field::new(
                "max_results",
                ArgSchema::integer()
                    .range(Some(1.0), Some(MAX_COUNT as f64))
                    .description("Maximum number of results (default 20)."),
            ),
        ])
        .description("Find files by name or content in the working directory")
    }

    /// It looks at the user's own files, and the searched word is itself personal
    /// data: the session is tainted.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            if let Err(e) = self.schema().validate(&args) {
                return ToolOutcome::failed(&e);
            }
            let trace = ctx.start_chip("search", "Searching files…");

            let (outcome, tainted) = match self.execute(&args, ctx) {
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
            // Tainting only when a search was ACTUALLY performed: the failure path
            // touched no file, and tainting the session for it would drag the user
            // into free approval prompts (the same rule as read_document).
            if tainted {
                ctx.taint();
            }
            outcome
        })
    }
}

impl FindFileTool {
    /// The synchronous body — `run` only holds the chip/tainting shell, and the
    /// error path is collected in a single point (`ToolOutcome::failed`).
    fn execute(&self, args: &serde_json::Value, ctx: &ToolContext) -> ToolResult<ToolOutcome> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::MissingField("pattern".into()))?;

        // The folder argument goes through the sandbox gate too: the model can
        // write "../.." here, and it will.
        let relative_folder = args
            .get("folder")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(".");
        // THE ROOT IS RESOLVED, NOT JUST CHECKED FOR SHAPE. `resolve_path` is
        // lexical and `is_dir()` FOLLOWS links, so `folder="gate"` (a link
        // planted in the sandbox by `run_code`) used to pass and the whole walk
        // ran on the tree OUTSIDE. The per-entry `symlink_metadata` skip below
        // only guards CHILDREN — it never sees the root. That escape is heavier
        // than a read_document one: the MATCHING LINE ITSELF goes to the model,
        // so the attacker exfiltrates exactly the secret searched for.
        let root = crate::sandbox_path::resolve_existing_dir(ctx, relative_folder)?;

        let search_content = args
            .get("search_content")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, MAX_COUNT))
            .unwrap_or(DEFAULT_COUNT);

        let found = search(&root, pattern, search_content, max_results)?;

        // AN EMPTY RESULT IS NOT AN ERROR. Let the model honestly say "I could not
        // find it"; turning it into an error would send the model the fixed
        // `tool_failed` text and the model could not tell "the search did not work"
        // from "there are no results".
        if found.matches.is_empty() {
            let partial = if found.partial {
                " (partial scan: limit reached)"
            } else {
                ""
            };
            return Ok(ToolOutcome::read_ok(
                "No matching file",
                format!("no_results_found in the working directory{partial}"),
            )
            .raw_output(format!("{} entries visited", found.visited)));
        }

        let count = found.matches.len();
        let full_list = found.lines(usize::MAX);
        let chip = format!("{count} files found");

        // The bypass channel: if the list does not fit the model it goes into the
        // store and the model gets the first few lines + a source_ref. Truncation
        // is thereby not DATA LOSS but a window decision — the next tool reaches the
        // remaining results in full by reference.
        if count > MODEL_LINES {
            let source = ctx.store("file", &format!("{count} file matches"), full_list.clone());
            let preview = found.lines(MODEL_LINES);
            return Ok(ToolOutcome::summarize(
                chip,
                format!("found {count} files:\n{preview}"),
                source.as_str(),
            )
            .raw_output(full_list));
        }

        let partial = if found.partial {
            "\n(partial scan: limit reached)"
        } else {
            ""
        };
        Ok(
            ToolOutcome::read_ok(chip, format!("found {count} files:\n{full_list}{partial}"))
                .raw_output(full_list),
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use tacet_kernel::{InMemoryDataStore, SilentReporter, SourceRef, ToolState};

    /// Core has no tokio; this crate must not pick a runtime either (the same
    /// pattern as `block_on` in the create_document tests).
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
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
        let mut future = pin!(future);
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-search-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn write(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("folder");
        }
        fs::write(path, content).expect("write");
    }

    fn context(root: &Path, store: Arc<InMemoryDataStore>) -> ToolContext {
        ToolContext::new(store, root, Arc::new(SilentReporter))
    }

    /// 1) A name match and a content match are found separately and LABELLED.
    #[test]
    fn name_and_content_matches_are_marked_separately() {
        let root = temp_dir("name-content");
        write(&root, "meeting-notes.md", "the weather is nice today");
        write(&root, "journal.md", "there is a meeting this evening");
        write(&root, "unrelated.md", "nothing");

        let o = search(&root, "meeting", true, 20).expect("search");
        assert_eq!(o.matches.len(), 2, "{:?}", o.matches);
        let by_name = o
            .matches
            .iter()
            .find(|m| m.search_in == SearchIn::Name)
            .expect("name match");
        assert_eq!(by_name.path, "meeting-notes.md");
        assert!(by_name.line.is_none(), "a name match must carry no line");
        let by_content = o
            .matches
            .iter()
            .find(|m| m.search_in == SearchIn::Content)
            .expect("content");
        assert_eq!(
            by_content.line.as_deref(),
            Some("there is a meeting this evening")
        );
        fs::remove_dir_all(&root).ok();
    }

    /// 2) Turkish characters and case differences do not break the search.
    #[test]
    fn accents_and_case_are_folded() {
        let root = temp_dir("folding");
        write(&root, "MEETING.md", "x");
        write(&root, "notes.md", "GÖRÜŞME in February");

        assert_eq!(search(&root, "meeting", true, 20).unwrap().matches.len(), 1);
        assert_eq!(search(&root, "MEETING", true, 20).unwrap().matches.len(), 1);
        // The user types the ASCII-folded "gorusme" while the file says "GÖRÜŞME": the
        // two must still match. This fixture stays non-ASCII on purpose — the
        // accent folding it exercises is the point of the test.
        assert_eq!(search(&root, "gorusme", true, 20).unwrap().matches.len(), 1);
        assert_eq!(
            search(&root, "february", true, 20).unwrap().matches.len(),
            1
        );
        fs::remove_dir_all(&root).ok();
    }

    /// 3) The sandbox: even if the model writes "../.." it CANNOT GET OUT.
    #[test]
    fn the_sandbox_cannot_be_escaped() {
        let root = temp_dir("sandbox");
        write(&root, "inside.md", "data");
        let store = Arc::new(InMemoryDataStore::new());
        let mut ctx = context(&root, store);

        for escape in ["..", "../..", "/etc", "sub/../../outside"] {
            let outcome = block_on(
                FindFileTool::new().run(json!({"pattern": "data", "folder": escape}), &mut ctx),
            );
            assert!(
                matches!(outcome.state, ToolState::Failed(_)),
                "escape got through: {escape}"
            );
            assert_eq!(outcome.to_model, tacet_kernel::ERROR_MODEL_TEXT);
        }
        // An escape attempt MUST NOT TAINT the session: no data was read.
        assert!(!ctx.session_tainted());
        fs::remove_dir_all(&root).ok();
    }

    /// 3b) THE SEARCH ROOT ITSELF MAY BE A PLANTED LINK — and that used to be the
    ///     way out. `run_code` is allowed to write inside the sandbox, and
    ///     creating a symlink IS a write, so a poisoned prompt could plant
    ///     `gate -> /Users/<victim>` and then call
    ///     `find_file(pattern="seed phrase", folder="gate")`. `resolve_path` is
    ///     lexical and `is_dir()` follows links, so the whole walk ran on the
    ///     tree OUTSIDE. The per-entry skip in `search` never sees the root.
    ///
    /// This escape is heavier than the read_document one: THE MATCHING LINE
    /// ITSELF goes to the model, so the attacker gets exactly the secret they
    /// searched for. Measured before the fix, the model was handed
    /// "wallet.md [content] — seed phrase: correct horse battery".
    ///
    /// `folder="gate/deep"` is measured TOO: checking only the last component
    /// would let that one through.
    #[cfg(unix)]
    #[test]
    fn a_planted_link_cannot_be_the_search_root() {
        let root = temp_dir("link-root");
        let outside = temp_dir("victim");
        write(&outside, "wallet.md", "seed phrase: correct horse battery");
        write(
            &outside,
            "deep/wallet.md",
            "seed phrase: correct horse battery",
        );
        std::os::unix::fs::symlink(&outside, root.join("gate")).expect("plant the link");

        let store = Arc::new(InMemoryDataStore::new());
        let mut ctx = context(&root, store);
        for folder in ["gate", "gate/deep"] {
            let outcome = block_on(FindFileTool::new().run(
                json!({"pattern": "seed phrase", "folder": folder}),
                &mut ctx,
            ));
            assert!(
                matches!(outcome.state, ToolState::Failed(_)),
                "the walk escaped through {folder}: {}",
                outcome.to_model
            );
            assert_eq!(outcome.to_model, tacet_kernel::ERROR_MODEL_TEXT);
            assert!(
                !outcome.to_model.contains("correct horse battery"),
                "the secret leaked to the model"
            );
        }
        // A refused escape read nothing, so it must not taint the session.
        assert!(!ctx.session_tainted());
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&outside).ok();
    }

    /// 4) Binary and large files do not enter the content scan; a symbolic link is
    ///    not followed (following it would pierce the sandbox).
    #[test]
    fn binary_files_and_symbolic_links_are_skipped() {
        let root = temp_dir("binary");
        fs::write(root.join("data.json"), b"secret\0\x01secret").expect("binary write");
        write(&root, "plain.md", "secret word");

        let o = search(&root, "secret", true, 20).expect("search");
        // The CONTENT of the binary file must not match — only the plain text file
        // should be found.
        assert_eq!(o.matches.len(), 1, "{:?}", o.matches);
        assert_eq!(o.matches[0].path, "plain.md");

        // The link is not followed: even if the outside file matches by name it is
        // not entered.
        #[cfg(unix)]
        {
            let outside = temp_dir("link-target");
            write(&outside, "secret-outside.md", "secret");
            std::os::unix::fs::symlink(&outside, root.join("gate")).expect("symlink");
            let o2 = search(&root, "secret", true, 20).expect("search");
            assert_eq!(
                o2.matches.len(),
                1,
                "the link was followed: {:?}",
                o2.matches
            );
            fs::remove_dir_all(&outside).ok();
        }
        fs::remove_dir_all(&root).ok();
    }

    /// 5) Many results go through the bypass channel: the model does not see THE
    ///    WHOLE list.
    #[test]
    fn many_results_go_through_the_bypass_channel() {
        let root = temp_dir("bypass");
        for i in 0..40 {
            write(&root, &format!("report-{i:03}.md"), "x");
        }
        let store = Arc::new(InMemoryDataStore::new());
        let mut ctx = context(&root, store);
        let outcome = block_on(
            FindFileTool::new().run(json!({"pattern": "report", "max_results": 40}), &mut ctx),
        );

        assert_eq!(outcome.state, ToolState::Read);
        assert!(
            outcome.to_model.contains("source_ref"),
            "{}",
            outcome.to_model
        );
        // The 40-line list WAS NOT DUMPED on the model: the last file name must not
        // appear in the model text.
        assert!(
            !outcome.to_model.contains("report-039"),
            "{}",
            outcome.to_model
        );
        // But no data was lost: the body in the store carries the full list.
        let record = ctx
            .from_store(&SourceRef("file#1".into()))
            .expect("store record");
        assert!(record.body.contains("report-039.md"));
        assert!(ctx.session_tainted(), "the user's files were read");
        fs::remove_dir_all(&root).ok();
    }

    /// 6) An empty result IS NOT AN ERROR: the model must be able to say "I could
    ///    not find it".
    #[test]
    fn an_empty_result_is_not_an_error() {
        let root = temp_dir("empty");
        write(&root, "a.md", "alpha");
        let store = Arc::new(InMemoryDataStore::new());
        let mut ctx = context(&root, store);
        let outcome = block_on(FindFileTool::new().run(json!({"pattern": "zeta"}), &mut ctx));

        assert_eq!(
            outcome.state,
            ToolState::Read,
            "an empty result must not fail"
        );
        assert!(outcome.to_model.starts_with("no_results_found"));
        assert_ne!(outcome.to_model, tacet_kernel::ERROR_MODEL_TEXT);
        fs::remove_dir_all(&root).ok();
    }

    /// 7) The schema contract: the model cannot send an invented field, and a count
    ///    out of range cannot pass.
    #[test]
    fn the_schema_contract_is_binding() {
        let js = FindFileTool::new().schema().json_schema();
        assert_eq!(js["type"], "object");
        assert_eq!(js["required"], json!(["pattern"]));
        assert_eq!(js["additionalProperties"], json!(false));
        assert_eq!(js["properties"]["max_results"]["type"], "integer");

        let schema = FindFileTool::new().schema();
        assert!(schema.validate(&json!({"pattern": "a"})).is_ok());
        assert!(schema.validate(&json!({})).is_err(), "required field");
        assert!(schema.validate(&json!({"pattern": 5})).is_err(), "type");
        assert!(
            schema
                .validate(&json!({"pattern": "a", "max_results": 9999}))
                .is_err(),
            "range"
        );
        assert!(
            schema
                .validate(&json!({"pattern": "a", "search_content": "yes"}))
                .is_err(),
            "bool"
        );
    }

    /// 8) `search_content: false` leaves only name matches; the `max_results` cap is
    ///    genuinely binding.
    #[test]
    fn content_off_and_the_count_cap_are_applied() {
        let root = temp_dir("cap");
        write(&root, "one.md", "lock");
        write(&root, "two.md", "key");
        write(&root, "key-file.md", "empty");

        let names_only = search(&root, "key", false, 20).expect("search");
        assert_eq!(names_only.matches.len(), 1);
        assert_eq!(names_only.matches[0].search_in, SearchIn::Name);

        let capped = search(&root, "key", true, 2).expect("search");
        assert_eq!(capped.matches.len(), 2, "the cap must be applied");
        fs::remove_dir_all(&root).ok();
    }

    /// 9) The path going to the model is RELATIVE: an absolute path leaks the
    ///    user's home directory.
    #[test]
    fn a_relative_path_is_returned_to_the_model() {
        let root = temp_dir("relative");
        write(&root, "sub/folder/target.md", "content");
        let store = Arc::new(InMemoryDataStore::new());
        let mut ctx = context(&root, store);
        let outcome = block_on(FindFileTool::new().run(json!({"pattern": "target"}), &mut ctx));

        // The separator is the platform's, not `/`. What the guarantee is about
        // is that the path stays RELATIVE; asserting a forward slash would make
        // this a test of Unix rather than of the guarantee, and it failed on
        // Windows for exactly that reason.
        let expected = Path::new("sub").join("folder").join("target.md");
        let expected = expected.to_string_lossy();
        assert!(
            outcome.to_model.contains(expected.as_ref()),
            "{}",
            outcome.to_model
        );
        assert!(
            !outcome.to_model.contains(&root.display().to_string()),
            "an absolute path leaked: {}",
            outcome.to_model
        );
        fs::remove_dir_all(&root).ok();
    }

    /// 10) The router must be able to pick this tool for a "find a file" intent —
    ///     do the name and the description carry the General profile's hints.
    #[test]
    fn the_router_hints_hold() {
        use crate::router::{Router, simplify};
        let text = simplify(&format!(
            "{} {}",
            FindFileTool::new().name(),
            FindFileTool::new().description()
        ));
        // The General profile's hints: "file", "find".
        assert!(text.contains("file"));
        assert!(text.contains("find"));

        let mut catalog = tacet_kernel::ToolCatalog::new();
        catalog.add(Arc::new(crate::calc::CalcTool));
        catalog.add(Arc::new(FindFileTool::new()));
        let chosen = Router::new().max(1).select("find that file", &catalog);
        assert_eq!(
            chosen[0].name(),
            "find_file",
            "the router picked the wrong tool"
        );
    }
}
