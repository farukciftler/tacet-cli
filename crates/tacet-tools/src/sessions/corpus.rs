//! Where a captured session file goes in the fixture corpus (RG-28).
//!
//! WHY THIS IS CODE AND NOT A PARAGRAPH. The corpus is read by pointing each
//! reader at `corpus/<agent>/<version>/`, and every reader finds a session by
//! where its file sits: Claude Code by `projects/<encoded cwd>/<id>.jsonl`,
//! Codex by `sessions/YYYY/MM/DD/rollout-…`, Antigravity by
//! `<root>/brain/<id>/…`. The capture steps used to leave that layout to the
//! person running them, and the two easy mistakes are silent: a transcript
//! filed under its ORIGINAL project directory still reads by id (and that
//! directory name is the unmasked home path, committed), and one filed under
//! the wrong version directory reads fine and makes the corpus vouch for a
//! version nobody captured. `place` derives the path from what the captured
//! file itself records, so neither can be typed wrong.
//!
//! WHAT IT TRUSTS. Nothing in the file is used as a path until it has been
//! checked to be the shape it claims: a version must parse as dotted numbers
//! (a `version` of `../../x` is refused, not joined), a working directory is
//! only ever used through Claude Code's own encoding (every character outside
//! `[A-Za-z0-9]` becomes `-`), and the remaining components come from the
//! input path the person named, never from the file.
//!
//! It reads nothing and writes nothing: `corpus_capture --into` does both,
//! and only on a file the person chose.

use std::path::{Component, Path, PathBuf};

use super::antigravity::ROOTS;
use super::claude::project_dir_name;
use super::codex::rollout_id;
use super::model::AgentKind;
use super::version::Version;

/// Claude Code cuts an encoded project directory at 200 characters and
/// appends a hash of the runtime it was built with; that hash is not
/// recomputed here (see `claude::project_dir_matches`), so a longer path
/// cannot be placed and is refused rather than filed under a name the reader
/// would not match.
const MAX_PROJECT_DIR: usize = 200;

/// The path, relative to the corpus root, that `captured` belongs at.
///
/// `input` is the path the file was captured FROM: it supplies only what the
/// file does not record (an Antigravity conversation's root and id, a Claude
/// Code subagent's session directory, the file's own name). `captured` is the
/// anonymised text — the working directory in it is the masked one, which is
/// the one the project directory must encode. `version` is required for
/// Antigravity, whose files record none, and otherwise must agree with the
/// version the file records.
pub fn place(input: &Path, captured: &str, version: Option<&str>) -> Result<PathBuf, String> {
    let name = input
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{}: no file name", input.display()))?;
    if let Some(at) = antigravity_parts(input) {
        let version = version.ok_or(
            "an Antigravity file records no version: pass the product's version with --version",
        )?;
        let version = dotted(version)?;
        let mut out = PathBuf::from(AgentKind::Antigravity.slug());
        out.push(version);
        out.push(at.root);
        out.push("brain");
        for part in at.rest {
            out.push(part);
        }
        return Ok(out);
    }
    if !name.ends_with(".jsonl") {
        return Err(format!(
            "{name}: only a transcript or a rollout records where it belongs; give the output \
             path yourself"
        ));
    }
    let records = Records::of(captured);
    if let Some(cli) = records.codex_version {
        let cli = agreed(dotted(&cli)?, version)?;
        let (_, compressed) =
            rollout_id(name).ok_or_else(|| format!("{name} is not named like a Codex rollout"))?;
        if compressed {
            return Err(format!("{name} is compressed"));
        }
        // `rollout_id` has already held the name to `rollout-YYYY-MM-DDT…`.
        let (y, m, d) = (&name[8..12], &name[13..15], &name[16..18]);
        let mut out = PathBuf::from(AgentKind::Codex.slug());
        for part in [cli.as_str(), "sessions", y, m, d, name] {
            out.push(part);
        }
        return Ok(out);
    }
    if let Some(recorded) = records.claude_version {
        let recorded = agreed(dotted(&recorded)?, version)?;
        let cwd = records
            .cwd
            .ok_or_else(|| format!("{name}: no line records a working directory"))?;
        let project = project_dir_name(&cwd);
        if project.len() > MAX_PROJECT_DIR {
            return Err(format!(
                "{name}: its working directory encodes to {} characters, past the {MAX_PROJECT_DIR} \
                 at which the agent cuts the name and appends a hash; file it by hand",
                project.len()
            ));
        }
        let mut out = PathBuf::from(AgentKind::ClaudeCode.slug());
        out.push(recorded);
        out.push("projects");
        out.push(project);
        // A subagent's transcript lives in its session's directory:
        // `<id>/subagents/agent-*.jsonl` beside `<id>.jsonl`.
        let parent = input.parent();
        if parent.and_then(Path::file_name).and_then(|n| n.to_str()) == Some("subagents") {
            let session = parent
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("{name}: a subagent file outside a session directory"))?;
            out.push(session);
            out.push("subagents");
        }
        out.push(name);
        return Ok(out);
    }
    Err(format!(
        "{name}: no line records a Claude Code `version` or a Codex `cli_version`, so its corpus \
         directory cannot be told"
    ))
}

