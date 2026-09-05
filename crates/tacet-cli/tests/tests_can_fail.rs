//! Every `#[test]` must be able to go red.
//!
//! WHY THIS EXISTS. CONTRIBUTING says it in words — "a green test that cannot go
//! red is worse than no test" — and words did not hold. Five test functions were
//! found in one evening with no assertion, no `expect`, no `unwrap`, no `?` and
//! no `panic!`: they printed and returned. Two of them shipped in published
//! crates and counted toward the test total this project quotes.
//!
//! The cost was not theoretical. `tmp_space_probe.rs` printed, in its own
//! output, that the mask offered a space the automaton then refused — a real
//! defect that killed any tool call carrying an indent past sixteen columns.
//! It ran green for as long as it existed, because there was nothing in it that
//! could fail.
//!
//! THE RULE IS NARROW ON PURPOSE: a test is flagged only if it PRINTS and
//! contains no assertion, panic, expect, unwrap or `?`. That is the exact shape
//! of the five that were found, and it leaves alone two large classes of
//! legitimate test that a broader rule caught by mistake:
//!
//!   * "does not panic" tests — `a_malformed_deflate_stream_does_not_panic`
//!     corrupts every byte of a deflate stream and asserts nothing, because the
//!     failure it is looking for IS a panic.
//!   * tests that delegate their assertions to a shared helper, like the
//!     router-budget guards, whose whole body is one call.
//!
//! Neither of those prints. A test that prints and asserts nothing is telling a
//! human to read the output — which is a probe, not a test, and the probe that
//! prompted this file recorded a real defect in its own output for as long as it
//! existed without ever failing.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

fn sources(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let crates = root.join("crates");
    let entries = std::fs::read_dir(&crates)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", crates.display()));
    for krate in entries.flatten() {
        for sub in ["src", "tests"] {
            walk(&krate.path().join(sub), &mut out);
        }
    }
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The name and body of the function following `#[test]` at `from`.
fn body_after(text: &str, from: usize) -> Option<(String, &str)> {
    let rest = &text[from..];
    let open = rest.find('{')?;
    let name = rest[..open]
        .rsplit("fn ")
        .next()?
        .split('(')
        .next()?
        .trim()
        .to_string();
    let bytes = rest.as_bytes();
    let (mut depth, mut i) = (0usize, open);
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((name, &rest[open..=i]));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn markers() -> [String; 6] {
    // Built at runtime so this file does not contain the literals it scans for
    // and trip over its own source.
    [
        "assert".to_string(),
        format!("pan{}", "ic!"),
        format!("unreach{}", "able!"),
        format!(".exp{}", "ect("),
        format!(".unw{}", "rap("),
        format!("?{}", ";"),
    ]
}

#[test]
fn no_test_is_incapable_of_failing() {
    let root = repo_root();
    let can_fail = markers();
    let mut inert: Vec<String> = Vec::new();

    for path in sources(&root) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut at = 0;
        let needle = format!("#[te{}]", "st");
        while let Some(i) = text[at..].find(&needle) {
            let abs = at + i;
            at = abs + needle.len();
            let Some((name, body)) = body_after(&text, abs) else {
                continue;
            };
            let prints = body.contains("println!") || body.contains("eprintln!");
            if prints && !can_fail.iter().any(|m| body.contains(m.as_str())) {
                let rel = path.strip_prefix(&root).unwrap_or(&path);
                inert.push(format!("{}::{name}", rel.display()));
            }
        }
    }

    assert!(
        inert.is_empty(),
        "these tests print and assert nothing, so they pass no matter what the \
         code does — they are probes, and the last one of these recorded a real \
         defect in its own output for as long as it existed:\n  {}\n\n\
         Either assert what the output was meant to show, or delete it.",
        inert.join("\n  ")
    );
}
