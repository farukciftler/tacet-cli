//! The CI and release workflows do what their comments and the README say.
//!
//! WHY A RUST TEST FOR YAML. Every finding below was a workflow whose comment,
//! or the README, described a property the YAML did not have: a "hard failure"
//! inside a job allowed to fail, a gate CONTRIBUTING said CI ran and CI did not,
//! write permissions the comment justified for one job and the file granted to
//! all of them. Nothing could go red, because nothing read the file. These
//! tests read it, and each one was watched failing against the shape it
//! replaced (audit of 9 Oct 2026, M25-M28, L32, L33, L35, L36).
//!
//! IT PARSES NO YAML, on purpose — the same rule as `network_monopoly.rs` and
//! `run_code.rs`'s `yaml_settings`: no new dependency to police a file. The
//! reader below is cruder than YAML and the crude direction is the safe one: a
//! shape it does not recognise is a shape it cannot find, which is a red test,
//! never a green one.

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

const WORKFLOWS: [&str; 3] = [
    ".github/workflows/ci.yml",
    ".github/workflows/release.yml",
    ".github/workflows/nightly.yml",
];

/// A line with its comment removed and trailing space trimmed, indentation
/// KEPT — indentation is the structure. A comment is a whole line starting
/// with `#`, or ` #` and everything after it; `${#array[@]}` in a shell line
/// has no space before its `#` and survives.
fn uncomment(line: &str) -> String {
    if line.trim_start().starts_with('#') {
        return String::new();
    }
    match line.find(" #") {
        Some(i) => line[..i].trim_end().to_string(),
        None => line.trim_end().to_string(),
    }
}

/// The file's lines, comments gone, blank lines dropped.
fn settings(text: &str) -> Vec<String> {
    text.lines()
        .map(uncomment)
        .filter(|l| !l.trim().is_empty())
        .collect()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The lines nested under the line at `at` — everything indented deeper than it,
/// up to the first line that is not.
fn nested(lines: &[String], at: usize) -> Vec<String> {
    let depth = indent(&lines[at]);
    lines[at + 1..]
        .iter()
        .take_while(|l| indent(l) > depth)
        .cloned()
        .collect()
}

/// The lines under a top-level key such as `permissions:`.
fn top_level(text: &str, key: &str) -> Option<Vec<String>> {
    let lines = settings(text);
    let at = lines.iter().position(|l| l == &format!("{key}:"))?;
    Some(nested(&lines, at))
}

struct Job {
    name: String,
    lines: Vec<String>,
}

impl Job {
    /// The lines under one of this job's own keys (`permissions:`, `needs:` …),
    /// trimmed. A key whose value is on the same line comes back as that value.
    fn key(&self, key: &str) -> Vec<String> {
        let prefix = format!("{key}:");
        for (i, line) in self.lines.iter().enumerate() {
            if indent(line) == 4
                && let Some(rest) = line.trim().strip_prefix(&prefix)
            {
                let rest = rest.trim();
                if !rest.is_empty() {
                    return vec![rest.to_string()];
                }
                return nested(&self.lines, i)
                    .iter()
                    .map(|l| l.trim().to_string())
                    .collect();
            }
        }
        Vec::new()
    }

    /// Each step's lines, in order. A step starts at `- ` six spaces in, which
    /// is where every workflow here puts them; a matrix `include` entry is
    /// deeper and is not mistaken for one.
    fn steps(&self) -> Vec<Vec<String>> {
        let mut out: Vec<Vec<String>> = Vec::new();
        let mut inside = false;
        for line in &self.lines {
            if indent(line) == 4 {
                inside = line.trim() == "steps:";
                continue;
            }
            if !inside {
                continue;
            }
            if indent(line) == 6 && line.trim_start().starts_with("- ") {
                out.push(Vec::new());
            }
            if let Some(step) = out.last_mut() {
                step.push(line.clone());
            }
        }
        out
    }

    fn text(&self) -> String {
        self.lines.join("\n")
    }
}

fn jobs(text: &str) -> Vec<Job> {
    let lines = settings(text);
    let at = lines
        .iter()
        .position(|l| l == "jobs:")
        .expect("a workflow has a `jobs:` key");
    let mut out: Vec<Job> = Vec::new();
    for line in nested(&lines, at) {
        if indent(&line) == 2 && line.trim_end().ends_with(':') {
            out.push(Job {
                name: line.trim().trim_end_matches(':').to_string(),
                lines: Vec::new(),
            });
        } else if let Some(job) = out.last_mut() {
            job.lines.push(line);
        }
    }
    assert!(
        !out.is_empty(),
        "no job was found; the reader is looking for the wrong thing and every \
         test below would pass on an empty file"
    );
    out
}

fn job<'a>(all: &'a [Job], name: &str) -> &'a Job {
    all.iter()
        .find(|j| j.name == name)
        .unwrap_or_else(|| panic!("no job named {name:?}"))
}

