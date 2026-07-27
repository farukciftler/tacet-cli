//! tacet-memory — the persistent context layer: user preferences and facts.
//!
//! In cloud assistants memory is a privacy concern; here it IS the brand:
//! **memory that never leaves the device**. The file sits in the config
//! directory (`memory.json`), plain JSON, 0600. No network surface at all.
//!
//! ## Core decision: EXTRACTION TO THE MODEL, RECALL TO THE CODE
//!
//! How a note is born (text, kind, keys) may be asked of the model — but which
//! note enters which turn is decided by DETERMINISTIC CODE
//! (`MemoryStore::matching`). That way the same message always brings back the
//! same note and the behaviour stays testable.
//!
//! ## Principles
//!
//! 1. **No silent learning.** Every stored note can be listed, updated and
//!    deleted. When the cap fills there is NO AUTOMATIC EVICTION — deleting
//!    silently is the mirror image of learning silently, and just as forbidden.
//! 2. **The budget is sacred.** Injection has a HARD cap of `INJECTION_LIMIT`
//!    (600 characters, ~200 tokens), and it is accounted for that it can land
//!    in the same turn as a skill injection (700). A design that bakes all of
//!    memory into every prompt kills the 4096 context.
//! 3. **Few but right.** Ten correct notes beat fifty noisy ones; the filters
//!    are aggressive and the caps are low. When in doubt, do not store.
//!
//! ## Work left to the shell
//!
//! This crate DOES NOT BUILD THE PROMPT. The shell phase filters the
//! `store.injection_text(message)` output (if any) through `NoteWatcher` and
//! places it at the head of that turn's prompt.

pub mod note;
pub mod store;

pub use note::{
    KEY_LIMIT, MIN_TEXT, MemoryKind, MemoryNote, TEXT_LIMIT, TOTAL_CAP, fix_keys,
};
pub use store::{INJECTION_LIMIT, MAX_NOTES, MemoryError, MemoryStore, NoteWatcher};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_to_end_remember_recall_forget() {
        // This layer's contract: put a fact in -> get it back on a relevant
        // message -> have it gone when "forget" is said.
        let mut store = MemoryStore::in_memory();
        store
            .add(
                "The user is a vegetarian.",
                MemoryKind::Preference,
                &["food".to_string(), "restaurant".to_string()],
            )
            .unwrap();

        let prompt = store
            .injection_text("suggest a restaurant for tonight")
            .unwrap();
        assert!(prompt.contains("The user is a vegetarian."));
        assert!(prompt.chars().count() <= INJECTION_LIMIT);

        assert_eq!(store.forget("vegetarian"), 1);
        assert!(
            store
                .injection_text("suggest a restaurant for tonight")
                .is_none()
        );
    }

    #[test]
    fn an_irrelevant_message_eats_nothing_from_the_budget() {
        let mut store = MemoryStore::in_memory();
        for i in 0..TOTAL_CAP {
            store
                .add(
                    &format!("The user stated fact number {i}."),
                    MemoryKind::Fact,
                    &["fact".to_string()],
                )
                .unwrap();
        }
        // 50 notes are stored but the message matches none of them: the prompt
        // MUST NOT GROW.
        assert!(store.injection_text("what is two times two").is_none());
    }
}
