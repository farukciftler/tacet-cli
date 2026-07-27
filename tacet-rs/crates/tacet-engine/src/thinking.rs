//! Stripping thinking (chain-of-thought) blocks out of the answer.
//!
//! WHY THIS IS NEEDED: the HYBRID Qwen3 builds (8B/14B) emit a
//! `<think>...</think>` block by default. That block is the model's own inner
//! reasoning, NOT the answer meant for the user — and worse, the model does its
//! inner reasoning in English even while answering in another language, so
//! foreign-language text spills onto the screen.
//!
//! We build two layers of defence, because neither is enough on its own:
//!   1. `prompt.rs` pre-writes an empty `<think></think>` pair on the
//!      generation anchor (Qwen3's official `enable_thinking=false` path) — the
//!      model never starts thinking at all. That is the cheap, correct fix.
//!   2. This module strips the block if it shows up anyway. Layer 1 relies on a
//!      CONTRACT that varies from model to model; layer 2 looks at the text
//!      itself.
//!
//! The stripped thinking is NOT DISCARDED, it lives in `Generation.thinking`:
//! it is valuable for diagnosis (the model explains there why it picked that
//! tool) and a "show thinking" mode could be added later. Separating rather
//! than silently destroying is also consistent with the chip principle: what is
//! shown is what actually happened.

/// Splits an answer text into (thinking, visible_answer).
///
/// Supported shapes — all seen in the field:
///   - `<think>...</think>Answer`   : normal hybrid generation
///   - `<think></think>Answer`      : the empty pair left when thinking is off
///   - `<think>Answer`              : NO CLOSING TAG. This happened with
///     `/no_think`, and a naive "delete up to the first `</think>`" would
///     delete THE WHOLE ANSWER — that is exactly the bug that invalidated the
///     8B half of a 10-question measurement.
///   - No tag at all: the text is returned unchanged.
pub fn extract(text: &str) -> (Option<String>, String) {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";

    let Some(start) = text.find(OPEN) else {
        return (None, text.to_string());
    };

    // The text before the opening tag is normally empty, but if the model wrote
    // something before the tag that is THE USER'S answer and is not discarded.
    let before = &text[..start];
    let rest = &text[start + OPEN.len()..];

    match rest.find(CLOSE) {
        Some(end) => {
            let thinking = rest[..end].trim();
            let after = &rest[end + CLOSE.len()..];
            let visible = format!("{}{}", before, after);
            (
                (!thinking.is_empty()).then(|| thinking.to_string()),
                visible.trim().to_string(),
            )
        }
        // No closing tag: drop the tag, treat the rest as the ANSWER. The
        // alternative (treating the rest as thinking and returning an empty
        // answer) would have shown the user a blank screen.
        None => (None, format!("{}{}", before, rest).trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_without_tags_is_unchanged() {
        let (t, v) = extract("Hello, how are you?");
        assert!(t.is_none());
        assert_eq!(v, "Hello, how are you?");
    }

    #[test]
    fn a_complete_block_is_stripped() {
        let (t, v) = extract("<think>Let me compute first.</think>Result is 42.");
        assert_eq!(t.as_deref(), Some("Let me compute first."));
        assert_eq!(v, "Result is 42.");
    }

    #[test]
    fn an_empty_block_produces_no_thinking() {
        let (t, v) = extract("<think>\n\n</think>\n\nResult is 42.");
        assert!(t.is_none());
        assert_eq!(v, "Result is 42.");
    }

    /// The bug that invalidated the 8B measurement: an unclosed `<think>` was
    /// swallowing THE WHOLE ANSWER. The emphasis lives here rather than in the
    /// test name — an uppercase identifier trips `non_snake_case` under
    /// `-D warnings`.
    #[test]
    fn the_answer_survives_an_unclosed_tag() {
        let (t, v) = extract("<think>1247 times 89 equals 110983.");
        assert!(t.is_none());
        assert_eq!(v, "1247 times 89 equals 110983.");
    }

    #[test]
    fn text_before_the_tag_is_not_discarded() {
        let (_, v) = extract("Okay. <think>inner voice</think> Carry on.");
        assert!(v.starts_with("Okay."));
        assert!(v.ends_with("Carry on."));
        assert!(!v.contains("inner voice"));
    }

    #[test]
    fn multi_line_thinking() {
        let (t, v) = extract("<think>line1\nline2\nline3</think>Answer");
        assert!(t.unwrap().contains("line2"));
        assert_eq!(v, "Answer");
    }

    /// A tool call coming AFTER the thinking must stay parseable — otherwise
    /// the model calls the tool and the executor never sees it.
    #[test]
    fn a_tool_call_after_thinking_stays_visible() {
        let (_, v) = extract("<think>need arithmetic</think>calculate({\"expression\":\"12*8\"})");
        assert_eq!(v, "calculate({\"expression\":\"12*8\"})");
    }
}