/// `version` if it is dotted numbers and nothing else. `Version::parse` trims,
/// so the untrimmed text is compared too: " 2.1.0" is a directory name with a
/// space in it, not a version.
fn dotted(version: &str) -> Result<String, String> {
    if version == version.trim() && Version::parse(version).is_some() {
        Ok(version.to_string())
    } else {
        Err(format!("{version:?} is not a version (dotted numbers)"))
    }
}

/// The recorded version, unless the person said another one: a file filed
/// under a version it does not record makes `every_file_records_the_version_of_its_directory`
/// fail later, and is better refused now.
fn agreed(recorded: String, asked: Option<&str>) -> Result<String, String> {
    match asked {
        Some(asked) if asked != recorded => {
            Err(format!("the file records version {recorded}, not {asked}"))
        }
        _ => Ok(recorded),
    }
}

struct AntigravityParts<'a> {
    root: &'a str,
    /// `<id>/…`: everything after `brain/`.
    rest: Vec<&'a str>,
}

/// `…/<root>/brain/<id>/<file…>` where `<root>` is one of the data roots the
/// reader knows. Matched from the END, so a home directory that happens to be
/// called `brain` does not decide where the file goes.
fn antigravity_parts(input: &Path) -> Option<AntigravityParts<'_>> {
    // A `..` would make "the component before `brain`" mean something other
    // than the directory the file is in.
    if input.components().any(|c| c == Component::ParentDir) {
        return None;
    }
    let parts: Vec<&str> = input
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    let at = (1..parts.len())
        .rev()
        .find(|&i| parts[i] == "brain" && ROOTS.iter().any(|(root, _)| *root == parts[i - 1]))?;
    // An id and a file under it, at least.
    (parts.len() >= at + 3).then(|| AntigravityParts {
        root: parts[at - 1],
        rest: parts[at + 1..].to_vec(),
    })
}

/// What a captured `.jsonl` records about where it belongs.
#[derive(Default)]
struct Records {
    claude_version: Option<String>,
    codex_version: Option<String>,
    cwd: Option<String>,
}

impl Records {
    fn of(text: &str) -> Self {
        let mut r = Records::default();
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if v.get("type").and_then(|t| t.as_str()) == Some("session_meta")
                && let Some(cli) = v.pointer("/payload/cli_version").and_then(|c| c.as_str())
            {
                r.codex_version.get_or_insert_with(|| cli.to_string());
            }
            // A Claude Code line carries its session id beside the version; a
            // `version` key alone (inside some tool's data) is not one.
            if v.get("sessionId").and_then(|s| s.as_str()).is_some() {
                if let Some(ver) = v.get("version").and_then(|s| s.as_str()) {
                    r.claude_version.get_or_insert_with(|| ver.to_string());
                }
                if let Some(cwd) = v.get("cwd").and_then(|s| s.as_str()) {
                    r.cwd.get_or_insert_with(|| cwd.to_string());
                }
            }
        }
        r
    }
}
