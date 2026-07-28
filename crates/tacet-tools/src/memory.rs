//! `MemoryTool` — how the model gets to say "remember this" / "forget that".
//!
//! WHY A TOOL: the recall READ path is MODEL-FREE (the code decides via
//! `matching`), but the WRITE path needs the user's intent. Saving the sentence
//! "I am a vegetarian" on its own initiative is "silent learning"; the user
//! saying "don't forget this" is an explicit request and maps to a tool call. The
//! second benefit of it being a tool is transparency: every save leaves a CHIP on
//! screen, i.e. what went into memory cannot be hidden from the user.
//!
//! IT TAINTS: notes are personal data outright. Once read, the session counts as
//! tainted and every call that would send data out hits the approval gate.

use std::sync::{Arc, Mutex};
use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolState,
    TraceUpdate, boxed,
};
use tacet_memory::{MemoryError, MemoryKind, MemoryStore};

/// The limit on the note text shown in the chip; a chip is one line.
const CHIP_TEXT_LIMIT: usize = 40;

/// The wrapper that shares the memory store between the tools and the shell.
///
/// The same pattern as `SharedStore`: ownership in one place, access via a
/// `Mutex`. Not global state — the shell decides who sees which store, so eval
/// can run with an in-memory store without touching the real home directory.
#[derive(Clone)]
pub struct SharedMemory(Arc<Mutex<MemoryStore>>);

impl SharedMemory {
    pub fn new(store: MemoryStore) -> Self {
        Self(Arc::new(Mutex::new(store)))
    }

    /// A store that never touches the disk (test/eval).
    pub fn in_memory() -> Self {
        Self::new(MemoryStore::in_memory())
    }

    /// Takes the lock and runs `job`. If the lock is POISONED (another thread
    /// panicked) it returns `None`: an `unwrap` inside a tool takes the whole turn
    /// with it, whereas carrying on without memory is a legitimate degradation.
    pub fn with<T>(&self, job: impl FnOnce(&mut MemoryStore) -> T) -> Option<T> {
        self.0.lock().ok().map(|mut s| job(&mut s))
    }
}

pub struct MemoryTool {
    store: SharedMemory,
}

impl MemoryTool {
    pub fn new(store: SharedMemory) -> Self {
        Self { store }
    }
}

impl Tool for MemoryTool {
    fn name(&self) -> &str {
        "remember"
    }

    fn description(&self) -> &str {
        // CAME FROM MEASUREMENT — THE QUOTED VERBS WERE REMOVED. The old text said
        // "call this when the user says 'remember this' / 'forget that'", and the
        // model produced a `hatirla(...)` call for the message "Remember this: ..."
        // and a `list(...)` call for "list the notes" — that is, it took the
        // quoted word for THE TOOL NAME. Both are VALUES of the `action` argument;
        // the text now says so explicitly and leaves no verb standing in tool-name
        // position.
        "Saves, deletes or lists a lasting fact the user stated about themselves. Call this \
         ONLY when the user explicitly asks you to remember or to forget something, in any \
         language; never mine ordinary conversation for notes on your own. Which of the three \
         it is goes in the 'action' argument (save, forget, list) - those are argument values, \
         not tool names."
    }

    fn schema(&self) -> ArgSchema {
        // THE SCHEMA IS THE MODEL'S BOUNDARY: the grammar turns it into a
        // constraint one-to-one.
        ArgSchema::object(vec![
            Field::new(
                "action",
                ArgSchema::choice(["save", "forget", "list"]).description(
                    "save: store a new fact | forget: delete | list: count the records",
                ),
            )
            .required(),
            Field::new(
                "text",
                ArgSchema::text().description(
                    "save: a one-sentence fact (example: The user is a vegetarian.) | \
                     forget: a phrase describing the fact to delete.",
                ),
            ),
            Field::new(
                "keywords",
                ArgSchema::text().description(
                    "for save: 2-4 comma-separated keywords. Example: food, restaurant",
                ),
            ),
            Field::new(
                "kind",
                ArgSchema::choice(["identity", "preference", "relation", "fact"])
                    .description("The kind of fact; if unsure, fact."),
            ),
        ])
    }

    /// Carries personal data — taints the session.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            // The grammar may be disabled; the gate stands here too.
            if let Err(error) = self.schema().validate(&args) {
                return ToolOutcome::failed(&error);
            }
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();

            let trace = ctx.start_chip("~", "Memory");

            let outcome = match action {
                "save" => self.save(&args, text),
                "forget" => self.forget(text),
                "list" => self.list(ctx),
                // The schema gate already prevents falling here; even so, no
                // `unreachable!` was written — a tool must not take the turn down
                // with a panic.
                _ => ToolOutcome::failed(&ToolError::InvalidArgument("unknown action".into())),
            };

            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone()).text(outcome.chip_text.clone()),
            );
            // TAINTING only on a SUCCESSFUL call: a failed call put no personal
            // data into the context, and tainting the session for it would be
            // wrong.
            if !matches!(outcome.state, ToolState::Failed(_)) {
                ctx.taint();
            }
            outcome
        })
    }
}

