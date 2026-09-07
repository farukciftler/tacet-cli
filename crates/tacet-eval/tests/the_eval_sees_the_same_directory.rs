//! The eval must send the working-directory census the shell sends.
//!
//! WHY. The shell puts a `<cwd>` block in the SYSTEM block — the one piece
//! truncation never touches — on every turn, listing the files in the working
//! directory. Its own comment says why: "what's in here?" is the first thing a
//! person types, and answering it otherwise costs a tool round trip.
//!
//! The eval built its system block from `SYSTEM_INSTRUCTIONS` alone. So the
//! suite asked "which file is about the budget?" of a model that had not been
//! told which files exist, while a real user's model had — and more than thirty
//! of the suite's cases are about the files in the working directory. It was
//! measuring a harder program than the one it claims to measure. Sixth time,
//! same shape.

use std::sync::Arc;

use tacet_engine::EngineProvider;
use tacet_engine::fake::FakeEngine;
use tacet_eval::tool_selection::{SelectionCase, SelectionStep, run_selection_case};

#[test]
fn the_prompt_names_the_files_the_case_can_actually_reach() {
    let engine = Arc::new(FakeEngine::script([
        r#"find_file({"pattern":"budget"})"#,
        "it is budget.md.",
    ]));
    let provider: Arc<dyn EngineProvider> = engine.clone();
    let case = SelectionCase {
        name: "probe".into(),
        category: tacet_eval::Category::Tool,
        steps: vec![SelectionStep::new(
            "Which file is about the budget?",
            Some("find_file"),
        )],
    };
    let _ = run_selection_case(&case, &provider);

    let first = engine
        .seen_prompts()
        .first()
        .cloned()
        .expect("a prompt was built");
    assert!(
        first.contains("<cwd>"),
        "no working-directory census in the system block, so the model is being \
         asked which file is about the budget without being told what is \
         there:\n{}",
        &first[..first.len().min(600)]
    );
    // THE FIXTURE'S OWN FILES. `Env::setup` writes them; if the census is real
    // it names them, and if it is a stub it does not.
    assert!(
        first.contains(".md") || first.contains(".txt"),
        "the census is present but lists nothing the fixture wrote:\n{}",
        &first[..first.len().min(600)]
    );
}

/// The census must be in the SYSTEM block, not the history: that is the one
/// piece truncation never touches, which is the whole reason the shell puts it
/// there rather than in a first turn.
#[test]
fn the_census_is_in_the_system_block() {
    let engine = Arc::new(FakeEngine::script(["nothing to do here."]));
    let provider: Arc<dyn EngineProvider> = engine.clone();
    let case = SelectionCase {
        name: "probe".into(),
        category: tacet_eval::Category::Irrelevance,
        steps: vec![SelectionStep::new("Hello, how are you?", None)],
    };
    let _ = run_selection_case(&case, &provider);
    let first = engine.seen_prompts().first().cloned().expect("a prompt");
    let cwd_at = first.find("<cwd>").expect("a census");
    let system_end = first
        .find("</system>")
        .expect("a plain-format system block");
    assert!(
        cwd_at < system_end,
        "the census landed outside the system block, where truncation can reach \
         it:\n{}",
        &first[..first.len().min(600)]
    );
}
