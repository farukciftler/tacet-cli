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

/// The composite a model scores by calling nothing and answering every
/// irrelevance case acceptably — the most generous reading of "silent".
///
/// It is the CEILING of silence as well as the floor of the scale: no model can
/// score less than this without also failing an irrelevance case, which is the
/// axis that carries 0.40.
fn silence_score() -> (BenchScore, usize, usize, usize) {
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

    (
        BenchScore::from_counts(
            (0, tool_total),
            (irrelevance_total, irrelevance_total),
            (step_passed, step_total),
            (answer_passed, answer_total),
        ),
        step_passed,
        step_total,
        answer_total,
    )
}

#[test]
fn a_model_that_calls_nothing_does_not_score_forty() {
    let (score, step_passed, step_total, _) = silence_score();
    let floor = score.out_of_100();
    assert!(
        floor > 40.0,
        "the irrelevance axis is 0.40 of the composite, so 40 is a LOWER BOUND on \
         the floor and not the floor: a silent model also passes those cases' \
         steps ({step_passed}/{step_total}) and their answers"
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
    let (score, _, _, _) = silence_score();
    assert_eq!(score.tool, Some(0.0), "a silent model calls nothing");
    assert_eq!(
        score.irrelevance,
        Some(1.0),
        "and passes every irrelevance case, which is the 0.40"
    );
    let dragged = score.out_of_100() - 40.0;
    assert!(
        dragged > 5.0,
        "the step and answer axes carry {dragged:.1} points of silence, which is \
         the part 'the floor is 40' leaves out"
    );
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
    let (score, step_passed, step_total, answer_total) = silence_score();
    let line = format!(
        "silence: tool 0/160 · irrelevance 24/24 · step {step_passed}/{step_total} \
· answer {step_passed}/{answer_total}  =>  {:.1} / 100",
        score.out_of_100()
    );
    println!("{line}");
    assert_eq!(
        (step_passed, step_total, answer_total),
        (24, 190, 47),
        "the suite's shape moved; the README quotes these counts beside the floor"
    );
    assert!(
        line.ends_with("47.6 / 100"),
        "the README carries this exact line: {line}"
    );
}