impl MemoryTool {
    fn save(&self, args: &serde_json::Value, text: &str) -> ToolOutcome {
        let kind = args
            .get("kind")
            .and_then(|v| v.as_str())
            .and_then(MemoryKind::resolve)
            .unwrap_or_default();
        let keywords: Vec<String> = args
            .get("keywords")
            .and_then(|v| v.as_str())
            .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
            .unwrap_or_default();

        // The returned type is `Result<(), String>`: both a filter rejection and a
        // disk failure reach the user through THE SAME channel (a Turkish
        // sentence), but their reasons are written separately — "I already know
        // this" and "could not be written" are not the same thing.
        let Some(result) = self.store.with(|s| -> Result<(), String> {
            let id = s
                .add(text, kind, &keywords)
                .map_err(|e: MemoryError| e.short_error().to_string())?;
            // A disk error ROLLS THE NOTE BACK: a note held in memory but not
            // written to disk would silently vanish on the next launch — we would
            // have said "remembered" and it would have been a lie. The raw io text
            // is NOT SHOWN to the user.
            if s.save().is_err() {
                s.delete(id);
                return Err("The note could not be saved.".to_string());
            }
            Ok(())
        }) else {
            return ToolOutcome::failed(&ToolError::Other(
                "Memory cannot be reached right now.".into(),
            ));
        };

        match result {
            Ok(()) => {
                ToolOutcome::written(format!("Note taken · {}", truncate(text)), "note saved")
            }
            Err(reason) => ToolOutcome::failed(&ToolError::Other(reason)),
        }
    }

    fn forget(&self, phrase: &str) -> ToolOutcome {
        if phrase.is_empty() {
            // An empty phrase would match ALL notes; in a delete tool that is
            // unacceptable, so the gate is here.
            return ToolOutcome::failed(&ToolError::MissingField("arg.text".into()));
        }
        let Some(deleted) = self.store.with(|s| {
            let n = s.forget(phrase);
            if n > 0 {
                let _ = s.save();
            }
            n
        }) else {
            return ToolOutcome::failed(&ToolError::Other(
                "Memory cannot be reached right now.".into(),
            ));
        };

        if deleted == 0 {
            return ToolOutcome::read_ok("No note to forget was found.", "no matching note");
        }
        ToolOutcome::written(format!("{deleted} notes forgotten"), "note removed")
    }

