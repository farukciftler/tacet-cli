//! The web nudge fires on the questions it was written for.
//!
//! WHY THIS IS ITS OWN GUARD. The nudge is one sentence and the measured reason
//! a small model reaches for `web_search` at all — without it, ferry times came
//! back answered from memory and the user had to say "search the internet" as a
//! second turn. It fires on the DOMINANT intent profile, which is a different
//! rule from the one that ranks the tools, and nothing was checking it.
//!
//! WHAT THAT MISSED. `tacet why "search the web for the exchange rate"` — the
//! most explicit internet request there is — scored web 3 and FILES 6, because
//! `Files` owns bare "search" and `Web` only had the word "web". And adding one
//! word of diary flipped a weather question away: "what is the weather in
//! Istanbul tomorrow" scored clock 8, calendar 8, web 7. The tool RANKING
//! survived both (`web_search` still led on the hint product) which is exactly
//! why neither showed up in `eval --routing`: the ranking is what that measures.
//!
//! This asserts the other half.

use tacet_tools::router::{IntentProfile, score_intent};

/// Messages that must be dominant-web. Each is either a sentence a person
/// really typed into this project's suites and benchmarks, or the plainest
/// possible phrasing of one.
const MUST_NUDGE: &[&str] = &[
    "search the web for the exchange rate",
    "what is the weather in Istanbul",
    "what is the weather in Istanbul tomorrow",
    "what are the latest news headlines today",
    "what is the current stock price of Apple",
    "who won the 2026 election",
    "is there a train strike going on in France",
    "how bad is the air quality in Delhi",
    "what are the ortakoy uskudar ferry times",
];

/// Messages that must NOT be dominant-web, because the nudge tells the model to
/// call `web_search` FIRST and on these that is the wrong tool. A nudge that
/// fires everywhere is the same as no nudge, and worse: it spends the last line
/// before the question saying something false.
const MUST_NOT_NUDGE: &[&str] = &[
    "what is 125 times 8",
    "what is the current year",
    "what is the current UTC time",
    "which git branch am I currently on",
    "what was the most recent commit here about",
    "hello, how are you",
    "read the table inside report.md",
    "remind me to call the dentist tomorrow at 9",
];

#[test]
fn the_questions_the_nudge_exists_for_are_dominant_web() {
    let missed: Vec<&str> = MUST_NUDGE
        .iter()
        .copied()
        .filter(|m| score_intent(m).dominant() != IntentProfile::Web)
        .collect();
    assert!(
        missed.is_empty(),
        "these get no web nudge, and the nudge is the measured reason a small \
         model reaches for `web_search` at all: {missed:#?}\n\
         Run `tacet why` on one of them — the tool ranking can be right while \
         the dominant profile is not, and `eval --routing` only measures the ranking."
    );
}

#[test]
fn a_question_that_wants_another_tool_gets_no_nudge() {
    let wrongly: Vec<&str> = MUST_NOT_NUDGE
        .iter()
        .copied()
        .filter(|m| score_intent(m).dominant() == IntentProfile::Web)
        .collect();
    assert!(
        wrongly.is_empty(),
        "the nudge says 'call web_search FIRST; do not answer from memory', which \
         is false for these: {wrongly:#?}"
    );
}
