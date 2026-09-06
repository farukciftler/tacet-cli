//! A DEPENDENCY FLOOR THAT IS LOWER THAN THE CRATE IT NAMES IS A SILENT DOWNGRADE.
//!
//! `[workspace.dependencies]` declares each in-tree crate twice — a `path`, which
//! is what a build in this checkout uses, and a `version`, which is the ONLY
//! thing a crates.io consumer sees. Inside the workspace the two never disagree
//! visibly: cargo takes the path and the version is decoration. Outside it, the
//! version is the whole contract.
//!
//! So a floor left behind after a member is bumped is invisible here and load
//! bearing there. `tacet-memory` was published at 0.1.1 while the workspace still
//! declared `version = "0.1.0"`, which means `cargo add tacet-tools` could
//! resolve a memory crate one release old and nothing in this repository would
//! have said so. The manifest comments call these numbers FLOORS, not
//! preferences — nineteen of them record a fix that a lower resolution would
//! quietly undo — and a floor that is not the current version is not a floor.
//!
//! This is a class of bug, not an incident: it recurs every single time anyone
//! bumps a member, and it recurs SILENTLY. Hence a test rather than a note in
//! CONTRIBUTING.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf()
}

/// The `version = "..."` of the first `[package]`-level key in a member manifest,
/// or `None` when the member inherits it from the workspace.
fn member_version(manifest: &str) -> Option<String> {
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with("version.workspace") {
            return None;
        }
        if let Some(rest) = line.strip_prefix("version = \"") {
            return rest.split('"').next().map(str::to_string);
        }
    }
    panic!("a member manifest with no version at all");
}

/// `name = { path = "...", version = "X" }` out of `[workspace.dependencies]`.
fn declared_floors(root_manifest: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in root_manifest.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("tacet-") else {
            continue;
        };
        let Some((name, tail)) = rest.split_once(' ') else {
            continue;
        };
        if !tail.contains("path = ") {
            continue;
        }
        let Some(version) = tail.split("version = \"").nth(1) else {
            continue;
        };
        let Some(version) = version.split('"').next() else {
            continue;
        };
        out.push((format!("tacet-{name}"), version.to_string()));
    }
    out
}

#[test]
fn every_declared_floor_is_the_version_the_member_actually_has() {
    let root = repo_root();
    let root_manifest =
        std::fs::read_to_string(root.join("Cargo.toml")).expect("the workspace manifest");
    let workspace_version = root_manifest
        .split("[workspace.package]")
        .nth(1)
        .and_then(|s| s.split("\nversion = \"").nth(1))
        .and_then(|s| s.split('"').next())
        .expect("[workspace.package] declares a version")
        .to_string();

    let floors = declared_floors(&root_manifest);
    assert!(
        floors.len() >= 9,
        "only {} in-tree floors found — the parser has stopped parsing, which \
         would make this test pass while checking nothing",
        floors.len()
    );

    for (name, floor) in &floors {
        let manifest = std::fs::read_to_string(root.join("crates").join(name).join("Cargo.toml"))
            .unwrap_or_else(|e| panic!("{name} is declared but has no manifest: {e}"));
        let actual = member_version(&manifest).unwrap_or_else(|| workspace_version.clone());
        assert_eq!(
            floor, &actual,
            "`{name}` is at {actual} but `[workspace.dependencies]` declares the \
             floor as {floor}. Inside this checkout cargo takes the path and \
             nothing breaks; a crates.io consumer resolves {floor} and silently \
             misses everything since."
        );
    }
}

/// AND EVERY PUBLISHED CRATE CARRIES THE LICENCE TEXT, not just the field.
///
/// `license = "MIT"` in the manifest is metadata. Cargo packages only what is
/// inside the crate directory, so without a `LICENSE` file beside each
/// `Cargo.toml` the tarball on crates.io asserts a licence it does not contain —
/// which is exactly the thing a downstream legal review looks for and does not
/// find.
#[test]
fn every_member_ships_the_licence_text() {
    let root = repo_root();
    let expected = std::fs::read_to_string(root.join("LICENSE")).expect("the root LICENSE");
    let mut checked = 0;
    for entry in std::fs::read_dir(root.join("crates")).expect("crates/") {
        let dir = entry.expect("a directory entry").path();
        if !dir.join("Cargo.toml").exists() {
            continue;
        }
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let licence = std::fs::read_to_string(dir.join("LICENSE")).unwrap_or_else(|_| {
            panic!(
                "{name} has no LICENSE file, so its published tarball claims MIT \
                 without carrying the text"
            )
        });
        assert_eq!(
            licence.trim(),
            expected.trim(),
            "{name}'s LICENSE differs from the repository's"
        );
        checked += 1;
    }
    assert!(checked >= 11, "only {checked} members walked");
}

