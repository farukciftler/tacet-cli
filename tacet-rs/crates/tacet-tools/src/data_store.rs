//! The typed data store — the engine of the 4096-token bypass channel.
//!
//! THE HEART OF THE ARCHITECTURE: bulk device data DOES NOT PASS through the
//! model. A tool puts the raw data here, gets back a short `SourceRef`, and
//! returns to the model only "a few lines of summary + source_ref". The next step
//! that genuinely needs the data is again a TOOL; it uses the reference to reach
//! the raw body. That way a 40-page document or a 500-row table is processed
//! without straining the context window at all.
//!
//! WHY CORE'S `DataStore` IS NOT ENOUGH: the core contract keeps the body as a
//! `String` — correct for a contract layer, because format knowledge does not
//! belong there. But the tools need to tell a table from plain text: the
//! truncated summary of a table has to be VALID markdown (see the note below),
//! while the summary of plain text is the first n lines. We carry that
//! distinction, typed, here in the application layer.
//!
//! TWO LAYERS: `DataStore` is the typed store with clear ownership (written via
//! `&mut self`, `take` returns a real reference — a large `Bytes` body is not
//! cloned). `SharedStore` wraps it in a `Mutex` and binds it to core's contract,
//! which expects `Arc<dyn ...>`. Merged into a single type, either `take` would
//! have to return a clone or contract compatibility would be lost.

use std::sync::Mutex;
use tacet_kernel::{DataStore as CoreDataStore, Record, SourceRef};

/// The body formats that may be put into the store.
///
/// A closed set: every new variant also brings the question "how is its summary
/// produced", so it is deliberately kept narrow.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Table(Table),
    /// Binary data. Its summary NEVER shows the content — only the size. Bytes
    /// leaking to the model is both meaningless and a personal-data leak risk.
    Bytes(Vec<u8>),
}

impl Value {
    /// The coarse kind label used when producing a `SourceRef`.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Text(_) => "text",
            Value::Table(_) => "table",
            Value::Bytes(_) => "bytes",
        }
    }

    /// One line of size information, safe to show the model.
    pub fn short_summary(&self) -> String {
        match self {
            Value::Text(t) => format!("{} lines of text", t.lines().count()),
            Value::Table(t) => {
                format!(
                    "table of {} rows x {} columns",
                    t.row_count(),
                    t.column_count()
                )
            }
            Value::Bytes(b) => format!("{} bytes of binary data", b.len()),
        }
    }
}

/// Headers + rows. The column count is defined by `headers`; rows arriving short
/// or long are aligned while producing output (see `markdown_truncated`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(
        headers: impl IntoIterator<Item = impl Into<String>>,
        rows: impl IntoIterator<Item = Vec<String>>,
    ) -> Self {
        Self {
            headers: headers.into_iter().map(Into::into).collect(),
            rows: rows.into_iter().collect(),
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn column_count(&self) -> usize {
        self.headers.len()
    }

    /// Converts the table into markdown with at most `max_rows` data rows.
    ///
    /// THE LESSON LEARNED IN SWIFT: if truncation drops the alignment row or the
    /// column count shifts, the output is invalid markdown; the model cannot
    /// rebuild the table and says "the table was shown" while skipping the
    /// content. That is why the output is ALWAYS valid: a header row + an
    /// alignment row + exactly `column_count` cells in every row. The truncation
    /// note is written OUTSIDE the table, after a blank line, so it does not break
    /// the table block.
    pub fn markdown_truncated(&self, max_rows: usize) -> String {
        if self.headers.is_empty() {
            return String::new();
        }
        let columns = self.headers.len();
        let mut output = String::new();

        output.push_str(&write_row(&self.headers, columns));
        // The alignment row: its cell count must match the header exactly.
        let alignment: Vec<String> = (0..columns).map(|_| "---".to_string()).collect();
        output.push_str(&write_row(&alignment, columns));

        let shown = self.rows.len().min(max_rows);
        for row in self.rows.iter().take(shown) {
            output.push_str(&write_row(row, columns));
        }

        let hidden = self.rows.len() - shown;
        if hidden > 0 {
            output.push_str(&format!("\n(+{hidden} more rows not shown)\n"));
        }
        output
    }
}

/// Turns a row into a pipe-separated markdown row; pins the cell count to
/// `columns` (padding what is missing with empty cells, dropping the excess).
fn write_row(cells: &[String], columns: usize) -> String {
    let mut s = String::from("|");
    for i in 0..columns {
        let empty = String::new();
        let c = cells.get(i).unwrap_or(&empty);
        s.push(' ');
        s.push_str(&escape_cell(c));
        s.push_str(" |");
    }
    s.push('\n');
    s
}

/// So cell content cannot break the table: pipes are escaped, newlines become
/// spaces.
fn escape_cell(raw: &str) -> String {
    let mut s = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '|' => s.push_str("\\|"),
            '\n' | '\r' => s.push(' '),
            _ => s.push(c),
        }
    }
    s
}

