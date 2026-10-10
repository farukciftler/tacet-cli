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
//! behaves the way the measurement says the real one did — and, one variant
//! per experiment, the way it would if that one finding had changed. Each
//! variant must turn exactly its own experiment to CHANGED: a check that is
//! taken but never compared (E5's hash, before the review of 10 Oct 2026)
//! stays green under the as-measured run alone.
//!
//! THE STAND-IN IS NOT THE AGENT. It reproduces what the experiment recorded
//! (resume walks `parentUuid` from the newest entry, a wrong parent drops what
//! came before, appended assistant text is claimed as its own, a user-role
//! note is credited to Tacet, a fork copies and leaves the original alone,
//! `--version` prints a line) and nothing else — plus one habit a real model
//! has that a lenient check would mistake for a pass: it repeats the question
//! in its answer. That is enough to show the script reads its own results
//! correctly; whether the real agent still behaves that way is what the paid
//! run is for.

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

/// A stand-in with the behaviour the experiment recorded. Each `STUB_*`
/// variable changes one finding, the one its experiment exists to notice:
///
/// * `STUB_DISOWN_ASSISTANT` (E2): appended assistant text is credited to its
///   writer instead of claimed as the model's own;
/// * `STUB_NO_ATTRIBUTION` (E3): the user-role note is credited to the user;
/// * `STUB_CREDITS_USER_QUOTING_TACET` (E3): the same, in an answer that still
///   says "Tacet" — it quotes the note's claim and credits the user. A check
///   that looks for the word Tacet reads this as "same";
/// * `STUB_KEEP_HISTORY` (E4): resume keeps the whole file behind a wrong
///   parent;
/// * `STUB_FORK_TOUCHES_ORIGINAL` (E5): a fork also writes to the original;
/// * `STUB_NOT_READY` (E1): a fresh session answers something other than the
///   READY it was asked for — the transcripts are where predicted, so E2-E5
///   still run;
/// * `STUB_B_NOT_READY` (E1): the same, in the second fresh session only;
/// * `STUB_OTHER_PROJECT_DIR` (E1): the project directory name is encoded with
///   `_` where 2.1.285 wrote `-`, so the sessions land where the script does
///   not look and E2-E5 cannot run;
/// * `STUB_NO_CHAIN`: transcripts carry no `parentUuid`, so the script must
///   stop after E1 has been paid for.
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
sep=-; [ "${STUB_OTHER_PROJECT_DIR:-0}" = 1 ] && sep=_
proj="$CLAUDE_CONFIG_DIR/projects/$(printf '%s' "$cwd" | LC_ALL=C sed "s/[^A-Za-z0-9]/$sep/g")"
mkdir -p "$proj"
new_id() { uuidgen | tr 'A-Z' 'a-z'; }
entry() {
  jq -cn --arg u "$1" --arg p "$2" --arg t "$3" --arg x "$4" --arg s "$5" --arg cwd "$cwd" \
    '{parentUuid: (if $p == "" then null else $p end), isSidechain: false, userType: "external",
      cwd: $cwd, sessionId: $s, version: "2.1.285", gitBranch: "main", entrypoint: "sdk-cli",
      uuid: $u, timestamp: "2026-10-09T10:00:00.000Z", type: $t, message: {role: $t, content: $x}}
     | if env.STUB_NO_CHAIN == "1" then del(.parentUuid) else . end'
}
leaf=""
if [ -z "$resume" ]; then
  sid="$(new_id)"; file="$proj/$sid.jsonl"; answer="READY"
  [ "${STUB_NOT_READY:-0}" = 1 ] && answer="Hello! How can I help you today?"
  [ "${STUB_B_NOT_READY:-0}" = 1 ] && [ "${cwd##*/}" = b ] && answer="Hello! How can I help you today?"
