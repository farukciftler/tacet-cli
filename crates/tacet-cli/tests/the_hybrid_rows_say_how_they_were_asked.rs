//! A ROW MEASURED WITHOUT THE SHELL'S SOFT SWITCH SAYS SO.
//!
//! WHY THIS FILE EXISTS (audit M23, completed 10 Oct 2026). The shell appends
//! the Qwen soft switch — ` /think` or ` /no_think` — to every user turn on a
//! ChatML engine; the eval sent the bare message. The code half was fixed by
//! moving the rule into `tacet_engine::thinking_switch` and calling it from
//! both sides (`tacet-eval`'s `the_thinking_switch_is_the_shells`). What that
//! fix cannot reach is the numbers already on the page: the model table's
//! hybrid Qwen3 rows and the 53.7-minute `Qwen/Qwen3-4B` run were taken with no
//! switch, i.e. in the model's default thinking mode, and nobody can re-run them
//! here (rented card, no report kept). Left alone, "bigger did not mean better,
//! and it cost eight times the wall clock" reads as something a user of the
//! shell would see, when the shell sends `/no_think` on most turns.
//!
//! So the page carries the caveat, and this pins it from the code rather than
//! from memory: the split it quotes is the switch run over the suite, and every
//! hybrid row the table holds is named in it. A new hybrid row, a changed
//! keyword list or a re-worded paragraph fails here with the sentence expected.

use std::path::{Path, PathBuf};

use tacet_engine::Template;
use tacet_engine::thinking_switch::{ThinkingSetting, soft_switch};

fn readme() -> String {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf();
    std::fs::read_to_string(root.join("README.md")).expect("README.md is readable")
}

/// Whitespace folded, so a sentence the page wraps is one string.
fn folded(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The page's paragraphs, each folded to one line.
fn paragraphs(page: &str) -> Vec<String> {
    page.split("\n\n").map(folded).collect()
}

/// The one paragraph containing `marker`. Exactly one, so a copy of the
/// sentence elsewhere cannot satisfy a check meant for this one.
fn paragraph_with(page: &str, marker: &str) -> String {
    let hits: Vec<String> = paragraphs(page)
        .into_iter()
        .filter(|p| p.contains(marker))
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "README.md must have exactly one paragraph containing {marker:?}; found {}",
        hits.len()
    );
    hits.into_iter().next().expect("one")
}

/// The model column of the table under "### The model table", without bold
/// or a quantisation note: "**Qwen3-4B-Instruct-2507**" → "Qwen3-4B-Instruct-2507",
/// "Qwen3-0.6B (Q8_0)" → "Qwen3-0.6B".
fn model_names(page: &str) -> Vec<String> {
    table_names(page, "| model | score | tool selection |")
}

/// The model column of the table whose header starts with `header`.
fn table_names(page: &str, header: &str) -> Vec<String> {
    let start = page
        .find(header)
        .unwrap_or_else(|| panic!("README.md has no table headed {header:?}"));
    page[start..]
        .lines()
        .skip(2)
        .take_while(|l| l.starts_with('|'))
        .map(|l| {
            let cell = l.trim_matches('|').split('|').next().unwrap_or("").trim();
            let cell = cell.trim_matches('*');
            cell.split(" (").next().unwrap_or(cell).trim().to_string()
        })
        .collect()
}

/// Qwen3's first release (April 2025) is hybrid: one set of weights that
/// thinks unless told `/no_think`. The 2507 refresh split it into separate
/// `-Instruct-2507` and `-Thinking-2507` builds that ignore the switch. That
/// naming is the only thing the page records about a row's weights, so it is
/// what decides here.
///
/// NORMALISED BEFORE IT DECIDES, because the page spells one model three ways:
/// `Qwen/Qwen3-4B` in prose, `Qwen3-8B` in the table, `qwen3-8b` in a catalog
/// key. A case-sensitive prefix test took only the second, so a row written the
/// way the page's own prose writes it would have joined the table with no
/// caveat and this file green (review of 10 Oct 2026).
fn is_hybrid(name: &str) -> bool {
    let base = name.rsplit('/').next().unwrap_or(name).to_lowercase();
    base.starts_with("qwen3-") && !base.contains("2507")
}

/// NOT VACUOUS: the caveat check below is only as wide as this rule.
#[test]
fn every_spelling_of_a_hybrid_name_is_a_hybrid() {
    for hybrid in ["Qwen3-8B", "Qwen/Qwen3-4B", "qwen3-8b", "QWEN3-0.6B"] {
        assert!(is_hybrid(hybrid), "{hybrid} is a hybrid Qwen3 build");
    }
    for not in [
        "Qwen3-4B-Instruct-2507",
        "Qwen/Qwen3-4B-Instruct-2507",
        "qwen3-4b-thinking-2507",
        "gemma-3-12b",
        "Qwen2.5-7B",
    ] {
        assert!(!is_hybrid(not), "{not} has no soft switch to miss");
    }
}

