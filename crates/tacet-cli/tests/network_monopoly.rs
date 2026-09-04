//! THE NETWORK MONOPOLY, AS A TEST RATHER THAN A SENTENCE.
//!
//! The README makes this project's strongest claim in one line: "Exactly two
//! crates may open a socket, and the HTTP dependency appears in exactly those
//! two manifests. You do not have to trust a privacy claim you cannot audit —
//! `grep ureq crates/*/Cargo.toml` is the audit."
//!
//! WHY THAT WAS NOT ENOUGH, and it is a small gap in a large claim. Run the
//! grep as written and it prints NINE lines across FOUR manifests, because
//! seven of them are comments explaining why the crate does NOT take the
//! dependency. A reader who trusts the command sees `ureq` named in twice as
//! many manifests as the claim allows and has to read prose to find out which
//! two are real. An audit whose output has to be interpreted is a document, not
//! an audit.
//!
//! So the claim is checked here instead, where it fails the build. What is
//! asserted is exactly the two sentences the README makes:
//!
//! 1. Exactly `tacet-web` and `tacet-mcp` DECLARE an HTTP client.
//! 2. No crate reaches a socket some other way — no raw `std::net`, no second
//!    HTTP library slipped in under a different name.
//!
//! IT PARSES NOTHING. The manifests are read as text and a declaration is told
//! from a comment by the one rule that separates them: a comment line starts
//! with `#`. Pulling in a TOML parser to police a zero-dependency rule would be
//! its own joke.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The crates allowed to open a socket. Adding a name here is the architectural
/// decision the README describes; it is not a way to make this test pass.
const ALLOWED: &[&str] = &["tacet-web", "tacet-mcp"];

/// Every HTTP client that could stand in for `ureq`. The list is what makes the
/// test more than a `ureq` check: swapping the library must not swap the
/// guarantee out with it.
const HTTP_CRATES: &[&str] = &[
    "ureq",
    "reqwest",
    "hyper",
    "isahc",
    "attohttpc",
    "curl",
    "surf",
    "awc",
    "http-client",
    "minreq",
];

fn crates_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is `<repo>/crates/tacet-cli`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate lives under crates/")
        .to_path_buf()
}

/// Every `crates/*/Cargo.toml`, as (crate name, text).
fn manifests() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(crates_dir()).expect("crates/ is readable");
    for entry in entries.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&manifest).expect("a manifest is readable");
        out.push((name, text));
    }
    out.sort();
    assert!(
        out.len() > 5,
        "the workspace was not found where this test expects it: {:?}",
        crates_dir()
    );
    out
}

/// The lines of a manifest that DECLARE something, i.e. are not comments.
fn declarations(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
}

#[test]
fn exactly_two_crates_declare_an_http_client() {
    let mut declaring: BTreeSet<String> = BTreeSet::new();
    for (name, text) in manifests() {
        for line in declarations(&text) {
            // `ureq = { version = ... }` / `reqwest = "0.12"` — the dependency
            // name is what the line STARTS with. A version string mentioning
            // the word elsewhere is not a declaration.
            let key = line.split(['=', ' ']).next().unwrap_or_default();
            if HTTP_CRATES.contains(&key) {
                declaring.insert(name.clone());
            }
        }
    }
    let expected: BTreeSet<String> = ALLOWED.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        declaring, expected,
        "the set of crates that may reach the network has changed. \
         This is the README's headline claim; adding one is an architectural \
         decision that belongs in the manifest's own rationale comment, not in \
         this list."
    );
}

/// The directories of a crate that hold Rust the workspace compiles.
///
/// `src` ALONE WAS THE SCOPE UNTIL IT WASN'T ENOUGH TO SAY SO. A `#[test] fn
/// talks_to_a_local_server()` under `tests/` compiles, runs in `cargo test
/// --workspace`, and was invisible to this audit — and the workspace has since
/// grown ~2,700 lines of integration tests plus an `examples/` binary. Nothing
/// was wrong when this was widened (every file below `crates/*/tests` and
/// `crates/*/examples` was clean the day it was written, which is the right
/// moment to widen it), but the README's sentence names no scope, so the scope
/// had better be everything.
const CODE_DIRS: &[&str] = &["src", "tests", "examples", "benches"];

/// AND THE WAY AROUND THE MANIFEST: a crate that wants a socket without a
/// dependency can simply use the standard library. Nothing in the manifests
/// would show it.
#[test]
fn no_crate_outside_the_monopoly_opens_a_raw_socket() {
    // `std::net` also carries address types (`IpAddr`, `SocketAddr`) that are
    // pure data and open nothing; only the types that CONNECT or LISTEN are
    // evidence.
    const CONNECTORS: &[&str] = &["TcpStream", "TcpListener", "UdpSocket"];
    let mut offenders: Vec<String> = Vec::new();
    for (name, _) in manifests() {
        if ALLOWED.contains(&name.as_str()) {
            continue;
        }
        for dir in CODE_DIRS {
            for file in rust_files(&crates_dir().join(&name).join(dir)) {
                let text = std::fs::read_to_string(&file).unwrap_or_default();
                for line in text.lines() {
                    let line = line.trim();
                    // A mention in a comment is the rationale, not a socket.
                    if line.starts_with("//") {
                        continue;
                    }
                    let code = outside_string_literals(line);
                    if CONNECTORS.iter().any(|c| code.contains(c)) {
                        offenders.push(format!("{}: {}", file.display(), line));
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a crate outside the network monopoly opens a socket directly:\n  {}",
        offenders.join("\n  ")
    );
}

/// The parts of a line that are NOT inside a double-quoted string.
///
/// NEEDED THE MOMENT `tests/` CAME INTO SCOPE, and this file is the reason: the
/// `CONNECTORS` list two lines up writes all three type names as string
/// literals, so scanning `crates/tacet-cli/tests` would report the audit itself
/// as the offender. Naming a type is not opening a socket; calling one is. The
/// split is by `"` alone — no escape handling, which can only make a line look
/// like MORE code than it is, so the direction of the error is a false alarm a
/// human reads, never a socket waved through.
fn outside_string_literals(line: &str) -> String {
    line.split('"').step_by(2).collect::<Vec<_>>().join(" ")
}

/// Every `.rs` file under `dir`, recursively. A directory that does not exist —
/// most crates have no `examples/` — reads as empty, not as a failure.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out.sort();
    out
}
