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
        if !inside && !trimmed.contains("\"$BIN\" eval") {
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

/// THE BINARY THAT RUNS IS THE BINARY THAT WAS BUILT.
///
/// The build step is there so the number belongs to this commit, but the eval
/// then ran a hard-coded `./target/release/tacet`. `CARGO_TARGET_DIR`,
/// `build.target-dir` and `build.target` each send a release build somewhere
/// else, and the run would measure whatever older binary was left in
/// `target/` — the stale-binary number the build exists to prevent. An
/// adversarial review of audit M29 found the same hole in smoke.sh.
#[test]
fn the_eval_runs_the_binary_cargo_reports_building() {
    let code: String = script()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("target/release/tacet"),
        "scripts/nightly-eval.sh names `target/release/tacet` again; any \
         target-dir override makes that a stale binary"
    );
    let build = code
        .find("cargo build --release")
        .expect("the script builds");
    assert!(
        code[build..].contains("--message-format=json"),
        "the build must report the executable it produced"
    );
    let assigned = code.find("BIN=").expect("the script sets BIN");
    assert!(
        assigned > build,
        "BIN is fixed before the build, so it cannot be the build's output"
    );
    let assignment = &code[assigned..];
    let assignment = &assignment[..assignment.find("\nif ").unwrap_or(assignment.len())];
    assert!(
        assignment.contains("$build_json") && assignment.contains(".executable"),
        "BIN must be the `executable` cargo reported: `{assignment}`"
    );
    let first_run = code.find("\"$BIN\" eval").expect("the eval runs $BIN");
    assert!(first_run > assigned, "the eval runs before BIN is known");
}

/// THE RESUME KEY, TESTED BY RUNNING THE SCRIPT RATHER THAN READING IT.
///
/// Two defects in one line (audit of 9 Oct 2026, L34). The journal directory
/// was `<yyyymmdd>-<commit>[-dirty]`:
///
/// * THE DATE. A run interrupted before midnight and re-run after it opened a
///   new directory and started from case 1 — for a job that starts at 03:07
///   and takes most of an hour, the ordinary case — while the script promised
///   "running this script again continues".
/// * THE BARE `-dirty`. Every uncommitted state of a commit shared one
///   journal, so a run could resume across an edit and be half one build and
///   half another.
///
/// A temporary git repository holds a copy of the script; a `date` shim on
/// PATH plays two different days; `HOME` points into the temp dir, so the
/// user's `~/.tacet` is never named. Unix only: the script is bash, and the
/// Windows CI leg has no bash it can be trusted to be.
#[cfg(unix)]
#[test]
fn the_journal_directory_survives_midnight_and_tells_edits_apart() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root =
        std::env::temp_dir().join(format!("tacet-nightly-key-{}-{nanos}", std::process::id()));
    let repo = root.join("repo");
    let home = root.join("home");
    let shims = root.join("shims");
    for dir in [repo.join("scripts"), home.clone(), shims.clone()] {
        std::fs::create_dir_all(&dir).expect("temp dir");
    }
    std::fs::write(repo.join("scripts/nightly-eval.sh"), script()).expect("copy the script");
    std::fs::write(repo.join("a.txt"), "committed\n").expect("a tracked file");
    let date = shims.join("date");
    std::fs::write(&date, "#!/bin/sh\necho \"$FAKE_DATE\"\n").expect("date shim");
    std::fs::set_permissions(&date, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let path = format!(
        "{}:{}",
        shims.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // Isolated from the user's git configuration and from any GIT_DIR a hook
    // running `cargo test` might have exported.
    let command = |program: &str| {
        let mut c = Command::new(program);
        c.current_dir(&repo)
            .env("HOME", &home)
            .env("PATH", &path)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE");
        c
    };
    let git = |args: &[&str]| {
        let out = command("git").args(args).output().expect("git runs");
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-q",
        "-m",
        "init",
    ]);

    let run_dir = |day: &str| {
        let out = command("bash")
            .arg(repo.join("scripts/nightly-eval.sh"))
            .arg("--print-run-dir")
            .env("FAKE_DATE", day)
            .output()
            .expect("bash runs");
        assert!(out.status.success(), "--print-run-dir failed: {out:?}");
        let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert!(
            dir.starts_with(&home.join(".tacet/nightly").display().to_string()),
            "the run directory is not under $HOME/.tacet/nightly: {dir:?}"
        );
        dir
    };

    let before_midnight = run_dir("20261009");
    let after_midnight = run_dir("20261010");
    assert_eq!(
        before_midnight, after_midnight,
        "the same commit got a different journal on a different day, so an \
         interrupted run restarts from case 1 after midnight"
    );
    assert!(
        !before_midnight.contains("20261009"),
        "the date is still part of the key: {before_midnight}"
    );

    std::fs::write(repo.join("a.txt"), "edit one\n").expect("edit");
    let edit_one = run_dir("20261010");
    std::fs::write(repo.join("a.txt"), "edit two\n").expect("edit");
    let edit_two = run_dir("20261010");
    assert!(
        edit_one.contains("-dirty") && edit_one != before_midnight,
        "an uncommitted edit shares the clean commit's journal: {edit_one}"
    );
    assert_ne!(
        edit_one, edit_two,
        "two different uncommitted edits share one journal, so a resumed run \
         can be half one build and half the other"
    );
    std::fs::write(repo.join("a.txt"), "edit one\n").expect("edit");
    assert_eq!(
        run_dir("20261011"),
        edit_one,
        "the same edit must find the same journal again, or it cannot resume"
    );

    // A new source file is an untracked file, and it changes the build.
    std::fs::write(repo.join("new_module.rs"), "pub fn f() {}\n").expect("untracked");
    assert_ne!(
        run_dir("20261011"),
        edit_one,
        "an untracked file changed the build and not the journal"
    );

    assert!(
        !home.join(".tacet").exists(),
        "--print-run-dir must touch nothing, and it created {}",
        home.join(".tacet").display()
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// THE SCRIPT'S OWN ACCOUNT OF WHAT KEEPS TWO BUILDS APART MUST BE TRUE.
///
/// It said "the journal's own stamp would refuse them anyway" of two commits.
/// The stamp held the model, the engine, the catalog and the package version —
/// no commit — so after a prompt change at the same version the stamp accepted
/// both and the report silently mixed them (audit M24). Whatever the stamp
/// grows to hold, the script must not lean on it for this.
#[test]
fn the_script_does_not_claim_the_stamp_separates_commits() {
    let text = script();
    assert!(
        !text.contains("the journal's own stamp would refuse them"),
        "the script credits the journal stamp with separating commits, which it \
         did not do"
    );
}
