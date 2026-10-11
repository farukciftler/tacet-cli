//! The eval must shrink a prompt the way the shell shrinks it, and refuse what
//! the shell refuses.
//!
//! WHY. `FakeEngine` declares no model path and no context length, so
//! `context_budget` returns the floor — `CONTEXT_BUDGET`, 4096 — and the prompt
//! cap is 3072. That is not a test artefact: a real GGUF that declares nothing
//! lands there too, and so does every llama-family model on candle, whose rope
//! table is 4096 whatever the file says. A model that declares LESS than the
//! floor keeps its own, smaller window (audit L21): the floor no longer lifts it.
//!
//! The shell calls `TokenCounter::fit` on every prompt — drop the oldest turns,
//! then sacrifice the guide, then trim the question, and refuse the turn when
//! what is left still leaves the answer less than `MIN_ANSWER`. The eval used
//! to call nothing, then `truncate` alone; on such a machine it sent histories
//! the shell would have cut, and then prompts the shell would have refused,
//! with a generation cap of a few tokens or none. Either way it measured a
//! program nobody runs.
//!
//! WHAT THE FIRST VERSION OF THIS TEST ASSERTED, AND WHY IT WAS WRONG. It
//! required every prompt to be inside the cap. Neither program can promise
//! that: truncation never touches the system block or the tool descriptions —
//! by policy, because a truncated instruction makes the model forget who it is
//! and a truncated description makes it invent a signature — and with the
//! default catalog those two alone are over the 3072-token cap. That is a fact
//! about the floor, it applies to the shell exactly as much, and it is asserted
//! below rather than left to be rediscovered.

use std::sync::Arc;

use tacet_engine::fake::FakeEngine;
use tacet_engine::{
    Constrainer, EngineProvider, GenerationFuture, Prompt, SYSTEM_INSTRUCTIONS, SamplingSetting,
    TokenCounter,
};
use tacet_eval::tool_selection::{Ending, SelectionCase, SelectionStep, run_selection_case};

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

/// THE TOOL DESCRIPTIONS ARE INSIDE THE BUDGET.
///
/// WHY THE TEST ABOVE WAS NOT ENOUGH. Its first message is so long that the
/// history overflows the cap with or without the `<tools>` block, so it stayed
/// green while the eval truncated BEFORE attaching the tools and the shell
/// truncated AFTER (audit M22, T3). Measured on a 4096 window: the shell dropped
/// 21 turns and sent 3 016 tokens, the eval dropped none and sent 4 502.
///
/// So this fixture is sized to overflow ONLY with the tools: the second step's
/// prompt without its tools fits under the cap with a margin, and with them it
/// does not. The size is computed from the same pieces the eval assembles — the
/// system block with its census, the router's selection, the guide and the
/// nudge — rather than written down, because the tool descriptions change and a
/// hard-coded length would quietly stop being in the window that matters.
#[test]
fn a_history_that_fits_only_without_the_tools_is_still_cut() {
    use tacet_engine::Turn;
    use tacet_eval::env::Env;
    use tacet_eval::tool_selection::suite_catalog;
    use tacet_tools::memory::SharedMemory;

    const OLDEST: &str = "ZZ-OLDEST-TURN-MARKER-ZZ";
    const FIRST_ANSWER: &str = "the first answer.";
    const SECOND: &str = "and now the total";
    // A FEW HUNDRED TOKENS EITHER SIDE, so the test does not turn on the census
    // of one temporary directory being a few bytes longer than another's.
    const MARGIN: usize = 120;

    let Ok(env) = Env::setup() else {
        eprintln!("skipped: the eval environment could not be set up on this machine");
        return;
    };
    let memory = SharedMemory::in_memory();
    let catalog = suite_catalog(&env, &memory).catalog;
    let selected: tacet_kernel::ToolCatalog = tacet_tools::router::Router::new()
        .select(SECOND, &catalog)
        .into_iter()
        .collect();
    let selected_names: Vec<String> = selected.names().into_iter().map(String::from).collect();
    let guide = tacet_skills::SkillStore::default_set()
        .matching(SECOND, Some(&selected_names))
        .map(tacet_skills::injection_text);
    let note = (tacet_tools::router::score_intent(SECOND).dominant()
        == tacet_tools::router::IntentProfile::Web)
        .then_some(tacet_engine::WEB_NUDGE);
    let system =
        tacet_engine::system_text(tacet_engine::dir_context(&env.dir().to_string_lossy()).as_ref());

    let first_message = |k: usize| {
        format!(
            "{OLDEST} {}",
            "and the quarterly budget review totals against the summary from last week ".repeat(k)
        )
    };
    let second_step = |k: usize, with_tools: bool| {
        let mut p = Prompt::new(&system, SECOND)
            .with_history([Turn::user(first_message(k)), Turn::assistant(FIRST_ANSWER)]);
        if with_tools {
            p = p.with_tools(&selected);
        }
        if let Some(g) = &guide {
            p = p.with_guide(g);
        }
        if let Some(n) = note {
            p = p.with_note(n);
        }
        TokenCounter::estimate(&p.text())
    };

    let cap = floor_counter().prompt_cap();
    let mut k = 0;
    while second_step(k + 1, false) + MARGIN <= cap {
        k += 1;
    }
    assert!(k > 0, "the system block alone leaves no room for a history");
    assert!(
        second_step(k, false) + MARGIN <= cap,
        "the fixture must FIT without the tools, or it measures what the test above does"
    );
    assert!(
        second_step(k, true) > cap + MARGIN,
        "the fixture must NOT fit with the tools ({} of {cap}); the tools are too small \
         for this shape and it measures nothing",
        second_step(k, true)
    );

    let first = first_message(k);
    let engine =
        Arc::new(FakeEngine::script([FIRST_ANSWER, "the second answer."]).with_default("done."));
    let provider: Arc<dyn EngineProvider> = engine.clone();
    let case = SelectionCase {
        name: "fits-without-tools".into(),
        category: tacet_eval::Category::MultiTurn,
        steps: vec![
            SelectionStep::new(&first, None),
            SelectionStep::new(SECOND, None),
        ],
    };
    let _ = run_selection_case(&case, &provider);

    let prompts = engine.seen_prompts();
    assert_eq!(
        prompts.len(),
        2,
        "one pass per step: both answers are prose"
    );
    let sent = &prompts[1];
    assert!(
        sent.contains("<tools>"),
        "the second step must have been offered its tools, or there is nothing to budget"
    );
    assert!(
        !sent.contains(OLDEST),
        "the oldest turn survived although the prompt with its tools is over the cap — \
         the eval truncated before the tools were attached, and the shell does not"
    );
    assert!(
        TokenCounter::estimate(sent) <= floor_counter().prompt_cap(),
        "the prompt sent is over the cap: {} > {}",
        TokenCounter::estimate(sent),
        floor_counter().prompt_cap()
    );
}

