//! EVERY AGENT STARTS IN `net.rs` (audit M12) — the structural half.
//!
//! `tests/no_env_proxy.rs` checks the agents that exist by routing real
//! requests past a listening proxy. This file stops the next agent from being
//! written somewhere else with a bare `ureq::Agent::config_builder()` — the way
//! all five were before.
//!
//! A SEPARATE BINARY ON PURPOSE: `no_env_proxy.rs` calls `set_var`, which is
//! only sound while nothing else in the process reads the environment. These
//! tests call `std::env::temp_dir()` (it reads `TMPDIR`), so they cannot share
//! that process. A review on 10 Oct 2026 caught them living there, under a
//! SAFETY comment that still said the binary had one test.

use std::path::{Path, PathBuf};

/// A REFUSE-BY-DEFAULT READING OF EVERY `ureq::` PATH, NOT A LIST OF BAD ONES.
///
/// The first version looked for four spellings and read `src/` one level deep.
/// The second added `Agent::new_with_defaults()`, `Config::default()` and the
/// `ureq::put` / `head` / `delete` family. A review on 10 Oct 2026 then wrote
/// `ureq::run(request)`, `ureq::agent()`, `ureq::connect(uri)` and
/// `use ureq::get; get(url)` into a throwaway source file and the guard stayed
/// green for every one — each of them calls `Agent::new_with_defaults()`
/// inside ureq 3.3.0, which reads `ALL_PROXY` again. A deny-list is one ureq
/// release away from being wrong; an allow-list is wrong only by being too
/// strict, which shows up as a red test and not as a leak.
///
/// So outside `net.rs` a `ureq::` path must be one of the names below, none of
/// which can construct an agent. `Agent` is allowed only as a type: `Agent::`
/// is where the constructors live.
const ALLOWED_PATHS: &[&str] = &[
    // The re-exported `http` crate: `Uri` parsing, `Response`, header names.
    "http::",
    // `AgentScope`, the type parameter of the builder `net::builder` returns.
    "typestate::",
    // Matching on error variants.
    "Error::",
    "Error",
    "Agent",
    "Body",
    // The return type of `net::builder()`; it has no public constructor.
    "config::ConfigBuilder",
];

/// Doors that do not need a `ureq::` path in front of them: a method on a type
/// already in scope (`use ureq::Agent;` is allowed, `Agent::new_with_defaults()`
/// is not), or a trait method (`RequestExt::with_default_agent`). The three
/// free functions the review found are listed too, so the refusal reads as a
/// name and not only as "not on the allow-list".
const AGENT_DOORS: &[&str] = &[
    "config_builder(",
    "Config::builder(",
    "Config::default(",
    "new_with_defaults(",
    "new_with_config(",
    "Agent::default(",
    "with_default_agent(",
    "ureq::run(",
    "ureq::agent(",
    "ureq::connect(",
];

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn allowed(path: &str) -> bool {
    ALLOWED_PATHS.iter().any(|p| {
        let Some(rest) = path.strip_prefix(p) else {
            return false;
        };
        p.ends_with("::") || !(rest.starts_with(is_ident) || rest.starts_with("::"))
    })
}

/// Every way `text` could reach an agent built with the environment proxy on.
fn doors_in(text: &str) -> Vec<String> {
    let mut found: Vec<String> = AGENT_DOORS
        .iter()
        .filter(|d| text.contains(*d))
        .map(|d| d.to_string())
        .collect();
    for (at, _) in text.match_indices("ureq") {
        let before = text[..at].chars().next_back();
        let rest = &text[at + "ureq".len()..];
        if before.is_some_and(is_ident) || rest.starts_with(is_ident) {
            continue; // `ureq_proto`, `my_ureq`
        }
        let shown: String = rest.chars().take_while(|c| *c != '\n').take(40).collect();
        if let Some(path) = rest.strip_prefix("::") {
            // `use ureq::{get, post}`, `use ureq::get`, `ureq::*`, and every
            // name the list does not know.
            if !allowed(path) {
                found.push(format!("ureq{shown}"));
            }
        } else if rest.trim_start().starts_with("as ") {
            // `use ureq as u;` then `u::get(url)` — a rename hides every path.
            found.push(format!("ureq{shown}"));
        }
    }
    found
}

#[test]
fn no_source_file_builds_an_agent_outside_net_rs() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let net = src.join("net.rs");
    let mut offenders = Vec::new();
    let mut seen = 0usize;
    for path in rust_files(&src) {
        if path == net {
            continue;
        }
        seen += 1;
        let text = std::fs::read_to_string(&path).expect("read");
        for door in doors_in(&text) {
            offenders.push(format!("{}: {door}", path.display()));
        }
    }
    // A walk that found nothing would pass vacuously.
    assert!(
        seen >= 5,
        "only {seen} source files were read under {src:?}"
    );
    assert!(
        offenders.is_empty(),
        "an agent is configured outside net.rs, where the proxy rule lives: {offenders:?}"
    );
}