    /// Lists the notes — THROUGH THE BYPASS CHANNEL.
    ///
    /// Fifty notes are not dumped on the model: the body goes into the
    /// `DataStore` and the model gets a count plus a `source_ref`. Small as the
    /// list may look, it is personal data, and that is exactly why the channel
    /// exists.
    fn list(&self, ctx: &ToolContext) -> ToolOutcome {
        let Some((count, body)) = self.store.with(|s| {
            let body = s
                .notes()
                .iter()
                .map(|n| {
                    format!(
                        "- [{}] {} ({}){}",
                        n.kind.name(),
                        n.text,
                        n.summary(),
                        if n.active { "" } else { " [inactive]" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            (s.count(), body)
        }) else {
            return ToolOutcome::failed(&ToolError::Other(
                "Memory cannot be reached right now.".into(),
            ));
        };

        if count == 0 {
            return ToolOutcome::read_ok("There are no notes in memory.", "memory is empty");
        }
        let r = ctx.store("memory", &format!("{count} not"), body);
        ToolOutcome::summarize(
            format!("{count} not"),
            format!("{count} notes stored"),
            r.as_str(),
        )
    }
}

/// Shortens the note text so the chip stays on one line.
fn truncate(text: &str) -> String {
    if text.chars().count() <= CHIP_TEXT_LIMIT {
        return text.to_string();
    }
    let head: String = text.chars().take(CHIP_TEXT_LIMIT - 1).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_store::SharedStore;
    use serde_json::json;
    use tacet_kernel::{DataStore, Reporter, TraceCollector};

    fn context(store: Arc<SharedStore>, reporter: Arc<dyn Reporter>) -> ToolContext {
        ToolContext::new(store, "/tmp/tacet-memory-tool", reporter)
    }

    fn setup() -> (MemoryTool, SharedMemory, Arc<SharedStore>, ToolContext) {
        let memory = SharedMemory::in_memory();
        let tool = MemoryTool::new(memory.clone());
        let store = Arc::new(SharedStore::new());
        let ctx = context(store.clone(), Arc::new(TraceCollector::new()));
        (tool, memory, store, ctx)
    }

    #[test]
    fn save_stores_the_note_and_changes_the_world() {
        let (tool, memory, _, mut ctx) = setup();
        let o = execute(tool.run(
            json!({"action":"save","text":"The user is a vegetarian.",
                   "keywords":"food, restaurant","kind":"preference"}),
            &mut ctx,
        ));
        assert_eq!(o.state, ToolState::Written, "a save changes the world");
        assert!(o.chip_text.starts_with("Note taken"));
        assert_eq!(o.to_model, "note saved", "short and English to the model");
        assert_eq!(memory.with(|s| s.count()), Some(1));
        assert!(ctx.session_tainted(), "memory carries personal data");
    }

    #[test]
    fn a_short_note_is_rejected_and_the_session_is_not_tainted() {
        let (tool, memory, _, mut ctx) = setup();
        let o = execute(tool.run(
            json!({"action":"save","text":"short","keywords":"a"}),
            &mut ctx,
        ));
        assert!(matches!(o.state, ToolState::Failed(_)));
        assert_eq!(o.to_model, tacet_kernel::ERROR_MODEL_TEXT);
        assert_eq!(memory.with(|s| s.count()), Some(0));
        assert!(
            !ctx.session_tainted(),
            "a failed call does not taint the session"
        );
    }

    #[test]
    fn forget_deletes_the_note_and_rejects_an_empty_phrase() {
        let (tool, memory, _, mut ctx) = setup();
        execute(tool.run(
            json!({"action":"save","text":"The user is a vegetarian.","keywords":"food"}),
            &mut ctx,
        ));
        // An empty phrase would match ALL notes: the gate must hold.
        let empty = execute(tool.run(json!({"action":"forget","text":"  "}), &mut ctx));
        assert!(matches!(empty.state, ToolState::Failed(_)));
        assert_eq!(memory.with(|s| s.count()), Some(1));

        let o = execute(tool.run(json!({"action":"forget","text":"vegetarian"}), &mut ctx));
        assert_eq!(o.state, ToolState::Written);
        assert_eq!(memory.with(|s| s.count()), Some(0));
    }

    #[test]
    fn forget_says_so_honestly_when_nothing_matches() {
        let (tool, _, _, mut ctx) = setup();
        let o = execute(tool.run(
            json!({"action":"forget","text":"a thing that does not exist"}),
            &mut ctx,
        ));
        assert_eq!(
            o.state,
            ToolState::Read,
            "nothing deleted means the world did not change"
        );
        assert_eq!(o.to_model, "no matching note");
    }

    #[test]
    fn list_goes_through_the_bypass_channel() {
        let (tool, _, store, mut ctx) = setup();
        for i in 0..5 {
            execute(tool.run(
                json!({"action":"save","text":format!("The user stated fact number {i}."),
                       "keywords":"fact"}),
                &mut ctx,
            ));
        }
        let o = execute(tool.run(json!({"action":"list"}), &mut ctx));
        // Bulk data DOES NOT GO TO THE MODEL: a short summary + source_ref.
        assert!(o.to_model.contains("source_ref"));
        assert!(o.to_model.len() < 80, "{}", o.to_model);
        assert!(!o.to_model.contains("numbered fact"));
        // The body sits COMPLETE in the store.
        let r = tacet_kernel::SourceRef(
            o.to_model
                .split("source_ref=")
                .nth(1)
                .unwrap()
                .trim_end_matches(')')
                .trim()
                .to_string(),
        );
        assert!(store.take(&r).unwrap().body.contains("fact number 4"));
    }

    #[test]
    fn list_on_an_empty_memory_stores_no_body() {
        let (tool, _, _, mut ctx) = setup();
        let o = execute(tool.run(json!({"action":"list"}), &mut ctx));
        assert_eq!(o.to_model, "memory is empty");
        assert!(!o.to_model.contains("source_ref"));
    }

    #[test]
    fn the_schema_rejects_an_argument_that_does_not_match() {
        let (tool, _, _, mut ctx) = setup();
        // Required field missing.
        assert!(matches!(
            execute(tool.run(json!({}), &mut ctx)).state,
            ToolState::Failed(_)
        ));
        // An action outside the enum.
        assert!(matches!(
            execute(tool.run(json!({"action":"delete-all"}), &mut ctx)).state,
            ToolState::Failed(_)
        ));
        // An invented key must not be an escape hatch.
        let js = tool.schema().json_schema();
        assert_eq!(js["additionalProperties"], json!(false));
        assert_eq!(js["required"], json!(["action"]));
    }

    #[test]
    fn the_tool_declares_its_tainting_flag() {
        let (tool, _, _, _) = setup();
        assert!(
            tool.taints_session(),
            "personal data must hit the approval gate"
        );
        assert_eq!(tool.name(), "remember");
    }

    #[test]
    fn a_long_note_is_truncated_in_the_chip() {
        let (tool, _, _, mut ctx) = setup();
        let long =
            "The user described a very long fact in a single sentence and the sentence went on.";
        let o = execute(tool.run(
            json!({"action":"save","text":long,"keywords":"fact"}),
            &mut ctx,
        ));
        assert!(o.chip_text.chars().count() < 60, "{}", o.chip_text);
        assert!(o.chip_text.contains('…'));
    }

    #[test]
    fn the_same_fact_is_not_stored_twice() {
        let (tool, memory, _, mut ctx) = setup();
        let call = json!({"action":"save","text":"The user is a vegetarian.","keywords":"food"});
        execute(tool.run(call.clone(), &mut ctx));
        let second = execute(tool.run(call, &mut ctx));
        assert!(matches!(second.state, ToolState::Failed(_)));
        assert_eq!(memory.with(|s| s.count()), Some(1));
    }

    /// Core has no tokio; a minimal executor that is enough for tests (the same
    /// pattern as in calc.rs).
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
}
