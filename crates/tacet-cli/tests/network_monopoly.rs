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
//! asserted is the two sentences the README makes, three ways each:
//!
//! 1. Exactly `tacet-web` and `tacet-mcp` DECLARE an HTTP client — read from
//!    the manifests as TOML, and again from the dependency graph cargo itself
//!    resolves (`cargo metadata`), so a client that arrives one crate removed
//!    counts as much as one declared by name.
//! 2. No crate reaches a socket some other way — no raw `std::net` in any `.rs`
//!    file a crate holds, `build.rs` included, read by a lexer that knows what a
//!    string literal and a comment are.
//! 3. No crate spawns another agent's CLI (`claude -p`, `codex app-server`,
//!    `agy -p`) without that path being an external tool behind the approval
//!    gate — those programs ARE the network, one process away.
//!
//! THE FIRST VERSION OF THIS FILE PARSED NOTHING, on the theory that a TOML
//! parser to police a zero-dependency rule would be its own joke. The audit of
//! 9 Oct 2026 (T4) built a fake workspace and walked five shapes straight past
//! it, every one green: `net = { package = "ureq" }`, `ureq.version = "3"`,
//! `[dependencies.ureq]`, a `TcpStream::connect` after the char literal `'"'`,
//! and a `TcpStream` in `build.rs`. A guard that a rename defeats guards the
//! spelling, not the property. The parser below is still hand-written — the
//! rule it polices forbids the crate — but it is a parser: it knows tables,
//! dotted keys, inline tables, quoted keys and every string form Cargo accepts.
//! Each of the five bypasses has a self-test at the bottom of this file that
//! feeds it in and expects red.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// The crates allowed to open a socket. Adding a name here is the architectural
/// decision the README describes; it is not a way to make this test pass.
const ALLOWED: &[&str] = &["tacet-web", "tacet-mcp"];

/// Every HTTP client that could stand in for `ureq`. The list is what makes the
/// test more than a `ureq` check: swapping the library must not swap the
/// guarantee out with it.
///
/// `socket2` and the `hyper`/`curl` internals are here because the GRAPH check
/// below sees what a declaration hides: a crate that pulls a socket layer in
/// through a wrapper never names the client at all.
///
/// `mio` IS DELIBERATELY ABSENT, and that was measured rather than assumed:
/// `crossterm` (tacet-cli's terminal) pulls it in, and `signal-hook-mio`
/// switches on mio's `net` feature for the self-pipe it delivers signals over —
/// a `UnixStream` pair inside one process. `cargo metadata --all-features` on
/// 10 Oct 2026 showed mio resolved with `["default","log","net","os-ext",
/// "os-poll"]`. Listing it would make the test red on a terminal library, and a
/// test that is red for a reason everyone knows to ignore stops being read.
const HTTP_CRATES: &[&str] = &[
    "ureq",
    "reqwest",
    "hyper",
    "hyper-util",
    "h2",
    "isahc",
    "attohttpc",
    "curl",
    "curl-sys",
    "surf",
    "awc",
    "http-client",
    "minreq",
    "socket2",
    "tungstenite",
];

/// The `std::net` types that CONNECT or LISTEN. `std::net` also carries address
/// types (`IpAddr`, `SocketAddr`) that are pure data and open nothing.
const CONNECTORS: &[&str] = &["TcpStream", "TcpListener", "UdpSocket"];

/// Other agents' CLIs. Each one, spawned, sends the conversation to a hosted
/// model — an outbound call that happens to go through `fork` instead of
/// `connect`. The agent-session R&D (RG-33) plans readers for all three; the
/// readers are local file reads and need none of this, the WRITE-BACK paths
/// (`claude -p --resume`, `codex exec resume`, `codex app-server`, `agy -p`)
/// are the ones that do.
const AGENT_CLIS: &[&str] = &["claude", "codex", "agy", "antigravity", "gemini"];

/// The files allowed to spawn an agent CLI, each with the tool name it runs
/// under. Empty today. A path gets in here only together with its tool name in
/// tacet-cli's `EXTERNAL_TOOLS`, which `agent_clis_are_spawned_only_behind_the_
/// approval_gate` checks: the entry is the decision, the gate is what makes it
/// safe.
const AGENT_CLI_SPAWNERS: &[(&str, &str)] = &[];

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is `<repo>/crates/tacet-cli`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate lives under <repo>/crates/")
        .to_path_buf()
}

fn crates_dir() -> PathBuf {
    workspace_root().join("crates")
}

/// Every `<crates>/*/Cargo.toml`, as (crate directory name, text).
fn manifests_in(crates: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(crates).expect("crates/ is readable");
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
    out
}

fn manifests() -> Vec<(String, String)> {
    let out = manifests_in(&crates_dir());
    assert!(
        out.len() > 5,
        "the workspace was not found where this test expects it: {:?}",
        crates_dir()
    );
    out
}

// ---------------------------------------------------------------------------
// 1. Declarations, read as TOML.
// ---------------------------------------------------------------------------

#[test]
fn exactly_two_crates_declare_an_http_client() {
    let root = std::fs::read_to_string(workspace_root().join("Cargo.toml"))
        .expect("the workspace manifest is readable");
    let declaring = http_declarers(&root, &manifests());
    let expected: BTreeSet<String> = ALLOWED.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        declaring.keys().cloned().collect::<BTreeSet<_>>(),
        expected,
        "the set of crates that may reach the network has changed ({declaring:?}). \
         This is the README's headline claim; adding one is an architectural \
         decision that belongs in the manifest's own rationale comment, not in \
         this list."
    );
}

