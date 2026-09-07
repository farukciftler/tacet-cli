//! The eval must shrink a prompt the way the shell shrinks it.
//!
//! WHY. `FakeEngine` declares no model path and no context length, so
//! `context_budget` returns the floor — `CONTEXT_BUDGET`, 4096 — and the prompt
//! cap is 3072. That is not a test artefact: the floor is documented as what a
//! model "that declares less, or declares nothing" gets, so a real GGUF lands
//! there too.
//!
//! The shell calls `TokenCounter::truncate` on every prompt — drop the oldest
//! turns, then sacrifice the guide, then trim the question. The eval called it
//! on none, so on such a machine it sent histories the shell would have cut, and
//! measured a program nobody runs.
//!
//! WHAT THE FIRST VERSION OF THIS TEST ASSERTED, AND WHY IT WAS WRONG. It
//! required every prompt to be inside the cap. Neither program can promise
//! that: `truncate` never touches the system block or the tool descriptions —
//! by policy, because a truncated instruction makes the model forget who it is
//! and a truncated description makes it invent a signature — and those two
//! alone are over 3000 tokens with the default catalog. So on a 4096 window the
//! prompt does not fit however much history is dropped. That is a fact about the
//! floor, it applies to the shell exactly as much, and it is asserted below
//! rather than left to be rediscovered.

use std::sync::Arc;

use tacet_engine::fake::FakeEngine;
use tacet_engine::{EngineProvider, Prompt, SYSTEM_INSTRUCTIONS, TokenCounter};
use tacet_eval::tool_selection::{SelectionCase, SelectionStep, run_selection_case};

/// The counter the eval builds for an engine that declares nothing — the floor.
fn floor_counter() -> TokenCounter {
    TokenCounter::new(
        tacet_engine::context_budget(None, None, tacet_engine::Device::Cpu),
        tacet_engine::GENERATION_SHARE,
    )
}

#[test]
fn the_floor_is_the_window_an_engine_that_declares_nothing_gets() {
    assert_eq!(
        floor_counter().prompt_cap(),
        tacet_engine::CONTEXT_BUDGET - tacet_engine::GENERATION_SHARE,
        "if this moved, the numbers in the other tests' messages are stale"
    );
}

/// A marker in the OLDEST turn must be gone from a later prompt once the
/// history outgrows the budget — which is what "the eval truncates" means.
///
/// THE FIXTURE IS A TWO-STEP CASE WITH A VERY LONG FIRST MESSAGE, and the shape
/// took two attempts. The first put the marker inside a long generated CALL,
/// which `FakeEngine` cut on its own token cap (`StopReason::Length`) — so the
/// call never ran, no history accumulated, and the test passed for a reason
/// that had nothing to do with truncation. A long USER message goes into the
/// history whatever the engine does.
#[test]
fn the_oldest_turn_is_dropped_when_the_history_outgrows_the_budget() {
    const OLDEST: &str = "ZZ-OLDEST-TURN-MARKER-ZZ";
    let long_first = format!(
        "{OLDEST} {}",
        "and the quarterly budget review totals against the summary from last week ".repeat(90)
    );
    let engine = Arc::new(
        FakeEngine::script(["the first answer.", "the second answer."]).with_default("done."),
    );
    let provider: Arc<dyn EngineProvider> = engine.clone();
    let case = SelectionCase {
        name: "long".into(),
        category: tacet_eval::Category::MultiTurn,
        steps: vec![
            SelectionStep::new(&long_first, None),
            SelectionStep::new("and now the total", None),
        ],
    };
    let _ = run_selection_case(&case, &provider);

    let prompts = engine.seen_prompts();
    assert!(prompts.len() >= 2, "both steps must have run");
    let second_step = prompts.last().expect("a last prompt");
    assert!(
        TokenCounter::estimate(&long_first) + 2000 > floor_counter().prompt_cap(),
        "the fixture no longer overflows the cap, so it measures nothing"
    );
    assert!(
        !second_step.contains(OLDEST),
        "the oldest turn is still in the second step's prompt, so nothing was \
         dropped — the shell would have dropped it, and the two are measuring \
         different prompts:\n{}",
        &second_step[..second_step.len().min(400)]
    );
}

/// THE FLOOR CANNOT FIT THE CATALOG, and both programs are in the same position.
///
/// `truncate` never touches the system block or the tool descriptions. With the
/// default catalog those two alone are over the 3072-token cap a 4096 window
/// gives, so on such a machine the prompt is over budget with an EMPTY history.
/// Neither the shell nor the eval can do anything about it from here; what is
/// wrong on that machine is the window, not the truncation.
///
/// Asserted so it is a measurement rather than a surprise: if the descriptions
/// are ever trimmed enough to fit, this says so, and if they grow, it says that
/// too.
#[test]
fn the_floor_does_not_fit_the_default_catalog_and_that_is_not_the_truncators_fault() {
    let store = Arc::new(tacet_tools::data_store::SharedStore::new());
    let memory = tacet_tools::memory::SharedMemory::in_memory();
    let (catalog, _, _) =
        tacet_tools::catalog::production_catalog_with(&store, &memory, Some(0), true);
    let counter = floor_counter();
    // No history, no guide: the irreducible part.
    let mut bare = Prompt::new(SYSTEM_INSTRUCTIONS, "hi").with_tools(&catalog);
    let report = counter.truncate(&mut bare);
    assert!(!report.guide_dropped && report.dropped_turns == 0);
    assert!(
        report.final_estimate > counter.prompt_cap(),
        "the irreducible prompt now FITS a 4096-token window ({} of {}). That is \
         good news and this test is the wrong shape for it — replace it with one \
         that asserts the fit.",
        report.final_estimate,
        counter.prompt_cap()
    );
}
