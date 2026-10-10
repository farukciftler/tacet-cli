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
//! So: the flags it types are checked against the parser (the built binary's
//! `chat --help`), and the five failures it claims to detect are checked
//! against the file, from source.

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

/// The long flags `tacet chat` accepts, as the built binary's own `--help`
/// prints them.
///
/// NOT FROM `cli.rs`'S FIELD NAMES, which this read until the review of
/// 10 Oct 2026. It kebab-cased every field and called that "the flags", but
/// two fields are renamed — `#[arg(long = "continue")] continue_session` and
/// `#[arg(long = "session")] session_id` — so the list held `continue-session`
/// and `session-id`, which `tacet chat` rejects, and not `continue` or
/// `session`, which it takes. A smoke line passing `--session-id` was reported
/// clean while the script itself would exit 2. The parser is the authority on
/// the parser; the test binary is built for this suite anyway.
fn chat_flags_from_help() -> Vec<String> {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_tacet"))
        .args(["chat", "--help"])
        .env("NO_COLOR", "1")
        .output()
        .expect("the tacet binary runs");
    assert!(out.status.success(), "`tacet chat --help` failed");
    let help = String::from_utf8_lossy(&out.stdout);
    let mut flags: Vec<String> = Vec::new();
    for word in help.split(|c: char| c.is_whitespace() || c == ',' || c == '[' || c == ']') {
        let Some(name) = word.strip_prefix("--") else {
            continue;
        };
        let name: String = name
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
            .collect();
        if !name.is_empty() {
            flags.push(name);
        }
    }
    flags.sort();
    flags.dedup();
    assert!(
        ["message", "json", "continue", "session"]
            .iter()
            .all(|f| flags.iter().any(|g| g == f)),
        "the help was not parsed, so this test would pass on anything: {flags:?}"
    );
    flags
}

/// The review's case, held: a renamed flag is read under the name the parser
/// takes, so the field's own spelling is missing and the flag's is not.
#[test]
fn a_renamed_flag_is_read_as_the_parser_spells_it() {
    let accepted = chat_flags_from_help();
    let used = flags_used(
        "\"$BIN\" chat --message \"$1\" --session-id x --continue-session --json \\\n  --continue --session y\n",
    );
    let missing: Vec<&String> = used.iter().filter(|f| !accepted.contains(f)).collect();
    assert_eq!(
        missing,
        ["continue-session", "session-id"],
        "the guard does not read flags the way `tacet chat` does"
    );
}

#[test]
fn every_flag_the_smoke_check_passes_exists_in_the_cli() {
    let accepted = chat_flags_from_help();
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

    // THE BUILD AND THE MEASUREMENT AGREE ON WHERE THE BINARY IS. The first
    // fix built and then measured a hard-coded `target/release/tacet`; with
    // `CARGO_TARGET_DIR`, `build.target-dir` or `build.target` set the fresh
    // build landed elsewhere and the stale file in `target/` was measured —
    // and passed `--print-features` whenever its features matched. So `BIN`
    // must be read out of the build's own JSON, after the build.
    assert!(
        !text.contains("target/release/tacet"),
        "scripts/smoke.sh names `target/release/tacet` again; any target-dir \
         override sends the build elsewhere and that path is a stale binary"
    );
    let build_end = text[build..].find('\n').map_or(text.len(), |n| build + n);
    let invocation: String = text[build..]
        .lines()
        .scan(true, |more, line| {
            let keep = *more;
            *more = line.trim_end().ends_with('\\');
            keep.then_some(line)
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        invocation.contains("--message-format=json"),
        "the build must report what it produced (`--message-format=json…`) so \
         the path measured is cargo's answer, not a guess: `{invocation}`"
    );
    let assigned = text
        .find("BIN=")
        .expect("scripts/smoke.sh no longer sets BIN");
    assert!(
        assigned > build_end,
        "BIN is fixed before the build runs, so it cannot be the build's output"
    );
    let assignment = &text[assigned
        ..text[assigned..]
            .find("\nif ")
            .map_or(text.len(), |n| assigned + n)];
    assert!(
        assignment.contains("$build_json") && assignment.contains(".executable"),
        "BIN must be the `executable` cargo reported for this build: `{assignment}`"
    );
}

/// THE `--print-features` ANSWER IS COMPARED, AND A WRONG ONE STOPS THE RUN.
///
/// The test above checks that the question is asked. Review replaced the
/// comparison with `if false` and it stayed green: a script that asks and
/// ignores the answer reads the same to a text search. So this one runs the
/// script. A copy sits in a temp tree (its `cd "$REPO"` lands there, not in
/// this checkout); `HOME` points into the temp dir, and the script's own
/// `PATH="$HOME/.cargo/bin:$PATH"` puts stub `cargo` and `jq` first, which
/// report a stub `tacet` built with whatever features `STUB_FEATURES` says.
/// The model directory does not exist, so a run that gets past the check
/// stops at "no model.gguf" without measuring anything — which is also why
/// the exit code alone (127 both ways) cannot tell the two paths apart and the
/// `measuring` line is what is asserted. Unix only: the script is bash.
#[cfg(unix)]
#[test]
fn a_binary_with_the_wrong_features_is_not_measured() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root =
        std::env::temp_dir().join(format!("tacet-smoke-feat-{}-{nanos}", std::process::id()));
    let stubs = root.join("home/.cargo/bin");
    std::fs::create_dir_all(root.join("scripts")).expect("temp dir");
    std::fs::create_dir_all(&stubs).expect("temp dir");
    std::fs::write(root.join("scripts/smoke.sh"), read("scripts/smoke.sh")).expect("copy");
    let tacet = root.join("tacet");
    let log = root.join("tacet.log");
    let stub = |path: &Path, body: String| {
        std::fs::write(path, body).expect("stub");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    };
    stub(
        &tacet,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\n\
             [ \"$1\" = --print-features ] && echo \"$STUB_FEATURES\"\nexit 0\n",
            log.display()
        ),
    );
    stub(
        &stubs.join("cargo"),
        format!(
            "#!/bin/sh\necho '{{\"reason\":\"compiler-artifact\",\"executable\":\"{}\"}}'\n",
            tacet.display()
        ),
    );
    stub(
        &stubs.join("jq"),
        format!("#!/bin/sh\ncat >/dev/null\necho '{}'\n", tacet.display()),
    );

    let run = |reported: &str| {
        let out = Command::new("bash")
            .arg(root.join("scripts/smoke.sh"))
            .arg(root.join("no-such-model-dir"))
            .current_dir(&root)
            .env("HOME", root.join("home"))
            .env("TACET_SMOKE_FEATURES", "candle")
            .env("STUB_FEATURES", reported)
            .env_remove("TACET_SMOKE_SCRIPT")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .expect("bash runs");
        String::from_utf8_lossy(&out.stderr).to_string()
    };

    // The control: the right answer gets past the check. Without it a script
    // that failed for any other reason would satisfy the assertion below.
    let right = run("candle");
    assert!(
        right.contains("measuring ") && right.contains("no model.gguf"),
        "a binary reporting the expected features was not let through: {right}"
    );

    let wrong = run("metal");
    assert!(
        wrong.contains("reports features 'metal', expected 'candle'")
            && !wrong.contains("measuring "),
        "a binary reporting the wrong features was measured anyway: {wrong}"
    );
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !calls.contains("chat"),
        "the stub tacet was sent a message: {calls}"
    );
    let _ = std::fs::remove_dir_all(&root);
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