/// The manifests that declare an HTTP client, by crate, with what they declare.
/// The workspace root counts as a manifest of its own (`<workspace>`): an HTTP
/// client in `[workspace.dependencies]` is the README's "appears in a manifest"
/// even before any crate inherits it.
fn http_declarers(root: &str, manifests: &[(String, String)]) -> BTreeMap<String, Vec<String>> {
    let root_deps = dependencies(root).unwrap_or_else(|e| panic!("<workspace>/Cargo.toml: {e}"));
    // `name.workspace = true` in a member names the ALIAS in the root table;
    // the package behind it is whatever that row says.
    let inherited: BTreeMap<String, String> = root_deps
        .iter()
        .filter(|d| d.table == DepTable::Workspace)
        .map(|d| (d.alias.clone(), d.package()))
        .collect();
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in &root_deps {
        if HTTP_CRATES.contains(&d.package().as_str()) {
            out.entry("<workspace>".into())
                .or_default()
                .push(d.package());
        }
    }
    for (name, text) in manifests {
        let deps = dependencies(text).unwrap_or_else(|e| panic!("{name}/Cargo.toml: {e}"));
        for d in deps {
            let package = if d.workspace {
                inherited
                    .get(&d.alias)
                    .cloned()
                    .unwrap_or_else(|| d.package())
            } else {
                d.package()
            };
            if HTTP_CRATES.contains(&package.as_str()) {
                out.entry(name.clone()).or_default().push(package);
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DepTable {
    /// `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, and the
    /// same three under `[target.<cfg>]`. A dev-dependency counts: it compiles
    /// into `cargo test --workspace`, which runs on the same machine.
    Crate,
    /// `[workspace.dependencies]` in the root.
    Workspace,
}

#[derive(Debug, Default)]
struct DepRow {
    alias: String,
    package: Option<String>,
    workspace: bool,
}

#[derive(Debug)]
struct Dep {
    table: DepTable,
    alias: String,
    /// `package = "..."` — the crate actually fetched, whatever the key says.
    package: Option<String>,
    /// `workspace = true` — the package is decided by the root manifest.
    workspace: bool,
}

impl Dep {
    fn package(&self) -> String {
        self.package.clone().unwrap_or_else(|| self.alias.clone())
    }
}

/// Every dependency a manifest declares, in any table that declares one.
///
/// A dependency is a key directly under a dependency table, however TOML spells
/// the path to it: `ureq = "3"`, `ureq.version = "3"`, `[dependencies.ureq]`,
/// `[target.'cfg(unix)'.dependencies] ureq = ...` all flatten to the same
/// `dependencies.ureq` and are read the same way. What it is CALLED is the key;
/// what it IS is `package`, when present.
fn dependencies(text: &str) -> Result<Vec<Dep>, String> {
    const KINDS: &[&str] = &[
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        // The underscore spellings: rejected by edition 2024, still accepted
        // by Cargo for older editions — and a manifest's edition is one line.
        "dev_dependencies",
        "build_dependencies",
    ];
    let mut rows: BTreeMap<(String, String), (DepTable, DepRow)> = BTreeMap::new();
    for (path, value) in toml_lite::flatten(text)? {
        let at = match path.first().map(String::as_str) {
            Some(k) if KINDS.contains(&k) => Some((0, DepTable::Crate)),
            Some("target") if path.len() > 2 && KINDS.contains(&path[2].as_str()) => {
                Some((2, DepTable::Crate))
            }
            Some("workspace") if path.get(1).is_some_and(|k| k == "dependencies") => {
                Some((1, DepTable::Workspace))
            }
            _ => None,
        };
        let Some((i, table)) = at else { continue };
        let Some(alias) = path.get(i + 1) else {
            continue;
        };
        // The table prefix keeps two rows apart that differ only in where they
        // stand (`[dependencies] x` and `[target.a.dependencies] x`).
        let key = (path[..=i].join("."), alias.clone());
        let (_, row) = rows.entry(key).or_insert_with(|| {
            (
                table,
                DepRow {
                    alias: alias.clone(),
                    ..DepRow::default()
                },
            )
        });
        match (
            path.get(i + 2).map(String::as_str),
            path.len() == i + 3,
            &value,
        ) {
            (Some("package"), true, toml_lite::Value::Str(p)) => row.package = Some(p.clone()),
            (Some("workspace"), true, toml_lite::Value::Bool(true)) => row.workspace = true,
            _ => {}
        }
    }
    Ok(rows
        .into_values()
        .map(|(table, r)| Dep {
            table,
            alias: r.alias,
            package: r.package,
            workspace: r.workspace,
        })
        .collect())
}

/// Enough TOML to read a Cargo manifest without being lied to by one.
///
/// It FLATTENS: every value is reported with its full dotted path, so
/// `[dependencies.ureq] version = "3"`, `[dependencies] ureq.version = "3"` and
/// `[dependencies] ureq = { version = "3" }` produce one identical row. A table
/// header and an inline table also report their own path, so an empty
/// `[dependencies.ureq]` or `ureq = {}` is still a declaration. Anything it
/// cannot read is an error, never a skip: a manifest this cannot parse is a
/// manifest this cannot vouch for.
mod toml_lite {
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Str(String),
        Bool(bool),
        /// A table header, an inline table or an array — present, contents
        /// reported separately where they have paths.
        Table,
        /// A number, date or anything else this test has no use for.
        Other,
    }

    pub fn flatten(text: &str) -> Result<Vec<(Vec<String>, Value)>, String> {
        let mut p = Parser {
            s: text.chars().collect(),
            i: 0,
            out: Vec::new(),
        };
        p.document()?;
        Ok(p.out)
    }

    struct Parser {
        s: Vec<char>,
        i: usize,
        out: Vec<(Vec<String>, Value)>,
    }

    impl Parser {
        fn peek(&self) -> Option<char> {
            self.s.get(self.i).copied()
        }
        fn at(&self, lit: &str) -> bool {
            lit.chars()
                .enumerate()
                .all(|(k, c)| self.s.get(self.i + k) == Some(&c))
        }
        fn line(&self) -> usize {
            1 + self.s[..self.i.min(self.s.len())]
                .iter()
                .filter(|&&c| c == '\n')
                .count()
        }
        fn err<T>(&self, what: &str) -> Result<T, String> {
            Err(format!("line {}: {what}", self.line()))
        }
        /// Spaces and tabs only — a newline ends a key/value pair.
        fn blank(&mut self) {
            while matches!(self.peek(), Some(' ' | '\t')) {
                self.i += 1;
            }
        }
        /// Whitespace, newlines and comments — between statements and inside
        /// arrays and inline tables.
        fn gap(&mut self) {
            loop {
                match self.peek() {
                    Some(' ' | '\t' | '\r' | '\n') => self.i += 1,
                    Some('#') => {
                        while !matches!(self.peek(), None | Some('\n')) {
                            self.i += 1;
                        }
                    }
                    _ => return,
                }
            }
        }

        fn document(&mut self) -> Result<(), String> {
            let mut table: Vec<String> = Vec::new();
            loop {
                self.gap();
                match self.peek() {
                    None => return Ok(()),
                    Some('[') => {
                        let array = self.at("[[");
                        self.i += if array { 2 } else { 1 };
                        self.blank();
                        table = self.key()?;
                        self.blank();
                        let close = if array { "]]" } else { "]" };
                        if !self.at(close) {
                            return self.err("unterminated table header");
                        }
                        self.i += close.len();
                        self.out.push((table.clone(), Value::Table));
                    }
                    Some(_) => {
                        let mut path = table.clone();
                        self.pair(&mut path)?;
                    }
                }
                self.blank();
                if self.peek() == Some('#') {
                    self.gap();
                    continue;
                }
                match self.peek() {
                    None | Some('\n') | Some('\r') => {}
                    Some(c) => return self.err(&format!("unexpected `{c}` after a statement")),
                }
            }
        }

        /// `a.b."c d" = value`, under `path`.
        fn pair(&mut self, path: &mut Vec<String>) -> Result<(), String> {
            path.extend(self.key()?);
            self.blank();
            if self.peek() != Some('=') {
                return self.err("expected `=`");
            }
            self.i += 1;
            self.blank();
            self.value(path)
        }

        /// A dotted key; each segment bare, `"basic"` or `'literal'`.
        fn key(&mut self) -> Result<Vec<String>, String> {
            let mut parts = Vec::new();
            loop {
                self.blank();
                let part = match self.peek() {
                    Some('"') => self.basic_string()?,
                    Some('\'') => self.literal_string()?,
                    _ => {
                        let start = self.i;
                        while self
                            .peek()
                            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                        {
                            self.i += 1;
                        }
                        if start == self.i {
                            return self.err("expected a key");
                        }
                        self.s[start..self.i].iter().collect()
                    }
                };
                parts.push(part);
                self.blank();
                if self.peek() == Some('.') {
                    self.i += 1;
                } else {
                    return Ok(parts);
                }
            }
        }

        fn value(&mut self, path: &[String]) -> Result<(), String> {
            match self.peek() {
                Some('"') | Some('\'') => {
                    let s = self.string()?;
                    self.out.push((path.to_vec(), Value::Str(s)));
                }
                Some('{') => {
                    self.i += 1;
                    self.out.push((path.to_vec(), Value::Table));
                    loop {
                        self.gap();
                        if self.peek() == Some('}') {
                            self.i += 1;
                            break;
                        }
                        let mut inner = path.to_vec();
                        self.pair(&mut inner)?;
                        self.gap();
                        match self.peek() {
                            Some(',') => self.i += 1,
                            Some('}') => {}
                            _ => return self.err("expected `,` or `}` in an inline table"),
                        }
                    }
                }
                Some('[') => {
                    self.i += 1;
                    self.out.push((path.to_vec(), Value::Table));
                    loop {
                        self.gap();
                        if self.peek() == Some(']') {
                            self.i += 1;
                            break;
                        }
                        // Array elements have no path of their own; parse them
                        // so their strings cannot be mistaken for structure.
                        let before = self.out.len();
                        self.value(path)?;
                        self.out.truncate(before);
                        self.gap();
                        match self.peek() {
                            Some(',') => self.i += 1,
                            Some(']') => {}
                            _ => return self.err("expected `,` or `]` in an array"),
                        }
                    }
                }
                Some(_) => {
                    let start = self.i;
                    while !matches!(
                        self.peek(),
                        None | Some(',' | ']' | '}' | '\n' | '\r' | '#')
                    ) {
                        self.i += 1;
                    }
                    let raw: String = self.s[start..self.i].iter().collect();
                    let v = match raw.trim() {
                        "true" => Value::Bool(true),
                        "false" => Value::Bool(false),
                        "" => return self.err("expected a value"),
                        _ => Value::Other,
                    };
                    self.out.push((path.to_vec(), v));
                }
                None => return self.err("expected a value"),
            }
            Ok(())
        }

        fn string(&mut self) -> Result<String, String> {
            if self.at("\"\"\"") {
                self.i += 3;
                let mut s = String::new();
                loop {
                    if self.at("\"\"\"") {
                        self.i += 3;
                        // Up to two more quotes belong to the content.
                        while self.peek() == Some('"') {
                            s.push('"');
                            self.i += 1;
                        }
                        return Ok(s);
                    }
                    match self.peek() {
                        None => return self.err("unterminated multi-line string"),
                        Some('\\') => {
                            self.i += 1;
                            if let Some(c) = self.peek() {
                                s.push(c);
                                self.i += 1;
                            }
                        }
                        Some(c) => {
                            s.push(c);
                            self.i += 1;
                        }
                    }
                }
            }
            if self.at("'''") {
                self.i += 3;
                let start = self.i;
                while !self.at("'''") {
                    if self.peek().is_none() {
                        return self.err("unterminated multi-line literal string");
                    }
                    self.i += 1;
                }
                let s = self.s[start..self.i].iter().collect();
                self.i += 3;
                return Ok(s);
            }
            if self.peek() == Some('"') {
                self.basic_string()
            } else {
                self.literal_string()
            }
        }

        fn basic_string(&mut self) -> Result<String, String> {
            self.i += 1;
            let mut s = String::new();
            loop {
                match self.peek() {
                    None | Some('\n') => return self.err("unterminated string"),
                    Some('"') => {
                        self.i += 1;
                        return Ok(s);
                    }
                    Some('\\') => {
                        self.i += 1;
                        let c = self.peek().ok_or("unterminated escape")?;
                        self.i += 1;
                        match c {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'r' => s.push('\r'),
                            'b' => s.push('\u{8}'),
                            'f' => s.push('\u{c}'),
                            'u' | 'U' => {
                                let n = if c == 'u' { 4 } else { 8 };
                                let hex: String = self.s[self.i..(self.i + n).min(self.s.len())]
                                    .iter()
                                    .collect();
                                self.i += n;
                                let ch =
                                    u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
                                match ch {
                                    Some(ch) => s.push(ch),
                                    None => return self.err("bad unicode escape"),
                                }
                            }
                            other => s.push(other),
                        }
                    }
                    Some(c) => {
                        s.push(c);
                        self.i += 1;
                    }
                }
            }
        }

        fn literal_string(&mut self) -> Result<String, String> {
            self.i += 1;
            let start = self.i;
            while self.peek() != Some('\'') {
                if matches!(self.peek(), None | Some('\n')) {
                    return self.err("unterminated literal string");
                }
                self.i += 1;
            }
            let s = self.s[start..self.i].iter().collect();
            self.i += 1;
            Ok(s)
        }
    }
}

// ---------------------------------------------------------------------------
// 1b. The same claim, from the graph cargo resolves.
// ---------------------------------------------------------------------------

/// THE MANIFESTS SAY WHAT A CRATE ASKED FOR; THE GRAPH SAYS WHAT IT GOT. A
/// crate that depends on some `fancy-fetch` that depends on `reqwest` never
/// writes an HTTP client's name anywhere this file's manifest check can see.
/// So the graph is read too — every feature on, every target, every dependency
/// kind — and walked from each workspace crate outside the monopoly. The walk
/// STOPS at `tacet-web` and `tacet-mcp`: calling them is how a crate is meant to
/// reach the network, so their own `ureq` is theirs, not their caller's.
///
/// `--offline` because a test must open no connection, and by the time this
/// runs `cargo test` has already fetched every crate the graph names. A failure
/// to run cargo is a red test, not a skip: a guard that turns itself off when
/// its tool is missing has the shape this project already shipped once.
#[test]
fn no_crate_outside_the_monopoly_reaches_an_http_client_through_the_graph() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(&cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--offline",
            "--all-features",
            "--manifest-path",
        ])
        .arg(workspace_root().join("Cargo.toml"))
        .output()
        .unwrap_or_else(|e| panic!("could not run `{cargo} metadata`: {e}"));
    assert!(
        out.status.success(),
        "`cargo metadata --offline` failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("cargo metadata prints JSON");
    let members = meta["workspace_members"].as_array().map_or(0, Vec::len);
    assert!(
        members > 5,
        "cargo metadata returned {members} workspace members; this is not the workspace"
    );
    let offenders = graph_offenders(&meta);
    assert!(
        offenders.is_empty(),
        "a crate outside the network monopoly reaches an HTTP client:\n  {}",
        offenders.join("\n  ")
    );
}

/// Every path, in `cargo metadata` JSON, from a workspace crate outside
/// `ALLOWED` to a crate in `HTTP_CRATES`, written `a -> b -> ureq`.
fn graph_offenders<'a>(meta: &'a serde_json::Value) -> Vec<String> {
    let names: BTreeMap<&str, &str> = meta["packages"]
        .as_array()
        .expect("metadata has packages")
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p["name"].as_str()?)))
        .collect();
    let edges: BTreeMap<&str, Vec<&str>> = meta["resolve"]["nodes"]
        .as_array()
        .expect("metadata has a resolved graph")
        .iter()
        .filter_map(|n| {
            let deps = n["deps"]
                .as_array()?
                .iter()
                .filter_map(|d| d["pkg"].as_str())
                .collect();
            Some((n["id"].as_str()?, deps))
        })
        .collect();
    let members: Vec<&str> = meta["workspace_members"]
        .as_array()
        .expect("metadata has workspace members")
        .iter()
        .filter_map(|m| m.as_str())
        .collect();
    fn named<'a>(names: &BTreeMap<&'a str, &'a str>, id: &'a str) -> &'a str {
        names.get(id).copied().unwrap_or(id)
    }
    let name = |id: &'a str| named(&names, id);
    let mut offenders = Vec::new();
    for &start in &members {
        if ALLOWED.contains(&name(start)) {
            continue;
        }
        // Breadth-first, so the path reported is the shortest one.
        let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
        let mut queue = VecDeque::from([start]);
        let mut seen = BTreeSet::from([start]);
        while let Some(id) = queue.pop_front() {
            if HTTP_CRATES.contains(&name(id)) {
                let mut chain = vec![name(id)];
                let mut at = id;
                while let Some(&p) = parent.get(at) {
                    chain.push(name(p));
                    at = p;
                }
                chain.reverse();
                offenders.push(chain.join(" -> "));
                continue;
            }
            if id != start && members.contains(&id) && ALLOWED.contains(&name(id)) {
                continue;
            }
            for &next in edges.get(id).map(Vec::as_slice).unwrap_or_default() {
                if seen.insert(next) {
                    parent.insert(next, id);
                    queue.push_back(next);
                }
            }
        }
    }
    offenders
}