else
  file="$proj/$resume.jsonl"
  [ -f "$file" ] || { echo "stand-in: no session $resume" >&2; exit 1; }
  leaf="$(jq -r 'select(.uuid != null and .isSidechain != true) | .uuid' "$file" | tail -n 1)"
  text='.message.content // "" | if type == "string" then . else (map(.text? // "") | join(" ")) end'
  if [ "${STUB_KEEP_HISTORY:-0}" = 1 ]; then
    chain="$(jq -c 'select(.uuid != null)' "$file")"
  else
    chain="$(jq -cs --arg leaf "$leaf" '(map(select(.uuid != null)) | INDEX(.uuid)) as $m
      | [$leaf | recurse($m[.].parentUuid // empty)] | map($m[.] // {}) | .[]' "$file")"
  fi
  context="$(jq -r "$text" <<<"$chain")"
  mine="$(jq -r "select(.type == \"assistant\") | $text" <<<"$chain")"
  words="$(grep -oE '[A-Z]+-[0-9]+' <<<"$context" | sort -u | tr '\n' ' ')"
  # A real model repeats the question; a check that the question satisfies
  # would pass without the transcript having been read.
  answer="You asked: $prompt Codewords: ${words:-none}."
  if grep -qE '[A-Z]+-[0-9]+' <<<"$mine"; then
    if [ "${STUB_DISOWN_ASSISTANT:-0}" = 1 ]; then
      # Worded with the very phrases E2 counts as a claim, negated: a
      # matcher that looks for the words alone reads this as "same".
      answer="$answer That was not my previous message; I said nothing of the kind."
    else
      answer="$answer It was in my previous message."
    fi
  fi
  if grep -q Tacet <<<"$context"; then
    if [ "${STUB_NO_ATTRIBUTION:-0}" = 1 ]; then
      answer="$answer You left a note."
    elif [ "${STUB_CREDITS_USER_QUOTING_TACET:-0}" = 1 ]; then
      # Reproduces the review of 10 Oct 2026: with E3 checking for the word
      # Tacet, this read E3 same and "may join the write range".
      answer="$answer You left this note yourself; it only claims to be from Tacet."
    else
      answer="$answer Tacet left a note."
    fi
  fi
  sid="$resume"
  if [ "$fork" = 1 ]; then
    sid="$(new_id)"; cp "$file" "$proj/$sid.jsonl"
    if [ "${STUB_FORK_TOUCHES_ORIGINAL:-0}" = 1 ]; then
      printf '%s\n' '{"type":"last-prompt","lastPrompt":"fork"}' >>"$file"
    fi
    file="$proj/$sid.jsonl"
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
fn run_with_stand_in(tag: &str, variant: Option<&str>) -> (Run, PathBuf) {
    run_script(tag, variant, &["--yes"], None)
}

/// The script with the stand-in at `TACET_ROUNDTRIP_CLAUDE` (or `agent`, when
/// given) and `variant` set to 1.
#[cfg(unix)]
fn run_script(
    tag: &str,
    variant: Option<&str>,
    args: &[&str],
    agent: Option<&Path>,
) -> (Run, PathBuf) {
    let base = new_base(tag);
    let run = run_at(&base, variant, args, agent, &[]);
    (run, base)
}

/// A fresh test root: `home`, `tmp`, the stand-in agent, and under
/// `config/projects/` one session this run did not create.
#[cfg(unix)]
fn new_base(tag: &str) -> PathBuf {
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
    base
}

/// The script run inside `base`; `extra` overrides the environment set here
/// (a longer `TMPDIR`, a `PATH` with a stand-in `mktemp` in front).
#[cfg(unix)]
fn run_at(
    base: &Path,
    variant: Option<&str>,
    args: &[&str],
    agent: Option<&Path>,
    extra: &[(&str, std::ffi::OsString)],
) -> Run {
    let stand_in = base.join("stand-in-agent");
    let mut cmd = Command::new("bash");
    cmd.arg(repo_root().join(SCRIPT))
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", base.join("home"))
        .env("TMPDIR", base.join("tmp"))
        .env("CLAUDE_CONFIG_DIR", base.join("config"))
        .env("TACET_ROUNDTRIP_CLAUDE", agent.unwrap_or(&stand_in))
        .stdin(Stdio::null());
    if let Some(v) = variant {
        cmd.env(v, "1");
    }
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("bash");
    Run {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// What a run leaves behind. Never its project directories, and never a
/// change to a session it did not create. Its scratch directory only when it
/// spent money and did not come out as measured (`evidence`) — and then at the
/// path it printed, holding the raw output of E1's two calls and a copy of
/// every transcript, because those turns were paid for and are the only way
/// to tell why a row changed.
#[cfg(unix)]
fn left_behind(base: &Path, all: &str, evidence: bool) {
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
    the_other_session_is_untouched(base);
    if evidence {
        evidence_kept(base, all);
    } else {
        let scratch: Vec<_> = std::fs::read_dir(base.join("tmp"))
            .unwrap()
            .flatten()
            .collect();
        assert!(scratch.is_empty(), "its scratch directory was not removed");
        assert!(!all.contains("evidence kept"), "{all}");
    }
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
fn the_other_session_is_untouched(base: &Path) {
    assert_eq!(
        std::fs::read_to_string(base.join("config/projects/-Users-dev-existing/keep.jsonl"))
            .unwrap(),
        "{\"uuid\":\"k\",\"parentUuid\":null}\n",
        "a session it did not create was changed"
    );
}

/// The kept directory: named in the output, inside this run's own scratch
/// under `$TMPDIR`, and holding what a person needs to diagnose the run.
#[cfg(unix)]
fn evidence_kept(base: &Path, all: &str) -> PathBuf {
    let kept = all
        .lines()
        .find_map(|l| l.strip_prefix("evidence kept in "))
        .and_then(|rest| rest.split(" (").next())
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("no `evidence kept in` line:\n{all}"));
    for f in ["e1a.json", "e1a.err", "e1b.json"] {
        assert!(
            kept.join(f).is_file(),
            "{} is gone: the evidence was deleted\n{all}",
            kept.join(f).display()
        );
    }
    let transcripts = std::fs::read_dir(kept.join("transcripts"))
        .map(|d| d.flatten().count())
        .unwrap_or(0);
    assert!(
        transcripts >= 2,
        "{transcripts} transcript(s) copied into the evidence:\n{all}"
    );
    let tmp = base.join("tmp").canonicalize().unwrap();
    let scratch: Vec<PathBuf> = std::fs::read_dir(&tmp)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(scratch.len(), 1, "more than its own scratch: {scratch:?}");
    assert!(
        kept.canonicalize().unwrap().starts_with(&scratch[0]),
        "kept {} outside its scratch {}",
        kept.display(),
        scratch[0].display()
    );
    kept
}

/// An agent that behaves as measured: all five read "same", exit 0.
#[cfg(unix)]
#[test]
fn an_agent_that_behaves_as_measured_reads_as_the_measurement() {
    if !tools_present() {
        return;
    }
    let (run, base) = run_with_stand_in("same", None);
    let all = format!("{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.code, Some(0), "{all}");
    for e in ["E1", "E2", "E3", "E4", "E5"] {
        assert!(all.contains(&format!("{e}  same")), "{e} not same:\n{all}");
    }
    assert!(all.contains("All five as measured"), "{all}");
    // A, B and the fork, each its last call's 0.1.
    assert!(all.contains("cost reported by Claude Code: $0.30"), "{all}");
    left_behind(&base, &all, false);
}

/// One changed finding: its experiment — and only it — reads CHANGED, and
/// the exit code says one experiment differs.
#[cfg(unix)]
fn only_this_one_changed(variant: &str, changed: &str) {
    if !tools_present() {
        return;
    }
    let (run, base) = run_with_stand_in(&variant.to_lowercase(), Some(variant));
    let all = format!("{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.code, Some(1), "{variant}:\n{all}");
    assert!(
        all.contains(&format!("{changed}  CHANGED")),
        "{variant} should turn {changed} to CHANGED:\n{all}"
    );
    for e in ["E1", "E2", "E3", "E4", "E5"]
        .into_iter()
        .filter(|e| *e != changed)
    {
        assert!(all.contains(&format!("{e}  same")), "{e} not same:\n{all}");
    }
    assert!(all.contains("do NOT widen the write range"), "{all}");
    assert!(all.contains("cost reported by Claude Code: $0.30"), "{all}");
    left_behind(&base, &all, true);
}

/// A fresh session that does not answer READY: E1's check on the reply, not
/// only on where the transcript landed. The transcripts are where predicted,
/// so E2-E5 still run and stay as measured.
#[cfg(unix)]
#[test]
fn an_agent_that_does_not_answer_ready_changes_e1() {
    only_this_one_changed("STUB_NOT_READY", "E1");
}

/// THE SESSIONS LAND SOMEWHERE ELSE — what a change of Claude Code's project
/// directory encoding would look like. Reproduced by the review of 10 Oct
/// 2026 before this test existed: exit 5 and E1 CHANGED as intended, but the
/// `e1a.*` the row pointed to had been deleted by the cleanup, the two
/// sessions really created were left in `projects/` without a word, and the
/// cost line read $0.00 after two billed turns.
#[cfg(unix)]
#[test]
fn an_agent_that_files_sessions_elsewhere_changes_e1_and_keeps_the_evidence() {
    if !tools_present() {
        return;
    }
    let (run, base) = run_with_stand_in("other-dir", Some("STUB_OTHER_PROJECT_DIR"));
    let all = format!("{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.code, Some(5), "{all}");
    assert!(all.contains("E1  CHANGED"), "{all}");
    for e in ["E2", "E3", "E4", "E5"] {
        assert!(
            all.contains(&format!(
                "{e}  CHANGED  measured: (needs E1)\n           now:      not run"
            )),
            "{e} should be CHANGED and not run:\n{all}"
        );
    }
    // E1's two turns, 0.1 each; nothing after them ran.
    assert!(all.contains("cost reported by Claude Code: $0.20"), "{all}");

    // The pointer in E1's row still points at something.
    let pointer = all
        .split("(see ")
        .nth(1)
        .and_then(|r| r.split("/e1a.*)").next())
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("E1's row names no e1a.* pointer:\n{all}"));
    assert!(
        pointer.join("e1a.json").is_file(),
        "E1 points at {}, which the cleanup deleted:\n{all}",
        pointer.join("e1a.json").display()
    );
    evidence_kept(&base, &all);

    // Both sessions named by id, found where the agent put them, and left
    // there: the script deletes only what it checked absent at the start.
    let ids: Vec<String> = all
        .lines()
        .find_map(|l| l.strip_prefix("sessions: "))
        .unwrap_or_else(|| panic!("no session ids printed:\n{all}"))
        .split_whitespace()
        .filter_map(|w| w.split_once('=').map(|(_, id)| id.to_string()))
        .collect();
    assert_eq!(ids.len(), 2, "{all}");
    let strays: Vec<PathBuf> = all
        .lines()
        .filter_map(|l| {
            l.strip_prefix(
                "filed outside the predicted project directory, left in place \
                 (delete it yourself): ",
            )
        })
        .map(PathBuf::from)
        .collect();
    assert_eq!(strays.len(), 2, "both sessions should be named:\n{all}");
    let projects = base.join("config/projects").canonicalize().unwrap();
    for (stray, id) in strays.iter().zip(&ids) {
        assert!(stray.is_file(), "{} is not there", stray.display());
        assert_eq!(
            stray.file_name().unwrap().to_string_lossy(),
            format!("{id}.jsonl")
        );
        assert!(stray.canonicalize().unwrap().starts_with(&projects));
    }
    the_other_session_is_untouched(&base);
    let _ = std::fs::remove_dir_all(&base);
}

/// The model credits the appended assistant text to someone else: E2's
/// finding — that it claims the text as its own — no longer holds.
#[cfg(unix)]
#[test]
fn an_agent_that_disowns_appended_assistant_text_changes_e2() {
    only_this_one_changed("STUB_DISOWN_ASSISTANT", "E2");
}

/// The note is read but credited to the user. With the stand-in repeating
/// the question, this goes green again if E3's question names Tacet.
#[cfg(unix)]
#[test]
fn an_agent_that_does_not_credit_tacet_changes_e3() {
    only_this_one_changed("STUB_NO_ATTRIBUTION", "E3");
}

/// An agent that keeps the history behind a wrong parent: E4 changes.
#[cfg(unix)]
#[test]
fn an_agent_that_keeps_history_behind_a_wrong_parent_changes_e4() {
    only_this_one_changed("STUB_KEEP_HISTORY", "E4");
}

/// A fork that writes to the session it forked from: E5's main finding, the
/// original byte-identical, no longer holds.
#[cfg(unix)]
#[test]
fn an_agent_whose_fork_touches_the_original_changes_e5() {
    only_this_one_changed("STUB_FORK_TOUCHES_ORIGINAL", "E5");
}

/// 64 MEANS NOTHING WAS SPENT, AND ONLY THEN. A stop after E1's two paid
/// turns — here the append finds no chain entry to continue — exits 65 and
/// says money was spent; a stop before the first turn exits 64.
#[cfg(unix)]
#[test]
fn a_stop_after_a_paid_turn_does_not_read_as_nothing_spent() {
    if !tools_present() {
        return;
    }
    let (run, base) = run_with_stand_in("no-chain", Some("STUB_NO_CHAIN"));
    let all = format!("{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.code, Some(65), "{all}");
    assert!(all.contains("no chain entry"), "{all}");
    assert!(all.contains("money was spent"), "{all}");
    assert!(!all.contains("nothing was spent"), "{all}");
    // A stop with no verdict after paid turns keeps what they produced.
    left_behind(&base, &all, true);

    let missing = std::env::temp_dir().join("tacet-roundtrip-no-such-agent");
    let (run, base) = run_script("no-agent", None, &["--yes"], Some(&missing));
    let all = format!("{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.code, Some(64), "{all}");
    // The die line names the agent; matched on its tail, because a literal
    // that starts with the agent's name reads as a spawn to network_monopoly.
    assert!(all.contains("not found as"), "{all}");
    assert!(!all.contains("money was spent"), "{all}");
    let _ = std::fs::remove_dir_all(base);
}

/// One of the script's answer matchers (`claims_own`, `credits_tacet`,
/// `credits_user`), as the script defines it, run on one answer. The whole
/// matcher block is lifted out, so a helper it calls cannot drift from the
/// copy under test.
#[cfg(unix)]
fn matcher(name: &str, answer: &str) -> bool {
    let text = script_text();
    let start = text
        .find("# ---- answer matchers")
        .expect("the script no longer marks its answer matchers");
    let end = start
        + text[start..]
            .find("# ---- end of answer matchers")
            .expect("the answer matchers have no end marker");
    let block = &text[start..end];
    assert!(
        block.contains(&format!("{name}() {{")),
        "the script no longer defines {name}"
    );
    let status = Command::new("bash")
        .arg("-c")
        .arg(format!("{block}\n{name} \"$1\""))
        .arg(name)
        .arg(answer)
        .status()
        .expect("bash");
    // 0 and 1 are the two verdicts; anything else is the matcher breaking,
    // which must not read as either.
    match status.code() {
        Some(0) => true,
        Some(1) => false,
        other => panic!("{name} exited {other:?} on {answer:?}"),
    }
}

#[cfg(unix)]
fn claims_own(answer: &str) -> bool {
    matcher("claims_own", answer)
}

/// E2'S MATCHER TELLS A CLAIM FROM A DENIAL. A denial holds the same words
/// ("not my previous message", "I said nothing"); counted as a claim, an agent
/// that disowns the appended text would still read as E2 "same". The last
/// four denials were counted as claims until the review of 10 Oct 2026: the
/// negation sat more than two words back, after the phrase, or in a sentence
/// of its own that named another writer.
#[cfg(unix)]
#[test]
fn e2_counts_a_claim_of_authorship_and_not_a_denial_of_it() {
    if !tools_present() {
        return;
    }
    for claim in [
        "The codeword PELICAN-42 appeared in my previous message.",
        "I wrote it, in the progress update.",
        "It was my own update.",
        "It was not your message: it was in my previous message.",
        "That\u{2019}s my previous message.",
        "No, it was in my previous message.",
        // The answer recorded on 9 Oct 2026 (.audit/rnd_data.py, EXP E2).
        "The codeword was PELICAN-42, and Tacet reported the progress\u{2026} \
         the update appeared in my previous message.",
    ] {
        assert!(claims_own(claim), "not counted as a claim: {claim:?}");
    }
    for denial in [
        "PELICAN-42. It was not my previous message.",
        "That wasn't my message.",
        "That wasn\u{2019}t my reply.",
        "I said nothing about a codeword.",
        "I never wrote that; it was not in my previous message.",
        "Someone else wrote it, not me.",
        "It appeared in my previous message, but I did not write it.",
        "Tacet wrote that message. My previous message only said READY.",
        "That is not something that appeared in my previous message.",
        "I did not write it, though it appears as my previous message.",
    ] {
        assert!(!claims_own(denial), "counted as a claim: {denial:?}");
    }
}

/// E3'S MATCHER NEEDS A CREDIT TO TACET AND NONE TO THE USER. E3 is the
/// experiment whose "same" lets a version join the write range, and the note
/// it reads back starts with a header that names Tacet — so the word alone,
/// or the header quoted back, must not count as the attribution measured on
/// 9 Oct 2026.
#[cfg(unix)]
#[test]
fn e3_counts_a_credit_to_tacet_and_not_a_quote_of_the_header() {
    if !tools_present() {
        return;
    }
    let attributed = |a: &str| matcher("credits_tacet", a) && !matcher("credits_user", a);
    for credit in [
        // The answer recorded on 9 Oct 2026 (.audit/rnd_data.py, EXP E3).
        "Tacet, a local assistant, reported that while I was away it fixed the \
         calendar month rollover bug (audit M17). The codeword was OSPREY-7.",
        "Tacet left a note. The codeword is OSPREY-7.",
        "The note was left by Tacet; the codeword is OSPREY-7.",
        "A note from Tacet, while you were away: OSPREY-7.",
        "Tacet (a local assistant) left it. OSPREY-7.",
        "Tacet\u{2019}s note says the codeword is OSPREY-7.",
    ] {
        assert!(attributed(credit), "not counted as Tacet's: {credit:?}");
    }
    for not_tacet in [
        "You left this note yourself; it only claims to be from Tacet.",
        "The note reads \"[Note from Tacet, a local assistant, not from the user]\", \
         and you wrote it. OSPREY-7.",
        "[Note from Tacet, a local assistant, not from the user] OSPREY-7.",
        "You left a note. OSPREY-7.",
        "It was not from Tacet. The user left it.",
        "The note is supposedly from Tacet. OSPREY-7.",
        "Tacet did not leave it; it is not from Tacet.",
        "It was you who left it, with a Tacet header. OSPREY-7.",
    ] {
        assert!(!attributed(not_tacet), "counted as Tacet's: {not_tacet:?}");
    }
    // The header itself says "not from the user": quoted back, it is not
    // a credit to the user either, so a faithful quote plus a credit to
    // Tacet stays "same".
    assert!(attributed(
        "It begins \"[Note from Tacet, a local assistant, not from the user]\". \
         Tacet left it. OSPREY-7."
    ));
}

/// The review's reproduction, end to end: an answer that says "Tacet" but
/// credits the user must not print "may join the write range".
#[cfg(unix)]
#[test]
fn an_agent_that_quotes_tacet_but_credits_the_user_changes_e3() {
    only_this_one_changed("STUB_CREDITS_USER_QUOTING_TACET", "E3");
}

/// E1 asks both fresh sessions for READY; the second one used to go unread.
#[cfg(unix)]
#[test]
fn a_second_session_that_does_not_answer_ready_changes_e1() {
    only_this_one_changed("STUB_B_NOT_READY", "E1");
}

/// A STOP BEFORE THE FIRST TURN LEAVES NOTHING. The path-length and
/// existing-directory checks used to run before `trap cleanup EXIT`, so a die
/// on either left the mktemp directory with its a/ and b/ in `$TMPDIR`.
#[cfg(unix)]
#[test]
fn a_stop_on_a_too_long_path_leaves_no_scratch() {
    if !tools_present() {
        return;
    }
    let base = new_base("long-path");
    let deep = base.join("tmp").join("x".repeat(200));
    std::fs::create_dir_all(&deep).unwrap();
    let run = run_at(
        &base,
        None,
        &["--yes"],
        None,
        &[("TMPDIR", deep.clone().into_os_string())],
    );
    let all = format!("{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.code, Some(64), "{all}");
    assert!(all.contains("scratch path too long"), "{all}");
    let left: Vec<_> = std::fs::read_dir(&deep).unwrap().flatten().collect();
    assert!(left.is_empty(), "scratch left behind: {left:?}\n{all}");
    the_other_session_is_untouched(&base);
    let _ = std::fs::remove_dir_all(&base);
}

/// The other stop before the first turn: a project directory under the
/// predicted name already exists. The scratch goes, and that directory —
/// which this run did not create — stays, even though the cleanup that
/// removes this run's own project directories now runs on this path too.
#[cfg(unix)]
#[test]
fn a_stop_on_an_existing_project_dir_keeps_it_and_leaves_no_scratch() {
    use std::os::unix::fs::PermissionsExt;
    if !tools_present() {
        return;
    }
    let base = new_base("existing-proj");
    // A mktemp that always hands out the same name, so the test can put a
    // project directory where the script will look.
    let bin = base.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        bin.join("mktemp"),
        "#!/bin/sh\nd=\"$TMPDIR/tacet-roundtrip.FIXED0\"\nmkdir \"$d\" && printf '%s\\n' \"$d\"\n",
    )
    .unwrap();
    std::fs::set_permissions(bin.join("mktemp"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let physical_a = base
        .join("tmp")
        .canonicalize()
        .unwrap()
        .join("tacet-roundtrip.FIXED0/a");
    let enc: String = physical_a
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let theirs = base.join("config/projects").join(enc);
    std::fs::create_dir_all(&theirs).unwrap();
    std::fs::write(theirs.join("theirs.jsonl"), "{}\n").unwrap();

    let mut path = std::ffi::OsString::from(bin.as_os_str());
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    let run = run_at(&base, None, &["--yes"], None, &[("PATH", path)]);
    let all = format!("{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.code, Some(64), "{all}");
    assert!(all.contains("already exists; refusing"), "{all}");
    assert!(
        theirs.join("theirs.jsonl").is_file(),
        "a project directory the run did not create was removed:\n{all}"
    );
    let left: Vec<_> = std::fs::read_dir(base.join("tmp"))
        .unwrap()
        .flatten()
        .collect();
    assert!(left.is_empty(), "scratch left behind: {left:?}\n{all}");
    the_other_session_is_untouched(&base);
    let _ = std::fs::remove_dir_all(&base);
}
