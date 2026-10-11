//! ONE PASS'S PROMPT, built and shrunk in one place.
//!
//! WHY THIS IS A FUNCTION AND NOT TWO COPIES. The shell and the eval each built
//! the prompt for a pass and each called `TokenCounter::truncate` on it — and
//! they did it in a different ORDER. The shell attached the `<tools>` block and
//! then truncated; the eval truncated and then attached the tools. `truncate`
//! measures the whole prompt, so the eval's budget never saw roughly two
//! thousand tokens of tool descriptions.
//!
//! MEASURED (audit M22, 9 Oct 2026): a 4096 window, cap 3072, one long history.
//! The shell dropped 21 turns and sent 3 016 tokens; the eval dropped none and
//! sent 4 502 — a prompt the shell has never sent to anyone, measured on exactly
//! the small-window machines where the budget bites.
//!
//! The order below is the shell's, because the shell is what a user runs. It
//! lives in this crate rather than in `tacet-cli` because the dependency only
//! points one way: the shell already reads `tacet_eval::MAX_TURNS`, and the eval
//! cannot see anything in the binary.

use tacet_engine::{EngineResult, Prompt, TokenCounter, TruncationReport, Turn};
use tacet_kernel::ToolCatalog;

/// Everything one pass's prompt is made of. `tools` is `None` on the last pass,
/// which is offered no tools (see `FINAL_PASS_INSTRUCTION`).
pub struct TurnParts<'a> {
    pub system: &'a str,
    pub question: &'a str,
    pub history: Vec<Turn>,
    pub tools: Option<&'a ToolCatalog>,
    pub guide: Option<&'a str>,
    pub note: Option<&'a str>,
    pub memory: Option<&'a str>,
}

/// Builds the prompt and fits it into `counter`'s window, or refuses it.
///
/// THE TOOLS GO IN BEFORE THE TRUNCATION, and that is the whole reason this
/// function exists. `truncate` never cuts the tool descriptions — a truncated
/// description makes the model invent a signature — so they are fixed cost, and
/// the history, the guide and the question have to make room for them. Added
/// afterwards, they are a cost nobody budgeted for.
///
/// `fit`, NOT `truncate` ALONE (audit L21): a prompt whose fixed part leaves
/// less than `MIN_ANSWER` for the answer is refused, not sent with a generation
/// cap of a few tokens. The error is the caller's to report — the shell prints
/// it, the eval records `Ending::Refused` — so both refuse the same turns.
pub fn build_turn_prompt(
    counter: &TokenCounter,
    parts: TurnParts<'_>,
) -> EngineResult<(Prompt, TruncationReport)> {
    let mut prompt = Prompt::new(parts.system, parts.question).with_history(parts.history);
    if let Some(tools) = parts.tools {
        prompt = prompt.with_tools(tools);
    }
    if let Some(g) = parts.guide {
        prompt = prompt.with_guide(g);
    }
    if let Some(n) = parts.note {
        prompt = prompt.with_note(n);
    }
    if let Some(m) = parts.memory {
        prompt = prompt.with_memory(m);
    }
    let report = counter.fit(&mut prompt)?;
    Ok((prompt, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Env;
    use crate::tool_selection::suite_catalog;
    use tacet_tools::memory::SharedMemory;

    fn floor() -> TokenCounter {
        TokenCounter::new(
            tacet_engine::context_budget(None, None, tacet_engine::Device::Cpu),
            tacet_engine::GENERATION_SHARE,
        )
    }

    /// A history that fits the cap on its own and does not fit beside the tool
    /// descriptions must be cut. Truncating before the tools are attached keeps
    /// all of it and sends an over-budget prompt.
    #[test]
    fn the_tool_descriptions_are_inside_the_budget() {
        let Ok(env) = Env::setup() else {
            eprintln!("skipped: the eval environment could not be set up on this machine");
            return;
        };
        let host = suite_catalog(&env, &SharedMemory::in_memory());
        // Three tools, not the whole catalog: the whole catalog alone is over
        // the floor's cap (see `the_eval_truncates_like_the_shell`), and this
        // fixture needs room for a history that fits WITHOUT the tools.
        let tools: ToolCatalog = host.catalog.tools().iter().take(3).cloned().collect();
        let counter = floor();
        let cap = counter.prompt_cap();
        let tools_cost = TokenCounter::estimate(&Prompt::new("", "").with_tools(&tools).text())
            - TokenCounter::estimate(&Prompt::new("", "").text());
        assert!(
            tools_cost > 200,
            "the fixture needs tools that cost something: {tools_cost}"
        );

        // Grow the history until it sits just under the cap without tools.
        let mut history = Vec::new();
        let base = |h: &Vec<Turn>| {
            TokenCounter::estimate(
                &Prompt::new("You are a test.", "next?")
                    .with_history(h.clone())
                    .text(),
            )
        };
        while base(&history) + 60 < cap - tools_cost / 2 {
            history.push(Turn::user(
                "the quarterly budget review totals, line by line",
            ));
        }
        assert!(base(&history) <= cap, "fits without the tools");
        assert!(base(&history) + tools_cost > cap, "and not with them");

        let (prompt, report) = build_turn_prompt(
            &counter,
            TurnParts {
                system: "You are a test.",
                question: "next?",
                history,
                tools: Some(&tools),
                guide: None,
                note: None,
                memory: None,
            },
        )
        .expect("three tools and a short system block leave room for an answer");
        assert!(
            report.dropped_turns > 0,
            "nothing was dropped although the tools put the prompt over the cap"
        );
        assert!(
            counter.prompt_estimate(&prompt) <= cap,
            "the prompt that would be sent is over the cap: {} > {cap}",
            counter.prompt_estimate(&prompt)
        );
    }
}