/// THE RULES CRATES.IO ENFORCES AND CARGO DOES NOT.
///
/// `cargo package`, `cargo publish --dry-run` and every local check pass on a
/// manifest the registry will reject, because these limits live on the server.
/// The failure therefore arrives at the UPLOAD — the one step that is not
/// reversible and the one place a chain of eleven crates is half-way through.
///
/// It happened: `tacet-mcp` carried the keyword `model-context-protocol`, which
/// is 22 characters against a limit of 20. Ten crates had already been published
/// when the eleventh was refused, and `tacet-tools`, `tacet-eval` and
/// `tacet-cli` then failed to resolve because they depend on it.
///
/// The limits below are crates.io's: at most 5 keywords, each at most 20
/// characters, each starting alphanumeric and otherwise `[A-Za-z0-9_-]`; a
/// description is required; `license` and `repository` are what the page needs
/// to be readable at all.
#[test]
fn the_registry_metadata_rules_are_satisfied() {
    let root = repo_root();
    let mut checked = 0;
    for entry in std::fs::read_dir(root.join("crates")).expect("crates/") {
        let dir = entry.expect("a directory entry").path();
        let manifest_path = dir.join("Cargo.toml");
        if !manifest_path.exists() {
            continue;
        }
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let manifest = std::fs::read_to_string(&manifest_path).expect("a manifest");

        let keywords: Vec<String> = manifest
            .lines()
            .find(|l| l.trim_start().starts_with("keywords"))
            .and_then(|l| l.split_once('['))
            .and_then(|(_, rest)| rest.split_once(']'))
            .map(|(inner, _)| {
                inner
                    .split(',')
                    .map(|k| k.trim().trim_matches('"').to_string())
                    .filter(|k| !k.is_empty())
                    .collect()
            })
            .unwrap_or_else(|| panic!("{name} declares no keywords"));

        assert!(
            keywords.len() <= 5,
            "{name} declares {} keywords; crates.io allows 5",
            keywords.len()
        );
        for keyword in &keywords {
            assert!(
                keyword.chars().count() <= 20,
                "{name}: keyword `{keyword}` is {} characters; crates.io allows 20 \
                 and refuses the UPLOAD, not the package",
                keyword.chars().count()
            );
            assert!(
                keyword
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                    && keyword
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "{name}: keyword `{keyword}` is not [A-Za-z0-9_-] starting alphanumeric"
            );
        }
        for required in ["description", "license", "repository"] {
            assert!(
                manifest
                    .lines()
                    .any(|l| l.trim_start().starts_with(required)),
                "{name} declares no `{required}`, so its crates.io page is unreadable"
            );
        }
        checked += 1;
    }
    assert!(checked >= 11, "only {checked} manifests walked");
}

/// A CHECKED-IN BASELINE WITH NO WALL TIME CANNOT BE COMPARED ON TIME.
///
/// `run_selection_in` measures the run and puts it in the report, so a baseline
/// written by the tool always has one. `qwen3-4b-both.json` carries
/// `wall_ms: 0`, which means that file did not come out of the tool unedited —
/// and the cost lands on the README, where "44 min" on Metal and "6.4 min" on a
/// 3090 are hand-recorded with no artifact behind them. Two paragraphs of that
/// page disagreed by 0.2 min about the same run for exactly this reason.
///
/// THE LIST IS ASSERTED IN BOTH DIRECTIONS rather than used as a mute button: a
/// file that gains a real wall time must be taken out of it, or this fails. That
/// is what stops a grandfather clause from becoming permanent.
#[test]
fn a_baseline_carries_the_time_it_took() {
    /// Known to be missing it, with the reason. Shrink this list; never grow it.
    ///
    /// IT IS EMPTY, AND THE DAY IT EMPTIED IS THE POINT. `qwen3-4b-both.json`
    /// was in here with "re-deriving it means a 44-minute run, which is a NEW
    /// measurement rather than a repair". That run happened on 6 Sep 2026, the
    /// baseline was replaced, and this test failed — telling whoever did it to
    /// take the entry out. A one-directional guard would have stayed green and
    /// left the excuse standing forever.
    const MISSING: [&str; 0] = [];

    let dir = repo_root().join("crates/tacet-eval/baselines");
    let mut found_missing: Vec<String> = Vec::new();
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("baselines/") {
        let path = entry.expect("a directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let text = std::fs::read_to_string(&path).expect("a baseline is readable");
        // The logic-set baseline has no `wall_ms` at all: it measures no model
        // and has no wall time worth recording. Only a file that HAS the field
        // is making the claim.
        let Some(after) = text.split(r#""wall_ms":"#).nth(1) else {
            continue;
        };
        checked += 1;
        let value: u128 = after
            .trim_start()
            .chars()
            .take_while(|c: &char| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        if value == 0 {
            found_missing.push(name);
        }
    }
    found_missing.sort();
    let mut expected: Vec<String> = MISSING.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(
        found_missing, expected,
        "the baselines missing a wall time are not the ones recorded as missing \
         it. If a file gained one, take it out of MISSING; if a new file has \
         none, it was not written by `eval --tool-selection --json`."
    );
    assert!(checked > 0, "no baseline carries a `wall_ms` field at all");
}
