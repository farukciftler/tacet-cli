//! DataStore — the heart of the 4096-token bypass channel.
//!
//! The rule: bulk device data DOES NOT PASS THROUGH THE MODEL. The tool puts
//! the data here and returns only a short summary + a `source_ref` to the
//! model. Whoever needs the data in the next step is again a TOOL; it takes the
//! data out of the store by reference. That way 100 calendar records or a
//! 40-page document can be processed without inflating the context.
//!
//! It is defined as a trait because the concrete store (memory, disk,
//! encrypted) will change in later phases; tools must know nothing beyond this
//! contract.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// The address of a record in the store. This is the ONLY piece of data that
/// goes to the model; it carries no hint about its content (only kind + ordinal,
/// so no personal data leaks).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceRef(pub String);

impl SourceRef {
    /// The reference as written — this is the exact text the model sees.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SourceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A single record put into the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    /// Its address in the store, and the only part of it the model is given.
    pub source_ref: SourceRef,
    /// A coarse kind label ("calendar", "document", "file") — used when producing
    /// the source_ref and when tools filter.
    pub kind: String,
    /// A short summary that is SAFE to show the model ("12 events, 3 days").
    pub summary: String,
    /// The actual body. Whatever is put here is never handed to the model directly.
    pub body: String,
}

/// THE BYPASS CHANNEL. Bulk data goes in here and a short summary goes to the
/// model, so a 40 000-row spreadsheet costs a sentence of context rather than a
/// window. The next tool that needs the body fetches it by reference.
pub trait DataStore: Send + Sync {
    /// Stores the data and returns its reference.
    fn put(&self, kind: &str, summary: &str, body: String) -> SourceRef;
    /// The record behind a reference, if the store still holds it. `None` for a
    /// reference the model invented, which is the case that matters.
    fn take(&self, source_ref: &SourceRef) -> Option<Record>;
    /// The records of the given kind, in insertion order.
    fn of_kind(&self, kind: &str) -> Vec<Record>;
    /// Called when the chat is reset — session-scoped data must not spill out.
    fn clear(&self);
}

/// The default concrete store: process-scoped, in memory. It writes nothing to
/// disk — device data disappears by itself when the process ends.
#[derive(Default)]
pub struct InMemoryDataStore {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    records: Vec<Record>,
    counter: u64,
}

impl InMemoryDataStore {
    /// An empty in-memory store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl DataStore for InMemoryDataStore {
    fn put(&self, kind: &str, summary: &str, body: String) -> SourceRef {
        let mut inner = self.inner.lock().expect("data store lock");
        inner.counter += 1;
        let source_ref = SourceRef(format!("{kind}#{}", inner.counter));
        let record = Record {
            source_ref: source_ref.clone(),
            kind: kind.to_string(),
            summary: summary.to_string(),
            body,
        };
        inner.records.push(record);
        source_ref
    }

    fn take(&self, source_ref: &SourceRef) -> Option<Record> {
        let inner = self.inner.lock().expect("data store lock");
        inner
            .records
            .iter()
            .find(|r| &r.source_ref == source_ref)
            .cloned()
    }

    fn of_kind(&self, kind: &str) -> Vec<Record> {
        let inner = self.inner.lock().expect("data store lock");
        inner
            .records
            .iter()
            .filter(|r| r.kind == kind)
            .cloned()
            .collect()
    }

    fn clear(&self) {
        let mut inner = self.inner.lock().expect("data store lock");
        inner.records.clear();
        // The counter is NOT RESET: if an old source_ref is still held
        // somewhere, it must not land on a new record and open the wrong data.
    }
}
