//! Every loop that hands a generation to the executor re-splits it with the
//! catalog first.
//!
//! WHY THIS IS A SOURCE GUARD (audit H5). The engine strips the `<think>` block
//! in `Generation::new`, where no tool name is known, and the glued call shape
//! (`write_filepath: "a.md"` at a line start) is a call only by a CATALOG name.
//! So the engine's split cut it like prose: a review measured
//! `write_filepath: "a.md", content: "Qwen emits <think> first"` reaching the
//! executor as `Qwen emits  first`, and a `</think>` inside one glued argument
//! leaving `delete_filepath: "victim.md"` at a line start as a second call.
//! `Generation::split_against` closes that, but only where it is called, and
//! the shell's turn loop is one 1500-line function no unit test drives. The
//! eval runner's own test (`a_close_tag_inside_a_glued_argument_cannot_launch_another_call`)
//! covers the behaviour; this covers the wiring, in every copy of the loop —
//! including one somebody adds later.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

// Split in two so this file's own constant is not one of the sites it finds.
const EXECUTES: &str = concat!("execute_raw(", "&generation.text");
const SPLITS: &str = "generation.split_against(&executor.catalog().names())";

/// Each `execute_raw(&generation.text …)` with, for each, whether a CODE line
/// between the `generation` binding above it and the call re-splits it.
///
/// COMMENT LINES DO NOT COUNT. The loops explain the split in prose right
/// next to it, and a guard satisfied by that prose would stay green with the
/// call deleted — the failure `the_smoke_check_names_real_flags` records.
fn sites() -> Vec<(String, bool)> {
    let mut files = Vec::new();
    rust_files(&repo_root().join("crates"), &mut files);
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.contains(EXECUTES) || line.trim_start().starts_with("//") {
                continue;
            }
            let binding = (0..i)
                .rev()
                .find(|&j| {
                    lines[j].contains("let mut generation =")
                        || lines[j].contains("let generation =")
                })
                .unwrap_or(0);
            let split = lines[binding..i]
                .iter()
                .any(|l| !l.trim_start().starts_with("//") && l.contains(SPLITS));
            let rel = file.strip_prefix(repo_root()).unwrap_or(&file);
            out.push((format!("{}:{}", rel.display(), i + 1), split));
        }
    }
    out
}

#[test]
fn every_executing_loop_resplits_with_the_catalog() {
    let found = sites();
    // NOT VACUOUS: the three loops this was written for must be found, or the
    // scan has stopped matching and every assertion below passes on nothing.
    for expected in [
        "crates/tacet-cli/src/chat.rs",
        "crates/tacet-eval/src/runner.rs",
        "crates/tacet-eval/src/tool_selection.rs",
    ] {
        assert!(
            found.iter().any(|(at, _)| at.starts_with(expected)),
            "no `{EXECUTES}` found in {expected}: {found:?}"
        );
    }
    let missing: Vec<&String> = found
        .iter()
        .filter(|(_, split)| !split)
        .map(|(at, _)| at)
        .collect();
    assert!(
        missing.is_empty(),
        "these hand the executor a generation the catalog never re-split, so a \
         glued call in it is cut like prose (audit H5) — call `{SPLITS}` after \
         the generation is bound: {missing:#?}"
    );
}
