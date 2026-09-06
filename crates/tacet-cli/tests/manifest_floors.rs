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