/// The shell source of every `run:` in the file: the inline value, or the
/// block under `run: |`.
fn run_blocks(text: &str) -> Vec<String> {
    let lines = settings(text);
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start().trim_start_matches("- ");
        let Some(rest) = trimmed.strip_prefix("run:") else {
            continue;
        };
        let column = line.find("run:").expect("the prefix was just matched");
        let mut block = vec![rest.trim().to_string()];
        block.extend(
            lines[i + 1..]
                .iter()
                .take_while(|l| indent(l) > column)
                .cloned(),
        );
        out.push(block.join("\n"));
    }
    assert!(!out.is_empty(), "no `run:` was found");
    out
}

/// M25 — WRITE ACCESS AND THE OIDC TOKEN BELONG TO THE ONE JOB THAT PUBLISHES.
///
/// release.yml granted `contents: write`, `id-token: write` and
/// `attestations: write` at the workflow level, so the build jobs — where every
/// dependency's `build.rs` runs — could mint the token that signs provenance.
/// The attestation would then vouch for whatever such a script swapped in.
#[test]
fn write_permissions_belong_to_the_publish_job_alone() {
    for workflow in WORKFLOWS {
        let text = read(workflow);
        let top = top_level(&text, "permissions").unwrap_or_else(|| {
            panic!(
                "{workflow} sets no workflow-level `permissions:`, so every job \
                 gets the repository default — which can be write-all"
            )
        });
        assert!(
            top.iter().any(|l| l.trim() == "contents: read"),
            "{workflow}: the workflow-level permissions must be `contents: read`, got {top:?}"
        );
        assert!(
            top.iter().all(|l| l.trim().ends_with(": read")),
            "{workflow}: the workflow-level permissions grant more than read: {top:?}"
        );

        for job in jobs(&text) {
            let permissions = job.key("permissions");
            let is_publish = workflow.ends_with("release.yml") && job.name == "publish";
            if is_publish {
                for grant in ["contents: write", "id-token: write", "attestations: write"] {
                    assert!(
                        permissions.iter().any(|l| l == grant),
                        "the publish job must grant `{grant}` itself, or the \
                         release and its attestation fail: {permissions:?}"
                    );
                }
                assert!(
                    !job.text().contains("cargo "),
                    "the publish job holds the write token and must run no \
                     cargo command: building runs dependency code"
                );
            } else {
                assert!(
                    permissions.iter().all(|l| !l.contains("write")),
                    "{workflow}: job {:?} grants itself write access: {permissions:?}",
                    job.name
                );
            }
        }
    }
}

/// M26 — EVERY ACTION IS THE CODE THAT WAS READ, NOT A TAG ITS OWNER CAN MOVE.
#[test]
fn every_action_is_pinned_to_a_commit_and_names_its_tag() {
    let mut seen = 0;
    for workflow in WORKFLOWS {
        for raw in read(workflow).lines() {
            let code = uncomment(raw);
            let code = code.trim().trim_start_matches("- ");
            let Some(action) = code.strip_prefix("uses:") else {
                continue;
            };
            seen += 1;
            let action = action.trim();
            let (_, reference) = action
                .split_once('@')
                .unwrap_or_else(|| panic!("{workflow}: `{action}` names no version at all"));
            assert!(
                reference.len() == 40
                    && reference
                        .chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "{workflow}: `{action}` is pinned to `{reference}`, which is a tag or a \
                 branch its owner can move. Pin the full commit SHA and put the tag in a \
                 comment: `git ls-remote https://github.com/<owner>/<repo> refs/tags/<tag>`"
            );
            assert!(
                raw.contains(" # "),
                "{workflow}: `{action}` carries no `# <tag>` comment, so neither a \
                 reader nor Dependabot can tell which release the SHA is"
            );
        }
    }
    assert!(
        seen >= 10,
        "only {seen} `uses:` lines found; the scanner is broken"
    );

    let dependabot = read(".github/dependabot.yml");
    assert!(
        settings(&dependabot)
            .iter()
            .any(|l| l.trim().trim_start_matches("- ") == "package-ecosystem: \"github-actions\""),
        "nothing proposes bumps for the pinned actions, so pinning becomes freezing"
    );
}

