//! The pre-publish check must not name a flag that does not exist.
//!
//! WHY, AND WHY IT IS A DIFFERENT FAILURE FROM THE NIGHTLY SCRIPT'S. That one
//! runs unattended and its silence is discovered days later. This one runs
//! MINUTES BEFORE A PUBLISH, with somebody watching, in the worst possible
//! frame of mind for reading an exit code carefully — the release is cut, the
//! tag is written, everything else is green. A `scripts/smoke.sh` that exits 2
//! because `--message` was renamed looks close enough to "0 failures" at a
//! glance, and the whole reason the file exists is that a release once went out
//! on exactly that kind of glance.
//!
//! So: the flags it types are checked against the parser, and the five failures
//! it claims to detect are checked against the file, both from source.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} : {e}", path.display()))
}

/// The file with every comment line removed.
///
/// WITHOUT THIS THE DETECTOR TEST BELOW CANNOT GO RED, and it was written that
/// way first. `scripts/smoke.sh` explains each failure in its header, so the
/// literal `<unused` appears in prose several lines above the `case` that looks
/// for it. Breaking the code and leaving the comment — which is exactly what a
/// careless edit does — left the test green. A guard satisfied by a file's
/// documentation is checking that the file still describes itself, not that it
/// still works.
fn code_only(relative: &str) -> String {
    read(relative)
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every long flag the script passes to `tacet`, taken from the file.
///
/// ONLY THE `tacet` INVOCATION, for the reason the nightly test records: a
/// scanner that reads every `--word` in a shell script reports `grep`'s `-o`
/// and `perl`'s flags as missing `tacet` flags, and then the test is measuring
/// its own scanner. An invocation runs until a line not ending in a backslash.
fn flags_used(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if !inside && !trimmed.contains("\"$BIN\" chat") {
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
        "no `tacet chat` invocation was found; the scanner is looking for the \
         wrong thing and this test would pass on an empty file"
    );
    out
}

/// The long flags `tacet chat` accepts, read out of the `Chat` variant in
/// `cli.rs`. clap kebab-cases each field name, so the fields ARE the flags and
/// this cannot drift from the parser the way a hand-written list would.
fn chat_flags_from_source() -> Vec<String> {
    let text = read("crates/tacet-cli/src/cli.rs");
    let start = text.find("    Chat {").expect("the Chat variant");
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
        out.contains(&"message".to_string()) && out.contains(&"json".to_string()),
        "the variant was not parsed, so this test would pass on anything: {out:?}"
    );
    out
}

#[test]
fn every_flag_the_smoke_check_passes_exists_in_the_cli() {
    let accepted = chat_flags_from_source();
    let missing: Vec<String> = flags_used(&read("scripts/smoke.sh"))
        .into_iter()
        .filter(|f| !accepted.contains(f))
        .collect();
    assert!(
        missing.is_empty(),
        "scripts/smoke.sh passes {missing:?}, which `tacet chat` does not accept. \
         This runs minutes before a publish; a script that exits 2 reads like a \
         clean run to somebody who has already decided to ship."
    );
}

/// THE ANSWER MUST COME FROM `--json`, NOT FROM THE SCREEN.
///
/// The first draft scraped the `Tacet: ` prefix off the banner. That works
/// until the banner changes — and then every answer parses as empty, every
/// message is reported as "no answer", and the check that was meant to gate a
/// release blocks all of them for a reason nobody can act on. `--json` is a
/// contract the CLI states in its own help text; the banner is decoration.
#[test]
fn the_smoke_check_reads_the_machine_output() {
    let text = code_only("scripts/smoke.sh");
    assert!(
        text.contains("--json"),
        "the answer must be read from `--json`, whose contract is one line of \
         JSON and nothing else"
    );
    assert!(
        !text.contains("s/^Tacet: //p"),
        "scraping the human banner is how this check starts silently reading \
         nothing at all"
    );
    assert!(
        text.contains("out.err"),
        "stdout and stderr must stay apart: merging them feeds the MCP load \
         notice to `jq`, which turns a healthy answer into a reported failure"
    );
}