// ---------------------------------------------------------------------------
// 2. Sockets, read by a lexer.
// ---------------------------------------------------------------------------

/// AND THE WAY AROUND THE MANIFEST: a crate that wants a socket without a
/// dependency can simply use the standard library. Nothing in the manifests
/// would show it.
#[test]
fn no_crate_outside_the_monopoly_opens_a_raw_socket() {
    let offenders = socket_offenders(&crates_dir());
    assert!(
        offenders.is_empty(),
        "a crate outside the network monopoly opens a socket directly:\n  {}",
        offenders.join("\n  ")
    );
}

/// Every `file:line: Type` under `crates` where a crate outside `ALLOWED` names
/// a socket type in code.
fn socket_offenders(crates: &Path) -> Vec<String> {
    let mut offenders = Vec::new();
    for (name, manifest) in manifests_in(crates) {
        if ALLOWED.contains(&name.as_str()) {
            continue;
        }
        for file in scanned_files(&crates.join(&name), &manifest) {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            for t in rust_lex::tokens(&text) {
                if t.kind == rust_lex::Kind::Ident && CONNECTORS.contains(&t.text.as_str()) {
                    offenders.push(format!("{}:{}: {}", file.display(), t.line, t.text));
                }
            }
        }
    }
    offenders
}

/// The Rust files a crate holds: EVERY `.rs` below its directory, plus a build
/// script the manifest points elsewhere.
///
/// THE SCOPE WAS `src` UNTIL THAT WASN'T ENOUGH TO SAY SO, then `src`, `tests`,
/// `examples` and `benches` — and the audit of 9 Oct 2026 put a `TcpStream` in
/// `build.rs`, which sits at the crate root, outside all four, and runs on
/// every build of every user who compiles from source. A directory list is a
/// list of the places someone remembered. The crate directory is the place.
/// A non-compiled `.rs` fixture can only make this report MORE, which is the
/// safe direction: a false alarm a human reads, never a socket waved through.
///
/// This is also why the agent-session module (RG-33, planned under
/// `tacet-tools/src/sessions/`) needs no line here to be covered;
/// `a_new_sessions_module_is_scanned_without_being_listed` holds that to it.
fn scanned_files(crate_dir: &Path, manifest: &str) -> Vec<PathBuf> {
    let mut files = rust_files(crate_dir);
    // `[package] build = "tools/gen.rs"` — Cargo runs it, wherever it lives.
    if let Ok(rows) = toml_lite::flatten(manifest) {
        for (path, value) in rows {
            if let (["package", "build"], toml_lite::Value::Str(p)) = (
                path.iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .as_slice(),
                &value,
            ) {
                let script = crate_dir.join(p);
                if !files.contains(&script) {
                    files.push(script);
                }
            }
        }
    }
    files
}