/// M26 — A TAG ON A RED COMMIT DOES NOT SHIP.
///
/// A tag push ran no tests and nothing checked CI on the tagged commit.
#[test]
fn a_release_is_published_only_after_its_tests_pass() {
    let text = read(".github/workflows/release.yml");
    let all = jobs(&text);
    let testers: Vec<&Job> = all
        .iter()
        .filter(|j| {
            j.lines
                .iter()
                .any(|l| l.trim().trim_start_matches("- ") == "run: cargo test --workspace")
        })
        .collect();
    assert!(
        !testers.is_empty(),
        "no job in release.yml runs `cargo test --workspace` on the tagged commit"
    );
    let needs = job(&all, "publish").key("needs").join(" ");
    for tester in testers {
        assert!(
            needs
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                .any(|n| n == tester.name),
            "publish needs {needs:?}, not the test job {:?} — a failing test would \
             not stop the release",
            tester.name
        );
    }
}

/// M27 — THE CHECKS ONLY THIS REPOSITORY CAN BREAK ARE A HARD GATE.
///
/// One `cargo deny check` job carried `continue-on-error: true`, so the
/// `unknown-git` and `unknown-registry` the README called hard failures
/// produced a green tick.
#[test]
fn the_supply_chain_checks_this_repository_can_break_are_a_hard_gate() {
    let text = read(".github/workflows/ci.yml");
    let deny: Vec<(String, String, bool)> = jobs(&text)
        .into_iter()
        .filter(|j| j.text().contains("cargo-deny-action"))
        .map(|j| {
            let command = j
                .lines
                .iter()
                .find_map(|l| {
                    l.trim()
                        .strip_prefix("command:")
                        .map(|c| c.trim().to_string())
                })
                .unwrap_or_else(|| "check".to_string());
            let soft = j.key("continue-on-error").iter().any(|v| v == "true");
            (j.name.clone(), command, soft)
        })
        .collect();
    assert!(!deny.is_empty(), "ci.yml runs cargo-deny nowhere");

    // `check` alone checks everything — the shape this replaced.
    let checks = |command: &str, what: &str| command == "check" || command.contains(what);
    for (name, command, soft) in &deny {
        if *soft {
            for hard in ["sources", "bans", "licenses"] {
                assert!(
                    !checks(command, hard),
                    "job {name:?} is allowed to fail and checks {hard} (`{command}`): \
                     a `git =` dependency would merge green"
                );
            }
        }
    }
    for hard in ["sources", "bans", "licenses"] {
        assert!(
            deny.iter().any(|(_, c, soft)| !soft && checks(c, hard)),
            "no gating job checks {hard}: {deny:?}"
        );
    }
    assert!(
        deny.iter().any(|(_, c, _)| checks(c, "advisories")),
        "advisories are no longer checked at all: {deny:?}"
    );
}

/// M28 + D17 — EVERY GATE CONTRIBUTING LISTS AND SAYS CI RUNS, CI RUNS.
///
/// The list held `cargo test --workspace --features candle --no-run` — "the one
/// that bites" — and only nightly.yml ran it, so a pull request could break the
/// candle-gated compile and merge green. Reading the list from CONTRIBUTING
/// rather than restating it here is what lets this fail when either side moves.
#[test]
fn every_gate_contributing_lists_runs_on_a_pull_request() {
    let contributing = read("CONTRIBUTING.md");
    let start = contributing
        .find("None of this is visible to CI")
        .expect("the gate list's introduction in CONTRIBUTING.md");
    let block = &contributing[start..];
    let open = block.find("```bash").expect("the gate list's code block") + "```bash".len();
    let close = block[open..].find("```").expect("the code block closes") + open;
    let gates: Vec<String> = block[open..close]
        .lines()
        .map(|l| l.split(" #").next().unwrap_or("").trim().to_string())
        .filter(|l| l.starts_with("cargo "))
        .collect();
    assert!(
        gates.len() >= 5
            && gates
                .iter()
                .any(|g| g == "cargo test --workspace --features candle --no-run"),
        "the gate list was not parsed, so this test would pass on anything: {gates:?}"
    );

    let ci = read(".github/workflows/ci.yml");
    assert!(
        top_level(&ci, "on")
            .unwrap_or_default()
            .iter()
            .any(|l| l.trim().trim_end_matches(':') == "pull_request"),
        "ci.yml no longer runs on pull requests"
    );
    let commands = run_blocks(&ci).join("\n");
    let missing: Vec<&String> = gates.iter().filter(|g| !commands.contains(*g)).collect();
    assert!(
        missing.is_empty(),
        "CONTRIBUTING lists {missing:?} as a gate, but no step in ci.yml runs it"
    );
}

