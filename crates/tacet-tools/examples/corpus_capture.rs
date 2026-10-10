//! Anonymises one file of a real agent session into a corpus fixture (RG-28).
//!
//! ```text
//! cargo run -p tacet-tools --example corpus_capture -- [--salt <n>] <input> <output>
//! cargo run -p tacet-tools --example corpus_capture -- [--salt <n>] [--version <v>] \
//!     --into <corpus> <input>
//! ```
//!
//! With `--into`, the output path is not typed: `sessions::corpus::place`
//! derives `<agent>/<version>/…` under `<corpus>` from what the captured file
//! records (and, for Antigravity, which records no version, from `--version`).
//! Typed by hand, the two easy mistakes were silent — a transcript left under
//! its original, unmasked project directory still reads by id, and one under
//! the wrong version directory makes the corpus vouch for a version nobody
//! captured.
//!
//! `<input>` is a `.jsonl` transcript or rollout, a `.json` (Antigravity's
//! `.metadata.json`, Claude Code's `tasks/<list>/<n>.json`) or a `.md`
//! artifact. It is opened through the same gate the readers use, so a
//! credential (`auth.json`, `.credentials.json`, a link to one) is refused
//! here exactly as it is there. A `.db` or a `.jsonl.zst` is refused: one is
//! protobuf the anonymiser cannot see into, the other compressed.
//!
//! A RUN NEVER READS ANYTHING BY ITSELF. It is a person's command, aimed at a
//! file they chose, with their consent to copy it; no test runs it. What it
//! writes is still to be read before it is committed (corpus README, step 2):
//! masking is mechanical, and a value under a kept key that is shaped like an
//! identifier (`"source":"AcmeCorp"`) can still say more than it should.
//!
//! One capture, one salt: the files of one session (a transcript and its
//! `task.md`) must share it, or the same path is masked two ways in them. The
//! first run prints the salt it drew; pass it to the next with `--salt`, and
//! do not write it down anywhere that is committed — with it, the masking can
//! be reversed by guessing words.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tacet_tools::sessions::anonymise::Scrub;
use tacet_tools::sessions::claude::project_dir_name;
use tacet_tools::sessions::corpus;
use tacet_tools::sessions::secrets;

/// A Claude Code transcript was measured up to tens of MB, an Antigravity log
/// up to 65 MB; the readers' own cap is 128 MB.
const CAP: u64 = 128 << 20;

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("corpus_capture: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut salt: Option<u64> = None;
    let mut into: Option<PathBuf> = None;
    let mut version: Option<String> = None;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--salt" => {
                let v = it.next().ok_or("--salt needs a number")?;
                salt = Some(
                    v.parse()
                        .map_err(|_| format!("--salt {v:?} is not a number"))?,
                );
            }
            "--into" => into = Some(PathBuf::from(it.next().ok_or("--into needs a directory")?)),
            "--version" => version = Some(it.next().ok_or("--version needs a version")?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => paths.push(PathBuf::from(a)),
        }
    }
    let (input, output) = match (into.as_ref(), paths.len()) {
        (Some(_), 1) => (paths.remove(0), None),
        (None, 2) => {
            let output = paths.remove(1);
            (paths.remove(0), Some(output))
        }
        _ => return Err(format!("usage: {USAGE}")),
    };
    if let Some(output) = &output
        && output.exists()
    {
        return Err(format!("{} exists; not overwriting it", output.display()));
    }
    let name = input
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let kind = kind_of(&name)?;
    let text = secrets::read_text(&input, CAP).map_err(|e| e.to_string())?;

    let salt = salt.unwrap_or_else(|| {
        let s = Scrub::fresh_salt();
        eprintln!(
            "salt {s} — pass `--salt {s}` for the other files of this session; commit it nowhere"
        );
        s
    });
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok();
    let scrub = Scrub::new(salt, user.as_deref());
    let out = match kind {
        Kind::Jsonl => scrub.jsonl(&text),
        Kind::Json => scrub.json(&text),
        Kind::Markdown => scrub.markdown(&text),
    };
    // Placed from the CAPTURED text: the project directory must encode the
    // masked working directory, never the one the input was filed under.
    let output = match (output, into) {
        (Some(output), _) => output,
        (None, Some(into)) => into.join(corpus::place(&input, &out, version.as_deref())?),
        (None, None) => return Err(format!("usage: {USAGE}")),
    };
    if output.exists() {
        return Err(format!("{} exists; not overwriting it", output.display()));
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&output, &out).map_err(|e| format!("{}: {e}", output.display()))?;
    eprintln!("wrote {}", output.display());
    if let Some(cwd) = first_cwd(&out) {
        eprintln!(
            "its working directory is now {cwd}; a Claude Code transcript belongs under projects/{}/",
            project_dir_name(&cwd)
        );
    }
    eprintln!(
        "read it before committing it, add a Case for it in tests/sessions_corpus.rs, and run \
         `cargo test -p tacet-tools --test sessions_corpus`"
    );
    Ok(())
}

const USAGE: &str = "corpus_capture [--salt <n>] <input> <output>\n       \
                     corpus_capture [--salt <n>] [--version <v>] --into <corpus> <input>";

enum Kind {
    Jsonl,
    Json,
    Markdown,
}

fn kind_of(name: &str) -> Result<Kind, String> {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    match ext {
        "jsonl" => Ok(Kind::Jsonl),
        "json" => Ok(Kind::Json),
        "md" => Ok(Kind::Markdown),
        "zst" => Err(format!(
            "{name} is compressed; run `zstd -d` on a copy first"
        )),
        "db" => Err(format!(
            "{name} is Antigravity's database (protobuf in SQLite): its text cannot be masked, \
             and the readers never open it — leave it out of the corpus"
        )),
        _ => Err(format!("{name}: only .jsonl, .json and .md are captured")),
    }
}

/// The first `cwd` a captured transcript records, already masked.
fn first_cwd(jsonl: &str) -> Option<String> {
    jsonl.lines().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        v.get("cwd")
            .or_else(|| v.get("payload").and_then(|p| p.get("cwd")))
            .and_then(|c| c.as_str())
            .map(str::to_string)
    })
}
