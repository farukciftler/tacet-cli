//! Anonymises one file of a real agent session into a corpus fixture (RG-28).
//!
//! ```text
//! cargo run -p tacet-tools --example corpus_capture -- [--salt <n>] <input> <output>
//! ```
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
            "-h" | "--help" => {
                println!("usage: corpus_capture [--salt <n>] <input> <output>");
                return Ok(());
            }
            _ => paths.push(PathBuf::from(a)),
        }
    }
    let [input, output] = <[PathBuf; 2]>::try_from(paths)
        .map_err(|_| "usage: corpus_capture [--salt <n>] <input> <output>".to_string())?;
    if output.exists() {
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