/// Every `.rs` file under `dir`, recursively, skipping a build directory. A
/// directory that does not exist reads as empty, not as a failure.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name() != "target" {
                out.extend(rust_files(&path));
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// A Rust tokenizer, as far as telling CODE from TEXT needs.
///
/// THE PREVIOUS SPLIT WAS BY `"` ALONE and carried a comment saying the error
/// could only go one way — "make a line look like MORE code than it is". It
/// goes both ways. `let q = '"';` opens a "string" that the next real string's
/// opening quote closes, so everything between — `TcpStream::connect(` in the
/// audit's fake workspace — was read as TEXT and passed. An escaped `"\""` does
/// the same. Quotes are only known by knowing which ones are inside a char
/// literal, a raw string or a comment, which is lexing; so this lexes.
mod rust_lex {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Kind {
        Ident,
        /// A string literal of any flavour, with its (unescaped enough) text.
        Str,
        Punct,
        Other,
    }

    #[derive(Debug, Clone)]
    pub struct Token {
        pub kind: Kind,
        pub text: String,
        pub line: usize,
    }

    pub fn tokens(src: &str) -> Vec<Token> {
        let s: Vec<char> = src.chars().collect();
        let mut i = 0;
        let mut line = 1;
        let mut out = Vec::new();
        let count_lines =
            |a: usize, b: usize, s: &[char]| s[a..b].iter().filter(|&&c| c == '\n').count();
        while i < s.len() {
            let c = s[i];
            let start = i;
            let start_line = line;
            if c.is_whitespace() {
                i += 1;
            } else if c == '/' && s.get(i + 1) == Some(&'/') {
                while i < s.len() && s[i] != '\n' {
                    i += 1;
                }
            } else if c == '/' && s.get(i + 1) == Some(&'*') {
                // Block comments NEST in Rust: `/* /* */ TcpStream */` is all comment.
                let mut depth = 0usize;
                while i < s.len() {
                    if s[i] == '/' && s.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if s[i] == '*' && s.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            } else if let Some((end, text)) = raw_string(&s, i) {
                i = end;
                out.push(Token {
                    kind: Kind::Str,
                    text,
                    line: start_line,
                });
            } else if c == '"' || (matches!(c, 'b' | 'c') && s.get(i + 1) == Some(&'"')) {
                i += if c == '"' { 1 } else { 2 };
                let mut text = String::new();
                while i < s.len() && s[i] != '"' {
                    if s[i] == '\\' {
                        i += 1;
                    }
                    if let Some(&ch) = s.get(i) {
                        text.push(ch);
                    }
                    i += 1;
                }
                i += 1;
                out.push(Token {
                    kind: Kind::Str,
                    text,
                    line: start_line,
                });
            } else if c == '\'' || (c == 'b' && s.get(i + 1) == Some(&'\'')) {
                let q = if c == '\'' { i } else { i + 1 };
                if s.get(q + 1) == Some(&'\\') {
                    // An escaped char: '\'' '\\' '\u{1F600}' — runs to the next
                    // unescaped quote.
                    i = q + 3;
                    while i < s.len() && s[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                    out.push(Token {
                        kind: Kind::Other,
                        text: String::new(),
                        line: start_line,
                    });
                } else if s.get(q + 2) == Some(&'\'') {
                    // 'x', and the case that broke the old split: '"'.
                    i = q + 3;
                    out.push(Token {
                        kind: Kind::Other,
                        text: String::new(),
                        line: start_line,
                    });
                } else {
                    // A lifetime or a loop label: `'a`, `'static`, `'outer:`.
                    i = q + 1;
                    while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_') {
                        i += 1;
                    }
                    out.push(Token {
                        kind: Kind::Other,
                        text: String::new(),
                        line: start_line,
                    });
                }
            } else if c.is_alphabetic() || c == '_' {
                // `r#TcpStream` is the identifier `TcpStream`.
                if c == 'r' && s.get(i + 1) == Some(&'#') {
                    i += 2;
                }
                let from = i;
                while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_') {
                    i += 1;
                }
                out.push(Token {
                    kind: Kind::Ident,
                    text: s[from..i].iter().collect(),
                    line: start_line,
                });
            } else if c.is_ascii_digit() {
                while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_') {
                    i += 1;
                }
                out.push(Token {
                    kind: Kind::Other,
                    text: String::new(),
                    line: start_line,
                });
            } else {
                i += 1;
                out.push(Token {
                    kind: Kind::Punct,
                    text: c.to_string(),
                    line: start_line,
                });
            }
            line += count_lines(start, i.min(s.len()), &s);
        }
        out
    }

    /// `r"…"`, `r#"…"#`, `br##"…"##`, `cr"…"` — no escapes, ends at the quote
    /// followed by as many `#` as opened it. `None` when `i` does not start one
    /// (`r#ident` is a raw identifier, not a string).
    fn raw_string(s: &[char], i: usize) -> Option<(usize, String)> {
        let mut j = i;
        if matches!(s.get(j), Some('b' | 'c')) {
            j += 1;
        }
        if s.get(j) != Some(&'r') {
            return None;
        }
        j += 1;
        let mut hashes = 0;
        while s.get(j) == Some(&'#') {
            hashes += 1;
            j += 1;
        }
        if s.get(j) != Some(&'"') {
            return None;
        }
        j += 1;
        let from = j;
        while j < s.len() {
            if s[j] == '"' && (1..=hashes).all(|k| s.get(j + k) == Some(&'#')) {
                return Some((j + 1 + hashes, s[from..j].iter().collect()));
            }
            j += 1;
        }
        Some((s.len(), s[from..].iter().collect()))
    }
}

