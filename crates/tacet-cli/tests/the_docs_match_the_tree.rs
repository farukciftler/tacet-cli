//! THE SMALL CLAIMS THE DOCUMENTS MAKE ABOUT THIS TREE, EACH READ BACK FROM IT.
//!
//! WHY THIS FILE EXISTS (audit D4, D10–D14, D19–D23, and RG-36's "the
//! gate list matches CI"). None of these was a large error. Each was a sentence
//! true on the day it was written about something that then moved — a version,
//! a directory, a manifest, a workflow — and each was found by a person reading
//! the page against the code, months later. The rule in CLAUDE.md says a README
//! change ships with the code change; this is the half of that rule a test can
//! hold, for the claims cheap enough to check mechanically.
//!
//! Every test reads the document and the thing it describes, never a copy of
//! either: a second literal of the number is a second thing that can drift.

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
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
}

/// D11: the `tacet --version` example prints the version this crate is.
///
/// It said 0.1.26 while the crate was 0.1.29 — an example of output is still a
/// claim about output.
#[test]
fn the_version_example_is_this_version() {
    let readme = read("README.md");
    let line = readme
        .lines()
        .find(|l| l.trim_start().starts_with("tacet --version") && l.contains("# tacet "))
        .expect("the README shows what `tacet --version` prints");
    let shown = line
        .split("# tacet ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("a version after `# tacet `");
    assert_eq!(
        shown,
        env!("CARGO_PKG_VERSION"),
        "README.md's `tacet --version` example prints {shown}; this crate is {}",
        env!("CARGO_PKG_VERSION")
    );
}

/// D10: no example sends a journal to `/tmp`.
///
/// The journal exists to outlive the session that started it, and a 184-case
/// run that wrote to `/tmp` lost 48 minutes when the directory was cleared. The
/// page's own example was the one shape the project's rule forbids.
#[test]
fn no_journal_example_writes_to_tmp() {
    let mut seen = 0;
    for document in ["README.md", "CONTRIBUTING.md"] {
        for (n, line) in read(document).lines().enumerate() {
            if !line.contains("--journal") {
                continue;
            }
            seen += 1;
            assert!(
                !line.contains("/tmp"),
                "{document}:{}: a journal under /tmp does not survive what it is for: {line}",
                n + 1
            );
        }
    }
    assert!(
        seen > 0,
        "no `--journal` line was found — the scan matched nothing"
    );
}

/// D13: SECURITY names the config directory the code resolves.
///
/// It said `~/.config/tacet`, a path the code has never used. The directory is
/// resolved by `tacet_kernel::env::config_dir`, so the check asks that
/// function rather than restating its rule: with only `HOME` set the answer
/// must end in the folder the page calls `~/.tacet`, and with
/// `XDG_CONFIG_HOME` set, in the one it calls `$XDG_CONFIG_HOME/tacet`.
///
/// ENVIRONMENT IS PROCESS-WIDE: this is the only test in this binary that
/// touches it, and nothing else here reads it.
#[cfg(unix)]
#[test]
fn security_names_the_config_directory_the_code_uses() {
    let security = read("SECURITY.md");
    assert!(
        !security.contains(".config/tacet"),
        "SECURITY.md names ~/.config/tacet, which the code never resolves"
    );
    for named in ["`$TACET_HOME`", "`$XDG_CONFIG_HOME/tacet`", "`~/.tacet`"] {
        assert!(
            security.contains(named),
            "SECURITY.md should name {named} as a config directory"
        );
    }

    let home = Path::new("/nonexistent-home-for-a-test");
    let xdg = Path::new("/nonexistent-xdg-for-a-test");
    let saved: Vec<(&str, Option<std::ffi::OsString>)> = ["TACET_HOME", "XDG_CONFIG_HOME", "HOME"]
        .into_iter()
        .map(|k| (k, std::env::var_os(k)))
        .collect();
    // SAFETY: no other test in this binary reads or writes the environment.
    unsafe {
        std::env::remove_var(tacet_kernel::env::HOME_VAR);
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::set_var("HOME", home);
    }
    let plain = tacet_kernel::env::config_dir();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", xdg) };
    let with_xdg = tacet_kernel::env::config_dir();
    unsafe {
        for (k, v) in &saved {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
    assert_eq!(
        plain.as_deref(),
        Some(home.join(".tacet").as_path()),
        "`~/.tacet`"
    );
    assert_eq!(
        with_xdg.as_deref(),
        Some(xdg.join("tacet").as_path()),
        "`$XDG_CONFIG_HOME/tacet`"
    );
}

/// D14: the workspace manifest's comment names exactly the crates that inherit
/// the workspace version.
///
/// It said `tacet-skills` AND `tacet-zip` for a month after `tacet-skills`
/// took its own number — and a reader bumping that version to release one of
/// them would have been told it moves both.
#[test]
fn the_workspace_version_comment_names_who_inherits_it() {
    let manifest = read("Cargo.toml");
    let comment = manifest
        .lines()
        .find(|l| l.starts_with("# THE VERSION") && l.contains("INHERIT"))
        .expect("the workspace manifest explains who inherits its version");
    let head = comment.split("INHERIT").next().unwrap_or_default();
    let mut named: Vec<String> = head
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect();
    named.sort();

    let mut inheriting = Vec::new();
    for entry in std::fs::read_dir(repo_root().join("crates")).expect("crates/") {
        let dir = entry.expect("an entry").path();
        let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        if text.lines().any(|l| l.trim() == "version.workspace = true") {
            let name = dir
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .to_string();
            inheriting.push(name);
        }
    }
    inheriting.sort();
    assert!(
        !inheriting.is_empty(),
        "no crate inherits — the scan matched nothing"
    );
    assert_eq!(
        named, inheriting,
        "the comment over `version` in Cargo.toml names {named:?}; the crates that \
         say `version.workspace = true` are {inheriting:?}"
    );
}

/// D23: a count of tests on the page carries the date it was counted.
///
/// The count is a measurement of a moving suite, so a number without a date is
/// wrong the next time a test is added, and a number with one is merely old.
///
/// BESIDE IT, NOT SOMEWHERE ON THE LINE. The platform table puts a whole
/// paragraph in one row, and the Linux row carries "4 Sep 2026" near its start
/// — so "anywhere on the line" passed the current-size claim with its own date
/// deleted, and so did a 25-word window either side, because the Linux
/// count's "4 Sep 2026" sits 21 words before it. A date must be at most
/// `BEFORE` words ahead of the count or `AFTER` words behind it; the page's
/// dated counts sit 8 words after a date (Windows) and 6 and 15 words before
/// one (Linux, the current size). A count followed by "that" (`24 tests that
/// used to skip`) says which tests, not how many the suite has, and is left
/// out.
#[test]
fn every_test_count_on_the_page_is_dated() {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const BEFORE: usize = 12;
    const AFTER: usize = 20;
    let is_date = |w: &[&str]| {
        w[0].trim_start_matches(['(', '*'])
            .chars()
            .all(|c| c.is_ascii_digit())
            && MONTHS.contains(&w[1])
            && w[2].starts_with("20")
            && w[2].trim_end_matches(|c: char| !c.is_ascii_digit()).len() == 4
    };
    let readme = read("README.md");
    let words: Vec<&str> = readme.split_whitespace().collect();
    let mut counts = 0;
    for i in 0..words.len().saturating_sub(2) {
        let number = words[i].trim_start_matches(['*', '(']);
        let is_count = !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit() || c == ',')
            && words[i + 1].trim_end_matches([',', '.', ')', '*']) == "tests"
            && words[i + 2] != "that";
        if !is_count {
            continue;
        }
        counts += 1;
        let from = i.saturating_sub(BEFORE);
        let to = (i + AFTER).min(words.len());
        let dated = words[from..to].windows(3).any(is_date);
        assert!(
            dated,
            "README.md: \"{} {}\" has no date beside it — it is wrong the \
             next time a test is added: …{}…",
            words[i],
            words[i + 1],
            words[from..to].join(" ")
        );
    }
    assert!(
        counts >= 3,
        "only {counts} test counts were found — the scan stopped matching"
    );
}

/// D4: no table on the page is cut in two by a blank line.
///
/// GitHub ends a table at the first blank line and renders what follows as
/// text, so the model table showed one model and three lines of pipes. A row
/// after the gap that is not a new table's header is the tell.
#[test]
fn no_table_is_split_by_a_blank_line() {
    for document in [
        "README.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "CHANGELOG.md",
    ] {
        let text = read(document);
        let lines: Vec<&str> = text.lines().collect();
        for i in 1..lines.len().saturating_sub(1) {
            let (before, gap, after) = (lines[i - 1], lines[i], lines[i + 1]);
            if !(before.starts_with('|') && gap.trim().is_empty() && after.starts_with('|')) {
                continue;
            }
            let starts_a_table = lines.get(i + 2).is_some_and(|l| {
                l.starts_with("|-") || l.starts_with("| -") || l.starts_with("|:")
            });
            assert!(
                starts_a_table,
                "{document}:{}: a blank line splits a table — the rows after it render \
                 as text:\n{before}\n\n{after}",
                i + 1
            );
        }
    }
}

/// The gate list in CONTRIBUTING, as commands.
fn contributing_gates() -> Vec<String> {
    let contributing = read("CONTRIBUTING.md");
    let start = contributing
        .find("None of this is visible to CI")
        .expect("the gate list's introduction in CONTRIBUTING.md");
    let block = &contributing[start..];
    let open = block.find("```bash").expect("the gate list's code block") + "```bash".len();
    let close = block[open..].find("```").expect("the code block closes") + open;
    block[open..close]
        .lines()
        .map(|l| l.split(" #").next().unwrap_or("").trim().to_string())
        .filter(|l| l.starts_with("cargo "))
        .collect()
}

/// RG-36: every `cargo` step CI runs is on CONTRIBUTING's list.
///
/// `the_workflows_keep_their_promises` holds the other direction (every line on
/// the list gates a pull request). This one is what lets a contributor run the
/// list and know CI will not fail on something it left out: the list did not
/// carry `eval --routing --routing-pressure 20`, and it spelled the fmt check
/// as `cargo fmt --all`, which rewrites files instead of failing.
#[test]
fn every_cargo_step_ci_runs_is_on_contributings_list() {
    let gates = contributing_gates();
    assert!(gates.len() >= 5, "the gate list was not parsed: {gates:?}");
    let ci = read(".github/workflows/ci.yml");
    let steps: Vec<String> = ci
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- run:"))
        .map(str::trim)
        .filter(|c| c.starts_with("cargo "))
        .map(str::to_string)
        .collect();
    assert!(
        steps.len() >= 5,
        "no `- run: cargo` step was found in ci.yml: {steps:?}"
    );
    let missing: Vec<&String> = steps.iter().filter(|s| !gates.contains(s)).collect();
    assert!(
        missing.is_empty(),
        "ci.yml runs {missing:?}, which CONTRIBUTING's gate list does not name — a \
         contributor who runs the list is surprised by CI. The list: {gates:?}"
    );
}

/// One step of a workflow job: its lines, from the `- ` that opens it to the
/// line before the next step or job.
fn workflow_steps(yaml: &str) -> Vec<Vec<&str>> {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut steps: Vec<Vec<&str>> = Vec::new();
    let mut open = false;
    for line in yaml.lines() {
        let t = line.trim_start();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if indent(line) == 6 && t.starts_with("- ") {
            steps.push(vec![line]);
            open = true;
        } else if indent(line) <= 6 {
            open = false;
        } else if open {
            steps.last_mut().expect("a step is open").push(line);
        }
    }
    steps
}

/// RG-36 (review): what CI can fail a pull request on, beyond the gate list, is
/// named in CONTRIBUTING.
///
/// The fix for RG-36 wrote "CI runs every line of it … and nothing else that can
/// fail one — except `cargo deny check`". ci.yml had two more: a Linux step that
/// starts a `bwrap` sandbox and stops the job if it cannot, and
/// `TACET_SANDBOX_MUST_RUN=1` on the Linux `cargo test`, which turns local skips
/// into failures. `every_cargo_step_ci_runs_is_on_contributings_list` compares
/// commands and could not see either. This walks every step: a `cargo` step's
/// `env:` keys must be shown on that command, a sandbox preflight must be
/// described, and any step it does not recognise fails, so a new kind of gate
/// is described before it lands rather than after someone is surprised by it.
#[test]
fn every_way_ci_can_fail_a_pull_request_is_named_in_contributing() {
    let contributing = read("CONTRIBUTING.md")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let ci = read(".github/workflows/ci.yml");
    let steps = workflow_steps(&ci);
    assert!(
        steps.len() >= 10,
        "ci.yml's steps were not parsed: {}",
        steps.len()
    );

    // Setup actions: they fail for reasons of GitHub's, not of the change.
    const SETUP: [&str; 3] = [
        "actions/checkout@",
        "dtolnay/rust-toolchain@",
        "Swatinem/rust-cache@",
    ];
    let mut env_checked = 0;
    let mut preflights = 0;
    for step in &steps {
        let head = step[0].trim_start().trim_start_matches("- ").trim();
        if let Some(action) = head.strip_prefix("uses:").map(str::trim) {
            if SETUP.iter().any(|s| action.starts_with(s)) {
                continue;
            }
            assert!(
                action.starts_with("EmbarkStudios/cargo-deny-action@")
                    && contributing.contains("`cargo deny check`"),
                "ci.yml runs the action {action}, which CONTRIBUTING does not name as a \
                 way CI can fail a pull request"
            );
            continue;
        }
        let body: Vec<&str> = step.iter().map(|l| l.trim()).collect();
        let run_line = body
            .iter()
            .find_map(|l| l.trim_start_matches("- ").strip_prefix("run:"))
            .map(str::trim)
            .unwrap_or_else(|| panic!("a ci.yml step that neither runs nor uses: {step:?}"));
        if run_line.starts_with("cargo ") {
            let in_env = body.iter().skip_while(|l| **l != "env:").skip(1);
            for key in in_env.filter_map(|l| l.split_once(':').map(|(k, _)| k.trim())) {
                env_checked += 1;
                let shown = contributing
                    .match_indices(&format!("{key}="))
                    .any(|(i, _)| {
                        contributing[i..]
                            .split_once(' ')
                            .is_some_and(|(_, rest)| rest.starts_with(run_line))
                    });
                assert!(
                    shown,
                    "ci.yml runs `{run_line}` with {key} set; CONTRIBUTING must show it \
                     as `{key}=… {run_line}` so a contributor can run what CI runs"
                );
            }
            continue;
        }
        let script = body.join("\n");
        if script.contains("bwrap ") {
            preflights += 1;
            assert!(
                contributing.contains("`bwrap --unshare-net` sandbox")
                    && contributing.contains("stops the job"),
                "ci.yml has a step that starts a bwrap sandbox and can stop the job; \
                 CONTRIBUTING does not say so"
            );
            continue;
        }
        panic!(
            "ci.yml has a step this test does not recognise; say in CONTRIBUTING how it \
             can fail a pull request, then teach this test: {step:?}"
        );
    }
    assert!(
        env_checked >= 1 && preflights >= 1,
        "the walk found {env_checked} cargo env keys and {preflights} sandbox preflights; \
         ci.yml has at least one of each"
    );
    assert!(
        !contributing.contains("nothing else that can fail"),
        "CONTRIBUTING still says nothing beyond the list and deny can fail CI"
    );
}

/// "1200" → "1,200", the way the page writes a count.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// RG-36: the README's section on other agents' sessions says what the version
/// table and the summary budget say.
///
/// The section quotes three things the code decides — which versions are read
/// without a warning, which one may be written, and how much of a transcript
/// reaches the model — and each of those is a number someone will move when a
/// new version is measured. Moving it without the page would put a write range
/// on the page that the code refuses, or the reverse.
#[test]
fn the_sessions_section_quotes_the_version_table() {
    use tacet_tools::sessions::AgentKind;
    use tacet_tools::sessions::version::KNOWN;

    let readme = read("README.md");
    let section = readme
        .split_once("## Other agents' sessions")
        .expect("the README has a section on other agents' sessions")
        .1;
    let section = section.split("\n## ").next().unwrap_or(section);
    let folded = section.split_whitespace().collect::<Vec<_>>().join(" ");
    let require = |want: String, why: &str| {
        assert!(
            folded.contains(&want),
            "the sessions section must say {want:?} — {why}"
        );
    };

    let row = |agent: AgentKind, flavor: Option<&str>| {
        KNOWN
            .iter()
            .find(|k| k.agent == agent && k.flavor == flavor)
            .unwrap_or_else(|| panic!("version::KNOWN has no {agent:?} {flavor:?} row"))
    };
    let range = |(lo, hi): (&str, &str)| {
        if lo == hi {
            lo.to_string()
        } else {
            format!("{lo}–{hi}")
        }
    };
    // NO LITERAL HERE NAMES AN AGENT WHERE A PROGRAM COULD STAND:
    // `network_monopoly` reads a string whose first word, or the first word
    // after a separator, is `claude`, `codex`, `antigravity` or `agy` as that
    // program being started, and it is right to — it cannot tell prose from a
    // command line. The names come from `AgentKind::display_name`, which is
    // also what keeps the page's spelling the code's.
    let claude = row(AgentKind::ClaudeCode, None);
    let ide = row(AgentKind::Antigravity, Some("ide"));
    let cli = row(AgentKind::Antigravity, Some("cli"));
    require(
        format!(
            "measured — {} {}, {} IDE {}, `",
            AgentKind::ClaudeCode.display_name(),
            range(claude.read),
            AgentKind::Antigravity.display_name(),
            range(ide.read),
        ),
        "the read ranges in version::KNOWN",
    );
    require(
        format!("` {} — and reads any other version", range(cli.read)),
        "the read range of the Antigravity CLI in version::KNOWN",
    );
    let (lo, hi) = claude
        .write
        .expect("a measured write range for Claude Code");
    assert_eq!(
        lo, hi,
        "the page calls the Claude Code write range one version"
    );
    let name = AgentKind::ClaudeCode.display_name();
    require(
        format!("The {name} write range is that one version, {lo}."),
        "the write range in version::KNOWN",
    );
    require(
        format!("with {name} {lo}"),
        "the experiment's version is the write range",
    );

    // Codex has no row, so it is read with a warning and never written; the
    // page must not promise more.
    assert!(
        !KNOWN.iter().any(|k| k.agent == AgentKind::Codex),
        "version::KNOWN now has a Codex row — the page says every Codex path is unmeasured"
    );
    require(
        "every Codex version, with a warning".to_string(),
        "there is no read range for Codex",
    );
    require(
        "**Not measured:** every Codex path".to_string(),
        "there is no measured range for Codex at all",
    );
    assert!(
        ide.write.is_none() && cli.write.is_none(),
        "an Antigravity write range exists — the page says task.md writes are refused"
    );
    require(
        "`task.md` checkboxes exist and are refused on every version".to_string(),
        "there is no write range for Antigravity",
    );

    require(
        format!(
            "A summary capped at {} characters",
            thousands(tacet_tools::sessions_tool::SUMMARY_BUDGET)
        ),
        "sessions_tool::SUMMARY_BUDGET",
    );
}

/// Where a test path the page cites lives, or why it does not.
///
/// `path` is a cargo filter, `module::inline_mod::test_name`, and cargo
/// matches it as a SUBSTRING of the real path — so `thinking::a_proven_split`
/// selects nothing when the test sits in `thinking::tests`, and runs zero tests
/// green. A citation is therefore checked as the whole path, not as "a
/// function by that name exists somewhere in the file": the first segment is a
/// file under `src/`, the last a `fn`, and the ones between exactly the inline
/// `mod`s that enclose it. A trailing `::` (`thinking::`) cites a whole module.
fn resolve_test_path(src: &Path, path: &str) -> Result<(), String> {
    let segments: Vec<&str> = path.split("::").filter(|s| !s.is_empty()).collect();
    let Some((&module, rest)) = segments.split_first() else {
        return Err(format!("`{path}` names nothing"));
    };
    let file = [
        src.join(format!("{module}.rs")),
        src.join(module).join("mod.rs"),
    ]
    .into_iter()
    .find(|f| f.is_file())
    .ok_or_else(|| format!("there is no module {module}"))?;
    let Some((&test, mods)) = rest.split_last() else {
        return Ok(());
    };
    let text = std::fs::read_to_string(&file).expect("the module is readable");
    let lines: Vec<&str> = text.lines().collect();
    let indent = |l: &str| l.len() - l.trim_start().len();
    let at = lines
        .iter()
        .position(|l| l.trim_start().starts_with(&format!("fn {test}(")))
        .ok_or_else(|| format!("{} has no `fn {test}`", file.display()))?;
    // Walk outwards: the first line at a smaller indent that opens a block is
    // the item the test sits in. Lines that open nothing (a raw string's
    // continuation at column 0) are passed over.
    let mut chain = Vec::new();
    let mut level = indent(lines[at]);
    for line in lines[..at].iter().rev() {
        if level == 0 {
            break;
        }
        let t = line.trim();
        if t.is_empty() || indent(line) >= level || !t.ends_with('{') {
            continue;
        }
        let name = t
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub ")
            .strip_prefix("mod ")
            .and_then(|m| m.strip_suffix(" {"))
            .ok_or_else(|| {
                format!(
                    "`fn {test}` in {} is inside `{t}`, not a module",
                    file.display()
                )
            })?;
        chain.push(name);
        level = indent(line);
    }
    chain.reverse();
    if chain != mods {
        return Err(format!(
            "`fn {test}` is at {module}::{}{test}, so `{path}` selects no test",
            chain.iter().map(|m| format!("{m}::")).collect::<String>()
        ));
    }
    Ok(())
}

/// Every `cargo test …` command the page quotes, whitespace folded, without
/// its backticks.
fn cited_commands(readme: &str) -> Vec<String> {
    // Fold first: the page wraps a long command across lines inside its
    // backticks.
    let folded = readme.split_whitespace().collect::<Vec<_>>().join(" ");
    // Anchored on the opening backtick and cut at the next one. Splitting the
    // page on backticks and taking every other piece is the obvious way and it
    // does not work: a ``` fence shifts the parity and the scan saw 3 of 10.
    folded
        .match_indices("`cargo test -p ")
        .map(|(at, _)| {
            let rest = &folded[at + 1..];
            rest[..rest.find('`').unwrap_or(rest.len())].to_string()
        })
        .collect()
}

/// D19–D22: every `cargo test` the page cites as the demonstration of a claim
/// names a test that exists — and a path that selects it.
///
/// The headline claims were re-checked after the fixes that made them true
/// (H5, L25, H3, H4, M4, H1), and the page now points at the test that shows
/// each one. A claim whose demonstration was renamed or deleted reads exactly
/// as well as one that still holds, so the citation is checked rather than
/// trusted: `--test <name>` must be `crates/<crate>/tests/<name>.rs`, and the
/// filter after `--lib` or `--bin <bin>` a path `resolve_test_path` accepts.
#[test]
fn every_test_the_page_cites_exists() {
    let readme = read("README.md");
    let mut cited = 0;
    for span in cited_commands(&readme) {
        let words: Vec<&str> = span["cargo test -p ".len()..].split_whitespace().collect();
        let krate = words[0];
        let root = repo_root().join("crates").join(krate);
        assert!(
            root.is_dir(),
            "README cites `{span}`, and there is no crate {krate}"
        );
        let filter_after = |i: usize| words.get(i).filter(|w| !w.starts_with('-')).copied();
        for (i, word) in words.iter().enumerate() {
            let filter = match *word {
                "--test" => {
                    cited += 1;
                    let file = root.join("tests").join(format!("{}.rs", words[i + 1]));
                    assert!(
                        file.is_file(),
                        "README cites `{span}`, and {} does not exist",
                        file.display()
                    );
                    None
                }
                "--lib" => {
                    cited += 1;
                    filter_after(i + 1)
                }
                "--bin" => {
                    cited += 1;
                    assert!(
                        root.join("src/main.rs").is_file(),
                        "README cites `{span}`, and {krate} has no binary"
                    );
                    filter_after(i + 2)
                }
                _ => None,
            };
            if let Some(path) = filter {
                if let Err(why) = resolve_test_path(&root.join("src"), path) {
                    panic!("README cites `{span}`: {why}");
                }
            }
        }
    }
    assert!(
        cited >= 8,
        "only {cited} cited tests were found — the scan stopped matching"
    );
}

/// D22: a question the page says a test types an answer at is a question that
/// test waits for.
///
/// The `db_write` paragraph said `consent_reaches_the_person` answers its
/// question through a pty, and the file drove only the OUTBOUND approval —
/// another prompt, another type (`TerminalApproval`, not
/// `TerminalWriteConfirm`). Both checks that existed passed: the cited file
/// existed and the approval worked. So a paragraph that cites a test and quotes
/// a `[y/N]` question is held to the test waiting for exactly that text.
#[test]
fn every_question_the_page_says_a_test_answers_is_one_it_waits_for() {
    let readme = read("README.md");
    let mut checked = 0;
    for paragraph in readme.split("\n\n") {
        let folded = paragraph.split_whitespace().collect::<Vec<_>>().join(" ");
        let questions: Vec<&str> = folded
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|q| q.ends_with("[y/N]"))
            .collect();
        if questions.is_empty() {
            continue;
        }
        for span in cited_commands(paragraph) {
            let words: Vec<&str> = span.split_whitespace().collect();
            let Some(i) = words.iter().position(|w| *w == "--test") else {
                continue;
            };
            let file = format!("crates/{}/tests/{}.rs", words[3], words[i + 1]);
            let test = read(&file);
            for q in &questions {
                checked += 1;
                assert!(
                    test.contains(&format!("wait_for(\"{q} \"")),
                    "README quotes `{q}` beside `{span}`, and {file} never waits for it"
                );
            }
        }
    }
    assert!(
        checked >= 1,
        "no quoted question beside a cited test — the db_write paragraph lost \
         its question or its citation"
    );
}

/// D19: the claim that a call carrying both thinking tags is refused cites the
/// place that refuses it.
///
/// The paragraph said every such call is refused and cited `thinking::`,
/// whose tests pin the opposite for a call the grammar proved
/// (`a_proven_split_has_one_reading`); the refusal of the RECOVERED shapes is
/// decided in the executor, and pinned there. A sentence about a refusal whose
/// only citation is in another crate cannot be read against its test.
#[test]
fn a_refusal_of_both_thinking_tags_cites_the_executor() {
    let readme = read("README.md");
    let paragraph = readme
        .split("\n\n")
        .find(|p| p.contains("`</think>`") && p.contains("is refused rather than"))
        .expect("the page still says which calls with both tags are refused");
    let cites = cited_commands(paragraph);
    assert!(
        cites
            .iter()
            .any(|c| c.starts_with("cargo test -p tacet-tools --lib executor::")),
        "the paragraph says a call is refused and cites only {cites:?} — the \
         refusal is in tacet-tools' executor"
    );
    assert!(
        cites
            .iter()
            .any(|c| c.starts_with("cargo test -p tacet-engine --lib thinking::")),
        "the paragraph says a proven call runs as written and no longer cites \
         the split that proves it: {cites:?}"
    );
}

/// D12: "the last publish" is the newest one the CHANGELOG records for
/// `tacet-cli`.
///
/// The paragraph was written to stop the page claiming it is what
/// `cargo install` gives you; it names a version and a date, and the next
/// publish makes both wrong. crates.io is not reachable from a test, but every
/// publish lands a `## Published YYYY-MM-DD` section whose table bolds the new
/// version — that is the record this reads.
#[test]
fn the_last_publish_is_the_changelogs() {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let changelog = read("CHANGELOG.md");
    let mut date = None;
    let mut found = None;
    for line in changelog.lines() {
        if let Some(rest) = line.strip_prefix("## Published ") {
            date = rest.split_whitespace().next();
        } else if line.starts_with("## ") {
            date = None;
        } else if let (Some(d), true) = (date, line.starts_with("| `tacet-cli` |")) {
            let version = line
                .split("**")
                .nth(1)
                .expect("the published version is in bold");
            found = Some((d, version));
            break;
        }
    }
    let (date, version) = found.expect("CHANGELOG.md records a tacet-cli publish");
    let mut parts = date.split('-');
    let (y, m, d) = (
        parts.next().expect("year"),
        parts
            .next()
            .and_then(|m| m.parse::<usize>().ok())
            .expect("month"),
        parts
            .next()
            .and_then(|d| d.parse::<u32>().ok())
            .expect("day"),
    );
    let want = format!(
        "The last publish was `tacet-cli` {version} on {d} {} {y}",
        MONTHS[m - 1]
    );
    let folded = read("README.md")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        folded.contains(&want),
        "README.md must say \"{want}\" — the newest publish CHANGELOG.md records"
    );
}

