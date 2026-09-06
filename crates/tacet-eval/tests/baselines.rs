//! THE CHECKED-IN BASELINES ARE STILL PAIRABLE.
//!
//! WHY A BASELINE NEEDS A GUARD AT ALL. `eval --compare` pairs two reports BY
//! CASE NAME and prints the unpaired ones as "only in after — EXCLUDED". So a
//! baseline whose names have drifted away from the suite does not fail: it
//! quietly compares a SMALLER suite and reports a verdict about it. A baseline
//! nobody can pair against is worse than no baseline, because it still prints a
//! number. This file is what turns that from a footnote into a build failure.
//!
//! WHAT IS CHECKED, and each of the three has already been a real defect
//! somewhere:
//!
//!   1. THE NAMES MATCH A SUITE EXACTLY. Add a case, delete a case, rename one —
//!      the baseline must be regenerated in the same change, and the failure
//!      message says which command does it.
//!   2. NO ABSOLUTE PATH IS COMMITTED. `SelectionReport` serializes an
//!      `identity` block and `EngineIdentity.model_path` is the local path to
//!      the GGUF — a maintainer's home directory, in a public repository. The
//!      fake-engine report carries no such field today; the rule is written here
//!      so it holds for the model baseline the moment somebody adds one.
//!   3. THE DIRECTORY IS NOT SILENTLY EMPTY. A test that iterates zero files
//!      passes while measuring nothing, which is the failure this project keeps
//!      writing guards against.
//!
//! WHAT IS NOT CHECKED, said plainly: whether the RESULTS in the baseline are
//! still what this build produces. For the fake-engine report that is already
//! gated (`cargo run -p tacet-cli -- eval` must read 100%); for a model report it
//! cannot be, because reproducing it needs 2.5 GB of weights and about twenty
//! minutes on a Metal Mac. See CONTRIBUTING for how a model claim is actually
//! made.

use std::path::{Path, PathBuf};

fn baselines_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("baselines")
}

/// The command that regenerates each baseline, by file name. A failure message
/// that says "this is stale" without saying how to fix it costs the next person
/// the same ten minutes every time.
fn regeneration_command(file: &str) -> &'static str {
    match file {
        "fake-engine.json" => {
            "cargo run -p tacet-cli -- eval --json > crates/tacet-eval/baselines/fake-engine.json"
        }
        _ => {
            "cargo run -p tacet-cli --features metal -- eval --tool-selection --json \
             > crates/tacet-eval/baselines/<name>.json   (needs real weights; see CONTRIBUTING)"
        }
    }
}

/// Every `*.json` under `baselines/`, as (file name, parsed value).
fn baselines() -> Vec<(String, serde_json::Value)> {
    let dir = baselines_dir();
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{} is readable: {e}", dir.display()));
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
        let value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} is not JSON: {e}"));
        out.push((name, value));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The case names in a report, whichever of the three shapes it is — the same
/// generic read `eval_compare` does, deliberately: a baseline this test can
/// parse and the comparator cannot is a baseline that fails at the one moment it
/// is needed.
fn case_names(report: &serde_json::Value) -> Vec<String> {
    let list = report
        .get("cases")
        .or_else(|| report.get("outcomes"))
        .and_then(|v| v.as_array())
        .expect("a report carries `cases` or `outcomes`");
    list.iter()
        .map(|e| {
            e.get("name")
                .or_else(|| e.get("case"))
                .and_then(|v| v.as_str())
                .expect("every case has a name")
                .to_string()
        })
        .collect()
}

#[test]
fn every_baseline_still_pairs_with_the_suite_it_came_from() {
    let suites: Vec<(&str, Vec<String>)> = vec![
        (
            "case::all()",
            tacet_eval::all().iter().map(|c| c.name.clone()).collect(),
        ),
        (
            "selection_suite()",
            tacet_eval::selection_suite()
                .iter()
                .map(|c| c.name.clone())
                .collect(),
        ),
        (
            "selection_cases()",
            tacet_eval::selection_cases()
                .iter()
                .map(|c| c.name.clone())
                .collect(),
        ),
        (
            "turkish_selection_cases()",
            tacet_eval::turkish_selection_cases()
                .iter()
                .map(|c| c.name.clone())
                .collect(),
        ),
    ];

    let files = baselines();
    assert!(
        !files.is_empty(),
        "no baseline in {} — the directory exists so that `--compare` has \
         something to pair against; an empty one makes this whole file vacuous",
        baselines_dir().display()
    );

    for (file, report) in &files {
        // AN EXTERNAL BENCHMARK IS NOT PAIRED BY NAME AND MUST NOT BE ASKED TO
        // BE. `--compare` pairs a report against a suite this repository
        // defines; a report of somebody ELSE'S benchmark — BFCL's irrelevance
        // category, say — has case names that belong to them and no suite here
        // will ever match. That is not the failure this test exists for.
        //
        // It is recognised by a field it must declare rather than by a filename
        // convention, so the exemption cannot be claimed by accident: the local-
        // path check below still applies to it, because that one is about the
        // maintainer's home directory and holds for every file in this directory.
        if report.get("benchmark").is_some() && report.get("source").is_some() {
            continue;
        }
        let mut names = case_names(report);
        names.sort();
        let matched = suites.iter().find(|(_, suite)| {
            let mut s = suite.clone();
            s.sort();
            s == names
        });
        assert!(
            matched.is_some(),
            "{file} holds {} case names that match NO suite exactly.\n\
             `eval --compare` pairs by name, so this baseline would silently \
             compare a smaller suite and still print a verdict.\n\
             Suite sizes now: {}\n\
             Regenerate with:\n  {}",
            names.len(),
            suites
                .iter()
                .map(|(what, s)| format!("{what} = {}", s.len()))
                .collect::<Vec<_>>()
                .join(", "),
            regeneration_command(file)
        );
    }
}

#[test]
fn no_baseline_carries_a_local_path() {
    // The shapes an absolute path takes on the three platforms this project
    // builds for. `SelectionReport.identity.model_path` is the field that would
    // carry one, and it is the maintainer's home directory.
    const LEAKS: &[&str] = &["/Users/", "/home/", "/root/", ":\\Users\\", ":/Users/"];
    for (file, report) in baselines() {
        let text = report.to_string();
        for leak in LEAKS {
            assert!(
                !text.contains(leak),
                "{file} contains {leak:?} — a report's `identity.model_path` is the \
                 absolute path to the GGUF on the machine that produced it, and this \
                 repository is public. Normalize it to a basename before checking a \
                 model baseline in."
            );
        }
    }
}

/// The fake-engine baseline is the one this build can actually reproduce, so it
/// gets the claim the others cannot have: EVERY case in it passed. A baseline
/// recorded from a red run would make the next `--compare` report the repairs as
/// an improvement.
#[test]
fn the_fake_engine_baseline_was_recorded_from_a_green_run() {
    let report = baselines()
        .into_iter()
        .find(|(name, _)| name == "fake-engine.json")
        .map(|(_, v)| v)
        .expect("fake-engine.json is checked in");
    let failed: Vec<&str> = report["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .filter(|c| c["passed"] == serde_json::Value::Bool(false))
        .map(|c| c["name"].as_str().unwrap_or("?"))
        .collect();
    assert!(
        failed.is_empty(),
        "the checked-in baseline records failures: {failed:?}. \
         Fix them, then regenerate with:\n  {}",
        regeneration_command("fake-engine.json")
    );
    assert_eq!(report["engine"], "fake");
}