/// The typed, session-lived store. It writes nothing to disk; when the session
/// ends it is emptied with `clear()`.
#[derive(Default)]
pub struct DataStore {
    records: Vec<(SourceRef, String, Value)>,
    counter: u64,
}

impl DataStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores the data and returns its reference. The label is derived from the
    /// `Value` variant.
    pub fn put(&mut self, data: Value) -> SourceRef {
        let kind = data.kind().to_string();
        self.put_labelled(&kind, data)
    }

    /// For when the caller wants to give its own kind ("calendar", "document"):
    /// more informative than the variant name, both for filtering and for
    /// source_ref readability.
    pub fn put_labelled(&mut self, kind: &str, data: Value) -> SourceRef {
        self.counter += 1;
        let source_ref = SourceRef(format!("{kind}#{}", self.counter));
        self.records
            .push((source_ref.clone(), kind.to_string(), data));
        source_ref
    }

    /// Access to the raw body by reference — NO clone, because the body may be
    /// megabytes and this is the hot path.
    pub fn take(&self, source_ref: &SourceRef) -> Option<&Value> {
        self.records
            .iter()
            .find(|(r, _, _)| r == source_ref)
            .map(|(_, _, v)| v)
    }

    pub fn kind(&self, source_ref: &SourceRef) -> Option<&str> {
        self.records
            .iter()
            .find(|(r, _, _)| r == source_ref)
            .map(|(_, k, _)| k.as_str())
    }

    pub fn of_kind(&self, kind: &str) -> Vec<&SourceRef> {
        self.records
            .iter()
            .filter(|(_, k, _)| k == kind)
            .map(|(r, _, _)| r)
            .collect()
    }

    pub fn count(&self) -> usize {
        self.records.len()
    }

    /// The text that GOES BACK to the model. It returns something for an unknown
    /// reference too: handed an empty string the model starts inventing, handed an
    /// explicit "not found" sentence it behaves correctly.
    pub fn summary(&self, source_ref: &SourceRef, max_rows: usize) -> String {
        let Some(value) = self.take(source_ref) else {
            return format!("{source_ref}: record not found");
        };
        match value {
            Value::Text(t) => {
                let lines: Vec<&str> = t.lines().collect();
                let shown = lines.len().min(max_rows);
                let mut s = lines[..shown].join("\n");
                let hidden = lines.len() - shown;
                if hidden > 0 {
                    if !s.is_empty() {
                        s.push('\n');
                    }
                    s.push_str(&format!("(+{hidden} more lines not shown)"));
                }
                s
            }
            Value::Table(t) => t.markdown_truncated(max_rows),
            // Byte content never reaches the model at any truncation level.
            Value::Bytes(b) => format!("{} bytes of binary data", b.len()),
        }
    }

    /// End of session. The counter IS NOT RESET: an old source_ref still held
    /// somewhere must not land on a new record and open the wrong data.
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

