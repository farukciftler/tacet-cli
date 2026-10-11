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
//! `[A-Za-z0-9]` becomes `-`), and a Claude Code session's file and
//! directory are named by the `sessionId` its lines record, held to
//! `validate_id`'s shape — the input path the person named only has to AGREE
//! with it. A subagent's `agent-…` name is held the same way to the `agentId`
//! its lines record, when they record one. The rest comes from the input
//! path, each part held to what its reader accepts: a rollout's name to
//! `rollout-YYYY-MM-DDT…`, an Antigravity root to the roots the reader knows,
//! its conversation id to `validate_id`, and the file under that id to the
//! names the Antigravity reader opens.
//!
//! It reads nothing and writes nothing: `corpus_capture --into` does both,
//! and only on a file the person chose.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use super::antigravity::{ARTIFACTS, ROOTS, reads_in_brain};
use super::claude::project_dir_name;
use super::codex::rollout_id;
use super::model::{AgentKind, validate_id};
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
/// file does not record (an Antigravity conversation's root and id, a Codex
/// rollout's name, a Claude Code subagent's `agent-…` name when its lines
/// record no `agentId`), and for a Claude Code transcript it must agree with
/// the `sessionId` (and a subagent's with the `agentId`) the file records. `captured` is the
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
        // The id and the names under it are the only components of this path
        // the file cannot vouch for, so each is held to what the reader
        // accepts. Unchecked, both were filed from the capturer's disk —
        // measured against ba7ece2: `…/brain/zeynepk private notes/task.md`
        // went to `antigravity/2.5.5/antigravity-ide/brain/zeynepk private notes/task.md`,
        // a directory `AntigravitySource` never lists (it holds ids to
        // `validate_id`), and `brain/<id>/zeynepk-diary.md` was filed verbatim.
        let (id, below) = at.rest.split_first().ok_or("no conversation id")?;
        validate_id(id).map_err(|e| format!("{name}: its conversation id is not an id: {e}"))?;
        if !reads_in_brain(below) {
            return Err(format!(
                "{}: the Antigravity reader opens task.md, {}, their `.metadata.json` and \
                 `.system_generated/logs/` transcripts under `brain/<id>/`, and nothing else",
                below.join("/"),
                ARTIFACTS.join(", ")
            ));
        }
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
    if let Some(cli) = only(&records.codex_versions, name)? {
        let cli = agreed(dotted(cli)?, version)?;
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
    if let Some(recorded) = only(&records.claude_versions, name)? {
        let recorded = agreed(dotted(recorded)?, version)?;
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
        let session = one_session(&records.sessions, name)?;
        let mut out = PathBuf::from(AgentKind::ClaudeCode.slug());
        out.push(recorded);
        out.push("projects");
        out.push(project);
        // Both names below are built from the recorded id, and the input path
        // is only required to agree with it. Taking them from the path instead
        // filed whatever directory the file happened to sit in — measured
        // against f7001e9: `-Users-zeynepk-acme-billing/subagents/agent-a1.jsonl`
        // went to `projects/-Users-dev-x/-Users-zeynepk-acme-billing/subagents/…`,
        // the unmasked home committed after all, and a main transcript saved as
        // `zeynepk-acme.jsonl` was filed under that name, where
        // `ClaudeSource::read(root, <its id>)` never finds it.
        let parent = input.parent();
        if parent.and_then(Path::file_name).and_then(|n| n.to_str()) == Some("subagents") {
            // A subagent's lines record its PARENT's sessionId (the fixture
            // `agent-a1.jsonl` records 11111111-…), and it lives in that
            // session's directory: `<id>/subagents/agent-*.jsonl` beside `<id>.jsonl`.
            let dir = parent
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|n| n.to_str());
            if dir != Some(session) {
                return Err(format!(
                    "{name}: a subagent file belongs in its session's directory `{session}/subagents/`, \
                     and this one is not in it"
                ));
            }
            let stem = name.strip_suffix(".jsonl").unwrap_or(name);
            if !stem.starts_with("agent-") || validate_id(stem).is_err() {
                return Err(format!(
                    "{name}: a subagent transcript is named `agent-<id>.jsonl`"
                ));
            }
            // Claude Code records the subagent's own id as `agentId` on its
            // lines, and names the file `agent-<agentId>.jsonl`; when the
            // file records one, the name is built from it and the path only
            // has to agree — as the session's names are above. Left to the
            // path, `agent-zeynepk-private.jsonl` was filed under that name
            // (measured against ba7ece2). The fixture `agent-a1.jsonl` records
            // none, and only then is the name the path's, held to the reader's
            // `agent-*` and to an id's shape.
            let filed = match one_agent(&records.agents, name)? {
                Some(agent) => {
                    let filed = format!("agent-{agent}.jsonl");
                    if name != filed {
                        return Err(format!(
                            "{name}: the file records agent {agent}, and a subagent transcript \
                             is named after it (`{filed}`)"
                        ));
                    }
                    filed
                }
                None => name.to_string(),
            };
            out.push(session);
            out.push("subagents");
            out.push(filed);
        } else {
            let filed = format!("{session}.jsonl");
            if name != filed {
                return Err(format!(
                    "{name}: the file records session {session}, and a transcript is named \
                     after its session (`{filed}`) — the reader finds it by that name alone"
                ));
            }
            out.push(filed);
        }
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

/// The one version the file records, or `None` if it records none.
///
/// MORE THAN ONE IS REFUSED, not settled by the first line. A transcript
/// resumed after the agent updated itself carries both versions, and
/// `every_file_records_the_version_of_its_directory` holds EVERY line against
/// the directory — so a first-wins pick passed here and broke that test only
/// after the file was committed (measured: 2.1.285 then 2.1.293 was filed
/// under 2.1.285). Such a file vouches for neither version on its own.
fn only<'a>(versions: &'a BTreeSet<String>, name: &str) -> Result<Option<&'a str>, String> {
    let mut each = versions.iter();
    match (each.next(), each.next()) {
        (Some(one), None) => Ok(Some(one)),
        (None, _) => Ok(None),
        (Some(a), Some(b)) => Err(format!(
            "{name}: the file records versions {a} and {b}; a session resumed across an \
             update vouches for neither alone — capture one recorded under a single version"
        )),
    }
}

