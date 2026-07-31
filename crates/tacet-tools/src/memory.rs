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
        // AND THE SECOND MEASUREMENT, in two languages at once. "Remember this:
        // I drink my coffee without milk." and "Kahve sevdiğimi unut artık" both
        // got an answer and NO CALL: the model wrote "I'll remember that" and
        // "unuttum, bu bilgiyi kaydettim" — it claimed to have done the thing.
        // Both failed in every single run of both suites.
        //
        // The tool that does not have this problem is `time`, and the reason is
        // the sentence "You do NOT know the current time or date on your own;
        // without this tool any answer you give is a guess." It denies the model
        // a capability it does not have, in the second person, before saying
        // what to call. Memory needs exactly the same sentence, because the
        // failure is exactly the same: a model that believes it can remember
        // has no reason to call anything.
        "Saves, deletes or lists a lasting fact the user stated about themselves. YOU HAVE NO \
         MEMORY OF YOUR OWN: nothing you say you will remember is remembered, and nothing you \
         say you have forgotten is forgotten, unless this tool is called - saying 'I will \
         remember that' without calling it is telling the user something untrue. Call this \
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
        // NO KEYWORDS IS NOT A REFUSAL ANY MORE — it is a job this tool does.
        //
        // THE INCONSISTENCY, found by the first eval case ever written for this
        // tool and then reproduced by a real model: the schema marks `keywords`
        // OPTIONAL, so the grammar cheerfully produces `remember({"action":
        // "save","text":"..."})` — and `MemoryStore::add` refuses it with
        // `NoKeys`. A model that followed the schema to the letter got a
        // GUARANTEED failure, and the user was told their note could not be
        // saved for a field they were never required to give.
        //
        // WHY DERIVE RATHER THAN MARK THE FIELD REQUIRED: the schema is ONE
        // object for three actions, and `list` needs neither text nor keys.
        // Making the field required would force keywords onto calls that have
        // nothing to key. The store's rule exists for RECALL — a note nobody can
        // find is a note nobody kept — and that is a need this tool can meet on
        // the model's behalf.
        //
        // THE DERIVED KEYS ARE A FLOOR, NOT A REPLACEMENT: `keywords` given by
        // the model always wins, because the model saw the sentence in context
        // and the fallback only sees its words.
        let keywords = if keywords.iter().any(|k| !k.trim().is_empty()) {
            keywords
        } else {
            keys_from(text)
        };

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

/// The shortest word a derived keyword may be.
///
/// Three is where the function words of both languages this ships in stop being
/// counted: `the`, `and`, `for`, `bir`, `ile`, `bu`. It is a crude filter and it
/// is meant to be — a derived key only has to make the note FINDABLE, and the
/// model's own keywords always win when it gives them.
const MIN_DERIVED_KEY: usize = 4;

/// Keywords taken from the note itself, for a `save` that arrived without any.
///
/// See the call site for why this exists at all. The rules are deliberately
/// small: split on anything that is not a letter or a digit, drop what is
/// shorter than `MIN_DERIVED_KEY`, lowercase, keep the first few in the order
/// they were written. Word ORDER is the ranking — the subject of a one-sentence
/// fact comes early, and a fact is what this tool stores.
///
/// `fix_keys` in `tacet-memory` still has the last word: it lowercases, dedupes
/// and caps the list. This function only has to produce candidates.
fn keys_from(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= MIN_DERIVED_KEY)
        .map(|w| w.to_lowercase())
        .take(4)
        .collect()
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

    /// A SAVE WITHOUT KEYWORDS MUST STILL SAVE.
    ///
    /// The schema marks the field optional and the store demanded it: a model
    /// that read the schema correctly got a guaranteed failure. Reproduced by
    /// qwen3-4b on the `remember-fact` eval case before this existed.
    #[test]
    fn a_save_with_no_keywords_is_still_stored() {
        let (tool, memory, _, mut ctx) = setup();
        let o = execute(tool.run(
            serde_json::json!({
                "action": "save",
                "text": "the user's sister is called Ayse"
            }),
            &mut ctx,
        ));
        assert_eq!(o.state, ToolState::Written, "{}", o.chip_text);
        assert!(o.to_model.contains("note saved"), "{}", o.to_model);
        assert_eq!(
            memory.with(|s| s.count()),
            Some(1),
            "the note did not reach the store"
        );
    }

    /// THE MODEL'S OWN KEYWORDS WIN. The fallback is a floor, not a rewrite:
    /// the model saw the sentence in context, this function only sees words.
    #[test]
    fn given_keywords_are_not_replaced_by_derived_ones() {
        let (tool, memory, _, mut ctx) = setup();
        execute(tool.run(
            serde_json::json!({
                "action": "save",
                "text": "the user's sister is called Ayse",
                "keywords": "family, sibling"
            }),
            &mut ctx,
        ));
        let keys = memory
            .with(|s| s.notes().first().map(|n| n.keys.clone()))
            .flatten()
            .expect("a note");
        assert!(keys.contains(&"family".to_string()), "{keys:?}");
        assert!(!keys.contains(&"sister".to_string()), "{keys:?}");
    }

    #[test]
    fn short_words_are_not_keys_and_the_list_is_capped() {
        // `the`, `is` and `a` are below the floor; the rest survive in order.
        assert_eq!(
            keys_from("the user is a vegetarian and dislikes mushrooms entirely"),
            vec!["user", "vegetarian", "dislikes", "mushrooms"]
        );
        // Nothing long enough: an empty list, and `add` still refuses — which is
        // correct, a note of only function words is a note nobody can find.
        assert!(keys_from("bu bir ile").is_empty());
    }
}