/// The shared wrapper that binds this to the core contract.
///
/// `ToolContext` holds an `Arc<dyn CoreDataStore>`, so every write comes through
/// `&self`; internal synchronisation is the concrete store's job (the same logic
/// as core decision 6). The `Mutex` lives here, not in the typed store — so the
/// typed store does not pay the lock cost when used with a single owner.
#[derive(Default)]
pub struct SharedStore {
    inner: Mutex<DataStore>,
}

impl SharedStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Locked access to the typed API — so tools wanting to put a `Value::Table`
    /// do not have to fall back to the core contract's `String` body.
    pub fn put_value(&self, kind: &str, data: Value) -> SourceRef {
        self.inner
            .lock()
            .expect("data store lock")
            .put_labelled(kind, data)
    }

    pub fn summary(&self, source_ref: &SourceRef, max_rows: usize) -> String {
        self.inner
            .lock()
            .expect("data store lock")
            .summary(source_ref, max_rows)
    }

    /// Cloning access: we cannot hand a reference out past the lock, so on this
    /// path a clone is unavoidable.
    pub fn value(&self, source_ref: &SourceRef) -> Option<Value> {
        self.inner
            .lock()
            .expect("data store lock")
            .take(source_ref)
            .cloned()
    }
}

impl CoreDataStore for SharedStore {
    fn put(&self, kind: &str, _summary: &str, body: String) -> SourceRef {
        // The contract gives a `String` body; there is no format information here,
        // so Text is the right default. Whoever wants to put a table uses
        // `put_value`.
        self.inner
            .lock()
            .expect("data store lock")
            .put_labelled(kind, Value::Text(body))
    }

    fn take(&self, source_ref: &SourceRef) -> Option<Record> {
        let inner = self.inner.lock().expect("data store lock");
        let value = inner.take(source_ref)?;
        let kind = inner.kind(source_ref).unwrap_or("unknown").to_string();
        Some(Record {
            source_ref: source_ref.clone(),
            kind,
            summary: value.short_summary(),
            body: inner.summary(source_ref, usize::MAX),
        })
    }

    fn of_kind(&self, kind: &str) -> Vec<Record> {
        let refs: Vec<SourceRef> = {
            let inner = self.inner.lock().expect("data store lock");
            inner.of_kind(kind).into_iter().cloned().collect()
        };
        refs.iter()
            .filter_map(|r| CoreDataStore::take(self, r))
            .collect()
    }

