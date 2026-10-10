//! The README's update paragraph says what `tacet update` does (audit D6, D7).
//!
//! The command changed under the page. `tacet update` used to only look and
//! now installs (after asking; `--check` is the look-only form — see the
//! `Update` command's doc in `cli.rs`), while the README still said the bare
//! command "tells you what's available". And "the only command that talks to
//! GitHub" stopped being true the day the daily check was added.
//!
//! The checker is a pure function so the old paragraph can be held to it:
//! `the_paragraph_the_audit_read_is_refused` is the red half, and
//! `the_corrected_paragraph_passes` the green one, without either depending on
//! the README's state at the time the test runs.

use std::path::{Path, PathBuf};

/// What the page must not say any more, and what it must say instead.
fn update_paragraph_agrees(readme: &str) -> Result<(), String> {
    let flat: String = readme.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut problems = Vec::new();
    // D6: the bare command installs. A comment calling it a look is the bug.
    for line in readme.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("tacet update") else {
            continue;
        };
        let (args, comment) = rest.split_once('#').unwrap_or((rest, ""));
        if args.trim().is_empty() {
            let comment = comment.to_lowercase();
            if comment.contains("what's available") || comment.contains("tells you") {
                problems.push(format!(
                    "bare `tacet update` is described as a look: {line:?}"
                ));
            }
            if !(comment.contains("install")
                || comment.contains("replace")
                || comment.contains("download"))
            {
                problems.push(format!(
                    "bare `tacet update` is not said to install: {line:?}"
                ));
            }
        }
    }
    if !readme
        .lines()
        .any(|l| l.trim().starts_with("tacet update --check"))
    {
        problems.push("the look-only `tacet update --check` is not shown".to_string());
    }
    // D7: the daily check also asks GitHub, so `update` is not the only door.
    if flat.contains("the only command that talks to GitHub") {
        problems.push("`tacet update` is called the only command that talks to GitHub".to_string());
    }
    if flat.contains("never runs when output is piped")
        && !flat.contains("--message")
        && !flat.contains("tacet -m")
    {
        problems.push(
            "the daily check's limits name the pipe but not the one-shot run it also skips"
                .to_string(),
        );
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// The paragraph as the audit of 9 Oct 2026 read it (README.md:146-150, 179).
const AUDITED: &str = "\
Check for a newer version at any time — this is the only command that talks to GitHub, and only when you run it:

```bash
tacet update            # tells you what's available
tacet update --install  # downloads and replaces the binary, with your confirmation
```

The check is throttled to once every 24 hours, prints a single line when a newer
version exists, stays silent when it fails, and never runs when output is piped.
";

/// The paragraph `doc_changes` hands the integrator for both READMEs.
const CORRECTED: &str = "\
Update at any time — `tacet update` asks GitHub for the newest release, and only when you run it:

```bash
tacet update            # downloads the newest release and replaces the binary — after asking
tacet update --check    # only looks: says whether a newer version exists, writes nothing
```

The check is throttled to once every 24 hours, prints a single line when a newer
version exists and stays silent when it fails. It runs only at the end of an
interactive session with both stdout and stderr on a terminal — never after
`tacet -m`/`--message`, under `--json`, or when output is piped or captured.
";

#[test]
fn the_paragraph_the_audit_read_is_refused() {
    let err = update_paragraph_agrees(AUDITED).expect_err("the audited paragraph passed");
    assert!(err.contains("described as a look"), "{err}");
    assert!(err.contains("--check"), "{err}");
    assert!(err.contains("only command that talks to GitHub"), "{err}");
}

#[test]
fn the_corrected_paragraph_passes() {
    update_paragraph_agrees(CORRECTED).unwrap_or_else(|e| panic!("{e}"));
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

/// THE PAGES THEMSELVES. Both carry the paragraph above since the wave-3
/// merge that changed the command; the README ships with the code
/// (CLAUDE.md), so a later edit to either page that drifts from what
/// `tacet update` does goes red here.
#[test]
fn both_readmes_say_what_update_does() {
    for page in ["README.md", "crates/tacet-cli/README.md"] {
        let text = std::fs::read_to_string(repo_root().join(page)).expect("the page is readable");
        update_paragraph_agrees(&text).unwrap_or_else(|e| panic!("{page}:\n{e}"));
    }
}

/// THE HELP TEXT, held to the same correction (audit D7, completeness review).
///
/// The README stopped saying nothing checks by itself the day the daily check
/// was added, and `tacet update --help` went on saying it: "NOTHING CHECKS BY
/// ITSELF. There is no start-up check and no timer … This runs when it is
/// typed." Once `update.check on` is set, a timer is exactly what there is.
/// The help is what a person reads at the moment they wonder whether the
/// program goes online, so it is held here like the page is.
fn update_help_agrees(help: &str) -> Result<(), String> {
    let flat = help
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let mut problems = Vec::new();
    if flat.contains("nothing checks by itself") || flat.contains("no timer") {
        problems.push("the help says nothing ever checks on its own".to_string());
    }
    if !flat.contains("update.check on") {
        problems.push("the help does not name the opt-in daily check".to_string());
    }
    if !flat.contains("24 hours") {
        problems.push("the help does not say how often the daily check looks".to_string());
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// The `Update` doc as the completeness review read it (cli.rs:388-390).
const AUDITED_HELP: &str = "\
Updates this binary to the newest release.

NOTHING CHECKS BY ITSELF. There is no start-up check and no timer: a program
whose promise is that it stays off the network cannot quietly go online to ask
about itself. This runs when it is typed.
";

#[test]
fn the_help_the_review_read_is_refused() {
    let err = update_help_agrees(AUDITED_HELP).expect_err("the audited help passed");
    assert!(err.contains("nothing ever checks"), "{err}");
    assert!(err.contains("opt-in daily check"), "{err}");
}

#[test]
fn update_help_names_the_daily_check() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_tacet"))
        .args(["update", "--help"])
        .env(
            "TACET_HOME",
            std::env::temp_dir().join("tacet-update-help-unused"),
        )
        .output()
        .expect("the tacet binary runs");
    assert!(out.status.success(), "`tacet update --help` failed");
    let help = String::from_utf8_lossy(&out.stdout);
    update_help_agrees(&help).unwrap_or_else(|e| panic!("`tacet update --help`:\n{e}\n\n{help}"));
}