// ---------------------------------------------------------------------------
// 3. Agent CLIs, spawned only behind the approval gate.
// ---------------------------------------------------------------------------

/// RG-33. Reading another agent's session is a local file read and needs
/// nothing here. Writing BACK by running `claude -p --resume`, `codex exec
/// resume`, `codex app-server` or `agy -p` hands the conversation to a hosted
/// model and costs a paid turn — measured at ~$0.2–0.6 a call in the R&D brief —
/// so every such path is an external tool, off by default, behind the approval
/// gate. This holds the code to that: a `Command::new("claude")` in a file not
/// listed in `AGENT_CLI_SPAWNERS`, or listed under a tool that is not in
/// tacet-cli's `EXTERNAL_TOOLS`, is red.
///
/// It sees a LITERAL program name. A name assembled at run time passes, and
/// that is the honest limit of a source scan; the review that adds the spawn is
/// what this cannot replace.
#[test]
fn agent_clis_are_spawned_only_behind_the_approval_gate() {
    let main = std::fs::read_to_string(crates_dir().join("tacet-cli/src/main.rs"))
        .expect("tacet-cli's main.rs is readable");
    let external = external_tools(&main);
    assert!(
        external.contains(&"web_search".to_string()),
        "EXTERNAL_TOOLS was not found in main.rs (read {external:?}); this check would be vacuous"
    );
    let problems = agent_spawn_problems(&crates_dir(), AGENT_CLI_SPAWNERS, &external);
    assert!(
        problems.is_empty(),
        "an agent CLI is spawned outside the approval gate:\n  {}",
        problems.join("\n  ")
    );
}