    fn clear(&self) {
        self.inner.lock().expect("data store lock").clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_table(rows: usize) -> Table {
        let rows = (0..rows)
            .map(|i| vec![format!("name{i}"), format!("{i}"), "TR".to_string()])
            .collect::<Vec<_>>();
        Table::new(["Name", "Number", "Country"], rows)
    }

    /// The essence of the bypass channel: a 5000-line body stays in the store and
    /// what leaves is a small summary text.
    #[test]
    fn large_data_is_carried_without_passing_through_the_model() {
        let mut store = DataStore::new();
        let large: String = (0..5000).map(|i| format!("line {i}\n")).collect();
        let length = large.len();
        let r = store.put(Value::Text(large));

        let summary = store.summary(&r, 3);
        assert!(
            summary.len() < 200,
            "the summary must fit the model: {}",
            summary.len()
        );
        assert!(summary.contains("(+4997 more lines not shown)"));

        // The raw body sits in the store, complete.
        match store.take(&r).expect("record") {
            Value::Text(t) => assert_eq!(t.len(), length),
            _ => panic!("text was expected"),
        }
    }

    #[test]
    fn truncated_markdown_stays_valid() {
        let t = sample_table(100);
        let md = t.markdown_truncated(4);
        let table_lines: Vec<&str> = md.lines().filter(|s| s.starts_with('|')).collect();
        // header + alignment + 4 data rows
        assert_eq!(table_lines.len(), 6);
        // The same number of pipes in every row: columns + 1
        for s in &table_lines {
            assert_eq!(s.matches('|').count(), 4, "broken row: {s}");
        }
        assert!(table_lines[1].contains("---"));
        assert!(md.contains("(+96 more rows not shown)"));
    }

    #[test]
    fn no_note_is_written_when_no_truncation_is_needed() {
        let md = sample_table(2).markdown_truncated(10);
        assert!(!md.contains("not shown"));
        assert_eq!(md.lines().filter(|s| s.starts_with('|')).count(), 4);
    }

    #[test]
    fn uneven_rows_are_aligned() {
        let t = Table::new(
            ["A", "B", "C"],
            vec![
                vec!["alone".into()],
                vec!["1".into(), "2".into(), "3".into(), "4".into()],
            ],
        );
        let md = t.markdown_truncated(10);
        for s in md.lines().filter(|s| s.starts_with('|')) {
            assert_eq!(s.matches('|').count(), 4, "column shift: {s}");
        }
    }

    #[test]
    fn a_pipe_inside_a_cell_is_escaped() {
        let t = Table::new(["A", "B"], vec![vec!["x|y".into(), "z\nw".into()]]);
        let md = t.markdown_truncated(5);
        assert!(md.contains("x\\|y"), "{md}");
        assert!(!md.contains("z\nw"));
        for s in md.lines().filter(|s| s.starts_with('|')) {
            // An escaped pipe must not break the column count: 3 pipes once \| is
            // not counted.
            assert_eq!(s.replace("\\|", "").matches('|').count(), 3, "{s}");
        }
    }

    #[test]
    fn a_table_summary_returns_markdown() {
        let mut store = DataStore::new();
        let r = store.put(Value::Table(sample_table(50)));
        assert_eq!(r.as_str(), "table#1");
        let summary = store.summary(&r, 2);
        assert!(summary.starts_with("| Name |"));
        assert_eq!(summary.lines().filter(|s| s.starts_with('|')).count(), 4);
    }

    #[test]
    fn byte_content_never_leaks() {
        let mut store = DataStore::new();
        // The bytes carry a READABLE signature: had the summary leaked them we
        // would see this string in the output by eye.
        let r = store.put(Value::Bytes(vec![b'S', b'E', b'C', b'R']));
        let summary = store.summary(&r, 1000);
        assert_eq!(summary, "4 bytes of binary data");
        assert!(!summary.contains("SECR"));
    }

    #[test]
    fn an_unknown_ref_returns_an_explicit_sentence() {
        let store = DataStore::new();
        let summary = store.summary(&SourceRef("missing#9".into()), 3);
        assert!(summary.contains("not found"));
    }

    #[test]
    fn clear_does_not_reset_the_counter() {
        let mut store = DataStore::new();
        let old = store.put(Value::Text("a".into()));
        store.clear();
        let new = store.put(Value::Text("b".into()));
        assert_ne!(old, new, "an old ref must not land on a new record");
        assert!(store.take(&old).is_none());
    }

    #[test]
    fn labelled_puts_are_filtered_by_of_kind() {
        let mut store = DataStore::new();
        store.put_labelled("calendar", Value::Text("c1".into()));
        store.put_labelled("document", Value::Text("d1".into()));
        let r = store.put_labelled("calendar", Value::Text("c2".into()));
        assert_eq!(store.of_kind("calendar").len(), 2);
        assert_eq!(r.as_str(), "calendar#3");
        assert_eq!(store.kind(&r), Some("calendar"));
    }

    #[test]
    fn the_shared_store_satisfies_the_core_contract() {
        let store = SharedStore::new();
        let d: &dyn CoreDataStore = &store;
        let r = d.put("document", "short", "one\ntwo\nthree".to_string());
        let record = d.take(&r).expect("record");
        assert_eq!(record.kind, "document");
        assert_eq!(record.body, "one\ntwo\nthree");
        assert_eq!(d.of_kind("document").len(), 1);
        d.clear();
        assert!(d.take(&r).is_none());
    }

    #[test]
    fn the_shared_store_keeps_a_table_typed() {
        let store = SharedStore::new();
        let r = store.put_value("report", Value::Table(sample_table(9)));
        assert!(matches!(store.value(&r), Some(Value::Table(_))));
        assert!(store.summary(&r, 2).contains("(+7 more rows"));
    }
}
