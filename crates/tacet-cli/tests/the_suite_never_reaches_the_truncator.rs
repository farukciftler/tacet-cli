//! The eval does not truncate, and this is what says it does not have to.
//!
//! THE DIVERGENCE, AND IT WAS NOT HARMLESS. The shell calls
//! `TokenCounter::truncate` on every prompt — drop old turns, then sacrifice the
//! guide, then trim the question — and the eval never called it. The assumption
//! was that the suite's prompts are far under the cap so it would be a no-op.
//!
//! Measured, that assumption is false on the floor. `context_budget` returns
//! `CONTEXT_BUDGET` — 4096 — whenever a model declares no context length or the
//! device cannot afford more, and it is documented as a floor precisely so such
//! a machine "must not end up worse off". The prompt cap there is 3072. The
//! worst prompt this suite can build measures **5617**, nearly twice that. On
//! such a machine the shell drops turns and then the guide, and the eval sent
//! the whole thing.
//!
//! The eval truncates now. What this test measures is the other half: on the
//! window the published numbers were taken at, the worst case must fit WITHOUT
//! truncation — because a number produced from a prompt whose guide was
//! sacrificed is a different measurement wearing the same name.
//!
//! NO WEIGHTS ARE LOADED. `TokenCounter` estimates from bytes when it has no
//! tokenizer, which is the same estimate the shell's truncation decision uses
//! before generation starts.

use std::sync::Arc;

use tacet_engine::{Prompt, SYSTEM_INSTRUCTIONS, TokenCounter, Turn, WEB_NUDGE};
use tacet_eval::tool_selection::{selection_cases, turkish_selection_cases};
use tacet_tools::data_store::SharedStore;
use tacet_tools::memory::SharedMemory;

/// The window the published numbers were measured at: `qwen3-4b` on Metal
/// reports a generation cap of ~13.9k against a ~2k prompt, so the budget is
/// 16384. This test is about THAT machine — the one the README quotes — and
/// says the guide was never sacrificed to produce those numbers.
///
/// It is deliberately NOT the 4096 floor. On the floor the suite does not fit
/// and the shell truncates; that is now true of the eval too, and it is a fact
/// about small machines rather than a defect. What must not happen is the
/// MEASUREMENT box quietly crossing the line.
const MEASUREMENT_WINDOW: usize = 16384;

#[test]
fn the_longest_prompt_the_suite_can_build_fits_without_truncation() {
    let store = Arc::new(SharedStore::new());
    let memory = SharedMemory::in_memory();
    let (catalog, _, _) =
        tacet_tools::catalog::production_catalog_with(&store, &memory, Some(0), true);

    let skills = tacet_skills::SkillStore::default_set();
    let names: Vec<String> = catalog.names().into_iter().map(String::from).collect();
    let longest_guide = skills
        .all()
        .filter(|s| s.has_tools(Some(&names)))
        .map(tacet_skills::injection_text)
        .max_by_key(String::len)
        .expect("at least one skill");

    let longest_message = selection_cases()
        .into_iter()
        .chain(turkish_selection_cases())
        .flat_map(|c| c.steps.into_iter().map(|s| s.message))
        .max_by_key(String::len)
        .expect("at least one case");

    // A FULL TURN'S HISTORY: the shell keeps the user message, then for each
    // pass the model's own call and the tool's result. `MAX_TURNS` passes is the
    // most a step can spend, and a two-step case carries the first step's turn
    // in front of it — so this is doubled.
    let mut history = vec![Turn::user(&longest_message)];
    for _ in 0..(tacet_engine::MAX_TURNS * 2) {
        history.push(Turn::assistant(
            r#"find_file({"pattern":"budget","folder":".","search_content":true,"max_results":20})"#,
        ));
        history.push(Turn::tool(
            "read_document: 40 rows · | Day | Lunch | Cost |\n| Monday | Lentils | 120 |\n\
             (source_ref: doc-1) — a bulk result of the length the store returns",
        ));
    }

    let prompt = Prompt::new(SYSTEM_INSTRUCTIONS, &longest_message)
        .with_tools(&catalog)
        .with_guide(&longest_guide)
        .with_note(WEB_NUDGE)
        .with_history(history);

    let counter = TokenCounter::new(MEASUREMENT_WINDOW, tacet_engine::GENERATION_SHARE);
    let estimate = counter.prompt_estimate(&prompt);
    let cap = counter.prompt_cap();

    assert!(
        estimate <= cap,
        "the worst-case suite prompt is {estimate} tokens against a {cap} cap on \
         the {MEASUREMENT_WINDOW}-token window the published numbers were taken \
         at. The guide would be SACRIFICED to fit, and a number produced from a \
         prompt without its guide is a different measurement wearing the same \
         name. Find out what grew."
    );

    // AND WITH ROOM TO SPARE. A prompt that fits by ten tokens is one case away
    // from not fitting, and this test would then report a real divergence as a
    // sudden failure rather than as the drift it was.
    assert!(
        estimate * 2 <= cap,
        "the worst-case prompt is {estimate} of a {cap} cap — over half. It still \
         fits, but a new tool description or a longer guide could take it over, \
         and then the published numbers would silently come from prompts with no \
         guide in them."
    );
}