/// The string literals of `const EXTERNAL_TOOLS: ... = &[ ... ];`.
fn external_tools(main_rs: &str) -> Vec<String> {
    let toks = rust_lex::tokens(main_rs);
    let Some(at) = toks
        .windows(2)
        .position(|w| w[0].text == "const" && w[1].text == "EXTERNAL_TOOLS")
    else {
        return Vec::new();
    };
    // Skip the type (`&[&str]`) to the `=`, then read to the `;`.
    toks[at..]
        .iter()
        .skip_while(|t| t.text != "=")
        .take_while(|t| t.text != ";")
        .filter(|t| t.kind == rust_lex::Kind::Str)
        .map(|t| t.text.clone())
        .collect()
}

/// Every `Command::new("<agent cli>")` outside `ALLOWED` that is not covered
/// by a `(file suffix, tool)` row in `spawners` whose tool is in `external`.
fn agent_spawn_problems(
    crates: &Path,
    spawners: &[(&str, &str)],
    external: &[String],
) -> Vec<String> {
    let mut problems = Vec::new();
    for (file_suffix, tool) in spawners {
        if !external.iter().any(|e| e == tool) {
            problems.push(format!(
                "{file_suffix} runs as `{tool}`, which is not in EXTERNAL_TOOLS"
            ));
        }
    }
    for (name, manifest) in manifests_in(crates) {
        if ALLOWED.contains(&name.as_str()) {
            continue;
        }
        for file in scanned_files(&crates.join(&name), &manifest) {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            let toks = rust_lex::tokens(&text);
            for w in toks.windows(6) {
                let program = match w {
                    [a, b, c, d, e, f]
                        if a.text == "Command"
                            && b.text == ":"
                            && c.text == ":"
                            && d.text == "new"
                            && e.text == "("
                            && f.kind == rust_lex::Kind::Str =>
                    {
                        f
                    }
                    _ => continue,
                };
                let base = Path::new(&program.text)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if !AGENT_CLIS.contains(&base.as_str()) {
                    continue;
                }
                let shown = file.to_string_lossy().replace('\\', "/");
                let listed = spawners.iter().any(|(suffix, _)| shown.ends_with(suffix));
                if !listed {
                    problems.push(format!(
                        "{}:{}: spawns `{}` and is not an external tool",
                        file.display(),
                        program.line,
                        program.text
                    ));
                }
            }
        }
    }
    problems
}

// ---------------------------------------------------------------------------
// The guards, broken on purpose. Each bypass from the audit goes in; red must
// come out. A guard that has never been seen to fail has not been tested.
// ---------------------------------------------------------------------------

fn declarers_of(manifest: &str) -> Vec<String> {
    let found = http_declarers("", &[("fake".to_string(), manifest.to_string())]);
    found.get("fake").cloned().unwrap_or_default()
}

#[test]
fn a_renamed_http_client_is_still_an_http_client() {
    // T4: `net = { package = "ureq" }` — the key is the alias, not the crate.
    assert_eq!(
        declarers_of("[dependencies]\nnet = { package = \"ureq\", version = \"3\" }\n"),
        ["ureq"]
    );
    // Same rename, written as a table.
    assert_eq!(
        declarers_of("[dependencies.net]\npackage = \"reqwest\"\nversion = \"0.12\"\n"),
        ["reqwest"]
    );
    // And as a dotted key with a quoted segment.
    assert_eq!(
        declarers_of("[dependencies]\n\"net\".package = 'hyper'\n"),
        ["hyper"]
    );
}

