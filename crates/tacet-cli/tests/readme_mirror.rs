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

    // LINE ENDINGS ARE NOT DRIFT, and comparing bytes said they were.
    //
    // On a Windows checkout with git's default `core.autocrlf=true` every `.md`
    // file arrives with CRLF, while `HEADER` above is a Rust string literal and
    // is therefore always LF. So `want` was LF header + CRLF body and `got` was
    // CRLF throughout: the same document, five bytes apart, on a test whose
    // subject is CONTENT. It failed on windows-latest and nowhere else, and the
    // message it printed was worse than useless — `str::lines()` strips the
    // trailing `\r`, so every line compared EQUAL and the report read
    // "792 lines against 792, first difference at line 793", a line that does
    // not exist. Reproduced locally by converting both files to CRLF.
    //
    // `.gitattributes` now pins `*.md` to LF, which fixes the checkout. This
    // normalisation stays anyway: the claim being tested is that the two files
    // say the same thing, and that claim should not depend on a git setting.
    let normalise = |text: &str| text.replace("\r\n", "\n");
    if normalise(&got) != normalise(&want) {
        let (g, w) = (got.lines().count(), want.lines().count());
        match got.lines().zip(want.lines()).position(|(a, b)| a != b) {
            Some(first) => panic!(
                "the crates.io page has drifted from the repository README \
                 ({g} lines against {w}), first difference at line {}:\n  \
                 mirror: {}\n  README: {}\n\
                 This is the page a new user reads before anything else. Fix it with:\n  \
                 TACET_UPDATE_README=1 cargo test -p tacet-cli --test readme_mirror",
                first + 1,
                got.lines().nth(first).unwrap_or("<none>"),
                want.lines().nth(first).unwrap_or("<none>"),
            ),
            // Every shared line matches, so the difference is length or the
            // trailing newline. Say which, rather than naming a line number one
            // past the end of the file.
            None => panic!(
                "the crates.io page has {g} lines against the README's {w}, and \
                 every line they share is identical — so the difference is at the \
                 end of the file (a missing or extra trailing newline), not in the \
                 text. Fix it with:\n  \
                 TACET_UPDATE_README=1 cargo test -p tacet-cli --test readme_mirror"
            ),
        }
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
