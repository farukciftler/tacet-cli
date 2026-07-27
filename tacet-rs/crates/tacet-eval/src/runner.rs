//! The eval runner — drives one case from start to finish.
//!
//! THE TURN LOOP: build the prompt -> generate -> is there a call -> execute ->
//! write the outcome into the history -> repeat. If there is no call the
//! generated text IS THE ANSWER and the loop ends.
//!
//! THE EVIDENCE POOL: the verification does not look only at the last sentence.
//! The model writing "1000" in its sentence is not evidence that the tool
//! computed 1000 — the model may have invented the number. That is why the pool
//! also carries tool outputs, chip texts and structural flags; the claim is made
//! at the level of "the SYSTEM PRODUCED this".
//!
//! THE ENGINE COMES FROM OUTSIDE: `EngineSelector` separates the two ends —
//! `FakeSelector` runs the case's script (a logic measurement, deterministic),
//! `SingleEngine` runs a real `EngineProvider` (a model measurement). The same
//! case list feeds both.

use crate::case::EvalCase;
use crate::env::{EXTERNAL_TOOL, Env};
use std::sync::Arc;
use tacet_core::{ToolCatalog, ToolContext, TraceCollector};
use tacet_engine::{
    EngineProvider, FakeEngine, MAX_TURNS, Prompt, SYSTEM_INSTRUCTIONS, SamplingSetting, Turn, wait,
};
use tacet_grammar::CallConstraint;
use tacet_tools::executor::{ExecutionReason, ToolExecutor};
use tacet_tools::router::Router;

/// The authority that produces an engine for a case.
pub trait EngineSelector {
    fn engine_for(&self, case: &EvalCase) -> Arc<dyn EngineProvider>;
    fn name(&self) -> &str;
}

/// The fake engine that runs the case's script. The default.
pub struct FakeSelector;

impl EngineSelector for FakeSelector {
    fn engine_for(&self, case: &EvalCase) -> Arc<dyn EngineProvider> {
        // When the script runs out, a fixed string rather than an error: the
        // case must not have to know the number of turns in advance.
        Arc::new(FakeEngine::script(case.script.clone()).with_default("Okay."))
    }
    fn name(&self) -> &str {
        "fake"
    }
}

/// The interface for a real engine: a single provider runs all the cases.
pub struct SingleEngine(pub Arc<dyn EngineProvider>);

impl EngineSelector for SingleEngine {
    fn engine_for(&self, _case: &EvalCase) -> Arc<dyn EngineProvider> {
        Arc::clone(&self.0)
    }
    fn name(&self) -> &str {
        self.0.name()
    }
}

/// The outcome of a single case.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaseOutcome {
    pub name: String,
    pub passed: bool,
    /// The tools that were called, in order.
    pub called: Vec<String>,
    /// Why it did not pass — empty means it passed.
    pub faults: Vec<String>,
    /// The model's last sentence.
    pub answer: String,
}