/// The guard above is only as good as `doors_in`, and a source tree that is
/// clean today cannot show that it sees anything. Each line here is a door
/// that a reviewer or a reading of ureq 3.3.0's `lib.rs` / `request_ext.rs`
/// found; each must be refused on its own.
#[test]
fn every_known_door_is_refused() {
    let doors = [
        "let a: ureq::Agent = ureq::Agent::config_builder().build().into();",
        "let a = ureq::Agent::new_with_defaults();",
        "let a = Agent::new_with_config(c);",
        "let c = ureq::config::Config::default();",
        "let c = Config::builder().build();",
        "let a = Agent::default();",
        "ureq::get(url).call()",
        "ureq::post(url).send(b)",
        "ureq::put(url).send(b)",
        "ureq::delete(url).call()",
        "ureq::head(url).call()",
        "ureq::patch(url).send(b)",
        "ureq::options(url).call()",
        "ureq::trace(url).call()",
        "ureq::request(\"GET\", url)",
        // The three the 10 Oct 2026 review found past the deny-list.
        "ureq::run(request)",
        "let a = ureq::agent();",
        "ureq::connect(uri).call()",
        // Imports that make a free function callable without its prefix.
        "use ureq::get;\nfn f() { get(url).call(); }",
        "use ureq::{get, post};",
        "use ureq::{Agent, Error};",
        "use ureq::*;",
        "use ureq as u;\nfn f() { u::get(url).call(); }",
        "extern crate ureq as u;",
        "use ::ureq::post;",
        // `http::Request::builder()...body(())?.with_default_agent().run()`.
        "use ureq::RequestExt;",
        "req.with_default_agent().run()",
        // A constructor reached through a type the list allows.
        "ureq::Body::builder()",
        "ureq::config::ConfigBuilder::new()",
    ];
    for door in doors {
        assert!(!doors_in(door).is_empty(), "not refused: {door}");
    }
}

/// The other direction: every shape the crate really uses must pass, or the
/// guard would be switched off by the first person it annoyed.
#[test]
fn the_shapes_the_crate_uses_are_not_refused() {
    let fine = [
        "use ureq::typestate::AgentScope;",
        "use ureq::config::ConfigBuilder;",
        "fn f() -> ureq::Agent {",
        "agent: fn(Option<u64>) -> ureq::Agent,",
        "Err(ureq::Error::StatusCode(416)) if existing > 0 => {",
        "fn convert(error: &ureq::Error) -> WebError {",
        "fn h(r: &ureq::http::Response<ureq::Body>) {}",
        "fn b() -> ureq::config::ConfigBuilder<AgentScope> {",
        "let uri: ureq::http::Uri = url.parse().ok()?;",
        "//! NETWORK MONOPOLY: `ureq` is called nowhere else but this file",
        "// `ureq` refuses a non-https address by itself.",
    ];
    for line in fine {
        assert!(
            doors_in(line).is_empty(),
            "refused: {line} -> {:?}",
            doors_in(line)
        );
    }
}

/// `no_env_proxy.rs` sets `ALL_PROXY` with `set_var`, and its SAFETY comment
/// rests on being the only test in its process. That held for one commit: the
/// next added two tests beside it and the comment went on saying "one test".
/// Counted from here, because a test added there to count itself would be the
/// second test.
#[test]
fn the_environment_test_is_alone_in_its_process() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/no_env_proxy.rs");
    let text = std::fs::read_to_string(&path).expect("read no_env_proxy.rs");
    assert!(
        text.contains("set_var"),
        "{path:?} no longer sets the proxy"
    );
    assert_eq!(
        text.matches("#[test]").count(),
        1,
        "{path:?} must hold exactly one test: `set_var` races any other thread that reads the environment"
    );
}

/// Every `.rs` file under `dir`, at any depth.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// The walk must reach a nested module — the case the first version missed.
#[test]
fn the_source_walk_reaches_a_nested_directory() {
    let root = std::env::temp_dir().join(format!("tacet-walk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("top.rs"), "").unwrap();
    std::fs::write(root.join("a/b/deep.rs"), "").unwrap();
    std::fs::write(root.join("a/notes.txt"), "").unwrap();
    let mut found: Vec<_> = rust_files(&root)
        .into_iter()
        .map(|p| p.strip_prefix(&root).unwrap().to_path_buf())
        .collect();
    found.sort();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(
        found,
        vec![PathBuf::from("a/b/deep.rs"), PathBuf::from("top.rs")]
    );
}
