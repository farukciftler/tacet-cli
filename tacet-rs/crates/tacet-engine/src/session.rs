//! Session constants — the ARCHITECTURAL settings of the tool loop.
//!
//! WHY HERE AND NOT IN EVAL: these two constants are production behaviour, not
//! measurement settings. They used to live inside `tacet-eval` and `tacet-cli`
//! pulled them out of the test harness; that is, the production binary depended
//! on the eval crate and the answer to "how many turns do we run" was moving
//! around inside a test file. The engine layer is the common ancestor of both
//! sides, so this is the right home: the app and eval see THE SAME constant and
//! neither depends on the other.

/// How many model turns a single user message may run at most.
///
/// Four: the longest legitimate chain is "read -> create -> answer" (3 turns).
/// The fourth leaves room for error recovery; a fifth would be a sign of a loop,
/// and running forever measures nothing.
pub const MAX_TURNS: usize = 4;

/// The fixed session instructions. Eval and the app MUST see the same text:
/// behaviour measured with a different prompt does not bind the app.
///
/// NO PLACEHOLDERS — MEASURED TWICE ON A REAL MODEL.
///
/// 1. The text used to show the shape as `tool_name({"field":"value"})`.
///    Qwen2.5-3B took this not as a pattern but as TEXT TO BE WRITTEN: for
///    "what time is it?" it emitted `tool_name({"kind":"clock"})` verbatim.
/// 2. Shortening the placeholder to `name(...)` MOVED the problem instead of
///    solving it: a message like "Hello" that needs no tool produced
///    `name({"field":"Hello"})` — the pattern triggered a call even where no
///    call was warranted.
///
/// 3. With the placeholder removed, the copying MOVED TO THE UPPERCASE
///    IMPERATIVES: with "do not CALL a tool" in the instructions, the model
///    answered a greeting with "... Do Not CALL A Suitable Tool; Hello In Your
///    Own Language". That is, uppercase used for emphasis was taken by the
///    model as CONTENT.
///
/// 4. An example given over a REAL tool got copied too — measured with
///    Gemma3-4B. With `Example: calculate({"expression":"12*8"})` in the
///    instructions, the message "Thanks, have a good day." got the answer
///    "Thanks, you too! Calculate({"expression":"12*8"})." — it did the
///    greeting correctly and then stuck the example on the end VERBATIM, made-up
///    arguments and all. So the bug in items 1 and 2 was not specific to
///    placeholders; it is specific to THE CONCRETE EXAMPLE ITSELF. Qwen3-4B did
///    not copy with the same prompt, which is why the bug was invisible when
///    measured on a single model.
///
/// Lesson drawn: a model of this size CANNOT SEPARATE AN ABSTRACT PATTERN FROM A
/// CONCRETE EXAMPLE; every `xxx(...)` visible in the prompt is a copy candidate
/// — placeholder or real tool alike — and so is every UPPERCASE word. That is
/// why the instructions no longer carry any call example: the shape is
/// described in words only. The gap the example left is already filled by the
/// `<tools>` list — since the tool description moved to the short signature form
/// (`calculate(expression: text, digits?: integer)`) the list itself shows the
/// call shape, and without carrying a concrete ARGUMENT VALUE to copy. The call
/// is protected by two defences (grammar + catalog gate), but the prompt should
/// push the model to the right place from the start — a wasted turn is a cost
/// too.
///
/// ---
///
/// 5. QUALIFYING THE LAST SENTENCE SO IT "DOES NOT LEAK ACROSS TURNS" WAS TRIED
///    and REVERTED — the record stays here so nobody tries it again.
///
///    The observation was right: the rule "if the requested information was
///    given in a <tool_response> block, do not call a tool any more" is correct
///    WITHIN a turn, but a PREVIOUS turn's result sitting in the history also
///    satisfies the condition; on steps 2 and 3 of multi-turn cases the model
///    called no tool at all.
///
///    The attempted fix: narrow the condition to "the information needed for the
///    user's LAST message..." and add the sentence "call the tool needed for the
///    new request". Measured with `tacet eval --tool-selection`:
///
///      * the multi-turn cases DID NOT IMPROVE — the intended gain never came;
///      * the `time` cluster DROPPED from 4/4 to 1/4. "What time is it?",
///        "What's the date today?" and "What is today's date?" suddenly stopped
///        calling a tool.
///
///    So on a model of this size, adding a condition to the END of the
///    instructions weakens the instructions as a whole: a lengthening, qualified
///    last block also suppresses the tendency to call tools in unrelated cases.
///    The same lesson as items 1-4 from another angle: this prompt behaves
///    GLOBALLY, not locally, and even a one-sentence addition cannot be made
///    without measuring the full set.
///
/// ---
///
/// 6. "IF THE REQUESTED INFORMATION WAS GIVEN" -> "IF IT ANSWERS THE QUESTION"
///    (measured, permanent).
///
///    The old sentence read "if the requested information has been given to you
///    in a <tool_response> block, do not call a new tool" and the model read it
///    as "if ANY tool result came in, stop". In a real user session the result
///    was: for "what are the ortakoy uskudar ferry times" the model first called
///    the WRONG tool (`time`), then noticed its own gap and WROTE "a web search
///    is needed" but COULD NOT CALL the tool — the rule had locked it. The user
///    had to force the model with a second message ("search the internet").
///
///    The rule now depends not on THE EXISTENCE of a tool result but on whether
///    that result ANSWERS the question; if it does not, switching to the right
///    tool is open.
///
///    HOW THIS DIFFERS FROM ITEM 5 — WHERE IT WAS WRITTEN. There the condition
///    was appended to the END of the instructions and weakened the whole set
///    (`time` 4/4 -> 1/4). Here the last sentence ("do not make up the result")
///    STAYS IN PLACE; what changed is the middle sentence itself.
///
///    MEASURED ON THE FULL SET (`tacet eval --tool-selection`, Qwen3-4B):
///    tool accuracy 23/26 -> 23/26, irrelevance 6/6 -> 6/6, per-step 32/36 ->
///    33/36. That is NEUTRAL: it breaks no cluster of the set.
///
///    HONESTY NOTE — this change on its own SAVED NOTHING. Single-question
///    probes (for instance "what is today's date and what is the weather in
///    istanbul") still had the model WRITE "this needs a web search" instead of
///    calling the second tool. So what unlocks it is not the rule; it is a model
///    of this size being willing to produce a second call. What actually fixed
///    the ferry bug in the field is THE ROUTER (see `router.rs` Web triggers):
///    the right tool was moved to the head of the list and the FIRST choice was
///    corrected, so no recovery was needed at all. The sentence still stands
///    because the old wording was PROVABLY harmful (it locked the model) and the
///    new one is free in measurement.
///
///    THE LOOP BAN MOVED FROM TEXT INTO CODE. The only legitimate job of the old
///    sentence was to cut loops; loosening it did not leave that job unowned:
///    `ToolExecutor`'s 5th gate DOES NOT RUN the same (tool, arguments) pair
///    twice in one turn. The sentence "do not call the same tool with the same
///    arguments twice" is no longer the only defence, just the cheap layer that
///    saves the model a wasted turn.
pub const SYSTEM_INSTRUCTIONS: &str = "You are Tacet: an assistant that runs entirely on the device. \
Data never leaves the device. If a tool is needed, first write that tool's name from the <tools> \
list on the line, open a parenthesis right after it, give its arguments as a single JSON object, \
close the parenthesis and end the line there. Do not write bare JSON without the tool name; \
a call without a name is invalid. Example: calculate({\"expression\":\"12*8\"}). \
Argument names are written in that tool's signature in the list. \
Use only names present in the list, and call a tool only if that message needs it. \
For messages that need no tool, such as greetings and small talk, answer directly in the user's \
language, in your own words. \
If a <tool_response> block answers what was asked, do not call a new tool any more; \
place the value inside that block into your own sentence and answer the user in the user's \
language, short and direct. If it does not answer, call the right tool from the list; do not call \
the same tool with the same arguments a second time. Do not repeat the tool call or its JSON as \
the answer; do not make up the result.";

#[cfg(test)]
mod tests {
    use super::*;

    /// BRAND PROTECTION. `SYSTEM_INSTRUCTIONS` is the only place the model
    /// introduces itself: the name standing here comes straight back when asked
    /// "what is your name?". This test was written during a brand migration —
    /// in that migration the prompt had been missed and both the build and all
    /// 578 tests were green, because every existing test measured the prompt's
    /// CALL SHAPE, not its name. What stayed silent was what went unmeasured.
    ///
    /// There used to be a "no old brand name left" claim here too; once the
    /// migration finished and the old name was gone from the codebase that claim
    /// measured nothing and was removed. What protects is the POSITIVE claim:
    /// the prompt MUST START with the product name.
    #[test]
    fn system_instructions_state_the_product_name() {
        assert!(
            SYSTEM_INSTRUCTIONS.starts_with("You are Tacet:"),
            "the prompt must start with the product name, found: {}",
            &SYSTEM_INSTRUCTIONS[..SYSTEM_INSTRUCTIONS.len().min(40)]
        );
    }
}
