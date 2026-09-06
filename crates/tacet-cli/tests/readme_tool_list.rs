//! The README's tool line must name every tool a default install actually has.
//!
//! WHY THIS TEST EXISTS. `search_filter` and `message_intent` joined the default
//! catalog and the line under "Out of the box, with nothing installed" still
//! listed thirteen tools. Nobody noticed for several releases, because the only
//! thing that would have noticed was a person re-reading a sentence they wrote.
//!
//! CLAUDE.md names this exact case — "a tool joining or leaving the default
//! catalog" — as a change that must update the README in the same commit. That
//! rule had been written down twice and broken anyway; a rule that is not a
//! mechanism is a wish.
//!
//! THE TEST RUNS IN BOTH DIRECTIONS, and they are not the same assertion:
//!
//!   * every tool THIS MACHINE puts in a closed-gate catalog must be on the
//!     line. That is the direction that catches a tool joining.
//!   * no tool that lives behind an ADDON GATE may be on the line, because the
//!     sentence above it says "with nothing installed". That is the direction
//!     that catches the opposite error — advertising `web_search` as a default,
//!     which would be a privacy claim, not a typo.
//!
//! The first direction is deliberately not an equality. `calendar` is macOS
//! only and `run_code`/`write_code` need a verified sandbox, so on Linux CI the
//! catalog is a strict subset of the line. Requiring equality would force the
//! README to describe the CI runner rather than a default install, and the
//! README says what those three depend on a few lines further down.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tacet_tools::catalog::{AddonGates, production_catalog_gated};
use tacet_tools::data_store::SharedStore;
use tacet_tools::memory::SharedMemory;

/// Tools that only ever appear when an addon is installed. Named here rather
/// than derived, because the derivation would be "build the catalog twice and
/// diff" — which cannot distinguish an addon tool from one this machine simply
/// cannot carry, and would go green on a machine with no sqlite3 either way.
const BEHIND_AN_ADDON: [&str; 6] = [
    "web_search",
    "web_fetch",
    "db",
    "clipboard",
    "http",
    "shell",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

/// The backticked names on the line that follows "Out of the box".
fn advertised(readme: &str) -> Vec<String> {
    let anchor = "Out of the box, with nothing installed:";
    let after = readme
        .split_once(anchor)
        .unwrap_or_else(|| {
            panic!("README no longer says {anchor:?} — this test found it by that phrase")
        })
        .1;
    let line = after
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with('`'))
        .expect("no backticked tool line follows the 'Out of the box' sentence");
    line.split('·')
        .map(|t| t.trim().trim_matches('`').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn default_catalog_names() -> Vec<String> {
    let store = Arc::new(SharedStore::new());
    let memory = SharedMemory::in_memory();
    let (catalog, _, _) = production_catalog_gated(&store, &memory, None, AddonGates::closed());
    catalog.names().into_iter().map(String::from).collect()
}

#[test]
fn the_readme_names_every_tool_a_default_install_has() {
    let readme = std::fs::read_to_string(repo_root().join("README.md")).expect("README.md");
    let listed = advertised(&readme);
    let missing: Vec<String> = default_catalog_names()
        .into_iter()
        .filter(|n| !listed.contains(n))
        .collect();
    assert!(
        missing.is_empty(),
        "these tools are in the default catalog and not on the README's tool line: {missing:?}\n\
         the line reads: {listed:?}\n\
         CLAUDE.md: a tool joining the default catalog updates the README in the SAME commit."
    );
}

#[test]
fn the_readme_does_not_advertise_an_addon_as_a_default() {
    let readme = std::fs::read_to_string(repo_root().join("README.md")).expect("README.md");
    let listed = advertised(&readme);
    let wrongly: Vec<&str> = BEHIND_AN_ADDON
        .iter()
        .copied()
        .filter(|n| listed.iter().any(|l| l == n))
        .collect();
    assert!(
        wrongly.is_empty(),
        "the sentence above this line says 'with nothing installed', but it lists {wrongly:?}, \
         which exist only once an addon is installed"
    );
}