/// (sent `/no_think`, sent `/think`) over every turn of the published suite,
/// under the default setting, on a ChatML engine — what the shell would have
/// sent where the eval of that date sent nothing.
fn switch_split() -> (usize, usize) {
    let mut no_think = 0;
    let mut think = 0;
    for case in tacet_eval::selection_suite() {
        for step in case.steps {
            match soft_switch(Template::ChatML, ThinkingSetting::Auto, &step.message) {
                " /no_think" => no_think += 1,
                " /think" => think += 1,
                other => panic!("a ChatML turn gets a switch, got {other:?}"),
            }
        }
    }
    (no_think, think)
}

#[test]
fn the_split_the_page_quotes_is_the_switch_run_over_the_suite() {
    let (no_think, think) = switch_split();
    // NOT VACUOUS: both sides of the switch occur in the suite, so a sentence
    // with either number wrong cannot match.
    assert!(no_think > 0 && think > 0, "{no_think} / {think}");
    let want = format!(
        "`/no_think` on {no_think} of the suite's {} turns and `/think` on {think}",
        no_think + think
    );
    assert!(
        folded(&readme()).contains(&want),
        "M23: README.md must say {want:?} — the shell's default switch over the suite \
         the hybrid rows were measured on without one"
    );
}

#[test]
fn every_hybrid_row_in_the_model_table_carries_the_caveat() {
    let page = readme();
    let names = model_names(&page);
    let hybrids: Vec<&String> = names.iter().filter(|n| is_hybrid(n)).collect();
    // NOT VACUOUS: the table holds hybrid rows today, and the instruct row is
    // not counted as one — a rule that took every Qwen3 name, or none, fails.
    assert!(!hybrids.is_empty(), "no hybrid row found in {names:?}");
    assert!(
        names.iter().any(|n| n == "Qwen3-4B-Instruct-2507") && !is_hybrid("Qwen3-4B-Instruct-2507"),
        "the instruct row is the control: {names:?}"
    );
    let caveat = paragraph_with(&page, "measured in a mode the shell does not give you");
    for name in &hybrids {
        assert!(
            caveat.contains(name.as_str()),
            "M23: {name} was measured with no soft switch, in its default thinking \
             mode, and the caveat under the model table does not name it: {caveat}"
        );
    }
    assert!(
        caveat.contains("thinking = on") && caveat.contains("has not been measured"),
        "M23: the caveat must say which setting the rows resemble and that the \
         shell's default was not measured: {caveat}"
    );
}

/// THE OTHER HYBRID FIGURES ON THE PAGE (review of 10 Oct 2026): the
/// `bench gap` table, which sent the bare message too until M23 reached it,
/// and the 8B's tokens per turn that the gemma paragraph compares against.
///
/// THE GAP TABLE'S `Qwen3-4B` IS NOT HELD TO THE CAVEAT, and that is the page's
/// own uncertainty rather than a hole: `models download qwen3-4b` moved from
/// the hybrid to the 2507 instruct build (3812b05, 01:04 on 5 Sep) during the
/// rental that produced the table, and which file the run loaded is not on
/// record. The paragraph says so; the check below holds it to saying so.
#[test]
fn the_gap_table_and_the_gemma_comparison_say_how_the_hybrid_was_asked() {
    let page = readme();
    let names = table_names(&page, "| model | started a call |");
    let hybrids: Vec<&String> = names
        .iter()
        .filter(|n| is_hybrid(n) && n.as_str() != "Qwen3-4B")
        .collect();
    assert!(!hybrids.is_empty(), "no hybrid row found in {names:?}");
    let caveat = paragraph_with(&page, "row was asked without the soft switch");
    for name in &hybrids {
        assert!(
            caveat.contains(name.as_str()) && caveat.contains("default thinking mode"),
            "M23: {name} in the `bench gap` table was asked with no soft switch, and \
             the paragraph under it does not say so: {caveat}"
        );
    }
    if names.iter().any(|n| n == "Qwen3-4B") {
        assert!(
            caveat.contains("not on record"),
            "the gap table's 4B row is exempt only while the page says why: {caveat}"
        );
    }
    let gemma = paragraph_with(&page, "Qwen3-8B's 280");
    assert!(
        gemma.contains("no soft switch") && gemma.contains("default thinking mode"),
        "M23: the 8B's 280 tokens per turn were taken in its default thinking mode: {gemma}"
    );
}

/// The two sentences the critic named: each reads as something a user of the
/// shell would see unless it says the mode it was measured in.
#[test]
fn the_hybrid_comparisons_say_the_mode_they_were_measured_in() {
    let page = readme();
    let bigger = paragraph_with(&page, "Bigger did not mean better");
    assert!(
        bigger.contains("thinking mode it was measured in") && bigger.contains("`/no_think`"),
        "M23: the 8B-against-4B paragraph quotes a wall clock taken in thinking mode: {bigger}"
    );
    let slow = paragraph_with(&page, "53.7 minutes with `Qwen/Qwen3-4B`");
    assert!(
        slow.contains("no soft switch") && slow.contains("default thinking mode"),
        "M23: the 53.7-minute run was the hybrid model with no switch: {slow}"
    );
}
