//! One case may not stall the whole run.
//!
//! WHY THIS EXISTS. A pass is bounded by the token cap and a case by
//! `MAX_TURNS`, but a TOOL is bounded by nothing the eval controls.
//! `calendar-day` reads as a 39-second case of which 30 seconds is `osascript`
//! waiting on the Calendar app; a helper that never returns would stall a
//! 48-minute run indefinitely — on rented hardware, with no partial report, and
//! with nothing in the output naming the case it stopped on.
//!
//! WHAT IS ASSERTED. The bound exists, it is generous enough not to fire on a
//! healthy run, and a case stopped by it is NOT scored as a model failure. The
//! last part is the one that matters: `TimedOut` is unmeasurable for the same
//! reason `HostFailed` is, and merging it into `OutOfTurns` would report our
//! own stopwatch as the model spending its passes.

use std::sync::Arc;

use tacet_engine::EngineProvider;
use tacet_engine::fake::FakeEngine;
use tacet_eval::tool_selection::{
    CASE_WALL_LIMIT, Ending, SelectionCase, SelectionStep, run_selection_case,
};

#[test]
fn the_bound_is_a_backstop_and_not_a_policy() {
    let slowest_case_ever_measured = std::time::Duration::from_secs(40);
    assert!(
        CASE_WALL_LIMIT >= slowest_case_ever_measured * 4,
        "the bound is meant to catch a hang, not to cut a slow case short; \
         at {CASE_WALL_LIMIT:?} it is closer to the measured worst case than \
         a backstop should be"
    );
}

#[test]
fn a_timed_out_case_is_not_scored_against_the_model() {
    assert!(
        !Ending::TimedOut.is_measurable(),
        "our own stopwatch is not a verdict about the model"
    );
    assert_ne!(
        Ending::TimedOut.name(),
        Ending::OutOfTurns.name(),
        "a report that cannot tell 'we stopped it' from 'the model spent its \
         passes' cannot tell a slow tool from a stubborn model"
    );
}

/// A healthy case does not come back `TimedOut`. The bound is checked before
/// each pass, so an off-by-one that fired on the first pass would silently
/// return every case unmeasurable — and every axis would read 0 with no error.
#[test]
fn a_normal_case_still_finishes_normally() {
    let engine: Arc<dyn EngineProvider> = Arc::new(FakeEngine::script([
        r#"calculate({"expression":"125*8"})"#,
        "125 times 8 is 1000.",
    ]));
    let case = SelectionCase {
        name: "probe".into(),
        category: tacet_eval::Category::Tool,
        steps: vec![SelectionStep::new(
            "What is 125 times 8?",
            Some("calculate"),
        )],
    };
    let outcome = run_selection_case(&case, &engine);
    assert_eq!(outcome.steps[0].ended, Ending::Answered);
}
