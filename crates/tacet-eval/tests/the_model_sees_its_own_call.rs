//! The eval must feed back what the shell feeds back.
//!
//! WHY. The shell pushes the model's generation into the history as an
//! `assistant` turn BEFORE the tool result, and its comment records the defect
//! that put it there: fed only the RESULT, the model saw a context-free line
//! with the user's question below it, took the question for unanswered, and
//! called the same tool again — up to the turn limit, without ever answering.
//! That is the `OutOfTurns` ending this suite reports and scores against the
//! model.
//!
//! The eval pushed only the result. So it measured a model with no record of
//! its own actions, counted the repeats against it, and published the number as
//! the shell's. It is also a deviation from the template the model was trained
//! on, in which a tool response follows the assistant turn that asked for it.
//!
//! This is the fourth eval/shell divergence found in one night, and they all
//! ran the same way: the eval was stricter than the program, in a direction that
//! looks like the model failing.

use std::sync::Arc;

use tacet_engine::EngineProvider;
use tacet_engine::fake::FakeEngine;
use tacet_eval::tool_selection::{SelectionCase, SelectionStep, run_selection_case};

#[test]
fn the_generation_that_made_the_call_is_in_the_next_prompt() {
    let engine = Arc::new(FakeEngine::script([
        r#"calculate({"expression":"125*8"})"#,
        "125 times 8 is 1000.",
    ]));
    let provider: Arc<dyn EngineProvider> = engine.clone();
    let case = SelectionCase {
        name: "probe".into(),
        category: tacet_eval::Category::Tool,
        steps: vec![SelectionStep::new(
            "What is 125 times 8?",
            Some("calculate"),
        )],
    };
    let outcome = run_selection_case(&case, &provider);
    assert!(outcome.passed, "the fixture must pass");

    let prompts = engine.seen_prompts();
    assert!(prompts.len() >= 2, "the loop must have made a second pass");
    let second = &prompts[1];
    assert!(
        second.contains(r#"calculate({"expression":"125*8"})"#),
        "the model's own call is missing from the prompt it gets after the tool \
         ran, so it has no record of having called anything:\n{second}"
    );
    assert!(
        second.contains("1000"),
        "the tool result must still be there too:\n{second}"
    );
}

/// The call comes BEFORE the result. The other order is not the template the
/// model was trained on, and it reads as a result that arrived first.
#[test]
fn the_call_comes_before_its_result() {
    let engine = Arc::new(FakeEngine::script([
        r#"calculate({"expression":"125*8"})"#,
        "125 times 8 is 1000.",
    ]));
    let provider: Arc<dyn EngineProvider> = engine.clone();
    let case = SelectionCase {
        name: "probe".into(),
        category: tacet_eval::Category::Tool,
        steps: vec![SelectionStep::new(
            "What is 125 times 8?",
            Some("calculate"),
        )],
    };
    let _ = run_selection_case(&case, &provider);
    let second = &engine.seen_prompts()[1];
    let call_at = second
        .find(r#"calculate({"expression":"125*8"})"#)
        .expect("the call");
    let result_at = second.rfind("1000").expect("the result");
    assert!(
        call_at < result_at,
        "the result appears before the call that asked for it:\n{second}"
    );
}