#[test]
fn every_toml_spelling_of_a_dependency_is_a_dependency() {
    // T4: the dotted key and the table header the line-start check missed.
    assert_eq!(
        declarers_of("[dependencies]\nureq.version = \"3\"\n"),
        ["ureq"]
    );
    assert_eq!(
        declarers_of("[dependencies.ureq]\nversion = \"3\"\n"),
        ["ureq"]
    );
    assert_eq!(declarers_of("[dependencies.ureq]\n"), ["ureq"]);
    assert_eq!(declarers_of("dependencies.ureq = \"3\"\n"), ["ureq"]);
    assert_eq!(declarers_of("[dependencies]\n  ureq={}\n"), ["ureq"]);
    // Target-specific, with the cfg quoted both ways.
    assert_eq!(
        declarers_of("[target.'cfg(unix)'.dependencies]\nureq = \"3\"\n"),
        ["ureq"]
    );
    assert_eq!(
        declarers_of("[target.\"cfg(windows)\".dev-dependencies.isahc]\nversion = \"1\"\n"),
        ["isahc"]
    );
    assert_eq!(
        declarers_of("[build-dependencies]\ncurl = \"0.4\"\n"),
        ["curl"]
    );
    // A multi-line inline value does not hide the next key.
    assert_eq!(
        declarers_of(
            "[dependencies]\nx = { version = \"1\", features = [\n  \"a\", # c\n  \"b\",\n] }\nsocket2 = \"0.5\"\n"
        ),
        ["socket2"]
    );
}

#[test]
fn an_inherited_http_client_is_charged_to_the_crate_and_the_root() {
    let root = "[workspace.dependencies]\nnet = { package = \"ureq\", version = \"3\" }\n";
    let member = "[dependencies]\nnet.workspace = true\n";
    let found = http_declarers(root, &[("fake".into(), member.into())]);
    assert_eq!(
        found.get("fake").map(Vec::as_slice),
        Some(&["ureq".to_string()][..])
    );
    assert!(found.contains_key("<workspace>"), "{found:?}");
}

#[test]
fn what_is_not_a_dependency_is_not_reported() {
    // The comments that made the README's grep print nine lines.
    assert!(declarers_of("[dependencies]\n# ureq = \"3\"\nserde = \"1\" # not ureq\n").is_empty());
    // Strings and other tables that merely NAME a client.
    assert!(declarers_of("[package]\nname = \"ureq\"\ndescription = \"ureq = 3\"\n").is_empty());
    assert!(declarers_of("[features]\nureq = []\nnet = [\"dep:ureq\"]\n").is_empty());
    assert!(declarers_of("[patch.crates-io]\nureq = { path = \"x\" }\n").is_empty());
    assert!(declarers_of("[package.metadata.dependencies]\nureq = \"3\"\n").is_empty());
    // A multi-line string whose text looks like a table header.
    assert!(
        declarers_of("[package]\ndescription = \"\"\"\n[dependencies]\nureq = \"3\"\n\"\"\"\n")
            .is_empty()
    );
}

#[test]
fn an_unreadable_manifest_is_an_error_not_a_pass() {
    assert!(dependencies("[dependencies\nureq = \"3\"\n").is_err());
    assert!(dependencies("[dependencies]\nureq = \"3\n").is_err());
    assert!(dependencies("[dependencies]\nureq \"3\"\n").is_err());
}

fn socket_lines(src: &str) -> Vec<usize> {
    rust_lex::tokens(src)
        .into_iter()
        .filter(|t| t.kind == rust_lex::Kind::Ident && CONNECTORS.contains(&t.text.as_str()))
        .map(|t| t.line)
        .collect()
}

#[test]
fn a_quote_inside_a_char_literal_does_not_hide_a_socket() {
    // T4, verbatim: the old `"`-split read the char's quote as the opening of a
    // string and everything up to the next real quote as text.
    assert_eq!(
        socket_lines("let _q = '\"';\nlet s = std::net::TcpStream::connect(\"127.0.0.1:1\");\n"),
        [2]
    );
    assert_eq!(
        socket_lines("let q = \"\\\"\"; let l = TcpListener::bind(\"x\");"),
        [1]
    );
    assert_eq!(socket_lines("let q = b'\"'; UdpSocket::bind(\"x\");"), [1]);
    assert_eq!(
        socket_lines("let q = '\\''; let r = '\\\\'; TcpStream::connect(a);"),
        [1]
    );
    // Lifetimes look like an unclosed char literal and must not eat the line.
    assert_eq!(
        socket_lines("fn f<'a>(x: &'a str) -> TcpStream { TcpStream::connect(x) }"),
        [1, 1]
    );
    assert_eq!(socket_lines("let s = r#TcpStream::connect(a);"), [1]);
}

#[test]
fn text_that_names_a_socket_is_not_a_socket() {
    assert!(socket_lines("// TcpStream::connect\n/// TcpStream\n").is_empty());
    assert!(socket_lines("/* outer /* TcpStream */ still comment TcpListener */").is_empty());
    assert!(socket_lines("let s = \"TcpStream \\\" TcpStream\";").is_empty());
    assert!(socket_lines("let s = r#\"a \" TcpStream \"#; let t = br\"UdpSocket\";").is_empty());
    assert!(socket_lines("let s = c\"TcpStream\"; let i: IpAddr = x;").is_empty());
    // The line numbers survive multi-line strings and comments.
    assert_eq!(
        socket_lines("/*\n\n*/\nlet s = \"\n\n\";\nTcpStream::connect(x);"),
        [7]
    );
}

/// A throwaway workspace under the system temp dir. Removed on drop.
struct FakeWorkspace(PathBuf);

impl FakeWorkspace {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "tacet-netmono-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        FakeWorkspace(dir)
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).expect("fixture dir");
        std::fs::write(p, text).expect("fixture file");
    }
}

impl Drop for FakeWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const SOCKET_RS: &str = "pub fn f() { let _ = std::net::TcpStream::connect(\"127.0.0.1:9\"); }\n";

