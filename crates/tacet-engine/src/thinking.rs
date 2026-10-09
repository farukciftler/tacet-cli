//! Stripping thinking (chain-of-thought) blocks out of the answer.
//!
//! WHY THIS IS NEEDED: the HYBRID Qwen3 builds (8B/14B) emit a
//! `<think>...</think>` block by default. That block is the model's own inner
//! reasoning, NOT the answer meant for the user — and worse, the model does its
//! inner reasoning in English even while answering in another language, so
//! foreign-language text spills onto the screen.
//!
//! THIS MODULE IS THE ONLY DEFENCE, and that is worth stating plainly because
//! this comment used to claim otherwise. It described a first layer in
//! `prompt.rs` that pre-wrote an empty `<think></think>` pair on the generation
//! anchor (Qwen3's official `enable_thinking=false` path). That layer was TRIED
//! AND REVERTED: it really did turn thinking off and it BROKE THE TOOL CALL
//! FORMAT with it (Qwen3-4B 7/10 -> 2/10). The measurement is stated here and
//! locked in by `the_chatml_template_separates_roles_with_fences` in `lib.rs`; this line
//! used to point at "the measurement record at the end of
//! `Prompt::chatml_text`", and `prompt.rs` does not contain the word `think`
//! anywhere — a reference that costs a reader a file search and gives them
//! nothing. Anyone reading this file must not go on believing
//! there is a cheaper layer upstream catching most of the cases.
//!
//! What is left upstream is the SOFT SWITCH (`/think` / `/no_think` appended to
//! the user message — `thinking_switch` in the CLI), which is a request the
//! model may ignore. So the block still shows up, and stripping it here is the
//! thing that actually holds.
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
///
/// A TOOL CALL IS NEVER EDITED (audit H5). This used to be blind text surgery
/// over the WHOLE output, call arguments included, and `<think>` is an added
/// but NOT special token in Qwen3 — the mask leaves it open inside a JSON
/// string, so the model can legitimately write it there. Run on the audit's
/// two strings, the old code turned
/// `write_file({"path":"a.md","content":"Qwen emits <think> first"})` into
/// content `"Qwen emits  first"`, and
/// `write_file({"path":"<think>","content":"</think>X"})` into
/// `write_file({"path":"X"})` — a value moved from `content` to `path`. The
/// call the executor ran was not the call the automaton had verified, which is
/// the one guarantee the grammar exists to give.
///
/// So the block is located with the call in mind:
///   - the OPENING tag counts only if it comes before the first call start
///     (`name(` followed by `{` or `)`, or a bare JSON object `{"`). Once a
///     call has started the grammar owns every character after it — it arms
///     at `name(` and production ends at the call's `)` — so a `<think>` past
///     that point is argument text, not a block;
///   - the CLOSING tag counts only if it is not inside a call's arguments. A
///     call MENTIONED inside the thinking is stepped over whole (strings and
///     nesting respected) so a `</think>` after it still closes the block; a
///     `</think>` inside the arguments leaves the block unclosed, and the
///     unclosed shape keeps every character of the call.
pub fn extract(text: &str) -> (Option<String>, String) {
    let Some(start) = text.find(OPEN) else {
        return (None, text.to_string());
    };
    if next_call_start(text, 0).is_some_and(|call| call < start) {
        return (None, text.to_string());
    }

    // The text before the opening tag is normally empty, but if the model wrote
    // something before the tag that is THE USER'S answer and is not discarded.
    let before = &text[..start];
    let rest = &text[start + OPEN.len()..];

    match find_close(rest) {
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

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

/// The first `</think>` in `rest` that is NOT inside a call's arguments.
///
/// Calls that start before the candidate tag are stepped over whole. A
/// candidate that falls INSIDE one means the block has no close before the
/// call, and the answer is `None` — not the next `</think>` further on, which
/// would move the call's tail into the thinking.
fn find_close(rest: &str) -> Option<usize> {
    let mut from = 0;
    loop {
        let close = from + rest[from..].find(CLOSE)?;
        match next_call_start(rest, from) {
            Some(call) if call < close => {
                let end = call_end(rest, call);
                if end > close {
                    return None;
                }
                from = end;
            }
            _ => return Some(close),
        }
    }
}

/// Byte index of the delimiter that opens the first call at or after `from`:
/// the `(` of `name(`, or the `{` of a bare JSON object.
///
/// `name(` needs an identifier character right before the `(` — the boundary
/// the grammar arms on (`is_id_char` in `tacet-grammar`) — AND `{` or `)` right
/// after it, whitespace allowed, because those are the only two characters the
/// armed grammar accepts there. The second condition is what keeps prose like
/// `f(x)` inside a thinking block from counting as a call. The bare `{"` form
/// is the executor's recovery path for nameless JSON: unconstrained, but the
/// executor still RUNS it, so it is protected the same way.
fn next_call_start(text: &str, from: usize) -> Option<usize> {
    let mut prev = text[..from].chars().next_back();
    for (offset, c) in text[from..].char_indices() {
        let at = from + offset;
        let after = text[at + c.len_utf8()..].trim_start();
        let opens = match c {
            '(' => {
                prev.is_some_and(|p| p.is_alphanumeric() || p == '_')
                    && (after.starts_with('{') || after.starts_with(')'))
            }
            '{' => after.starts_with('"'),
            _ => false,
        };
        if opens {
            return Some(at);
        }
        prev = Some(c);
    }
    None
}

/// Byte index just past the delimiter that closes the call opened at `open`,
/// or the end of the text when it never closes (a call cut by a cap). JSON
/// strings are skipped with their escapes, so a `)` or a `</think>` inside a
/// value stays part of the value.
fn call_end(text: &str, open: usize) -> usize {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, c) in text[open..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return open + offset + c.len_utf8();
                }
            }
            _ => {}
        }
    }
    text.len()
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

    // ---- H5: a tool call is never edited -------------------------------------

    /// The audit's first string, verbatim. The old code cut the tag out of the
    /// value and wrote `Qwen emits  first` to disk.
    #[test]
    fn a_think_tag_inside_an_argument_is_left_in_the_argument() {
        let call = r#"write_file({"path":"a.md","content":"Qwen emits <think> first"})"#;
        let (t, v) = extract(call);
        assert!(t.is_none());
        assert_eq!(v, call);
    }

    /// The audit's second string, verbatim. The old code returned
    /// `write_file({"path":"X"})`: a value moved from `content` to `path`.
    #[test]
    fn a_tag_pair_split_across_two_arguments_cannot_move_a_value() {
        let call = r#"write_file({"path":"<think>","content":"</think>X"})"#;
        let (t, v) = extract(call);
        assert!(t.is_none());
        assert_eq!(v, call);
    }

    /// An UNCLOSED block followed by a call whose argument carries `</think>`:
    /// the only close in the text is inside the call, so it must not be taken —
    /// taking it put the call's head into the thinking and showed `X"})`.
    #[test]
    fn a_close_tag_inside_a_later_call_does_not_close_the_block() {
        let call = r#"write_file({"path":"a.md","content":"</think>X"})"#;
        let (t, v) = extract(&format!("<think>plan it {call}"));
        assert!(t.is_none());
        assert!(v.ends_with(call), "call was edited: {v}");
    }

    /// The bare-JSON shape the executor recovers is protected the same way.
    #[test]
    fn a_nameless_json_call_is_not_edited() {
        let call = r#"{"tool":"write_file","args":{"path":"<think>","content":"</think>X"}}"#;
        let (t, v) = extract(call);
        assert!(t.is_none());
        assert_eq!(v, call);
    }

    /// The fix must not cost the original job: a call MENTIONED inside the
    /// thinking (with a `)` inside its string) is stepped over, and the real
    /// close after it still strips the block.
    #[test]
    fn a_call_mentioned_inside_thinking_does_not_hide_the_close() {
        let (t, v) = extract(
            r#"<think>maybe calculate({"expression":"(1)"}) first</think>calculate({"expression":"2"})"#,
        );
        assert_eq!(
            t.as_deref(),
            Some(r#"maybe calculate({"expression":"(1)"}) first"#)
        );
        assert_eq!(v, r#"calculate({"expression":"2"})"#);
    }

    /// Prose parentheses are not calls: `f(x)` has neither `{` nor `)` after
    /// its `(`. Counted as calls, prose before the tag would keep the block
    /// from being stripped at all, and an unbalanced `g(` inside the thinking
    /// would swallow the close — either way the thinking reaches the screen.
    #[test]
    fn prose_parentheses_are_not_calls() {
        let (t, v) = extract("<think>f(x) = x(x+1), so f(2) is 6</think>It is 6.");
        assert_eq!(t.as_deref(), Some("f(x) = x(x+1), so f(2) is 6"));
        assert_eq!(v, "It is 6.");

        let (t, v) = extract("Since f(x) grows, <think>inner voice</think> it is 6.");
        assert_eq!(t.as_deref(), Some("inner voice"));
        assert_eq!(v, "Since f(x) grows,  it is 6.");

        let (t, v) = extract("<think>if g(x is odd, stop</think>It is 6.");
        assert_eq!(t.as_deref(), Some("if g(x is odd, stop"));
        assert_eq!(v, "It is 6.");
    }

    /// The sweep: every call shape with tags in its arguments, behind every
    /// prefix the model produces, must come out of `extract` with the call
    /// byte-for-byte intact. The escaped-quote entry is shaped on purpose: a
    /// scanner that ends a string at `\"` reads the `})` after it as the end of
    /// the call and hands the `</think>` that follows back to the block.
    #[test]
    fn no_prefix_shape_edits_a_call_with_tags_in_its_arguments() {
        let calls = [
            r#"write_file({"path":"a.md","content":"Qwen emits <think> first"})"#,
            r#"write_file({"path":"<think>","content":"</think>X"})"#,
            r#"write_file({"path":"a","content":"say \"})</think> then <think>"})"#,
            r#"write_file({"path":"a","content":"<think>x</think>"})"#,
            r#"{"tool":"write_file","args":{"content":"<think>a</think>b"}}"#,
        ];
        let prefixes = ["", "Sure. ", "<think>reason</think>", "<think>reason "];
        for call in calls {
            for prefix in prefixes {
                let (_, v) = extract(&format!("{prefix}{call}"));
                assert!(v.ends_with(call), "prefix {prefix:?} edited {call}: {v}");
            }
        }
    }
}
