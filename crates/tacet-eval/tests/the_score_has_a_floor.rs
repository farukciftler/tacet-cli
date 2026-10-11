//! What a model that never calls a tool still scores.
//!
//! WHY THIS EXISTS. The README says "40 is the floor, not zero" — 0.40 of the
//! composite is the irrelevance axis, and a model that calls nothing passes
//! every irrelevance case. That is right about the mechanism and WRONG about the
//! number, and the evidence was on the same page: FunctionGemma-270M scored
//! **47.4 with 0/160 tool selection**. A model that calls nothing does not only
//! win the irrelevance axis. It also passes those cases' STEPS, and their
//! ANSWERS — every axis that can be satisfied by saying something sensible and
//! reaching for nothing.
//!
//! Stating the floor too low makes every model look better than it is by about
//! eight points, which is more than the gap between two of the four rows in the
//! model table.
//!
//! THE FLOOR IS DERIVED FROM THE SUITE, not written down. Add an irrelevance
//! case and it moves; that is the point of computing it. What the test pins is
//! that the number the README quotes is the number this suite produces.

use tacet_eval::bench::BenchScore;
use tacet_eval::tool_selection::{selection_cases, turkish_selection_cases};

/// The counts behind the floor, each axis from its own tally.
struct Silence {
    score: BenchScore,
    tool_total: usize,
    irrelevance_total: usize,
    step_passed: usize,
    step_total: usize,
    answer_passed: usize,
    answer_total: usize,
}

/// The composite a model scores by calling nothing and answering every
/// irrelevance case acceptably — the most generous reading of "silent".
///
/// WHAT IT IS NOT: a lower bound for a model that passes every irrelevance
/// case. This used to say "no model can score less than this without also
/// failing an irrelevance case", and the model table refutes it: FunctionGemma
/// passed 24/24 on the irrelevance gate and scored 47.4, below this 47.6 (audit
/// T5). Passing an irrelevance CASE is calling nothing; the answer axis also
/// asks that the reply carry its evidence, and a silent model can fail that —
/// FunctionGemma answered 23 of the 47, where this reading credits 24. It is the
/// score of silence answered well, and a model below it is doing worse than
/// saying nothing.
fn silence_score() -> Silence {
    let cases: Vec<_> = selection_cases()
        .into_iter()
        .chain(turkish_selection_cases())
        .collect();

    let mut irrelevance_total = 0;
    let mut tool_total = 0;
    let mut step_total = 0;
    let mut step_passed = 0;
    let mut answer_total = 0;
    let mut answer_passed = 0;

    for case in &cases {
        // A case with no expected tool on any step is an irrelevance case; the
        // report splits on `Category`, and this mirrors it without needing the
        // enum to be public in the same shape.
        let silent_case = case.steps.iter().all(|s| s.expected.is_none());
        if silent_case {
            irrelevance_total += 1;
        } else {
            tool_total += 1;
        }
        for step in &case.steps {
            step_total += 1;
            // A silent model passes exactly the steps that expect nothing.
            let passes = step.expected.is_none();
            step_passed += passes as usize;
            let claims =
                !step.evidence.is_empty() || !step.forbidden.is_empty() || step.language.is_some();
            if claims {
                answer_total += 1;
                // The generous reading: it answers those steps well.
                answer_passed += passes as usize;
            }
        }
    }

    Silence {
        score: BenchScore::from_counts(
            (0, tool_total),
            (irrelevance_total, irrelevance_total),
            (step_passed, step_total),
            (answer_passed, answer_total),
        ),
        tool_total,
        irrelevance_total,
        step_passed,
        step_total,
        answer_passed,
        answer_total,
    }
}

/// The line the README carries, from the counts — every axis from ITS OWN
/// tally. The answer numerator was `step_passed`, which is the same number
/// today and a different one the first time a silent step makes no claim
/// (audit T5); `each_axis_prints_its_own_count` holds it apart.
fn derivation_line(s: &Silence) -> String {
    format!(
        "silence: tool 0/{} · irrelevance {}/{} · step {}/{} · answer {}/{}  =>  {:.1} / 100",
        s.tool_total,
        s.irrelevance_total,
        s.irrelevance_total,
        s.step_passed,
        s.step_total,
        s.answer_passed,
        s.answer_total,
        s.score.out_of_100()
    )
}