/// THE FIVE FAILURES IT CLAIMS, AND THE CEILING THAT KEEPS IT HONEST.
///
/// Each string below names a failure that reached a user's screen from a
/// shipped binary. A detector quietly dropped from the script would leave the
/// file still claiming, in its own header, to catch something it no longer
/// catches — and `smoke-selftest.sh` would not notice either, because the case
/// that proved it would go with it.
#[test]
fn the_smoke_check_still_looks_for_every_failure_it_names() {
    let text = code_only("scripts/smoke.sh");
    for (needle, why) in [
        (
            "<unused",
            "gemma3 printed `<unused6088>` twelve times in one answer",
        ),
        ("<tool", "a call marker decayed into prose as `<tool.`"),
        (
            "<|im_",
            "the ChatML turn marker is the same failure on a Qwen model",
        ),
        (
            "engine error",
            "the sampler returned an id outside the vocabulary",
        ),
        ("no answer", "the turn produced nothing at all to read"),
        ("degenerate repetition", "`Do not be.` eight times over"),
        (
            "a character stuck in a loop",
            "`1033336333333333333` where the date belonged",
        ),
    ] {
        assert!(
            text.contains(needle),
            "scripts/smoke.sh no longer looks for {needle:?} — {why}"
        );
    }
    assert!(
        text.contains("alarm shift"),
        "every message needs a ceiling: a check that can hang cannot gate a \
         release, and this one hung for twenty-one minutes on seventeen seconds \
         of CPU with the GPU wedged"
    );
}

/// THE SELF-TEST MUST DRIVE EVERY DETECTOR, IN BOTH DIRECTIONS.
///
/// This repository has shipped a test comparing a list against itself and a
/// baseline guard aimed at the wrong layer; both were green every day of their
/// lives. The rule in CLAUDE.md is to break the thing a guard guards and watch
/// it fail, and `smoke-selftest.sh` is where that happens for this one. A
/// must-stay-green row matters as much as a red one: without `clean`, a script
/// that failed everything would satisfy every other case and block all releases.
#[test]
fn the_self_test_covers_both_directions() {
    let text = code_only("scripts/smoke-selftest.sh");
    for name in [
        "clean",
        "marker-unused",
        "marker-tool",
        "repetition",
        "empty",
        "digit-loop",
        "under-threshold",
        "rule-not-a-loop",
    ] {
        assert!(
            text.contains(name),
            "smoke-selftest.sh no longer exercises the {name:?} case"
        );
    }
    assert!(
        text.contains("TACET_SMOKE_SCRIPT"),
        "the self-test drives the detectors through FakeEngine; without that \
         seam it would need weights, and the machine that has weights is the \
         one place this is hardest to run"
    );
}

/// THE RELEASE GATE MEASURES THE CODE IT IS GATING (audit M29).
///
/// `smoke.sh` checked only that `target/release/tacet` existed, then measured
/// it. `nightly-eval.sh` builds into that same path from whatever was checked
/// out at 03:07, so the gate could pass on last night's binary while the tag
/// named this morning's commit — a green check standing for code it never ran.
/// The build has to come BEFORE the first message, and the binary has to be
/// asked what it is rather than assumed to be the build just made.
#[test]
fn the_smoke_check_builds_the_binary_it_measures() {
    let text = code_only("scripts/smoke.sh");
    let build = text
        .find("cargo build --release -p tacet-cli")
        .expect("scripts/smoke.sh no longer builds the binary it measures");
    let first_message = text
        .find("run_message \"$m\"")
        .expect("the per-message loop calls run_message");
    assert!(
        build < first_message,
        "the build comes after the first message is measured"
    );
    assert!(
        text.contains("\"$BIN\" --print-features"),
        "the binary must be asked what it was built with, not assumed to be the \
         build just made"
    );
}

/// THE SEAM IS `+x`, NOT `:-`.
///
/// An emptiness test here sends the EMPTY-ANSWER case — one of the five — to
/// the real model instead of the fake one. That is not hypothetical: it is how
/// this was first written, and the self-test silently became an unannounced
/// four-minute model run whose failures were real answers, not the fixture.
#[test]
fn the_fake_engine_seam_triggers_on_an_empty_script() {
    let text = code_only("scripts/smoke.sh");
    assert!(
        text.contains("${TACET_SMOKE_SCRIPT+x}"),
        "the seam must test SET, not NON-EMPTY, or the empty-answer case runs \
         against real weights"
    );
}
