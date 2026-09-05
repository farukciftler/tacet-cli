use tacet_eval::{EvalCase, FakeSelector, run_case};
use tacet_eval::tool_selection::{SelectionStep, check_answer_quality};

/// Does `expected_evidence` fire when only the MODEL's sentence carries it?
#[test]
fn evidence_can_be_satisfied_by_the_answer_alone() {
    // The tool is called with a DIFFERENT expression; 1000 exists nowhere in
    // any tool output. Only the scripted answer says it.
    let case = EvalCase::new("probe-evidence", "What is 125 times 8?")
        .tool("calculate")
        .script(&[r#"calculate({"expression":"2+2"})"#, "125 x 8 = 1000."])
        .evidence(&["1000"]);
    let outcome = run_case(&case, &FakeSelector);
    println!("faults: {:?}", outcome.faults);
    println!("answer: {:?}", outcome.answer);
    assert!(
        outcome.passed,
        "PROBE: expected the evidence claim to be satisfied by the answer"
    );
}

/// The same, on the selection set's answer-quality checker.
#[test]
fn selection_evidence_can_be_satisfied_by_the_answer_alone() {
    let step = SelectionStep::new("125 x 8?", Some("calculate")).with_evidence(&["1000"]);
    // No tool outcome carries 1000; the answer does.
    assert!(check_answer_quality(&step, "The answer is 1000.", &["4".into()]));
}

/// A `forbidden` entry that names a TOOL is never compared against the tools
/// that were called.
#[test]
fn a_forbidden_tool_name_is_only_a_substring_check() {
    let step = SelectionStep::new("summarise https://x", Some("web_fetch"))
        .with_forbidden(&["web_search"]);
    // Nothing here says which tools ran; the checker has no way to know.
    assert!(check_answer_quality(&step, "Here is the summary.", &["result".into()]));
}

/// And it fires on ordinary prose instead.
#[test]
fn a_forbidden_tool_name_fires_on_ordinary_words() {
    let git = SelectionStep::new("m", Some("run_code")).with_forbidden(&["git"]);
    assert!(!check_answer_quality(&git, "Each digit is checked.", &[]));
    let calc = SelectionStep::new("m", Some("time")).with_forbidden(&["calculate"]);
    assert!(!check_answer_quality(&calc, "I calculated 68 dollars.", &[]));
    let time = SelectionStep::new("m", Some("calculate")).with_forbidden(&["time"]);
    assert!(!check_answer_quality(&time, "That is 3 times 4.", &[]));
}
