//! AN `evidence` VALUE A WRONG ANSWER SATISFIES IS A TEST THAT CANNOT GO RED.
//!
//! `evidence` is matched as a PLAIN SUBSTRING of the model's answer — that is
//! what `bench.rs` says and what the runner does. So `"18"` is satisfied by an
//! answer of `18000`, and `"7"` by any sentence with a seven in it. A case like
//! that is green forever whatever the model says, which is worse than not having
//! the case: it occupies a denominator and measures nothing.
//!
//! FOUND BY AN ADVERSARIAL PASS OVER A BATCH OF NEW CASES, not by anyone reading
//! the runner. A verifier rejected a draft whose evidence was `"18"` where the
//! message contained `750` and the answer was `18000`, and the same shape turned
//! out to be in six more. Hence a test rather than a note: it is a class, it is
//! invisible on inspection, and it recurs every time somebody writes a case with
//! a number in it.
//!
//! TWO SHAPES ONLY, both mechanical:
//!
//! * a SINGLE DIGIT — any answer containing it passes;
//! * a value that is a SUBSTRING OF A NUMBER ALREADY IN THE MESSAGE — the model
//!   can pass by echoing the question back.
//!
//! A two-digit value is NOT flagged on its own. `36` as the answer to a discount
//! question is a real assertion when nothing in the message contains `36`, and
//! the existing suites are full of them.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

/// Every run of digits in a string, so an evidence value can be compared against
/// the numbers the question already handed the model.
fn numbers(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            current.push(c);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[test]
fn no_evidence_value_can_be_satisfied_by_a_wrong_answer() {
    let root = repo_root();
    let mut checked = 0usize;
    let mut complaints: Vec<String> = Vec::new();

    let mut files: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.join("benchmarks")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("benchmarks/ is readable") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
                files.push(path);
            }
        }
    }
    files.sort();
    assert!(
        files.len() >= 10,
        "only {} benchmark files found — the walk is no longer walking",
        files.len()
    );

    for path in files {
        let text = std::fs::read_to_string(&path).expect("a benchmark is readable");
        let value: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()));
        let Some(cases) = value.get("cases").and_then(|c| c.as_array()) else {
            continue;
        };
        let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        for case in cases {
            let name = case.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            for step in case
                .get("steps")
                .and_then(|s| s.as_array())
                .map(|s| s.as_slice())
                .unwrap_or_default()
            {
                let message = step.get("message").and_then(|m| m.as_str()).unwrap_or("");
                let in_message = numbers(message);
                for item in step
                    .get("evidence")
                    .and_then(|e| e.as_array())
                    .map(|e| e.as_slice())
                    .unwrap_or_default()
                {
                    let Some(evidence) = item.as_str() else {
                        continue;
                    };
                    // Only NUMERIC evidence is checked. A word is either in the
                    // answer or it is not, and shortening it does not make a
                    // wrong answer pass.
                    if evidence.is_empty() || !evidence.chars().all(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    checked += 1;
                    if evidence.len() == 1 {
                        complaints.push(format!(
                            "{file} · {name} · evidence \"{evidence}\" is a single digit, so any \
                             answer containing it passes"
                        ));
                    } else if let Some(bigger) = in_message
                        .iter()
                        .find(|n| n.contains(evidence) && n.as_str() != evidence)
                    {
                        complaints.push(format!(
                            "{file} · {name} · evidence \"{evidence}\" is a substring of \
                             \"{bigger}\", which the question already gave the model — echoing \
                             the question back would pass"
                        ));
                    }
                }
            }
        }
    }

    assert!(
        checked > 50,
        "only {checked} numeric evidence values seen; the reader has stopped reading"
    );
    assert!(
        complaints.is_empty(),
        "{} evidence value(s) a wrong answer could satisfy:\n  {}",
        complaints.len(),
        complaints.join("\n  ")
    );
}
