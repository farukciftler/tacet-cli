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
///     (`name(` followed by `{` or `)`, a bare JSON object `{"`, one of the
///     executor's call markers, or a glued call when the names are known). Once a call has started the grammar owns every
///     character after it — it arms at `name(` and production ends at the
///     call's `)` — so a `<think>` past that point is argument text, not a block;
///   - the CLOSING tag counts only if it is not inside a call's arguments. A
///     call MENTIONED inside the thinking is stepped over whole, but only when
///     it PARSES as one (see `proven_call_end`); a `</think>` inside the
///     arguments, or a call start that cannot be proven, leaves the block
///     unclosed, and the unclosed shape keeps every character of the call.
///
/// WHAT IS COVERED, exactly: a `name(` call, a bare JSON object, anything
/// behind a `CALL_MARKERS` marker and — through `extract_with_names` — the
/// glued shape `recover_glued_call` in `tacet-tools` recovers
/// (`write_filepath: "a.md"` at the start of a line). That last one needs the
/// catalog, which this crate never sees: its only licence is a CATALOG tool
/// name at a line start, and a catalog-free stand-in (any `word:` opening a
/// line) would also match `Note:` and `Answer:` in ordinary thinking and keep
/// those blocks on the screen. So `extract` alone does not cover it, and the
/// callers that execute (`chat`, both eval loops) re-split with the catalog via
/// `Generation::split_against` before the executor reads the text.
///
/// THE CUT NEVER MAKES A CALL EITHER. Removing the block joins the text on
/// its two sides, and a join can build a call that exists in neither half:
/// `delete_file<think>x</think>({"path":"v"})` became
/// `delete_file({"path":"v"})`, `web_<think>x</think>search(` became
/// `web_search(`. On the raw text the executor's parser finds no call in the
/// first and the unknown name `search` in the second, and the grammar armed on
/// neither (`<` and `>` are not identifier characters). When any call start's
/// syntax straddles the seam, the text comes back unchanged.
pub fn extract(text: &str) -> (Option<String>, String) {
    extract_with_names(text, &[])
}

/// `extract`, with the catalog's tool names, so the glued shape (a name at a
/// line start directly followed by an identifier character) counts as a call
/// start the way `name(` does. "Followed by an identifier character" is wider
/// than `recover_glued_call`'s "followed by a field name" on purpose: this
/// side has no schemas, and a wider net here only ever leaves a block visible —
/// it can never let a cut reach into a call the executor would run.
///
/// THE PRICE: a block that MENTIONS a glued call at a line start reads as
/// unclosed and reaches the screen, the same trade as an unprovable `{"`.
pub fn extract_with_names(text: &str, names: &[&str]) -> (Option<String>, String) {
    let unchanged = || (None, text.to_string());
    let Some(start) = text.find(OPEN) else {
        return unchanged();
    };
    if next_call_start(text, 0, names).is_some_and(|call| call.at < start) {
        return unchanged();
    }

    // The text before the opening tag is normally empty, but if the model wrote
    // something before the tag that is THE USER'S answer and is not discarded.
    let before = &text[..start];
    let body = start + OPEN.len();
    let (thinking, after) = match find_close(text, body, names) {
        Some(end) => (text[body..end].trim(), &text[end + CLOSE.len()..]),
        // No closing tag: drop the tag, treat the rest as the ANSWER. The
        // alternative (treating the rest as thinking and returning an empty
        // answer) would have shown the user a blank screen.
        None => ("", &text[body..]),
    };
    let visible = format!("{before}{after}");
    if seam_makes_a_call(&visible, before.len(), names) {
        return unchanged();
    }
    (
        (!thinking.is_empty()).then(|| thinking.to_string()),
        visible.trim().to_string(),
    )
}

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

/// The markers `recover_marked_call` in `tacet-tools` looks behind — the same
/// four, in the same spelling. The executor RUNS what follows one with no
/// grammar involved, so for this module a marker opens a call exactly as
/// `name(` does. Kept as a copy rather than an import: `tacet-engine` does not
/// depend on `tacet-tools`, and a dependency edge for four strings would point
/// the wrong way. `a_marked_call_is_not_edited` pins the shapes.
const CALL_MARKERS: [&str; 4] = ["```tool_code", "```tool", "```json", "<tool_call>"];

/// Where a call starts, and the bytes that MAKE it one.
///
/// `at` is what the close search orders by: the `(` of `name(`, the `{` of a
/// bare object, the first byte of a marker or of a glued name. `sig` is every
/// byte the shape test read to decide — the whole identifier before a `(` (the
/// executor's parser takes all of it as the name, not just the last
/// character), the `{` or `)` after it, the `"` after a `{`, the marker, the
/// name plus the one character it is glued to. A cut whose seam falls strictly
/// inside `sig` produced the call; a seam at its edge only moved it.
struct CallStart {
    at: usize,
    sig: std::ops::Range<usize>,
}

