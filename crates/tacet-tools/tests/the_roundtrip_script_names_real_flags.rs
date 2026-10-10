//! RG-29: `scripts/session-roundtrip.sh` reproduces the 9 Oct 2026 write-back
//! experiment against a real Claude Code, and nothing here runs it for real.
//!
//! WHY GUARD A SCRIPT NOBODY RUNS IN CI. Because it is run rarely — when Claude
//! Code jumps a version — by a person about to spend money on it, and its
//! answer decides whether `version::KNOWN` may admit writes to that version.
//! A script that dies on a renamed flag reads, to somebody who has already
//! decided, a lot like "nothing changed"; one that quietly stopped forking no
//! longer measures E5 at all. So the flags it passes are held to the ones the
//! experiment actually typed, the order "ask, then spend" is held from source,
//! and its verdict logic is driven both ways through a stand-in agent that
//! behaves the way the measurement says the real one did.
//!
//! THE STAND-IN IS NOT THE AGENT. It reproduces the four behaviours the
//! experiment recorded (resume walks `parentUuid` from the newest entry, a
//! wrong parent drops what came before, a fork copies, `--version` prints a
//! line) and nothing else. That is enough to show the script reads its own
//! results correctly; whether the real agent still behaves that way is what
//! the paid run is for.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tacet_tools::sessions::AgentKind;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate lives under <repo>/crates/")
        .to_path_buf()
}

const SCRIPT: &str = "scripts/session-roundtrip.sh";