/// L32 — NO `${{ }}` FROM AN INPUT OR AN EVENT IS PASTED INTO SHELL SOURCE.
///
/// The expression is substituted into the script text before bash parses it,
/// so `inputs.baseline` set to `x"; curl … #` ran that command. Through `env:`
/// it is only ever data. `matrix.*` values are written in the file itself and
/// are not covered.
#[test]
fn no_run_block_pastes_untrusted_context_into_a_shell() {
    for workflow in WORKFLOWS {
        for block in run_blocks(&read(workflow)) {
            let mut rest = block.as_str();
            while let Some(open) = rest.find("${{") {
                let close = rest[open..].find("}}").map_or(rest.len(), |c| open + c + 2);
                let expression = &rest[open..close];
                assert!(
                    !expression.contains("inputs.") && !expression.contains("github."),
                    "{workflow}: `{expression}` is pasted into a shell script; pass it \
                     through `env:` instead:\n{block}"
                );
                rest = &rest[close..];
            }
        }
    }

    let nightly = read(".github/workflows/nightly.yml");
    let compare = job(&jobs(&nightly), "compare-uploaded-report").text();
    assert!(
        compare.contains("BASELINE: ${{ inputs.baseline }}") && compare.contains("=~"),
        "the dispatched baseline name must arrive through `env:` and be validated \
         as a plain file name before it is used as a path"
    );
}

/// L33 — THE SBOM IS INSIDE THE CHECKSUMS AND THE ATTESTATION.
///
/// It was generated after both, so the asset describing what the binaries are
/// made of was the one asset neither covered.
#[test]
fn the_sbom_is_covered_by_the_checksums_and_the_attestation() {
    let text = read(".github/workflows/release.yml");
    let steps = job(&jobs(&text), "publish").steps();
    let at = |needle: &str| {
        steps
            .iter()
            .position(|s| s.join("\n").contains(needle))
            .unwrap_or_else(|| panic!("the publish job has no step with {needle:?}"))
    };
    let (sbom, sums, attest) = (
        at("sbom-action"),
        at("sha256sum"),
        at("attest-build-provenance"),
    );
    assert!(
        sbom < sums && sums < attest,
        "the SBOM is step {sbom}, the checksums {sums}, the attestation {attest}: \
         the SBOM must come first or neither covers it"
    );
    assert!(
        steps[sbom].iter().any(|l| l.contains("output-file: dist/"))
            && steps[attest]
                .iter()
                .any(|l| l.contains("subject-path: \"dist/*\"")),
        "the SBOM must land in dist/ and the attestation must cover dist/*"
    );
}

/// L35 — THE WINDOWS BINARY IS ASKED WHAT IT WAS BUILT WITH, TOO.
#[test]
fn every_release_binary_is_asked_for_its_features() {
    let text = read(".github/workflows/release.yml");
    let steps = job(&jobs(&text), "build").steps();
    let check = steps
        .iter()
        .find(|s| s.join("\n").contains("--print-features"))
        .expect("the build job no longer asks the binary for its features");
    assert!(
        !check
            .iter()
            .any(|l| l.trim().trim_start_matches("- ").starts_with("if:")),
        "the feature check is conditional again, so some target ships unasked: {check:?}"
    );
    assert!(
        check.iter().any(|l| l.trim() == "shell: pwsh"),
        "the check must run in a shell present on all three runner images (pwsh): {check:?}"
    );
}

/// L36 — A RELEASE LINKS NOTHING ANOTHER BRANCH LEFT IN A CACHE.
#[test]
fn the_release_builds_from_nothing() {
    let text = settings(&read(".github/workflows/release.yml")).join("\n");
    assert!(
        !text.contains("rust-cache"),
        "release.yml restores a build cache that another branch's run may have written"
    );
}