/// THE FLOOR CANNOT FIT THE WHOLE CATALOG UNDER THE CAP, and both programs are
/// in the same position.
///
/// Truncation never touches the system block or the tool descriptions. With
/// the default catalog those two alone are over the 3072-token cap a 4096
/// window gives, so on such a machine the prompt is over the cap with an EMPTY
/// history. What is wrong on that machine is the window, not the truncation.
/// Measured 10 Oct 2026 (macOS arm64): 4363 tokens with every tool, against a
/// cap of 3072 and a ceiling (`TokenCounter::prompt_ceiling`, the most `fit`
/// sends) of 3840 — over both, so a turn offered the whole catalog would be
/// refused. Turns run on the floor because the router offers at most nine.
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

/// An engine that is `FakeEngine` in every way but its window: it declares
/// `window` tokens, as a small GGUF does.
struct SmallWindow {
    inner: Arc<FakeEngine>,
    window: usize,
}

impl EngineProvider for SmallWindow {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn context_length(&self) -> Option<usize> {
        Some(self.window)
    }
    fn vocab(&self) -> Option<Vec<String>> {
        self.inner.vocab()
    }
    fn generate<'a>(
        &'a self,
        prompt: &'a Prompt,
        constraint: Option<&'a dyn Constrainer>,
        setting: SamplingSetting,
    ) -> GenerationFuture<'a> {
        self.inner.generate(prompt, constraint, setting)
    }
}

/// THE EVAL REFUSES WHAT THE SHELL REFUSES (review of audit L21). On a
/// 2048-token window the suite's system block and tool descriptions alone are
/// over what a prompt may take and still leave `MIN_ANSWER`. The shell sends
/// nothing; the eval used to `truncate` and generate anyway, with whatever cap
/// was left, and score the result against the model. The step must end as
/// `Refused`, with the engine never asked.
#[test]
fn a_prompt_the_shell_would_refuse_is_refused_here_too() {
    let engine = Arc::new(FakeEngine::script(["42"]).with_default("done."));
    let provider: Arc<dyn EngineProvider> = Arc::new(SmallWindow {
        inner: engine.clone(),
        window: 2048,
    });
    let case = SelectionCase {
        name: "small".into(),
        category: tacet_eval::Category::Tool,
        steps: vec![SelectionStep::new(
            "What is 125 times 8?",
            Some("calculate"),
        )],
    };
    let outcome = run_selection_case(&case, &provider);
    assert_eq!(
        outcome.steps[0].ended,
        Ending::Refused,
        "the eval sent a prompt the shell would refuse"
    );
    assert!(
        engine.seen_prompts().is_empty(),
        "the engine was asked anyway: {:?}",
        engine.seen_prompts().first().map(|p| p.len())
    );
    assert!(!outcome.passed, "a refused turn is not a pass");
    assert!(!Ending::Refused.is_measurable());
}
