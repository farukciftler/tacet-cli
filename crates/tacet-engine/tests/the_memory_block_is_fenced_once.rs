//! The memory block must be fenced once.
//!
//! MEASURED IN THE SHIPPED PROMPT. `MemoryStore::injection_text` returns its
//! notes ALREADY fenced — its budget arithmetic subtracts the fence characters
//! before it starts fitting notes, which is why the store owns it — and the
//! shell feeds that straight into `Prompt::with_memory`, which fenced it again.
//! So on every turn a note matched, the model received a nested `<memory>` with
//! a stray closing tag in the middle of it, in the SYSTEM block:
//!
//!     <memory>
//!     <memory>
//!     - the user is vegetarian
//!     </memory>
//!     Use it.
//!     </memory>
//!
//! Nothing failed and nothing said so. It is the kind of defect that appears
//! only when somebody prints the prompt and reads it, which is why it lasted.
//!
//! Both shapes have to work: the store's output, and raw notes from a caller
//! that is not the shell.

use tacet_engine::{Prompt, Template};

/// What `MemoryStore::injection_text` actually returns — fence, notes, and the
/// sentence that tells the model what the block is for.
const FROM_THE_STORE: &str =
    "<memory>\n- the user is vegetarian\n</memory>\nThese are notes about the user.";

fn memory_prompt(text: &str, template: Template) -> String {
    Prompt::new("sys", "what shall we eat")
        .with_memory(text)
        .text_with_template(template)
}

#[test]
fn the_stores_own_output_is_not_fenced_twice() {
    for template in [Template::Plain, Template::ChatML, Template::Gemma] {
        let out = memory_prompt(FROM_THE_STORE, template);
        assert_eq!(
            out.matches("<memory>").count(),
            1,
            "{template:?} wrapped the store's block a second time:\n{out}"
        );
        assert_eq!(
            out.matches("</memory>").count(),
            1,
            "{template:?} left a stray closing tag:\n{out}"
        );
        assert!(
            out.contains("the user is vegetarian"),
            "{template:?} lost the note:\n{out}"
        );
        assert!(
            out.contains("These are notes about the user"),
            "{template:?} lost the sentence that says what the block is:\n{out}"
        );
    }
}

/// `with_memory` is public and a caller that is not the shell may hand it raw
/// notes. Those must still be fenced, or they read as loose text in the system
/// block.
#[test]
fn raw_notes_are_still_fenced() {
    let out = memory_prompt("- the user prefers dark mode", Template::Plain);
    assert_eq!(out.matches("<memory>").count(), 1, "{out}");
    assert_eq!(out.matches("</memory>").count(), 1, "{out}");
    assert!(out.contains("the user prefers dark mode"), "{out}");
}

/// The defang must not eat the fence the store wrote. `<memory>` is not a turn
/// marker and is deliberately not in `CONTROL_SEQUENCES`; if it is ever added,
/// this is what says the store's block stops rendering.
#[test]
fn the_defang_leaves_the_memory_fence_alone() {
    let out = memory_prompt(FROM_THE_STORE, Template::Plain);
    assert!(
        !out.contains("< memory>"),
        "the fence was neutralised and the block is now loose text:\n{out}"
    );
}