fn script_text() -> String {
    let path = repo_root().join(SCRIPT);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The script without its comment lines. The header explains every flag and
/// every experiment in prose; a guard satisfied by the prose would stay green
/// with the code gone (the_smoke_check_names_real_flags.rs learned this).
fn code_only() -> Vec<String> {
    script_text()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// The flags the experiment typed, each with the row of `.audit/rnd_data.py`
/// (EXP) that typed it, all against Claude Code 2.1.285 on 9 Oct 2026. Not
/// read from `--help`: that would mean running the agent's binary in a test,
/// and "the flag exists" is weaker than "the flag did what E5 recorded".
const MEASURED_FLAGS: &[(&str, &str)] = &[
    (
        "--version",
        "the version line the findings sheet records (2.1.285)",
    ),
    ("-p", "E1-E5: every headless turn"),
    (
        "--output-format",
        "E1: `--output-format json`, the session id",
    ),
    ("--resume", "E2-E5: resuming the throwaway sessions"),
    (
        "--fork-session",
        "E5: the fork that left the original byte-identical",
    ),
];

/// Every flag passed to the agent: on lines that name `"$CC"`, and on calls of
/// `run <label> <dir> <args…>`, which hands its arguments to `"$CC"` — plus the
/// backslash-continued lines after either.
///
/// THE `run` LINES ARE MOST OF THEM. The `"$CC"` lines alone hold only
/// `--version` and the `"$@"` inside `run`: a scanner reading just those would
/// pass a script whose every experiment typed a flag that does not exist.
fn flags_passed() -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in code_only() {
        let t = line.trim();
        let via_run = t.starts_with("run ");
        if !inside && !via_run && !t.contains("\"$CC\"") {
            continue;
        }
        let mut words = t.split_whitespace();
        if !inside {
            if via_run {
                // `run`, the label, the directory.
                words.nth(2);
            } else {
                // Up to and including the program itself.
                for w in words.by_ref() {
                    if w.contains("\"$CC\"") {
                        break;
                    }
                }
            }
        }
        inside = true;
        for w in words {
            // The agent's arguments end where the shell starts the next
            // command (`| head -n 1` is head's flag, not the agent's).
            if ["|", "||", "&&", ";"].contains(&w) {
                break;
            }
            if w.starts_with('-') && w.len() > 1 && w != "-" && !w.starts_with("--$") {
                let flag = w.trim_end_matches(['"', ')', ';']);
                if flag
                    .chars()
                    .skip(1)
                    .all(|c| c.is_ascii_lowercase() || c == '-')
                {
                    out.push(flag.to_string());
                }
            }
        }
        if !t.ends_with('\\') {
            inside = false;
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn every_flag_the_script_passes_was_typed_in_the_measured_run() {
    let used = flags_passed();
    assert!(
        used.contains(&"-p".to_string()),
        "no headless invocation was found ({used:?}); the scanner is looking \
         for the wrong thing and would pass an empty file"
    );
    let unmeasured: Vec<&String> = used
        .iter()
        .filter(|f| !MEASURED_FLAGS.iter().any(|(m, _)| m == f))
        .collect();
    assert!(
        unmeasured.is_empty(),
        "the round-trip passes {unmeasured:?}, which the 9 Oct 2026 run never \
         typed: its outcome would no longer be the measurement it is compared to"
    );
    let unused: Vec<&str> = MEASURED_FLAGS
        .iter()
        .map(|(f, _)| *f)
        .filter(|f| !used.iter().any(|u| u == f))
        .collect();
    assert!(
        unused.is_empty(),
        "the round-trip no longer passes {unused:?}; the experiment that used it \
         is not reproduced"
    );
}

/// The agent is reached only through `$CC`, so the stand-in below is what
/// every call in the test meets — a bare name on some line would run the real
/// one, on the user's account.
#[test]
fn the_agent_is_run_only_through_one_variable() {
    let name = AgentKind::ClaudeCode.slug();
    let bare: Vec<String> = code_only()
        .into_iter()
        .filter(|l| {
            l.split(|c: char| c.is_whitespace() || "();&|`".contains(c))
                .any(|w| w == name)
        })
        .collect();
    assert!(bare.is_empty(), "the agent run by name, not $CC: {bare:#?}");
}

#[test]
fn it_asks_before_the_first_paid_turn_and_ci_never_runs_it() {
    let code = code_only();
    let asked = code
        .iter()
        .position(|l| l.trim() == "confirm_spend")
        .expect("confirm_spend is never called");
    let first_turn = code
        .iter()
        .position(|l| l.contains(" -p "))
        .expect("no headless turn");
    assert!(
        asked < first_turn,
        "a model turn (line {first_turn}) runs before the cost is agreed to (line {asked})"
    );
    let text = code.join("\n");
    assert!(
        text.contains("[ -r /dev/tty ] || die") && text.contains("[ \"$answer\" = yes ] || die"),
        "without --yes it must ask at a terminal and stop on anything but `yes`"
    );
    let workflows = repo_root().join(".github/workflows");
    for entry in std::fs::read_dir(&workflows).unwrap().flatten() {
        let body = std::fs::read_to_string(entry.path()).unwrap_or_default();
        assert!(
            !body.contains("session-roundtrip"),
            "{} runs the paid round-trip",
            entry.path().display()
        );
    }
}

#[test]
fn it_reproduces_every_measured_experiment() {
    let text = code_only().join("\n");
    for (needle, why) in [
        ("row E1 ", "E1: the session lands in projects/<enc>/"),
        ("row E2 ", "E2: the assistant-role pair"),
        ("row E3 ", "E3: the user-role note, the format Tacet writes"),
        ("row E4 ", "E4: the wrong parent that drops the history"),
        ("row E5 ", "E5: the fork that leaves the original alone"),
        ("PELICAN-42", "E2's codeword"),
        ("OSPREY-7", "E3's codeword"),
        ("HERON-9", "E4's codeword"),
        ("00000000-0000-4000-8000-000000000000", "E4's wrong parent"),
        (
            "[Note from Tacet, a local assistant, not from the user]",
            "the note prefix the ADR names as the only file format",
        ),
        (
            "append_note \"$A_FILE\" pair",
            "E2 writes the assistant role",
        ),
        (
            "before=\"$(sha \"$B_FILE\")\"",
            "E5 hashes the original first",
        ),
    ] {
        assert!(
            text.contains(needle),
            "{SCRIPT} no longer has {needle:?} — {why}"
        );
    }
}

/// Everything it deletes is its own, and it appends only inside its own
/// project directories.
#[test]
fn it_touches_only_what_it_created() {
    let code = code_only();
    for l in code.iter().filter(|l| l.contains("rm -rf")) {
        assert!(
            [
                "rm -rf \"$SCRATCH\"",
                "rm -rf \"$PROJ_A\"",
                "rm -rf \"$PROJ_B\""
            ]
            .iter()
            .any(|ok| l.contains(ok)),
            "deletes something other than its own scratch: {l}"
        );
    }
    for l in code.iter().filter(|l| l.contains(">>")) {
        assert!(
            l.contains(">>\"$file\""),
            "appends somewhere unchecked: {l}"
        );
    }
    let text = code.join("\n");
    assert!(text.contains("inside_scratch \"$file\" || die"));
    assert!(text.contains("already exists; refusing"));
}

#[cfg(unix)]
#[test]
fn the_script_is_executable_and_parses() {
    use std::os::unix::fs::PermissionsExt;
    let path = repo_root().join(SCRIPT);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "{SCRIPT} is not executable: {mode:o}");
    assert!(script_text().starts_with("#!/usr/bin/env bash\n"));
    let status = Command::new("bash")
        .arg("-n")
        .arg(&path)
        .status()
        .expect("bash");
    assert!(status.success(), "bash -n {SCRIPT}");
}

// ------------------------------------------------- the stand-in agent --

/// A stand-in with the behaviour the experiment recorded. `STUB_KEEP_HISTORY`
/// makes it keep the whole file on resume, which is the one change E4 exists
/// to notice.
const STAND_IN: &str = r#"#!/usr/bin/env bash
set -u
if [ "${1:-}" = --version ]; then echo "2.1.285 (stand-in)"; exit 0; fi
resume=""; fork=0; prompt=""
while [ $# -gt 0 ]; do
  case "$1" in
    -p) ;;
    --output-format) shift ;;
    --resume) shift; resume="$1" ;;
    --fork-session) fork=1 ;;
    -*) echo "stand-in: unknown flag $1" >&2; exit 2 ;;
    *) prompt="$1" ;;
  esac
  shift