/// Runs one case. It DOES NOT RETURN a `Result`: even if the environment cannot
/// be set up, that is a CASE outcome (a failed one), not the whole run's.
pub fn run_case(case: &EvalCase, selector: &dyn EngineSelector) -> CaseOutcome {
    let env = match Env::setup() {
        Ok(e) => e,
        Err(e) => {
            return CaseOutcome {
                name: case.name.clone(),
                passed: false,
                called: Vec::new(),
                faults: vec![format!("the environment could not be set up: {e}")],
                answer: String::new(),
            };
        }
    };

    let catalog = env.catalog();
    let executor = ToolExecutor::new(catalog.clone()).external_tool(EXTERNAL_TOOL);
    // THE TRACE COLLECTOR IS NOW HELD ON TO. It used to be created inline here
    // as `Arc::new(...)` and the handle thrown away: because `traces()` could
    // never be called, eval never observed the STATE updates made through
    // `update_chip`. If a tool set its chip to the wrong state (reporting a
    // write job as `Read`, say) eval could not catch it — the evidence pool only
    // saw the `ToolOutcome`.
    let traces = Arc::new(TraceCollector::new());
    let mut ctx = ToolContext::new(
        Arc::clone(&env.store) as Arc<dyn tacet_core::DataStore>,
        env.dir(),
        Arc::clone(&traces) as Arc<dyn tacet_core::Reporter>,
    );

    let engine = selector.engine_for(case);
    // The grammar constraint RUNS in eval too: if the production path were
    // constrained and eval unconstrained, what is measured would not be the
    // application's behaviour. The exception is the cases measuring the
    // executor's LOWER layer (see `EvalCase::unconstrained`).
    let constraint = (!case.unconstrained)
        .then(|| engine.vocab().map(|v| CallConstraint::new(&v, &catalog)))
        .flatten();
    let router = Router::new();
    let ticket = executor.active_turn();

    let mut history: Vec<Turn> = Vec::new();
    let mut called: Vec<String> = Vec::new();
    // The pool: everything produced by the SYSTEM, not by the model.
    let mut evidence = String::new();
    let mut answer = String::new();
    let mut faults: Vec<String> = Vec::new();

    for _ in 0..MAX_TURNS {
        // The tool budget is applied AGAIN on every turn: as the history grows
        // the selection must not change, the selection must derive only from the
        // user's message.
        let selected: ToolCatalog = router.select(&case.input, &catalog).into_iter().collect();
        let prompt = Prompt::new(SYSTEM_INSTRUCTIONS, &case.input)
            .with_tools(&selected)
            .with_history(history.clone());

        let generation = match wait(
            engine.generate(
                &prompt,
                constraint
                    .as_ref()
                    .map(|c| c as &dyn tacet_engine::Constrainer),
                SamplingSetting::default(),
            ),
        ) {
            Ok(g) => g,
            Err(e) => {
                faults.push(format!("engine error: {e}"));
                break;
            }
        };
        // Half output IS NOT PARSED: a tool call that hit the token cap is half
        // a JSON.
        if !generation.stop.is_complete() {
            faults.push("generation was cut off halfway".into());
            break;
        }

        let Some(outcome) = wait(executor.execute_raw(&generation.text, ticket, &mut ctx)) else {
            answer = generation.text.clone();
            evidence.push('\n');
            evidence.push_str(&answer);
            break;
        };

        called.push(outcome.tool_name.clone());
        evidence.push('\n');
        evidence.push_str(&outcome.to_model);
        evidence.push('\n');
        evidence.push_str(&outcome.chip_text);
        if let Some(raw) = &outcome.raw_output {
            evidence.push('\n');
            evidence.push_str(raw);
        }
        // The structural flags are evidence too: the "no retry" claim is
        // verified from here, not from the text (see the executor, the ban on
        // text matching).
        evidence.push_str(&format!(
            "\nretryable={} world_changed={} reason={:?}",
            outcome.retryable, outcome.world_changed, outcome.reason
        ));

        // An approval denial and a cancellation END the turn: the model must not
        // enter an insistence loop.
        if matches!(
            outcome.reason,
            ExecutionReason::ApprovalDenied | ExecutionReason::Cancelled
        ) {
            history.push(Turn::tool(outcome.to_model.clone()));
            continue;
        }
        history.push(Turn::tool(outcome.to_model.clone()));
    }

    // THE CHIP TRACES ARE EVIDENCE TOO. The state coming from the collector is
    // an observation INDEPENDENT of what the `ToolOutcome` declared: if a tool
    // outcome says "Written" while leaving the chip at "Read" (or never updating
    // it), the difference shows up only here.
    for trace in traces.traces() {
        evidence.push('\n');
        evidence.push_str(&format!(
            "chip[{}] {} {:?}",
            trace.icon, trace.text, trace.state
        ));
    }
    // The collector's independent view of the side effects — if it contradicts
    // the executor's, one of the two is lying.
    evidence.push_str(&format!("\nchip_world_changed={}", traces.world_changed()));

    // --- The claims ---

    match &case.expected_tool {
        Some(name) if !called.iter().any(|c| c == name) => {
            faults.push(format!(
                "the expected tool was not called: {name} (called: {called:?})"
            ));
        }
        None if !called.is_empty() => {
            // Tool appetite: the most frequent regression.
            faults.push(format!(
                "no tool should have been called, these were: {called:?}"
            ));
        }
        _ => {}
    }

    for part in &case.expected_evidence {
        if !evidence.contains(part.as_str()) {
            faults.push(format!("evidence missing: {part:?}"));
        }
    }
    for part in &case.forbidden {
        if evidence.contains(part.as_str()) {
            faults.push(format!("forbidden evidence appeared: {part:?}"));
        }
    }

    CaseOutcome {
        name: case.name.clone(),
        passed: faults.is_empty(),
        called,
        faults,
        answer,
    }
}

/// Runs all the cases.
pub fn run(cases: &[EvalCase], selector: &dyn EngineSelector) -> crate::report::EvalReport {
    let outcomes = cases.iter().map(|c| run_case(c, selector)).collect();
    crate::report::EvalReport::new(selector.name(), outcomes)
}
