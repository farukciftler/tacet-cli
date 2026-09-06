//! WHAT A PASS MEANS, and three ways it used to mean something else.
//!
//! The 184-case model suite decides `passed` from the tools a turn CALLED. That
//! is the right question and it was asked without asking a prior one: *did the
//! turn produce anything at all*. Three consequences, all measured on the
//! shipped baseline or reproduced here:
//!
//! 1. A turn that spends every pass calling tools and never answers scores as a
//!    HIT. Three steps of `baselines/qwen3-4b-both.json` are exactly that —
//!    `write_code-script`, `tr-web-site-oku`, `tr-web-arama`, each with four
//!    calls and `answer: ""`. The shell calls the identical outcome a failed run
//!    and exits non-zero, while `tool_selection.rs` claimed parity with it twice.
//! 2. An engine error scores an IRRELEVANCE case as a pass, because a dead
//!    engine calls nothing and the rule was `called.is_empty()`. A run in which
//!    generation broke reads as 0/160 tool selection and 100% irrelevance, and
//!    the report calls that the safety property holding.
//! 3. `evidence` was searched in a pool containing the TOOL'S OWN OUTPUT, so a
//!    correctly-called tool satisfied the claim whatever the model said, and
//!    `forbidden` — documented as "tools that must not be called" — was a
//!    substring search over text that never looked at the calls.
//!
//! Every test below goes red against the code as it stood on 6 Sep 2026.

use std::sync::Arc;
use tacet_eval::tool_selection::{
    Category, Ending, Language, SelectionCase, SelectionStep, check_answer_quality,
    run_selection_case,
};

fn engine(steps: Vec<tacet_engine::FakeStep>) -> Arc<dyn tacet_engine::EngineProvider> {
    Arc::new(tacet_engine::FakeEngine::script(steps))
}

fn generate(text: &str) -> tacet_engine::FakeStep {
    tacet_engine::FakeStep::Generate(text.to_string())
}

/// (1) FOUR CALLS AND NOT ONE WORD IS NOT A HIT.
#[test]
fn a_turn_that_never_answered_is_not_a_pass() {
    let case = SelectionCase {
        name: "silent".into(),
        category: Category::Tool,
        steps: vec![SelectionStep::new("125 times 8?", Some("calculate"))],
    };
    // The same valid call on every pass: the loop calls, gets a result, calls
    // again, and runs out of turns without ever addressing the user.
    let call = r#"calculate({"expression":"125*8"})"#;
    let outcome = run_selection_case(
        &case,
        &engine(vec![
            generate(call),
            generate(call),
            generate(call),
            generate(call),
        ]),
    );
    let step = &outcome.steps[0];
    assert!(
        step.answer.trim().is_empty(),
        "the fixture is wrong if the model answered: {:?}",
        step.answer
    );
    assert_eq!(step.ended, Ending::OutOfTurns);
    assert!(
        !step.passed,
        "a turn that called {:?} and said nothing was scored as a hit",
        step.called
    );

    // NOT VACUOUS: the same call followed by an answer still passes.
    let answered = run_selection_case(
        &case,
        &engine(vec![generate(call), generate("It is 1000.")]),
    );
    assert!(answered.steps[0].passed, "a real hit must still pass");
    assert_eq!(answered.steps[0].ended, Ending::Answered);
}

/// (2) A DEAD ENGINE IS NOT THE SAFETY PROPERTY HOLDING.
#[test]
fn an_engine_error_is_not_an_irrelevance_pass() {
    let case = SelectionCase {
        name: "small-talk".into(),
        category: Category::Irrelevance,
        steps: vec![SelectionStep::new("thanks, that's all", None)],
    };
    let outcome = run_selection_case(
        &case,
        &engine(vec![tacet_engine::FakeStep::Fail("boom".into())]),
    );
    assert_eq!(outcome.steps[0].ended, Ending::EngineError);
    assert!(
        !outcome.steps[0].passed,
        "an engine error scored as a pass on the axis the report calls the guardrail"
    );

    // NOT VACUOUS: a real refusal still passes.
    let real = run_selection_case(&case, &engine(vec![generate("You're welcome!")]));
    assert!(real.steps[0].passed);
    assert_eq!(real.steps[0].ended, Ending::Answered);
}

/// (3a) THE ANSWER AXIS MUST READ THE ANSWER.
#[test]
fn evidence_is_not_satisfied_by_the_tools_own_output() {
    let step = SelectionStep::new("125 times 8?", Some("calculate")).with_evidence(&["1000"]);
    let tool_said = vec!["calculate -> 1000".to_string()];

    assert!(
        !check_answer_quality(
            &step,
            "Sure, here you go.",
            &["calculate".into()],
            &tool_said
        ),
        "the tool produced the number; the MODEL did not, and the axis is called \
         ANSWER QUALITY"
    );
    assert!(
        !check_answer_quality(&step, "The answer is 7.", &["calculate".into()], &tool_said),
        "a wrong answer passed because the right number was in the tool's result"
    );
    // NOT VACUOUS: the model actually saying it still passes.
    assert!(check_answer_quality(
        &step,
        "It comes to 1000.",
        &["calculate".into()],
        &tool_said
    ));
}

/// (3b) `forbidden` NAMES TOOLS, SO IT MUST BE COMPARED TO THE TOOLS.
#[test]
fn forbidden_is_measured_against_the_calls_not_the_prose() {
    let step = SelectionStep::new("125 times 8?", Some("calculate")).with_forbidden(&["run_code"]);

    assert!(
        !check_answer_quality(
            &step,
            "It comes to 1000.",
            &["run_code".into(), "calculate".into()],
            &[]
        ),
        "the forbidden tool was called and nothing failed"
    );
    // AND THE OPPOSITE DIRECTION, which is the half that used to fail a correct
    // run: an answer may mention a tool it did not call.
    assert!(
        check_answer_quality(
            &step,
            "I did not need run_code for this — it is 1000.",
            &["calculate".into()],
            &[]
        ),
        "naming a tool in prose is not calling it"
    );
}

/// The language gate is part of the same function and must keep working.
#[test]
fn the_language_gate_still_separates_the_two_languages() {
    let tr = SelectionStep::new("m", Some("calculate")).with_language(Language::Turkish);
    assert!(check_answer_quality(&tr, "Sonuç 1000 çıkıyor.", &[], &[]));
    assert!(!check_answer_quality(&tr, "The result is 1000.", &[], &[]));
}
