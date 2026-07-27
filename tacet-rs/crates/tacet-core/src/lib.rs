//! tacet-core — the contract layer of the architecture.
//!
//! NO WORK IS DONE in this crate; only the types everyone agrees on live here:
//! what a tool is (`Tool`), how it describes its arguments (`ArgSchema`), what
//! it returns (`ToolOutcome`), how it fails (`ToolError`) and how it keeps bulk
//! data AWAY from the model (`DataStore`).
//!
//! Deliberately independent: it looks neither at the file system, nor at OOXML,
//! nor at the model runner. Every crate above depends on this one, this one
//! depends on none of them — so the contract does not bend under the pressure
//! of the implementations.
//!
//! The `env` module does NOT BREAK that rule: where the configuration lives is
//! a contract too (memory, skills and MCP have to point at the same directory),
//! and that module does not touch the file system — it only COMPUTES a path
//! from environment variables.
//!
//! NO NETWORK: nowhere in this crate, or below it, is there a network call.

pub mod catalog;
pub mod context;
pub mod data_store;
pub mod env;
pub mod error;
pub mod outcome;
pub mod reporter;
pub mod schema;
pub mod state;
pub mod tool;

pub use catalog::ToolCatalog;
pub use context::ToolContext;
pub use data_store::{DataStore, InMemoryDataStore, Record, SourceRef};
pub use env::{config_dir, config_path, env_var};
pub use error::{ERROR_MODEL_TEXT, ToolError, ToolResult};
pub use outcome::{ToolOutcome, source_ref_suffix};
pub use reporter::{Reporter, SilentReporter, ToolTrace, TraceCollector, TraceId, TraceUpdate};
pub use schema::{ArgSchema, Field, SchemaKind};
pub use state::ToolState;
pub use tool::{Tool, ToolFuture, boxed};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    /// Proves the dyn-compatibility of the contract at the TEST level: if this
    /// tool compiles, then `Vec<Arc<dyn Tool>>` compiles too.
    struct FakeTool;

    impl Tool for FakeTool {
        fn name(&self) -> &str {
            "fake_search"
        }
        fn description(&self) -> &str {
            "For testing; produces many records."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![
                Field::new(
                    "query",
                    ArgSchema::text().description("the text to search for"),
                )
                .required(),
                Field::new("scope", ArgSchema::choice(["all", "near"])),
                Field::new("count", ArgSchema::integer().range(Some(1.0), Some(50.0))),
            ])
        }
        fn taints_session(&self) -> bool {
            true
        }
        fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move {
                if let Err(e) = self.schema().validate(&args) {
                    return ToolOutcome::failed(&e);
                }
                // Bypass channel: the body stays in the store, a summary + a
                // reference go to the model.
                let r = ctx.store("fake", "3 records", "a very long body".repeat(100));
                ctx.taint();
                ToolOutcome::summarize("3 records found", "3 records", r.as_str())
            })
        }
    }

    #[test]
    fn catalog_holds_dyn_and_finds_by_name() {
        let mut c = ToolCatalog::new();
        c.add(Arc::new(FakeTool));
        assert!(c.find("fake_search").is_some());
        assert!(c.find("nonexistent").is_none());
        assert_eq!(c.tainting_tools(), vec!["fake_search"]);
    }

    #[test]
    fn schema_validates_required_field_and_choice() {
        let s = FakeTool.schema();
        assert!(s.validate(&json!({"query": "a"})).is_ok());
        assert!(s.validate(&json!({})).is_err());
        assert!(s.validate(&json!({"query": "a", "scope": "far"})).is_err());
        assert!(s.validate(&json!({"query": "a", "count": 99})).is_err());
        assert!(s.validate(&json!({"query": 5})).is_err());
    }

    #[test]
    fn schema_json_schema_and_serde_round_trip() {
        let s = FakeTool.schema();
        let js = s.json_schema();
        assert_eq!(js["type"], "object");
        assert_eq!(js["required"], json!(["query"]));
        let text = serde_json::to_string(&s).unwrap();
        let back: ArgSchema = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn error_returns_fixed_english_to_the_model() {
        let e = ToolError::FileNotFound("/a/b".into());
        let outcome = ToolOutcome::failed(&e);
        assert_eq!(outcome.to_model, ERROR_MODEL_TEXT);
        assert_eq!(outcome.chip_text, "File not found.");
        assert!(matches!(outcome.state, ToolState::Failed(_)));
    }

    #[test]
    fn sandbox_blocks_stepping_outside() {
        let ctx = ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            "/tmp/tacet",
            Arc::new(SilentReporter),
        );
        assert!(ctx.resolve_path("a/b.txt").is_ok());
        assert!(ctx.resolve_path("a/../b.txt").is_ok());
        assert!(ctx.resolve_path("../outside.txt").is_err());
        assert!(ctx.resolve_path("/etc/passwd").is_err());
    }

    #[test]
    fn bypass_channel_keeps_the_body_away_from_the_model() {
        let mut ctx = ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            "/tmp/tacet",
            Arc::new(TraceCollector::new()),
        );
        let outcome = block_on(FakeTool.run(json!({"query": "test"}), &mut ctx));
        assert!(
            outcome.to_model.len() < 80,
            "the text sent to the model must be short"
        );
        assert!(outcome.to_model.contains("source_ref"));
        assert!(ctx.session_tainted());
        let r = SourceRef("fake#1".into());
        assert!(ctx.from_store(&r).unwrap().body.len() > 1000);
    }

    /// There is no tokio dependency in the core; the minimum executor that is
    /// enough for a test.
    fn block_on<F: std::future::Future>(mut f: F) -> F::Output {
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
}