/// The one session the file records, held to the shape of an id.
///
/// TWO ARE REFUSED, as two versions are by `only`: a file whose lines name
/// two sessions belongs under neither name, and filing it under one would
/// hand the reader the other's lines when it asks for that id.
fn one_session<'a>(sessions: &'a BTreeSet<String>, name: &str) -> Result<&'a str, String> {
    let mut each = sessions.iter();
    match (each.next(), each.next()) {
        (Some(one), None) => validate_id(one)
            .map(|()| one.as_str())
            .map_err(|e| format!("{name}: its sessionId is not an id: {e}")),
        (None, _) => Err(format!("{name}: no line records a sessionId")),
        (Some(a), Some(b)) => Err(format!(
            "{name}: the file records sessions {a} and {b}; it belongs under neither alone"
        )),
    }
}

/// The one subagent id the file records, if any, held to an id's shape. Two
/// are refused for the reason two sessions are: the file is neither agent's.
fn one_agent<'a>(agents: &'a BTreeSet<String>, name: &str) -> Result<Option<&'a str>, String> {
    let mut each = agents.iter();
    match (each.next(), each.next()) {
        (Some(one), None) => validate_id(one)
            .map(|()| Some(one.as_str()))
            .map_err(|e| format!("{name}: its agentId is not an id: {e}")),
        (None, _) => Ok(None),
        (Some(a), Some(b)) => Err(format!(
            "{name}: the file records agents {a} and {b}; it belongs under neither alone"
        )),
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
    /// Every distinct one, not the first: see `only`.
    claude_versions: BTreeSet<String>,
    codex_versions: BTreeSet<String>,
    /// Every distinct `sessionId`: see `one_session`.
    sessions: BTreeSet<String>,
    /// Every distinct top-level `agentId`: see `one_agent`.
    agents: BTreeSet<String>,
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
                r.codex_versions.insert(cli.to_string());
            }
            // A Claude Code line carries its session id beside the version; a
            // `version` key alone (inside some tool's data) is not one.
            if let Some(session) = v.get("sessionId").and_then(|s| s.as_str()) {
                r.sessions.insert(session.to_string());
                if let Some(agent) = v.get("agentId").and_then(|s| s.as_str()) {
                    r.agents.insert(agent.to_string());
                }
                if let Some(ver) = v.get("version").and_then(|s| s.as_str()) {
                    r.claude_versions.insert(ver.to_string());
                }
                // `cwd: ""` is no working directory. It encodes to an empty
                // name, which pushes nothing, and the file landed directly in
                // `projects/` — measured: `claude/2.1.285/projects/c1.jsonl` —
                // where the reader never looks (it lists `projects/<dir>/<id>.jsonl`).
                // Any non-empty cwd encodes to a non-empty name, so this is the
                // one place the rule needs to live.
                if let Some(cwd) = v
                    .get("cwd")
                    .and_then(|s| s.as_str())
                    .filter(|c| !c.is_empty())
                {
                    r.cwd.get_or_insert_with(|| cwd.to_string());
                }
            }
        }
        r
    }
}
