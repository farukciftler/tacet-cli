//! The guards that exist only under `candle` and need no weights must be RUN
//! somewhere, not only compiled.
//!
//! WHY (review of audits L21, L23 and T1). The call watch's lexer, its wiring
//! through `run_loop`, the window candle can really run and the marker rules on
//! the tokenizer fixture all sit behind `#[cfg(feature = "candle")]`. ci.yml
//! never builds that feature, and the nightly job built it with `--no-run` and
//! executed two named gguf tests — so every one of those guards ran on the
//! author's machine and nowhere else, while their docs said they ran
//! "everywhere the file compiles".
//!
//! This file is in the default build on purpose: it can read the workflow and
//! the source without the candle tree, so the claim is checked on every PR.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-engine")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The `-ge N` floor the workflow checks after the command that contains
/// `command`.
fn floor_after(workflow: &str, command: &str) -> usize {
    let at = workflow
        .find(command)
        .unwrap_or_else(|| panic!("nightly.yml no longer runs `{command}`"));
    let rest = &workflow[at..];
    let ge = rest.find("-ge ").expect("no count check after the command");
    rest[ge + 4..]
        .split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .expect("the floor is a number")
}

/// Lines that ARE the attribute, not prose that mentions it.
fn attributes(source: &str, attribute: &str) -> usize {
    source
        .lines()
        .filter(|line| line.trim_start().starts_with(attribute))
        .count()
}

#[test]
fn the_nightly_job_runs_the_weightless_candle_guards_with_a_count_floor() {
    let workflow = read(".github/workflows/nightly.yml");
    let engine = read("crates/tacet-engine/src/candle_engine.rs");
    let markers = read("crates/tacet-engine/tests/a_marker_is_not_text.rs");

    // The filter covers the modules the review named.
    for module in [
        "mod budget_and_backstop",
        "mod call_watch_through_generate",
        "mod window_tests",
    ] {
        assert!(engine.contains(module), "candle_engine.rs lost `{module}`");
    }

    // The floor is not above what exists, or the step is red forever; and it
    // is not trivially low, or it would pass with most of the guards gone.
    let lib_tests = attributes(&engine, "#[test]");
    let lib_floor = floor_after(&workflow, "--features candle --lib -- candle_engine::");
    assert!(
        lib_floor <= lib_tests && lib_floor * 2 > lib_tests,
        "nightly checks {lib_floor} candle_engine tests; the file has {lib_tests}"
    );

    let marker_tests = attributes(&markers, "#[test]") - attributes(&markers, "#[ignore");
    let marker_floor = floor_after(&workflow, "--features candle --test a_marker_is_not_text");
    assert!(
        marker_floor <= marker_tests && marker_floor * 2 > marker_tests,
        "nightly checks {marker_floor} marker tests; {marker_tests} are not ignored"
    );
}
