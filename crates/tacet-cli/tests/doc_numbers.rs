//! THE NUMBERS IN THE DOCS ARE PINNED TO THE SUITES THEY DESCRIBE.
//!
//! WHY THIS FILE EXISTS: three documents claimed a case count and all three were
//! wrong at the same time. `README.md` and `crates/tacet-cli/README.md` both said
//! "21-case behavioural suite" while `case::all()` returned 51; `CONTRIBUTING.md`
//! said "one case is worth 3.1 points on a 32-case suite" while the pooled
//! selection suites had grown past 170. Nothing had gone wrong — cases were
//! added, which is the point of a suite, and the prose was the only part that
//! could not notice.
//!
//! SO THE FIX IS NOT "CORRECT THE NUMBERS", IT IS "MAKE THEM UNABLE TO ROT". Any
//! `<n>-case` written in these three files must be a size some suite in
//! `tacet_eval` actually has. Change the suites and this test names the file and
//! the line that now lies.
//!
//! WHY BOTH READMEs. The crate manifest declares `readme = "README.md"`
//! (crates/tacet-cli/Cargo.toml), so `crates/tacet-cli/README.md` is what
//! crates.io publishes. Checking only the root one would leave the PUBLISHED
//! document wrong while the test went green — which is how the "21-case" line
//! survived in two places at once.
//!
//! WHAT IT DELIBERATELY DOES NOT DO: it does not check that a document mentions
//! a suite at all, and it does not check prose that names a count in words
//! ("one case is worth 1.28 points"). It checks the one shape that is both
//! mechanical and load-bearing. A claim this test cannot make is better left
//! unmade than faked with a looser pattern.

use std::path::{Path, PathBuf};

/// The repo root.
///
/// TWO `parent()` CALLS, NOT ONE. `network_monopoly.rs` has a similar helper and
/// it stops at `<repo>/crates` on purpose — that is the directory it walks.
/// `CARGO_MANIFEST_DIR` is `<repo>/crates/tacet-cli`, so reaching README.md and
/// CONTRIBUTING.md needs the grandparent; copying the neighbouring helper would
/// look for them inside `crates/`.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

/// The documents that quote a case count, relative to the repo root.
const DOCUMENTS: &[&str] = &["README.md", "crates/tacet-cli/README.md", "CONTRIBUTING.md"];

/// Every `<digits>-case` in `text`, as (line number, the number).
///
/// Hand-rolled rather than regex: this workspace adds no dependency for a scan
/// of three files, and the shape being looked for is one token wide.
fn case_counts(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if !chars[i].is_ascii_digit() {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            // The digits must be a whole word: "v0.1.10-case" is not a claim
            // about a suite, and neither is the tail of a longer number.
            let word_start = start == 0 || !chars[start - 1].is_alphanumeric();
            let suffix: String = chars[i..].iter().take(5).collect();
            if word_start && suffix.starts_with("-case") {
                let digits: String = chars[start..i].iter().collect();
                out.push((line_no + 1, digits.parse().expect("ascii digits parse")));
            }
        }
    }
    out
}

#[test]
fn every_case_count_in_the_docs_is_a_suite_that_exists() {
    let behavioural = tacet_eval::all().len();
    let english = tacet_eval::selection_cases().len();
    let turkish = tacet_eval::turkish_selection_cases().len();
    let pooled = english + turkish;
    // Every size a document may legitimately quote, with the name it goes by, so
    // a failure can say what the writer probably meant.
    let known = [
        (behavioural, "the behavioural suite, tacet_eval::all()"),
        (english, "selection_cases()"),
        (turkish, "turkish_selection_cases()"),
        (pooled, "the pooled selection suite"),
    ];

    let mut found = 0usize;
    for document in DOCUMENTS {
        let path = repo_root().join(document);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
        for (line, quoted) in case_counts(&text) {
            found += 1;
            assert!(
                known.iter().any(|(n, _)| *n == quoted),
                "{document}:{line} claims a {quoted}-case suite; the suites are {}",
                known
                    .iter()
                    .map(|(n, what)| format!("{n} ({what})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    // THE GUARD AGAINST A TEST THAT PASSES BY LOOKING AT NOTHING. If the scan
    // stops matching — a document reworded, a helper broken — every assertion
    // above becomes vacuous and this is the only line that would notice.
    assert!(
        found >= 3,
        "expected at least three '<n>-case' claims across {DOCUMENTS:?}, found {found}; \
         either the docs stopped quoting counts or the scan stopped matching"
    );
}

/// The docs promise `eval` needs no model and no socket. That promise is worth a
/// test of its own elsewhere; here it is only the count that is pinned.
#[test]
fn the_behavioural_suite_is_the_one_the_readmes_name() {
    // Read from the same function the runner uses, not from a literal: a second
    // copy of the number is a second thing that can drift.
    let text = std::fs::read_to_string(repo_root().join("README.md")).expect("README.md");
    let claimed = format!("{}-case behavioural suite", tacet_eval::all().len());
    assert!(
        text.contains(&claimed),
        "README.md should describe `tacet eval` as the {claimed}"
    );
}