#[test]
fn a_socket_in_build_rs_is_a_socket() {
    let ws = FakeWorkspace::new("buildrs");
    ws.write("tacet-x/Cargo.toml", "[package]\nname = \"tacet-x\"\n");
    ws.write("tacet-x/src/lib.rs", "pub fn clean() {}\n");
    // T4: the crate root, outside src/tests/examples/benches.
    ws.write("tacet-x/build.rs", SOCKET_RS);
    // A build script the manifest moved out of the crate directory.
    ws.write(
        "tacet-y/Cargo.toml",
        "[package]\nname = \"tacet-y\"\nbuild = \"../shared/gen.rs\"\n",
    );
    ws.write("shared/gen.rs", SOCKET_RS);
    // An allowed crate is allowed.
    ws.write("tacet-web/Cargo.toml", "[package]\nname = \"tacet-web\"\n");
    ws.write("tacet-web/build.rs", SOCKET_RS);
    let offenders = socket_offenders(&ws.0);
    assert_eq!(offenders.len(), 2, "{offenders:#?}");
    assert!(
        offenders
            .iter()
            .any(|o| o.contains("tacet-x") && o.contains("build.rs"))
    );
    assert!(offenders.iter().any(|o| o.contains("gen.rs")));
}

#[test]
fn a_new_sessions_module_is_scanned_without_being_listed() {
    // RG-33: the agent-session readers land in `tacet-tools/src/sessions/`. A
    // socket there must be red the day the directory appears, with nobody
    // remembering to add it to a list.
    let ws = FakeWorkspace::new("sessions");
    ws.write(
        "tacet-tools/Cargo.toml",
        "[package]\nname = \"tacet-tools\"\n",
    );
    ws.write("tacet-tools/src/lib.rs", "pub mod sessions;\n");
    ws.write("tacet-tools/src/sessions/mod.rs", "pub mod claude;\n");
    ws.write("tacet-tools/src/sessions/claude.rs", SOCKET_RS);
    let offenders = socket_offenders(&ws.0);
    assert!(
        offenders
            .iter()
            .any(|o| o.contains("sessions") && o.contains("claude.rs")),
        "{offenders:#?}"
    );

    // And on the real tree: tacet-tools is not part of the monopoly, and every
    // file under its `src/sessions/` — when there is one — is in the scan.
    assert!(!ALLOWED.contains(&"tacet-tools"));
    let tools = crates_dir().join("tacet-tools");
    let manifest = std::fs::read_to_string(tools.join("Cargo.toml")).expect("tacet-tools manifest");
    let scanned = scanned_files(&tools, &manifest);
    assert!(scanned.contains(&tools.join("src/lib.rs")));
    for file in rust_files(&tools.join("src/sessions")) {
        assert!(scanned.contains(&file), "{} is not scanned", file.display());
    }
}

#[test]
fn an_agent_cli_outside_the_gate_is_red() {
    let ws = FakeWorkspace::new("agentcli");
    ws.write(
        "tacet-tools/Cargo.toml",
        "[package]\nname = \"tacet-tools\"\n",
    );
    ws.write(
        "tacet-tools/src/sessions/claude.rs",
        "fn resume() { let _ = std::process::Command::new(\"claude\").args([\"-p\"]); }\n",
    );
    ws.write(
        "tacet-tools/src/sessions/codex.rs",
        "use std::process::Command;\nfn serve() { Command::new(\"/opt/bin/codex\").arg(\"app-server\"); }\n",
    );
    ws.write(
        "tacet-tools/src/git.rs",
        "fn g() { Command::new(\"git\"); } // Command::new(\"agy\")\n",
    );
    let external = vec!["web_search".to_string(), "session_resume".to_string()];

    let unlisted = agent_spawn_problems(&ws.0, &[], &external);
    assert_eq!(unlisted.len(), 2, "{unlisted:#?}");
    assert!(unlisted.iter().any(|p| p.contains("claude.rs:1")));
    assert!(unlisted.iter().any(|p| p.contains("codex.rs:2")));

    let listed = [
        ("sessions/claude.rs", "session_resume"),
        ("sessions/codex.rs", "session_resume"),
    ];
    assert!(agent_spawn_problems(&ws.0, &listed, &external).is_empty());

    // Listed, but under a tool the approval gate does not know.
    let ungated = [
        ("sessions/claude.rs", "session_resume"),
        ("sessions/codex.rs", "codex_bridge"),
    ];
    let problems = agent_spawn_problems(&ws.0, &ungated, &external);
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(problems[0].contains("codex_bridge"));
}

#[test]
fn external_tools_are_read_from_the_const_not_from_anywhere() {
    let src = "// const EXTERNAL_TOOLS = [\"comment\"];\nconst OTHER: &[&str] = &[\"x\"];\nconst EXTERNAL_TOOLS: &[&str] = &[\"web_search\", \"http\"];\nconst AFTER: &str = \"y\";\n";
    assert_eq!(external_tools(src), ["web_search", "http"]);
}

#[test]
fn the_graph_walk_finds_a_client_one_crate_removed() {
    let meta = serde_json::json!({
        "packages": [
            {"id": "cli", "name": "tacet-cli"},
            {"id": "tools", "name": "tacet-tools"},
            {"id": "web", "name": "tacet-web"},
            {"id": "fetch", "name": "fancy-fetch"},
            {"id": "ureq", "name": "ureq"},
            {"id": "rq", "name": "reqwest"},
            {"id": "s2", "name": "socket2"},
            {"id": "serde", "name": "serde"},
        ],
        "workspace_members": ["cli", "tools", "web"],
        "resolve": {"nodes": [
            {"id": "cli", "deps": [{"pkg": "web"}, {"pkg": "tools"}, {"pkg": "serde"}]},
            {"id": "tools", "deps": [{"pkg": "fetch"}]},
            {"id": "web", "deps": [{"pkg": "ureq"}, {"pkg": "s2"}]},
            {"id": "fetch", "deps": [{"pkg": "rq"}]},
            {"id": "ureq", "deps": []},
            {"id": "rq", "deps": []},
            {"id": "s2", "deps": []},
            {"id": "serde", "deps": []},
        ]},
    });
    // tacet-tools -> fancy-fetch -> reqwest is red, from both members that
    // reach it; tacet-web's own ureq and socket2 are not charged to tacet-cli.
    assert_eq!(
        graph_offenders(&meta),
        [
            "tacet-cli -> tacet-tools -> fancy-fetch -> reqwest",
            "tacet-tools -> fancy-fetch -> reqwest",
        ]
    );
}