#[test]
fn a_model_that_calls_nothing_does_not_score_forty() {
    let s = silence_score();
    let floor = s.score.out_of_100();
    assert!(
        floor > 40.0,
        "the irrelevance axis is 0.40 of the composite, so 40 is a LOWER BOUND on \
         the floor and not the floor: a silent model also passes those cases' \
         steps ({}/{}) and their answers",
        s.step_passed,
        s.step_total
    );
    // The README quotes this number. If the suite changes, the README changes
    // with it — that is CLAUDE.md's rule and this is the derivation.
    assert!(
        (floor - 47.6).abs() < 0.5,
        "the floor is {floor:.1}; the README says 47.6. Re-derive the page or fix \
         the suite, but do not let the two drift."
    );
}

/// The mechanism, so the number above is readable rather than magic.
#[test]
fn the_floor_is_the_irrelevance_axis_plus_the_axes_it_drags_with_it() {
    let s = silence_score();
    assert_eq!(s.score.tool, Some(0.0), "a silent model calls nothing");
    assert_eq!(
        s.score.irrelevance,
        Some(1.0),
        "and passes every irrelevance case, which is the 0.40"
    );
    let dragged = s.score.out_of_100() - 40.0;
    assert!(
        dragged > 5.0,
        "the step and answer axes carry {dragged:.1} points of silence, which is \
         the part 'the floor is 40' leaves out"
    );
}

/// A MODEL CAN PASS EVERY IRRELEVANCE CASE AND STILL SCORE BELOW SILENCE. The
/// model table's own row, rebuilt: FunctionGemma-270M, 0/160 · 24/24 · 24/190 ·
/// 23/47, is 47.4 — under the 47.6 above. The sentence this file used to carry
/// said that could not happen.
#[test]
fn passing_every_irrelevance_case_is_not_a_floor() {
    let s = silence_score();
    let function_gemma = BenchScore::from_counts(
        (0, s.tool_total),
        (s.irrelevance_total, s.irrelevance_total),
        (s.step_passed, s.step_total),
        (s.answer_passed - 1, s.answer_total),
    );
    assert_eq!(function_gemma.out_of_100(), 47.4, "the model table's row");
    assert!(function_gemma.out_of_100() < s.score.out_of_100());
}

/// The derivation, printed AND asserted, so the README's line can be re-derived
/// by running one test rather than by reading this file.
///
///     cargo test -p tacet-eval --test the_score_has_a_floor -- --nocapture
///
/// The counts are asserted as well as printed: a test that only prints passes
/// whatever the code does, this repository has a guard that says so, and it
/// caught this one.
#[test]
fn the_printed_derivation_is_the_line_the_readme_carries() {
    let s = silence_score();
    let line = derivation_line(&s);
    println!("{line}");
    assert_eq!(
        (s.step_passed, s.step_total, s.answer_passed, s.answer_total),
        (24, 190, 24, 47),
        "the suite's shape moved; the README quotes these counts beside the floor"
    );
    assert_eq!(
        line,
        "silence: tool 0/160 · irrelevance 24/24 · step 24/190 · answer 24/47  =>  47.6 / 100",
        "the README carries this exact line"
    );
}

/// EACH AXIS PRINTS ITS OWN COUNT. On today's suite the step and answer
/// numerators are both 24, so a line that printed the step count in the answer
/// slot read correctly and was wrong (audit T5). Counts that differ are the only
/// way to tell the two apart.
#[test]
fn each_axis_prints_its_own_count() {
    let mut s = silence_score();
    s.step_passed = 24;
    s.answer_passed = 20;
    let line = derivation_line(&s);
    assert!(line.contains("step 24/"), "{line}");
    assert!(
        line.contains("answer 20/"),
        "the answer slot printed another axis: {line}"
    );
}
