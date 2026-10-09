//! The eval must ask a ChatML model what the shell asks it.
//!
//! WHY THIS EXISTS (audit M23). The shell appends the Qwen soft switch —
//! ` /think` or ` /no_think` — to every user turn on a ChatML engine. The rule
//! was a private function in `tacet-cli`'s `main.rs`, the eval could not reach
//! it, and so it sent the bare message. Hybrid Qwen3 builds were measured in
//! their default thinking mode, which no user of this program is ever given;
//! the cost of that mode is on record in `tool_selection.rs` as a median of 237
//! tokens per turn against 19.
//!
//! The rule now lives in `tacet_engine::thinking_switch`, and this pins that the
//! eval uses it — on the question, and in the history the next step replays.

use std::sync::Arc;

use tacet_engine::fake::FakeEngine;
use tacet_engine::{
    Constrainer, EngineProvider, GenerationFuture, Prompt, SamplingSetting, Template,
};
use tacet_eval::tool_selection::{SelectionCase, SelectionStep, run_selection_case};

/// `FakeEngine` declaring the ChatML template, which is the only thing the
/// switch keys on. Everything else is the fake's, including the record of every
/// prompt it was sent.
struct ChatMlFake(Arc<FakeEngine>);

impl EngineProvider for ChatMlFake {
    fn name(&self) -> &str {
        "fake-chatml"
    }
    fn template(&self) -> Template {
        Template::ChatML
    }
    fn generate<'a>(
        &'a self,
        prompt: &'a Prompt,
        constraint: Option<&'a dyn Constrainer>,
        setting: SamplingSetting,
    ) -> GenerationFuture<'a> {
        self.0.generate(prompt, constraint, setting)
    }
}

fn one_step(message: &str) -> SelectionCase {
    SelectionCase {
        name: "switch".into(),
        category: tacet_eval::Category::Irrelevance,
        steps: vec![SelectionStep::new(message, None)],
    }
}

fn prompts_for(case: &SelectionCase, chatml: bool) -> Vec<String> {
    let fake = Arc::new(FakeEngine::script(Vec::<String>::new()).with_default("an answer."));
    let provider: Arc<dyn EngineProvider> = if chatml {
        Arc::new(ChatMlFake(fake.clone()))
    } else {
        fake.clone()
    };
    let _ = run_selection_case(case, &provider);
    fake.seen_prompts()
}

#[test]
fn a_plain_turn_on_a_chatml_model_is_sent_with_no_think() {
    let prompts = prompts_for(&one_step("What is 2 plus 2?"), true);
    assert!(
        prompts[0].contains("What is 2 plus 2? /no_think"),
        "the shell sends `/no_think` on this message and the eval did not:\n{}",
        prompts[0]
    );
}

#[test]
fn a_planning_turn_on_a_chatml_model_is_sent_with_think() {
    let prompts = prompts_for(&one_step("Summarise the quarterly budget"), true);
    assert!(
        prompts[0].contains("Summarise the quarterly budget /think"),
        "{}",
        prompts[0]
    );
}

/// NOT VACUOUS: a template without the convention gets nothing appended, or the
/// assertions above would be satisfied by suffixing every prompt.
#[test]
fn a_model_without_the_convention_gets_the_bare_message() {
    let prompts = prompts_for(&one_step("What is 2 plus 2?"), false);
    assert!(!prompts[0].contains("/no_think") && !prompts[0].contains("/think"));
}

/// The history the second step replays is what the model was given on the
/// first, as in the shell — `asked`, not the bare message.
#[test]
fn the_next_step_replays_the_switched_question() {
    let case = SelectionCase {
        name: "two".into(),
        category: tacet_eval::Category::MultiTurn,
        steps: vec![
            SelectionStep::new("What is 2 plus 2?", None),
            SelectionStep::new("and 3 plus 3?", None),
        ],
    };
    let prompts = prompts_for(&case, true);
    let second = prompts.last().expect("a second prompt");
    assert!(
        second.contains("What is 2 plus 2? /no_think"),
        "the first step's question is in the history without its switch:\n{second}"
    );
}

/// THE TWO FOLDS MUST AGREE. `thinking_switch::fold` is a copy of
/// `tacet_tools::router::simplify` because the engine crate sits below the tools
/// crate; if they ever differ, the shell and the eval would pick different
/// modes for the same Turkish message. Every message in the suite, both
/// languages, plus the letters the fold exists for.
#[test]
fn the_engines_fold_is_the_routers_fold() {
    let mut messages: Vec<String> = tacet_eval::selection_suite()
        .into_iter()
        .flat_map(|c| c.steps.into_iter().map(|s| s.message))
        .collect();
    messages.push("IİıŞşĞğÜüÖöÇçÂâÎîÛû ÖZETLE Adım Adım KARŞILAŞTIR".into());
    assert!(messages.len() > 100, "the suite is the corpus");
    for m in &messages {
        assert_eq!(
            tacet_engine::thinking_switch::fold(m),
            tacet_tools::router::simplify(m),
            "the two folds disagree on {m:?}"
        );
    }
}
