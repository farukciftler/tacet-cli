//! The crates.io page must not drift from the repository README.
//!
//! WHY THIS TEST EXISTS. `crates/tacet-cli/README.md` opens with a comment
//! saying it "mirrors the repository README" and is "duplicated here at publish
//! time". Nothing duplicated it. It had drifted to 198 lines against the root's
//! 736 and still carried wording the root had since corrected — on the page a
//! potential adopter sees FIRST, because crates.io only packages files inside
//! the crate directory.
//!
//! That is the same failure the repository's own rule is written against: a
//! stale page of measured claims is not a documentation problem, it is a false
//! measurement with a date on it. A comment asking the next person to remember
//! is not a mechanism; this is.
//!
//! RELATIVE LINKS ARE REWRITTEN rather than copied. `](training/)` resolves on
//! GitHub and 404s on crates.io, so the mirror carries absolute URLs — which is
//! also why a plain `cp` would not have been enough and why the drift was never
//! going to fix itself by hand.
//!
//! To update after editing the root README:
//!     TACET_UPDATE_README=1 cargo test -p tacet-cli --test readme_mirror

use std::path::{Path, PathBuf};

const HEADER: &str = "<!-- GENERATED FROM THE REPOSITORY README — DO NOT EDIT.\n     \
     crates.io only packages files inside the crate directory, so the canonical\n     \
     copy at the repo root is mirrored here with its relative links absolutised.\n     \
     Edit ../../README.md, then run:\n       \
     TACET_UPDATE_README=1 cargo test -p tacet-cli --test readme_mirror -->\n\n";

const BLOB: &str = "https://github.com/farukciftler/tacet-cli/blob/main/";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

/// The root README with every repo-relative link made absolute.
///
/// Anchors (`](#install`) and absolute URLs are left alone; `](../../releases)`
/// is the GitHub releases page, which has its own home.
fn mirrored(root: &str) -> String {
    let mut out = String::with_capacity(root.len() + 512);
    out.push_str(HEADER);
    let mut rest = root;
    while let Some(i) = rest.find("](") {
        let (before, after) = rest.split_at(i + 2);
        out.push_str(before);
        let end = match after.find(')') {
            Some(e) => e,
            None => break,
        };
        let target = &after[..end];
        let absolute = if target.starts_with('#')
            || target.starts_with("http://")
            || target.starts_with("https://")
        {
            target.to_string()
        } else if target == "../../releases" {
            "https://github.com/farukciftler/tacet-cli/releases".to_string()
        } else {
            format!("{BLOB}{target}")
        };
        out.push_str(&absolute);
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[test]
fn the_crates_io_page_matches_the_repository_readme() {
    let root_path = repo_root().join("README.md");
    let mirror_path = repo_root().join("crates/tacet-cli/README.md");
    let root = std::fs::read_to_string(&root_path)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", root_path.display()));
    let want = mirrored(&root);

    if std::env::var("TACET_UPDATE_README").is_ok() {
        std::fs::write(&mirror_path, &want).expect("mirror is writable");
        eprintln!("rewrote {}", mirror_path.display());
        return;
    }

    let got = std::fs::read_to_string(&mirror_path)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", mirror_path.display()));

    if got != want {
        let (g, w) = (got.lines().count(), want.lines().count());
        let first = got
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(g.min(w));
        panic!(
            "the crates.io page has drifted from the repository README \
             ({g} lines against {w}), first difference at line {}. \
             This is the page a new user reads before anything else. Fix it with:\n  \
             TACET_UPDATE_README=1 cargo test -p tacet-cli --test readme_mirror",
            first + 1
        );
    }
}

/// The rewriting itself, because a mirror full of dead links is its own defect.
#[test]
fn repo_relative_links_are_absolutised_and_anchors_are_not() {
    let out = mirrored(
        "see [training](training/) and [install](#install) and [rust](https://rust-lang.org)",
    );
    assert!(
        out.contains(&format!("({BLOB}training/)")),
        "a repo-relative link must become absolute: {out}"
    );
    assert!(
        out.contains("(#install)"),
        "an anchor stays an anchor: {out}"
    );
    assert!(
        out.contains("(https://rust-lang.org)"),
        "an absolute URL is left alone: {out}"
    );
}
