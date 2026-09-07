//! The overnight script must not name a flag that does not exist.
//!
//! WHY. It runs unattended, at night, on a machine nobody is watching. A
//! renamed flag turns it into a script that exits 2 and produces nothing, and
//! the way that is discovered is by wondering, days later, where the numbers
//! went. This is the same check CLAUDE.md asks for on the README — "every
//! subcommand the page names exists" — applied to the one file whose failure is
//! silent by design.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

fn script() -> String {
    let path = repo_root().join("scripts/nightly-eval.sh");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} : {e}", path.display()))
}

/// Every long flag the script passes TO `tacet`, taken from the file rather
/// than listed here.
///
/// ONLY THE `tacet` INVOCATIONS. The script also runs `git rev-parse --short`
/// and `git diff --cached --quiet`; scanning every line for `--word` reported
/// `--short`, `--cached` and `--quiet` as missing `tacet` flags, which is the
/// test measuring its own scanner. An invocation runs until a line that does
/// not end in a backslash.
fn flags_used(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if !inside && !trimmed.contains("target/release/tacet") {
            continue;
        }
        inside = true;
        for word in trimmed.split_whitespace() {
            if let Some(name) = word.strip_prefix("--")
                && !name.is_empty()
                && name.chars().all(|c| c.is_ascii_lowercase() || c == '-')
            {
                out.push(name.to_string());
            }
        }
        if !trimmed.ends_with('\\') {
            inside = false;
        }
    }
    out.sort();
    out.dedup();
    assert!(
        !out.is_empty(),
        "no `tacet` invocation was found in the script; the scanner is looking \
         for the wrong thing and this test would pass on an empty file"
    );
    out
}

/// The long flags `tacet eval` accepts, read out of the `Eval` variant in
/// `cli.rs`. clap derives a long flag from each field name by kebab-casing it,
/// so the fields ARE the flags — and reading them means this test cannot drift
/// from the parser the way a hand-written list would.
///
/// The binary is not run: `tacet eval --help` would need a built one, and this
/// test has to be cheap enough to sit in `cargo test`.
fn eval_flags_from_source() -> Vec<String> {
    let text = std::fs::read_to_string(repo_root().join("crates/tacet-cli/src/cli.rs"))
        .expect("cli.rs is readable");
    let start = text.find("    Eval {").expect("the Eval variant");
    let body = &text[start..];
    let end = body.find("\n    },").map(|i| i + 6).unwrap_or(body.len());
    let body = &body[..end];

    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        if let Some((name, _)) = line.split_once(':')
            && !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
        {
            out.push(name.replace('_', "-"));
        }
    }
    assert!(
        out.contains(&"tool-selection".to_string()) && out.contains(&"journal".to_string()),
        "the variant was not parsed, so this test would pass on anything: {out:?}"
    );
    out
}

#[test]
fn every_flag_the_script_passes_exists_in_the_cli() {
    let text = script();
    // THE FLAG NAMES COME FROM `cli.rs`, not from a list here. clap derives a
    // long flag from each field name by kebab-casing it, so the fields of the
    // `Eval` variant ARE the flags — and reading them means this test cannot
    // drift from the parser the way a hand-written list would.
    //
    // The binary is not run: `tacet eval --help` would need a built binary and
    // this test has to be cheap enough to sit in `cargo test`.
    let accepted = eval_flags_from_source();

    // Flags the script passes to CARGO, not to tacet. Named, because deriving
    // "which binary is this word for" from a shell script is guesswork.
    const CARGO_FLAGS: [&str; 3] = ["release", "features", "model"];

    let missing: Vec<String> = flags_used(&text)
        .into_iter()
        .filter(|f| !CARGO_FLAGS.contains(&f.as_str()))
        .filter(|f| !accepted.contains(f))
        .collect();
    assert!(
        missing.is_empty(),
        "scripts/nightly-eval.sh passes {missing:?}, which `tacet eval` does not \
         accept. The script runs unattended at night; a renamed flag makes it \
         exit 2 and produce nothing, and nobody is watching."
    );
}

/// The three things that make an unattended run trustworthy. Each has cost a
/// real measurement in this repository.
#[test]
fn the_script_keeps_its_three_promises() {
    let text = script();
    assert!(
        text.contains("cargo build --release"),
        "it must BUILD: a run against a stale binary produces a number \
         attributed to a commit it did not measure, which has happened here"
    );
    assert!(
        text.contains("--journal"),
        "it must be RESUMABLE, or a closed lid costs the whole night"
    );
    assert!(
        text.contains("HOME/.cargo/bin"),
        "cargo is not on a non-interactive PATH; the last time that was missed, \
         the rebuild was skipped and the run measured the old binary"
    );
    assert!(
        text.contains("-s \"$RUN/report.json\""),
        "an interrupted run leaves a zero-byte report, and comparing against it \
         reads as a run in which nothing passed"
    );
}