/// Does joining at `seam` produce a call start neither half had?
fn seam_makes_a_call(joined: &str, seam: usize, names: &[&str]) -> bool {
    let mut from = 0;
    while let Some(call) = next_call_start(joined, from, names) {
        if call.sig.start < seam && seam < call.sig.end {
            return true;
        }
        from = call.at + joined[call.at..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// The first `</think>` at or after `from` that is NOT inside a call's
/// arguments, as a byte index into `text`.
///
/// Indices are into the WHOLE text, not the slice after the opening tag, and
/// that matters for the glued shape: its licence is a line start, and the
/// first byte after `<think>` is not one — a slice would make it one.
///
/// Calls that start before the candidate tag are stepped over whole. A
/// candidate that falls INSIDE one means the block has no close before the
/// call, and the answer is `None` — not the next `</think>` further on, which
/// would move the call's tail into the thinking.
///
/// A CALL START THAT CANNOT BE PROVEN IS NOT STEPPED OVER — it ends the search
/// with `None`. The first version stepped with a structural scan (brace depth,
/// quote parity) and a review broke it with one stray quote: in
/// `<think>use {"k":"v write_file({"path":"a","content":"x}y</think>Z"})` the
/// scan took the prose `{"` for a call, its string parity came out inverted
/// over the real call, it stopped at the `}` inside `x}y`, and the `</think>`
/// in the real argument closed the block — visible text `Z"})`, the verified
/// call gone. Skipping an unprovable start instead of stopping is not safe
/// either: the executor runs shapes `serde_json` rejects (a trailing comma,
/// key=value behind a marker, every glued call), and a skip hands their
/// `</think>` to the block.
///
/// THE PRICE: thinking that contains a `{"` which is not a whole JSON value
/// now shows on the screen with its tags (pinned by
/// `an_unprovable_span_in_thinking_leaves_the_block_unclosed`). A visible
/// block is a nuisance the user can see; an edited call is a write nobody sees
/// go wrong.
fn find_close(text: &str, from: usize, names: &[&str]) -> Option<usize> {
    let mut from = from;
    loop {
        let close = from + text[from..].find(CLOSE)?;
        match next_call_start(text, from, names) {
            Some(call) if call.at < close => {
                let end = proven_call_end(text, call.at)?;
                if end > close {
                    return None;
                }
                from = end;
            }
            _ => return Some(close),
        }
    }
}

/// The first call at or after `from`.
///
/// `name(` needs an identifier character right before the `(` — the boundary
/// the grammar arms on (`is_id_char` in `tacet-grammar`) — AND `{` or `)` right
/// after it, whitespace allowed, because those are the only two characters the
/// armed grammar accepts there. The second condition is what keeps prose like
/// `f(x)` inside a thinking block from counting as a call. The bare `{"` form
/// is the executor's recovery path for nameless JSON: unconstrained, but the
/// executor still RUNS it, so it is protected the same way; markers likewise.
/// A glued call is a catalog name opening a line with an identifier character
/// right after it — `find_at_line_start` plus the glue in `tacet-tools`.
fn next_call_start(text: &str, from: usize, names: &[&str]) -> Option<CallStart> {
    let mut prev = text[..from].chars().next_back();
    for (offset, c) in text[from..].char_indices() {
        let at = from + offset;
        if let Some(sig) = call_signature(text, at, c, prev, names) {
            return Some(CallStart { at, sig });
        }
        prev = Some(c);
    }
    None
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The bytes that make `text[at..]` (whose first char is `c`) a call start.
fn call_signature(
    text: &str,
    at: usize,
    c: char,
    prev: Option<char>,
    names: &[&str],
) -> Option<std::ops::Range<usize>> {
    let next = at + c.len_utf8();
    // Just past the first non-space character after `c`, if it is one of `want`.
    let first_after = |want: &[char]| {
        let tail = &text[next..];
        let trimmed = tail.trim_start();
        let first = trimmed.chars().next().filter(|f| want.contains(f))?;
        Some(next + (tail.len() - trimmed.len()) + first.len_utf8())
    };
    match c {
        '(' if prev.is_some_and(is_id_char) => {
            let end = first_after(&['{', ')'])?;
            let ident = text[..at]
                .char_indices()
                .rev()
                .take_while(|(_, p)| is_id_char(*p))
                .last()
                .map_or(at, |(i, _)| i);
            Some(ident..end)
        }
        '{' => Some(at..first_after(&['"'])?),
        '`' | '<' => CALL_MARKERS
            .iter()
            .find(|m| text[at..].starts_with(**m))
            .map(|m| at..at + m.len()),
        _ => {
            let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
            if !text[line_start..at].chars().all(char::is_whitespace) {
                return None;
            }
            names.iter().find_map(|name| {
                let glue = text[at..]
                    .strip_prefix(*name)
                    .filter(|_| !name.is_empty())?
                    .chars()
                    .next()
                    .filter(|g| is_id_char(*g))?;
                Some(at..at + name.len() + glue.len_utf8())
            })
        }
    }
}

/// Byte index just past the call that starts at `start`, IF the text there
/// parses as one; `None` when it does not, or when it never ends (a call cut by
/// a cap runs to the end of the text, so a close after it is inside it).
///
/// Parsing with `serde_json` rather than counting braces is the point: a
/// grammar-armed call is valid JSON by construction, so a real call always
/// proves, and prose that merely looks like a call start can only fail — it
/// can never be mistaken for a call of a different length and pull the scan
/// out of step. A marker never proves: what follows it is read key=value by
/// the executor and has no end this module could check. Neither does a glued
/// call — it is key=value too, ended only by the executor's own scan.
fn proven_call_end(text: &str, start: usize) -> Option<usize> {
    let object_end = |at: usize| {
        let mut values =
            serde_json::Deserializer::from_str(&text[at..]).into_iter::<serde_json::Value>();
        match values.next() {
            Some(Ok(serde_json::Value::Object(_))) => Some(at + values.byte_offset()),
            _ => None,
        }
    };
    let skip_space = |at: usize| at + (text[at..].len() - text[at..].trim_start().len());
    match text[start..].chars().next()? {
        '{' => object_end(start),
        '(' => {
            let args = skip_space(start + 1);
            let after_args = if text[args..].starts_with(')') {
                args
            } else {
                skip_space(object_end(args)?)
            };
            text[after_args..]
                .starts_with(')')
                .then_some(after_args + 1)
        }
        _ => None,
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

    /// The review's two inputs, verbatim. Prose in an unclosed block opens a
    /// `{"` with an odd number of quotes after it; the old structural scan
    /// stepped over that "call", its string parity came out inverted, it ended
    /// at a `}` inside the REAL call's string, and the `</think>` in the real
    /// call's argument closed the block. Measured on the previous fix: visible
    /// text `Z"})` (the verified call gone) and `delete_file({","path":"victim.md"})`
    /// (a fragment shaped like a different call). The grammar accepts both real
    /// calls — it arms on `write_file(` after a space and the JSON is valid.
    #[test]
    fn prose_braces_in_thinking_cannot_desync_the_close_search() {
        for (prefix, call) in [
            (
                r#"<think>use {"k":"v "#,
                r#"write_file({"path":"a","content":"x}y</think>Z"})"#,
            ),
            (
                r#"<think>The format is {"tool":"write_file then "#,
                r#"write_file({"content":"x}y</think>delete_file({","path":"victim.md"})"#,
            ),
        ] {
            let (t, v) = extract(&format!("{prefix}{call}"));
            assert!(t.is_none(), "a block closed inside the call: {t:?}");
            assert!(v.ends_with(call), "call was edited: {v}");
        }
    }

    /// What the conservative close search COSTS, pinned so nobody buys it back
    /// by skipping: a `{"` inside the thinking that does not parse as a whole
    /// JSON value is not stepped over, so the block reads as unclosed and the
    /// thinking reaches the screen. A visible block is a nuisance the user can
    /// see; a value cut out of a call is a write nobody sees go wrong.
    #[test]
    fn an_unprovable_span_in_thinking_leaves_the_block_unclosed() {
        let (t, v) = extract(r#"<think>shape is {"k": v}</think>It is v."#);
        assert!(t.is_none());
        assert_eq!(v, r#"shape is {"k": v}</think>It is v."#);
    }

    /// The marked recovery shapes (`recover_marked_call` in `tacet-tools`) are
    /// not grammar-verified, but the executor RUNS them, so a marker is a call
    /// start like `name(` is. Measured on the previous fix: the key=value form
    /// came back as `<tool_call>write_file path="X"` (a value moved between
    /// fields) and the paren form as `content="Qwen emits  first"`.
    #[test]
    fn a_marked_call_is_not_edited() {
        for call in [
            r#"<tool_call>write_file path="<think>" content="</think>X""#,
            r#"<tool_call>write_file(path="a.md", content="Qwen emits <think> first")"#,
            "```json\n{\"action\":\"write_file\",\"content\":\"<think>a</think>b\"}\n```",
        ] {
            let (t, v) = extract(call);
            assert!(t.is_none());
            assert_eq!(v, call);

            let (t, v) = extract(&format!("<think>reason {call}"));
            assert!(t.is_none(), "a block closed inside the call: {t:?}");
            assert!(v.ends_with(call), "call was edited: {v}");
        }
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

    /// The review's two glued probes, verbatim. Without the names the first
    /// came back as `Qwen emits  first` and `recover_glued_call` ran exactly
    /// that; the second closed the block inside `content` and left
    /// `delete_filepath: "victim.md""` opening a line — a value from inside one
    /// argument turned into a different glued call.
    #[test]
    fn a_glued_call_is_not_edited_when_the_names_are_known() {
        let names = ["write_file", "delete_file"];

        let call = r#"write_filepath: "a.md", content: "Qwen emits <think> first""#;
        assert_eq!(extract_with_names(call, &names), (None, call.to_string()));

        let call = "write_filepath: \"a\", content: \"</think>\ndelete_filepath: \"victim.md\"\"";
        let (t, v) = extract_with_names(&format!("<think>reason\n{call}"), &names);
        assert!(t.is_none(), "a block closed inside the call: {t:?}");
        assert!(v.ends_with(call), "call was edited: {v}");
        assert!(
            !v.starts_with("delete_file"),
            "an argument became a call: {v}"
        );
    }

    /// The names must not cost the job either: a block before a glued answer
    /// is still stripped, and a name that is not glued to anything (`Note:`
    /// style prose, a name mid-line) is not a call start.
    #[test]
    fn the_names_still_strip_a_block_before_a_glued_call() {
        let names = ["find_file"];
        let (t, v) = extract_with_names(r#"<think>plan</think>find_filepattern: "x""#, &names);
        assert_eq!(t.as_deref(), Some("plan"));
        assert_eq!(v, r#"find_filepattern: "x""#);

        let (t, v) = extract_with_names(
            "<think>use find_file\nfind_file is the tool</think>Found it.",
            &names,
        );
        assert_eq!(t.as_deref(), Some("use find_file\nfind_file is the tool"));
        assert_eq!(v, "Found it.");
    }

    /// The review's splice: the cut joined the two sides of the block and built
    /// a call neither side held. The raw text of each is no call the executor
    /// would run (no identifier before the `(`, an unknown `search`, a `(` the
    /// grammar never armed on); the joined text was `delete_file({…})`,
    /// `web_search(…)`, a glued `delete_filepath` and so on. Each must come
    /// back unchanged.
    #[test]
    fn the_cut_never_joins_two_halves_into_a_call() {
        let names = ["delete_file", "write_file"];
        for (text, forged) in [
            (
                r#"delete_file<think>x</think>({"path":"victim.md"})"#,
                "delete_file({",
            ),
            (
                r#"web_<think>x</think>search({"query":"q"})"#,
                "web_search(",
            ),
            (r#"delete_file<think>x</think>path: "v""#, "delete_filepath"),
            (
                r#"write_file(<think>x</think>{"path":"a"})"#,
                "write_file({",
            ),
            (
                r#"{<think>x</think>"tool":"write_file","args":{}}"#,
                r#"{"tool""#,
            ),
            (
                r#"<tool_<think>x</think>call>write_file path="a""#,
                "<tool_call>",
            ),
            // Unclosed: dropping the lone tag is a cut too.
            (
                r#"delete_file<think>({"path":"victim.md"})"#,
                "delete_file({",
            ),
        ] {
            let (t, v) = extract_with_names(text, &names);
            assert!(!v.contains(forged), "the cut forged {forged:?}: {v}");
            assert!(t.is_none());
            assert_eq!(v, text);
        }
    }

    /// The sweep: every call shape with tags in its arguments, behind every
    /// prefix the model produces, must come out of `extract` with the call
    /// byte-for-byte intact. The escaped-quote entry is shaped on purpose: a
    /// scanner that ends a string at `\"` reads the `})` after it as the end of
    /// the call and hands the `</think>` that follows back to the block. The
    /// `{"k":"v ` prefix is the review's desync: an odd quote in prose flips a
    /// structural scan's string parity, the scan stops at the `}` inside `x}y`,
    /// and the `</think>` in the real call was taken as the block's close.
    #[test]
    fn no_prefix_shape_edits_a_call_with_tags_in_its_arguments() {
        let calls = [
            r#"write_file({"path":"a.md","content":"Qwen emits <think> first"})"#,
            r#"write_file({"path":"<think>","content":"</think>X"})"#,
            r#"write_file({"path":"a","content":"say \"})</think> then <think>"})"#,
            r#"write_file({"path":"a","content":"<think>x</think>"})"#,
            r#"write_file({"path":"a","content":"x}y</think>Z"})"#,
            r#"{"tool":"write_file","args":{"content":"<think>a</think>b"}}"#,
        ];
        let prefixes = [
            "",
            "Sure. ",
            "<think>reason</think>",
            "<think>reason ",
            r#"<think>use {"k":"v "#,
        ];
        for call in calls {
            for prefix in prefixes {
                let (_, v) = extract(&format!("{prefix}{call}"));
                assert!(v.ends_with(call), "prefix {prefix:?} edited {call}: {v}");
            }
        }
    }
}