done
cwd="$(pwd -P)"
proj="$CLAUDE_CONFIG_DIR/projects/$(printf '%s' "$cwd" | LC_ALL=C sed 's/[^A-Za-z0-9]/-/g')"
mkdir -p "$proj"
new_id() { uuidgen | tr 'A-Z' 'a-z'; }
entry() {
  jq -cn --arg u "$1" --arg p "$2" --arg t "$3" --arg x "$4" --arg s "$5" --arg cwd "$cwd" \
    '{parentUuid: (if $p == "" then null else $p end), isSidechain: false, userType: "external",
      cwd: $cwd, sessionId: $s, version: "2.1.285", gitBranch: "main", entrypoint: "sdk-cli",
      uuid: $u, timestamp: "2026-10-09T10:00:00.000Z", type: $t, message: {role: $t, content: $x}}'
}
leaf=""
if [ -z "$resume" ]; then
  sid="$(new_id)"; file="$proj/$sid.jsonl"; answer="READY"
else
  file="$proj/$resume.jsonl"
  [ -f "$file" ] || { echo "stand-in: no session $resume" >&2; exit 1; }
  leaf="$(jq -r 'select(.uuid != null and .isSidechain != true) | .uuid' "$file" | tail -n 1)"
  text='.message.content // "" | if type == "string" then . else (map(.text? // "") | join(" ")) end'
  if [ "${STUB_KEEP_HISTORY:-0}" = 1 ]; then
    context="$(jq -r "select(.uuid != null) | $text" "$file")"
  else
    context="$(jq -rs --arg leaf "$leaf" "(map(select(.uuid != null)) | INDEX(.uuid)) as \$m
      | [\$leaf | recurse(\$m[.].parentUuid // empty)] | map(\$m[.] // {} | $text) | .[]" "$file")"
  fi
  words="$(grep -oE '[A-Z]+-[0-9]+' <<<"$context" | sort -u | tr '\n' ' ')"
  answer="Codewords: ${words:-none}."
  if grep -q Tacet <<<"$context"; then answer="$answer Tacet left a note."; fi
  sid="$resume"
  if [ "$fork" = 1 ]; then
    sid="$(new_id)"; cp "$file" "$proj/$sid.jsonl"; file="$proj/$sid.jsonl"
  fi
fi
u1="$(new_id)"; u2="$(new_id)"
{ entry "$u1" "$leaf" user "$prompt" "$sid"; entry "$u2" "$u1" assistant "$answer" "$sid"; } >>"$file"
jq -cn --arg r "$answer" --arg s "$sid" \
  '{type: "result", subtype: "success", is_error: false, result: $r, session_id: $s, total_cost_usd: 0.1}'
"#;

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// The tools the script needs. A missing one on a developer machine skips
/// with a line on stderr; on CI (`CI` set, as GitHub sets it) it fails, so
/// the runners that are meant to drive this test cannot pass it by not
/// having `jq`.
#[cfg(unix)]
fn tools_present() -> bool {
    let missing: Vec<&str> = ["jq", "uuidgen", "perl"]
        .into_iter()
        .filter(|t| {
            !Command::new("sh")
                .arg("-c")
                .arg(format!("command -v {t}"))
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
        .collect();
    if missing.is_empty() {
        return true;
    }
    assert!(
        std::env::var_os("CI").is_none(),
        "the round-trip self-test needs {missing:?} on PATH"
    );
    eprintln!("skipped: the round-trip self-test needs {missing:?} on PATH");
    false
}

#[cfg(unix)]
fn run_with_stand_in(tag: &str, keep_history: bool) -> (Run, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!(
        "tacet-roundtrip-test-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["home", "tmp", "config/projects/-Users-dev-existing"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let stand_in = base.join("stand-in-agent");
    std::fs::write(&stand_in, STAND_IN).unwrap();
    std::fs::set_permissions(&stand_in, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A session the run did not create, which it must not touch.
    std::fs::write(
        base.join("config/projects/-Users-dev-existing/keep.jsonl"),
        "{\"uuid\":\"k\",\"parentUuid\":null}\n",
    )
    .unwrap();
    let out = Command::new("bash")
        .arg(repo_root().join(SCRIPT))
        .arg("--yes")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", base.join("home"))
        .env("TMPDIR", base.join("tmp"))
        .env("CLAUDE_CONFIG_DIR", base.join("config"))
        .env("TACET_ROUNDTRIP_CLAUDE", &stand_in)
        .env("STUB_KEEP_HISTORY", if keep_history { "1" } else { "0" })
        .stdin(Stdio::null())
        .output()
        .expect("bash");
    (
        Run {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        base,
    )
}

#[cfg(unix)]
fn left_behind(base: &Path) {
    let projects: Vec<String> = std::fs::read_dir(base.join("config/projects"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        projects,
        ["-Users-dev-existing"],
        "its project directories were not removed"
    );
    assert_eq!(
        std::fs::read_to_string(base.join("config/projects/-Users-dev-existing/keep.jsonl"))
            .unwrap(),
        "{\"uuid\":\"k\",\"parentUuid\":null}\n",
        "a session it did not create was changed"
    );
    let scratch: Vec<_> = std::fs::read_dir(base.join("tmp"))
        .unwrap()
        .flatten()
        .collect();
    assert!(scratch.is_empty(), "its scratch directory was not removed");
    let _ = std::fs::remove_dir_all(base);
}

/// An agent that behaves as measured: all five read "same", exit 0.
#[cfg(unix)]
#[test]
fn an_agent_that_behaves_as_measured_reads_as_the_measurement() {
    if !tools_present() {
        return;
    }
    let (run, base) = run_with_stand_in("same", false);
    let all = format!("{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.code, Some(0), "{all}");
    for e in ["E1", "E2", "E3", "E4", "E5"] {
        assert!(all.contains(&format!("{e}  same")), "{e} not same:\n{all}");
    }
    assert!(all.contains("All five as measured"), "{all}");
    left_behind(&base);
}

/// An agent that keeps the history behind a wrong parent: E4 — and only E4 —
/// reads CHANGED, and the exit code says one experiment differs.
#[cfg(unix)]
#[test]
fn an_agent_that_changed_is_reported_as_changed() {
    if !tools_present() {
        return;
    }
    let (run, base) = run_with_stand_in("changed", true);
    let all = format!("{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.code, Some(1), "{all}");
    assert!(all.contains("E4  CHANGED"), "{all}");
    for e in ["E1", "E2", "E3", "E5"] {
        assert!(all.contains(&format!("{e}  same")), "{e} not same:\n{all}");
    }
    assert!(all.contains("do NOT widen the write range"), "{all}");
    left_behind(&base);
}
