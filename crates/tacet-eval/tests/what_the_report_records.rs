//! What the report keeps of a run that cost 48 minutes.
//!
//! WHY THIS EXISTS. The trace printed the token count, the rate, the stop reason,
//! the tool and its wall clock to stderr; the JSON report kept a list of tool
//! NAMES. So the ordinary follow-up questions after a run — which pass was slow,
//! what arguments did the model actually write, was that second call refused
//! before it ran — could only be answered by running the 48 minutes again. And a
//! re-run on a sampled model answers a different question than the one asked.
//!
//! This drives one case through the fake engine and asserts the record survives
//! into the outcome. It is the fake engine on purpose: a scripted generation is
//! the only way to know what the record SHOULD say.

use std::sync::Arc;

use tacet_engine::EngineProvider;
use tacet_engine::fake::FakeEngine;
use tacet_eval::tool_selection::{SelectionCase, SelectionStep, run_selection_case};

fn case(name: &str, message: &str, expected: &str) -> SelectionCase {
    SelectionCase {
        name: name.into(),
        category: tacet_eval::Category::Tool,
        steps: vec![SelectionStep::new(message, Some(expected))],
    }
}

#[test]
fn a_pass_records_what_it_cost_and_what_it_did() {
    // Two passes: a call, then the sentence for the user.
    let engine: Arc<dyn EngineProvider> = Arc::new(FakeEngine::script([
        r#"calculate({"expression":"125*8"})"#,
        "125 times 8 is 1000.",
    ]));
    let outcome = run_selection_case(&case("probe", "What is 125 times 8?", "calculate"), &engine);
    let step = &outcome.steps[0];
    assert!(
        step.passes.len() >= 2,
        "one record per pass of the loop; got {:?}",
        step.passes
    );

    let first = &step.passes[0];
    assert_eq!(first.pass, 1);
    assert_eq!(first.tool.as_deref(), Some("calculate"));
    assert_eq!(
        first.reason.as_deref(),
        Some("Ok"),
        "a call that ran and a call that was refused before running are the same \
         entry in `called`; this is where they differ"
    );
    assert_eq!(
        first.args.as_deref(),
        Some(r#"{"expression":"125*8"}"#),
        "the arguments come from the executor's own parse, so a recovered call \
         shape records what actually ran rather than null"
    );
    assert!(
        first.tool_seconds.is_some(),
        "the tool's own wall clock is separate from the model's — calendar-day \
         reads as a 39 s case of which 30 s is osascript"
    );

    let last = step.passes.last().expect("at least one pass");
    assert!(
        last.tool.is_none(),
        "the pass that produced the answer ran no tool"
    );
    assert!(
        !last.stop.is_empty(),
        "every pass records why generation stopped"
    );
}

/// A call the executor REFUSED is not the same event as one that ran, and
/// `called` records them identically — both are just the name.
#[test]
fn a_refused_call_is_distinguishable_from_one_that_ran() {
    let engine: Arc<dyn EngineProvider> = Arc::new(FakeEngine::script([
        r#"calculate({"expression":"125*8"})"#,
        r#"calculate({"expression":"125*8"})"#,
        "125 times 8 is 1000.",
    ]));
    let outcome = run_selection_case(&case("probe", "What is 125 times 8?", "calculate"), &engine);
    let step = &outcome.steps[0];
    let reasons: Vec<&str> = step
        .passes
        .iter()
        .filter_map(|p| p.reason.as_deref())
        .collect();
    assert!(
        reasons.contains(&"RepeatedCall"),
        "the same call twice in one turn is refused before it runs; the report \
         should say so rather than listing the tool name twice. reasons: {reasons:?}"
    );
}

/// The recorded passes must reach the printed table.
///
/// A FIELD NO READER CONSULTS IS A FIELD THAT ROTS. `PassRecord` carried the
/// cost and the verdict of every pass and nothing read it, so the three
/// questions actually asked after a run — which case was slow, how many passes
/// were spent on a call that never ran, how many ended on a cap — still meant
/// re-running the suite with the trace on.
#[test]
fn the_table_says_where_the_time_went() {
    let engine: Arc<dyn EngineProvider> = Arc::new(FakeEngine::script([
        r#"calculate({"expression":"125*8"})"#,
        r#"calculate({"expression":"125*8"})"#,
        "125 times 8 is 1000.",
    ]));
    let report = tacet_eval::run_selection(
        &[case("probe", "What is 125 times 8?", "calculate")],
        &engine,
    );
    let table = report.table();
    assert!(
        table.contains("PASSES"),
        "the pass count and what it was spent on must be in the table:\n{table}"
    );
    assert!(
        table.contains("1 spent on a call that never ran"),
        "the repeat cost a pass and the table must say so:\n{table}"
    );
    assert!(
        table.contains("SLOWEST"),
        "one case at 39s among 184 at 15s is not a slow model, it is one tool \
         waiting on something — and the table must name it:\n{table}"
    );
    assert!(
        table.contains("probe"),
        "the slowest line must name the case:\n{table}"
    );
}

/// A report with no passes recorded — an older JSON read back, or a host
/// failure — must print no section rather than a row of zeros that reads as a
/// measurement.
#[test]
fn a_report_with_no_passes_prints_no_section() {
    let engine: Arc<dyn EngineProvider> = Arc::new(FakeEngine::script(["hello"]));
    let report = tacet_eval::run_selection(&[], &engine);
    assert!(!report.table().contains("PASSES"));
}