/// The page as prose: fenced blocks removed (their text is output, not a
/// claim in a sentence) and every run of whitespace folded to one space, so a
/// sentence wrapped across lines reads as one.
fn prose(markdown: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.push_str(line);
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// L38: the actions the page lists for `git` are the tool's actions.
///
/// It said "status, log, diff, show" while `Action::ALL` held three — the
/// model was never offered `show`. Read from `Action::ALL` itself, the
/// constant the tool's schema and parser are built from, so an action added
/// to the tool without a word on the page fails here as well.
#[test]
fn the_git_actions_the_page_lists_are_the_tools() {
    const MARKER: &str = "`git` is **read-only**:";
    let page = prose(&read("README.md"));
    assert_eq!(
        page.matches(MARKER).count(),
        1,
        "README.md states the git tool's actions once, after {MARKER:?}"
    );
    let listed = page
        .split(MARKER)
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .expect("a list after the marker");
    let mut shown: Vec<String> = listed
        .split(',')
        .map(|a| a.trim().trim_start_matches("and ").trim().to_string())
        .filter(|a| !a.is_empty())
        .collect();
    let mut actual: Vec<String> = tacet_tools::git::Action::ALL
        .iter()
        .map(|a| a.name().to_string())
        .collect();
    shown.sort();
    actual.sort();
    assert_eq!(
        shown, actual,
        "README.md lists the git actions as {listed:?}; the tool has {actual:?}"
    );
}

/// D8: the page never says `--compare` refuses two runs whose catalogs
/// differ. It does not: it leaves out the cases that touch a tool one side
/// lacked, names them, and pairs the rest.
///
/// The sentence said it "refuses to pair across either a different model or a
/// different catalog". The model half is true
/// (`a_comparison_across_two_models_is_refused`); the catalog half was the
/// comparator's first design and was replaced on purpose, because refusing
/// threw away 164 sound cases to avoid 20 unsound ones (eval_cmd.rs). The
/// guard reads every refusal the page attributes to `--compare` to the end of
/// its SENTENCE, and the warning the comparator really prints.
///
/// To the sentence, not the clause: a first version stopped at the first
/// comma, and "`--compare` refuses, when catalogs differ, to pair two
/// different models" passed it (review of 10 Oct 2026). The misstatements the
/// guard has been shown to miss or catch are checked below as well, so a
/// rewrite of the scan that goes blind to one fails here.
#[test]
fn the_page_says_compare_leaves_out_what_a_differing_catalog_touches() {
    let page = prose(&read("README.md"));
    let (refusals, wrong) = compare_refusals(&page);
    assert!(
        wrong.is_empty(),
        "README.md says `--compare` refuses on a catalog difference; it leaves those \
         cases out and pairs the rest: {wrong:#?}"
    );
    assert!(
        refusals >= 1,
        "no refusal attributed to `--compare` was found — the scan stopped matching"
    );
    for misstated in [
        "`--compare` refuses two runs whose tool sets differ. Next.",
        "`--compare` refuses, when catalogs differ, to pair two different models. Next.",
        "`--compare` refuses to pair two different models, and runs whose catalogs differ. Next.",
        "`--compare` refuses to pair two models and two catalogs. Next.",
    ] {
        assert!(
            !compare_refusals(misstated).1.is_empty(),
            "the scan no longer sees a misstatement: {misstated}"
        );
    }
    let sentences: Vec<&str> = page
        .split(". ")
        .filter(|s| s.contains("--compare"))
        .collect();
    assert!(
        sentences.iter().any(|s| {
            let s = s.to_lowercase();
            s.contains("catalog") && (s.contains("leaves out") || s.contains("exclud"))
        }),
        "README.md must say what `--compare` does when the catalogs differ: it leaves \
         out the cases that touch a tool one side lacked"
    );
    // The behaviour the sentence describes, read from the comparator: on a
    // catalog difference it warns and excludes; it does not stop.
    let comparator = read("crates/tacet-cli/src/eval_cmd.rs");
    assert!(
        comparator.contains("touch a tool one side lacked are EXCLUDED from the test below"),
        "eval_cmd.rs no longer excludes on a catalog difference — the README sentence \
         this test guards must change with it"
    );
}

/// How many refusals `prose` attributes to `--compare`, and those that are
/// about a catalog.
///
/// From each "refus" to the end of its sentence, split at ", and " / "; " into
/// clauses. The refusal's own clause may not mention a catalog or a tool at
/// all; a later clause may, only when it is the one that says what happens
/// instead ("leaves out", "exclud"). The true sentence is exactly that shape:
/// "refuses to pair two different models, and when the catalogs differ it
/// leaves out every case that touches a tool one side lacked".
fn compare_refusals(prose: &str) -> (usize, Vec<String>) {
    let about_catalog = |c: &str| c.contains("catalog") || c.contains("tool");
    let says_instead = |c: &str| c.contains("leaves out") || c.contains("exclud");
    let mut refusals = 0;
    let mut wrong = Vec::new();
    for sentence in prose.split(". ").filter(|s| s.contains("--compare")) {
        let lower = sentence.to_lowercase();
        let mut rest = lower.as_str();
        while let Some(at) = rest.find("refus") {
            refusals += 1;
            let tail = &rest[at..];
            let mut clauses = tail.split(", and ").flat_map(|c| c.split("; "));
            let own = clauses.next().unwrap_or_default();
            let misstated =
                about_catalog(own) || clauses.any(|c| about_catalog(c) && !says_instead(c));
            if misstated {
                wrong.push(tail.to_string());
            }
            rest = &rest[at + "refus".len()..];
        }
    }
    (refusals, wrong)
}
