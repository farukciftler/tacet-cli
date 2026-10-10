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
//! 2. No crate reaches a socket some other way — no raw `std::net` connector,
//!    no name resolution (`to_socket_addrs`), no socket-layer crate (`libc`,
//!    `nix`, `rustix`, the Windows bindings) declared under any name or
//!    reached with its socket feature on, no libc socket call reached through
//!    one of them under any name the file gives it, and no foreign function
//!    outside a three-name allow-list (`EXTERN_FNS`), in any file a
//!    crate COMPILES (its whole directory, `build.rs`, and whatever `#[path]`,
//!    `include!` and the manifest's target paths point rustc at), read by a
//!    lexer that decodes string literals the way rustc does; every crate cargo
//!    builds from this repository has to live under `crates/`, where these
//!    scans look (checked against `cargo metadata`); and no program
//!    whose job is the network (`docker`, `curl`, `ssh`, …) started outside the
//!    two crates except by a row that says why — today one: the y/N-approved
//!    local SearXNG setup in `tacet-cli/src/addon.rs`.
//! 3. No crate — the two that own a socket included — names another agent's
//!    CLI (`claude -p`, `codex app-server`, `agy -p`) as a program without that
//!    path being an external tool behind the approval gate: those programs ARE
//!    the network, one process away, and each call is a paid turn. A program
//!    name built at run time is the one thing a source scan cannot read; every
//!    file that spawns one is a reviewed row (`RUNTIME_SPAWN_SITES`).
//!
//! THE FIRST VERSION OF THIS FILE PARSED NOTHING, on the theory that a TOML
//! parser to police a zero-dependency rule would be its own joke. The audit of
//! 9 Oct 2026 (T4) built a fake workspace and walked five shapes straight past
//! it, every one green: `net = { package = "ureq" }`, `ureq.version = "3"`,
//! `[dependencies.ureq]`, a `TcpStream::connect` after the char literal `'"'`,
//! and a `TcpStream` in `build.rs`. A guard that a rename defeats guards the
//! spelling, not the property. The parser below is still hand-written — the
//! rule it polices forbids the crate — but it is a parser: it knows tables,
//! dotted keys, inline tables, quoted keys and all four TOML string forms,
//! escapes and line-ending backslashes included. Each of the five bypasses has
//! a self-test at the bottom of this file that feeds it in and expects red —
//! and so does each of the nine the review of 10 Oct 2026 then found in the
//! rewrite (multi-line strings, `#[path]`, `include!`, a `Command` alias, `npx`
//! and `sh -c` launchers, a spawn inside `tacet-mcp`), and each of the five
//! its second pass found after that (a program name written with a Rust escape,
//! a TOML `\x` escape, an `extern "C"` socket, DNS, a crate outside `crates/`),
//! and each of the six its third pass found after that (`use libc as c`,
//! `rustix::net`, a `link_name` built by `concat!`, an extern block written by
//! a macro, `Command::new("cla".to_owned() + "ude")`, a `type` alias).

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

/// Name resolution: `"host:80".to_socket_addrs()` sends a DNS query, and a query
/// for `<data>.attacker.example` carries data out without a socket of the
/// crate's own. `lookup_host` is tokio's spelling of the same call.
const RESOLVERS: &[&str] = &["ToSocketAddrs", "to_socket_addrs", "lookup_host"];

/// The C library's socket calls. `std` already links libc, so `unsafe extern
/// "C" { fn socket(..) -> i32; fn connect(..) -> i32; }` is a socket with no
/// dependency and no `std::net` — the review of 10 Oct 2026 wrote exactly that
/// and `socket_offenders` returned nothing. They count when DECLARED in an
/// `extern` block (as a function or a `#[link_name]`) or called through a
/// `libc::` or `nix::` path; `bind` and `connect` are too common as method names
/// to count anywhere else.
///
/// `syscall`, `dlsym` and `dlopen` are here because they defeat the name:
/// `syscall(SYS_connect, …)` and `dlsym(…, "connect")` reach the same call with
/// the word only in a number or a string. Outside the two crates there is no
/// reason to declare them, so declaring one is red too. tacet-tools' two extern
/// blocks (`setsid`, `killpg`) are the only ones in the tree (10 Oct 2026).
const SOCKET_FFI: &[&str] = &[
    "socket",
    "socketpair",
    "connect",
    "bind",
    "listen",
    "accept",
    "accept4",
    "send",
    "sendto",
    "sendmsg",
    "recvfrom",
    "getaddrinfo",
    "gethostbyname",
    "gethostbyname2",
    "res_query",
    // Winsock, under `extern "system"`; `socket`/`connect` above are its
    // BSD-named entry points too.
    "WSAStartup",
    "WSASocketA",
    "WSASocketW",
    "WSAConnect",
    "GetAddrInfoW",
    "syscall",
    "dlsym",
    "dlopen",
];

/// The crates that put a socket call within reach of a crate's own code without
/// any HTTP client: the C library's bindings (`libc`, `nix`, `rustix`) and the
/// Windows ones (`windows-sys`, `winapi`, `windows`, which carry Winsock).
///
/// WHY A DECLARATION CHECK AND NOT A PATH CHECK. `raw_network` used to count a
/// `SOCKET_FFI` name only behind a path whose root was literally `libc` or
/// `nix`. The review of 10 Oct 2026 compiled three ways past that into
/// tacet-tools and kept the suite green: `use libc as c; c::socket(2, 1, 0)`, a
/// manifest rename, and `rustix::net::connect`, rustix being a crate the
/// scan did not know at all although it is already in Cargo.lock. A second
/// name for the same crate is the T4 bypass again, one layer down. No crate
/// outside the monopoly declares any of these today (10 Oct 2026), so the
/// declaration is the line: declaring one is red unless `SOCKET_LAYER_ROWS`
/// says why, and `dependencies()` already sees through `package =` renames.
const SOCKET_LAYER_CRATES: &[&str] = &["libc", "nix", "rustix", "windows-sys", "winapi", "windows"];

/// `(crate directory, socket-layer package, why it is safe there)`. Empty.
const SOCKET_LAYER_ROWS: &[(&str, &str, &str)] = &[];

/// The functions a crate outside the monopoly may declare in an `extern` block.
///
/// AN ALLOW-LIST, BECAUSE A DENY-LIST OF NAMES CAN BE WALKED AROUND WITHOUT
/// WRITING THE NAME. The review of 10 Oct 2026 compiled two such shapes with
/// rustc on macOS and got a socket fd and a `connect` from each, while
/// `raw_network` returned nothing: `#[link_name = concat!("soc", "ket")] fn
/// tidy(..)`, and an extern block written by `macro_rules!` with the name as
/// `$n`. So every foreign function is named here or is red, a `link_name` must
/// be one string literal, and an extern block a macro fills in is red.
/// `setsid`/`killpg` are tacet-tools' process-group calls (shell.rs,
/// run_code.rs); `poll` is tacet-cli's piped-stdin check (main.rs);
/// `signal`/`raise` are tacet-cli's SIGINT-cancels-the-turn handler (chat.rs,
/// audit M4).
const EXTERN_FNS: &[&str] = &["setsid", "killpg", "poll", "signal", "raise"];

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

/// The npm packages the agent CLIs ship as. `npx @anthropic-ai/claude-code -p`
/// runs `claude` without the word `claude` ever being a program name.
const AGENT_CLI_PACKAGES: &[&str] = &[
    "@anthropic-ai/claude-code",
    "@openai/codex",
    "@google/gemini-cli",
];

/// `(file suffix, exact literal)`: a literal whose first word is a watched
/// program's name and which runs nothing — prose, or a source label such as
/// the session readers' `"codex"`. The row is the reviewer's sentence that the
/// literal never reaches a spawn; it covers that literal in that file and
/// nothing else. Program names are matched without case (macOS's default file
/// system runs `Docker` as `docker`), which is what puts prose here.
const NAMED_NOT_RUN: &[(&str, &str)] = &[
    // The routing eval's Turkish tool descriptions; tacet-eval spawns nothing.
    (
        "tacet-eval/src/routing.rs",
        "Docker konteynerlerini listeler.",
    ),
    (
        "tacet-eval/src/routing.rs",
        "Docker konteynerini baslatir veya durdurur.",
    ),
    (
        "tacet-eval/src/routing.rs",
        "Docker compose yiginini yonetir.",
    ),
    // The agent-session ground (RG-01..05): `AgentKind`'s slug, the name its
    // handoff files are written under, and its display name. Labels for a
    // source, read by nothing that starts a process; the readers open local
    // files only (`sessions::secrets`), and a write-back path would need a row
    // in AGENT_CLI_SPAWNERS, not here.
    ("tacet-tools/src/sessions/model.rs", "claude"),
    ("tacet-tools/src/sessions/model.rs", "codex"),
    ("tacet-tools/src/sessions/model.rs", "antigravity"),
    ("tacet-tools/src/sessions/model.rs", "Claude Code"),
    ("tacet-tools/src/sessions/model.rs", "Codex CLI"),
    ("tacet-tools/src/sessions/model.rs", "Antigravity"),
    // The Antigravity reader (RG-14/15): the app's data directory under
    // `~/.gemini/` is named `antigravity`, the same word as the IDE launcher;
    // the reader lists it with `read_dir` and opens files in it through
    // `sessions::secrets`, and spawns nothing. The other two literals are a
    // test's expected root and the integration test's fixture directory.
    ("tacet-tools/src/sessions/antigravity.rs", "antigravity"),
    (
        "tacet-tools/src/sessions/antigravity.rs",
        "/home/u/.gemini/antigravity",
    ),
    (
        "tacet-tools/tests/an_antigravity_session_is_read_from_its_log.rs",
        "tests/fixtures/sessions/antigravity",
    ),
    // The router's Sessions profile (RG-19): the agents' product names as
    // words a user types ("continue my claude code session", "codex'te
    // kaldığım yer"), matched against the message and run by nothing — the
    // router scores text; the tools it lifts read local files.
    ("tacet-tools/src/router.rs", "claude code"),
    ("tacet-tools/src/router.rs", "codex"),
    ("tacet-tools/src/router.rs", "antigravity"),
    // The write gate's refusal text, and a version string `claude --version`
    // prints, fed to the parser in a test as a thing it must refuse.
    (
        "tacet-tools/src/sessions/version.rs",
        "Antigravity storage is read-only to Tacet; only task.md may be written, behind the experimental flag",
    ),
    (
        "tacet-tools/src/sessions/version.rs",
        "2.1.285 (Claude Code)",
    ),
    // The `task.md` writer's liveness probe (RG-26, RG-31): the file name of
    // the Antigravity CLI, looked for in `/bin/ps -A -o comm=` output so a
    // running `agy` stops the write. Compared, never run.
    ("tacet-tools/src/sessions/writeback/antigravity.rs", "agy"),
];

/// Programs whose job IS the network. A program that merely CAN reach it with
/// one subcommand among many local ones (`git fetch`, `cargo install`, `npm
/// i`) is not here: listing it would make every `git rev-parse` a row, and the
/// list would be read as noise. Those, and every user-allowlisted program, are
/// what `shell` runs — itself an external tool.
const NETWORK_PROGRAMS: &[&str] = &[
    "docker",
    "docker-compose",
    "podman",
    "curl",
    "wget",
    "ssh",
    "scp",
    "sftp",
    "rsync",
    "nc",
    "ncat",
    "netcat",
    "socat",
    "telnet",
    "ftp",
];

/// `(file suffix, program, why it is safe there)`.
const NETWORK_PROGRAM_SPAWNERS: &[(&str, &str, &str)] = &[
    (
        "tacet-cli/src/addon.rs",
        "docker",
        "`tacet addon` sets up a local SearXNG: it prints the image, files, command and \
         bind address, then waits on `take_approval` (y/N) before `docker compose up -d` \
         pulls anything; `docker --version` / `compose version` before that are local probes",
    ),
    (
        "tacet-cli/src/addon.rs",
        "docker-compose",
        "the compose v1 fallback of the same approved setup",
    ),
];

/// `(file suffix, where the program name comes from)`: the files that call
/// `Command::new` with something other than a literal.
const RUNTIME_SPAWN_SITES: &[(&str, &str)] = &[
    (
        "tacet-tools/src/shell.rs",
        "a program the user allowlisted at addon install; `shell` is in EXTERNAL_TOOLS",
    ),
    (
        "tacet-tools/src/db.rs",
        "the sqlite3 binary `find_binary` resolved",
    ),
    ("tacet-tools/src/db_write.rs", "the same sqlite3 binary"),
    (
        "tacet-tools/src/git.rs",
        "the git binary, three local-only subcommands",
    ),
    (
        "tacet-tools/src/clipboard.rs",
        "pbcopy/pbpaste, xclip and friends from a fixed backend table",
    ),
    (
        "tacet-tools/src/run_code.rs",
        "sandbox-exec or the interpreter, inside the network shield",
    ),
    (
        "tacet-tools/src/time.rs",
        "the fixed `offset_candidates` list",
    ),
    (
        "tacet-cli/src/bench_cmd.rs",
        "`shell_line`, called with literals the name scans read",
    ),
    (
        "tacet-cli/src/update.rs",
        "the tacet binary just downloaded, asked `--print-features`",
    ),
    (
        THIS_FILE,
        "`$CARGO`, for `cargo metadata --offline` in the graph check",
    ),
    (
        "tacet-cli/src/addon.rs",
        "the compose command `compose_command` built from the literals `docker`/`docker-compose`",
    ),
    (
        "tacet-tools/src/write_code.rs",
        "the python interpreter `discover` resolved, asked `find_spec` for a name (audit M3); \
         its tests use a stand-in from a temp dir and fixed python paths",
    ),
    (
        "tacet-tools/src/calendar.rs",
        "a macOS-only test: the fixed path `/usr/bin/osascript`, pure date arithmetic",
    ),
    (
        "tacet-cli/tests/the_nightly_script_names_real_flags.rs",
        "a test helper called with the literals `git` and `bash`, inside a temp repository",
    ),
    (
        "tacet-cli/tests/the_shell_budgets_its_tools.rs",
        "`env!(\"CARGO_BIN_EXE_tacet\")`, the binary under test, on the scripted engine",
    ),
    (
        "tacet-cli/tests/a_session_from_another_agent_is_continued.rs",
        "`env!(\"CARGO_BIN_EXE_tacet\")`, the binary under test, on the scripted engine \
         (RG-20 `--from`); no agent program is run, the fixtures are files",
    ),
    (
        "tacet-cli/tests/consent_reaches_the_person.rs",
        "`env!(\"CARGO_BIN_EXE_tacet\")`, the binary under test, on the scripted engine — \
         directly, and under the literal `/usr/bin/script` for a pty (audit H1)",
    ),
    (
        "tacet-cli/tests/the_session_keeps_its_promises.rs",
        "`env!(\"CARGO_BIN_EXE_tacet\")`, the binary under test, on the scripted engine \
         (audit H2-b, L29, L27, D7)",
    ),
    (
        "tacet-cli/tests/ctrl_c_reaches_the_running_tool.rs",
        "the binary under test on the scripted engine, and the literals `kill -INT` and \
         `pgrep -f` aimed at it and its `sleep` (audit M4)",
    ),
    (
        "tacet-tools/src/sessions/writeback/codex.rs",
        "the Codex binary the USER names with `tacet handoff note codex --program <path>` \
         (a bare name is refused, never looked up on PATH; the path is resolved and the \
         resolved file is what runs), run as `app-server` only after the y/N question that \
         names that file and says it can reach the network (RG-23); no model turn is started. Not a \
         model tool: no AGENT_CLI_SPAWNERS row, because the model cannot reach this path. Its \
         tests run a stand-in shell script from a temp directory",
    ),
    (
        "tacet-cli/tests/the_handoff_hook_answers_its_agent.rs",
        "`env!(\"CARGO_BIN_EXE_tacet\")`, the binary under test, as `tacet handoff hook` \
         with a hook event on stdin (RG-21/23/25); no agent program is run",
    ),
];

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

/// See `SOCKET_LAYER_CRATES`. Read the same way as the HTTP clients: as TOML,
/// renames and workspace inheritance included.
#[test]
fn no_crate_outside_the_monopoly_declares_a_socket_layer() {
    let root = std::fs::read_to_string(workspace_root().join("Cargo.toml"))
        .expect("the workspace manifest is readable");
    let found = socket_layer_declarers(&root, &manifests());
    assert!(
        found.is_empty(),
        "a crate outside the network monopoly declares a crate that binds the C or \
         Windows socket calls; add a SOCKET_LAYER_ROWS entry that says why, or use \
         tacet-web / tacet-mcp:\n  {}",
        found.join("\n  ")
    );
}

/// `crate: package` for every socket-layer crate a manifest outside `ALLOWED`
/// declares (under any name) and no `SOCKET_LAYER_ROWS` entry covers.
fn socket_layer_declarers(root: &str, manifests: &[(String, String)]) -> Vec<String> {
    let root_deps = dependencies(root).unwrap_or_else(|e| panic!("<workspace>/Cargo.toml: {e}"));
    let inherited: BTreeMap<String, String> = root_deps
        .iter()
        .filter(|d| d.table == DepTable::Workspace)
        .map(|d| (d.alias.clone(), d.package()))
        .collect();
    let mut out = Vec::new();
    for (name, text) in manifests {
        if ALLOWED.contains(&name.as_str()) {
            continue;
        }
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
            let reviewed = SOCKET_LAYER_ROWS
                .iter()
                .any(|(c, p, _)| *c == name && *p == package);
            if SOCKET_LAYER_CRATES.contains(&package.as_str()) && !reviewed {
                out.push(format!("{name}: {package} (as `{}`)", d.alias));
            }
        }
    }
    out.sort();
    out.dedup();
    out
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

        /// A newline straight after an opening `"""` or `'''` is not content
        /// (TOML 1.0, "Multi-line basic strings"). The first version kept it, so
        /// `package = """\nureq"""` read as `"\nureq"`, matched no client, and
        /// the audit's review of 10 Oct 2026 walked a rename past the manifest
        /// check with it.
        fn skip_opening_newline(&mut self) {
            if self.at("\r\n") {
                self.i += 2;
            } else if self.peek() == Some('\n') {
                self.i += 1;
            }
        }

        fn string(&mut self) -> Result<String, String> {
            if self.at("\"\"\"") {
                self.i += 3;
                self.skip_opening_newline();
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
                            // A LINE-ENDING BACKSLASH: `\`, optional blanks, a
                            // newline — then every blank and newline after it is
                            // dropped. `"""ure\⏎   q"""` is `ureq`; checked
                            // against real Cargo, whose `cargo metadata` on a
                            // scratch crate written that way started locking
                            // ureq's own dependencies.
                            let mut j = self.i;
                            while matches!(self.s.get(j), Some(' ' | '\t')) {
                                j += 1;
                            }
                            if matches!(self.s.get(j), Some('\n' | '\r')) {
                                self.i = j;
                                while matches!(self.peek(), Some(' ' | '\t' | '\r' | '\n')) {
                                    self.i += 1;
                                }
                            } else {
                                self.escape(&mut s)?;
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
                self.skip_opening_newline();
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
                        self.escape(&mut s)?;
                    }
                    Some(c) => {
                        s.push(c);
                        self.i += 1;
                    }
                }
            }
        }

        /// The character after a `\`, decoded into `s`. ONE decoder for both
        /// basic-string forms: the multi-line one used to copy the escape
        /// letter through, so `"""ureq"""` read as `u0075req`.
        fn escape(&mut self, s: &mut String) -> Result<(), String> {
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
                    let ch = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
                    match ch {
                        Some(ch) => s.push(ch),
                        None => return self.err("bad unicode escape"),
                    }
                }
                '"' | '\\' => s.push(c),
                // TOML 1.1, and Cargo takes it: on 10 Oct 2026 `cargo` 1.98.1
                // read `net = { package = "\x75req" }` in a scratch crate as
                // ureq and began locking ureq's dependency tree. Copying the
                // letter through (the first decoder) read it as `x75req`.
                'x' => {
                    let hex: String = self.s[self.i..(self.i + 2).min(self.s.len())]
                        .iter()
                        .collect();
                    self.i += 2;
                    match u8::from_str_radix(&hex, 16) {
                        Ok(b) if hex.len() == 2 && hex.chars().all(|d| d.is_ascii_hexdigit()) => {
                            s.push(char::from(b))
                        }
                        _ => return self.err("bad \\x escape"),
                    }
                }
                'e' => s.push('\u{1b}'),
                // Any other letter is an invalid manifest to Cargo, and so to
                // this reader: `\q` read as `q` was a pass for a file Cargo
                // refuses ("missing escaped value, expected `b`, `e`, `f`, `n`,
                // `r`, `\`, `"`, `x`, `u`, `U`" — cargo 1.98.1, 10 Oct 2026; the
                // arms above are that list).
                other => return self.err(&format!("invalid escape `\\{other}`")),
            }
            Ok(())
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
    let meta = metadata();
    let offenders = graph_offenders(&meta);
    assert!(
        offenders.is_empty(),
        "a crate outside the network monopoly reaches an HTTP client:\n  {}",
        offenders.join("\n  ")
    );
}

/// A SOCKET LAYER CAN ARRIVE ONE CRATE REMOVED, as an HTTP client can, and
/// then it is the FEATURE that opens the socket API: rustix's `net`, nix's
/// `socket`/`net`. rustix is already in the graph (crossterm's terminal code),
/// resolved 10 Oct 2026 with `alloc, default, fs, std, stdio, termios` and no
/// `net`; a crate that switches `net` on hands every crate reaching rustix a
/// socket without a declaration this file would read.
///
/// `windows-sys` IS NOT CHECKED THIS WAY, and that was measured: mio's `net`
/// (see `HTTP_CRATES`) turns on `Win32_Networking_WinSock` for the same
/// self-pipe, so the feature is on in today's graph for a reason everyone knows
/// to ignore. The declaration check still covers it.
#[test]
fn no_crate_outside_the_monopoly_reaches_a_socket_layer_with_its_socket_feature_on() {
    let offenders = socket_feature_offenders(&metadata());
    assert!(
        offenders.is_empty(),
        "a crate outside the network monopoly reaches a socket layer with its \
         socket API switched on:\n  {}",
        offenders.join("\n  ")
    );
}

/// `member -> package[feature]` for every socket-layer package resolved with
/// its socket feature on and reachable from a member outside `ALLOWED`
/// without passing through `ALLOWED`.
fn socket_feature_offenders(meta: &serde_json::Value) -> Vec<String> {
    const SOCKET_FEATURES: &[(&str, &[&str])] =
        &[("rustix", &["net"]), ("nix", &["socket", "net"])];
    let names: BTreeMap<&str, &str> = meta["packages"]
        .as_array()
        .expect("metadata has packages")
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p["name"].as_str()?)))
        .collect();
    let nodes = meta["resolve"]["nodes"]
        .as_array()
        .expect("metadata has a resolved graph");
    let edges: BTreeMap<&str, Vec<&str>> = nodes
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
    let features: BTreeMap<&str, Vec<&str>> = nodes
        .iter()
        .filter_map(|n| {
            let f = n["features"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            Some((n["id"].as_str()?, f))
        })
        .collect();
    let members: Vec<&str> = meta["workspace_members"]
        .as_array()
        .expect("metadata has workspace members")
        .iter()
        .filter_map(|m| m.as_str())
        .collect();
    let name = |id: &str| names.get(id).copied().unwrap_or("?").to_string();
    let mut out = Vec::new();
    for &start in &members {
        if ALLOWED.contains(&name(start).as_str()) {
            continue;
        }
        let mut queue = VecDeque::from([start]);
        let mut seen = BTreeSet::from([start]);
        while let Some(id) = queue.pop_front() {
            if id != start && members.contains(&id) && ALLOWED.contains(&name(id).as_str()) {
                continue;
            }
            let n = name(id);
            if let Some((_, wanted)) = SOCKET_FEATURES.iter().find(|(p, _)| *p == n) {
                for f in features.get(id).map(Vec::as_slice).unwrap_or_default() {
                    if wanted.contains(f) {
                        out.push(format!("{} -> {n}[{f}]", name(start)));
                    }
                }
            }
            for &next in edges.get(id).map(Vec::as_slice).unwrap_or_default() {
                if seen.insert(next) {
                    queue.push_back(next);
                }
            }
        }
    }
    out
}

/// EVERY SCAN IN THIS FILE STARTS FROM `crates/*/Cargo.toml`, and nothing
/// made that the whole workspace. The review of 10 Oct 2026 named two ways
/// out, both green: a path dependency outside `crates/` (`vendor/helper` with a
/// `TcpStream`, depended on by tacet-tools — the graph walk matches client
/// NAMES, and `helper` is not one), and a workspace member outside `crates/`
/// (`xtask/`), which no scan opens. So cargo's own list is the reference: every
/// workspace member, and every package it resolved from a local path, must
/// have its manifest at `crates/<dir>/Cargo.toml` — where the scans look.
#[test]
fn every_local_crate_lives_where_the_scans_look() {
    let outside = local_packages_outside(&metadata(), &crates_dir());
    assert!(
        outside.is_empty(),
        "a crate cargo builds from this repository lives outside crates/, where the \
         socket, agent-CLI, network-program and spawn scans never read it. Move it \
         under crates/ (or teach `manifests_in` to read it):\n  {}",
        outside.join("\n  ")
    );
}

/// `cargo metadata` for this workspace, every feature on, offline.
fn metadata() -> serde_json::Value {
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
    meta
}

/// `name (manifest)` for every workspace member, and every package resolved
/// from a local path (`source` is null), whose manifest is not
/// `<crates>/<dir>/Cargo.toml`.
fn local_packages_outside(meta: &serde_json::Value, crates: &Path) -> Vec<String> {
    let members: BTreeSet<&str> = meta["workspace_members"]
        .as_array()
        .expect("metadata has workspace members")
        .iter()
        .filter_map(|m| m.as_str())
        .collect();
    // cargo prints canonical paths; CARGO_MANIFEST_DIR may not be (a symlinked
    // checkout). A path that does not exist (the self-test's) compares as written.
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| normalized(p));
    let crates = canon(crates);
    let mut out = Vec::new();
    for p in meta["packages"].as_array().expect("metadata has packages") {
        let id = p["id"].as_str().unwrap_or_default();
        if !(p["source"].is_null() || members.contains(id)) {
            continue;
        }
        let manifest = Path::new(p["manifest_path"].as_str().unwrap_or_default());
        let under_crates = manifest.file_name().is_some_and(|f| f == "Cargo.toml")
            && manifest
                .parent()
                .and_then(Path::parent)
                .is_some_and(|d| canon(d) == crates);
        if !under_crates {
            out.push(format!(
                "{} ({})",
                p["name"].as_str().unwrap_or(id),
                manifest.display()
            ));
        }
    }
    out
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

/// Every `file:line: name` under `crates` where a crate outside `ALLOWED`
/// reaches the network in code (see `raw_network`).
fn socket_offenders(crates: &Path) -> Vec<String> {
    let mut offenders = Vec::new();
    for (name, manifest) in manifests_in(crates) {
        if ALLOWED.contains(&name.as_str()) {
            continue;
        }
        let scan = crate_scan(&crates.join(&name), &manifest);
        offenders.extend(scan.unfollowable);
        for file in scan.files {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            for (line, what) in raw_network(&rust_lex::tokens(&text)) {
                offenders.push(format!("{}:{line}: {what}", file.display()));
            }
        }
    }
    offenders
}

/// `(line, name)` for each way `toks` reaches the network without a client
/// crate: a `std::net` connector, a resolver, a `SOCKET_FFI` name declared in
/// an `extern` block or called through a socket-layer crate under any name this
/// file gives it, and any foreign function `EXTERN_FNS` does not list.
fn raw_network(toks: &[rust_lex::Token]) -> Vec<(usize, String)> {
    use rust_lex::Kind;
    let mut out = Vec::new();
    // THE ROOTS A SOCKET CALL IS REACHED THROUGH: the socket-layer crates by
    // their own names, and every name this file gives one — `use libc as c`,
    // `use libc::{self as c}`, `extern crate libc as sys`, `use rustix::net as
    // n`, and an alias of an alias. See `SOCKET_LAYER_CRATES` for the review
    // that compiled `use libc as c; c::socket(2, 1, 0)` past the spelling check.
    let mut roots: BTreeSet<String> = ["libc", "nix", "rustix"].map(String::from).into();
    loop {
        let before = roots.len();
        for (i, t) in toks.iter().enumerate() {
            let from = match (t.kind, t.text.as_str()) {
                (Kind::Ident, "use") => i + 1,
                (Kind::Ident, "crate") if i >= 1 && toks[i - 1].text == "extern" => i + 1,
                _ => continue,
            };
            let stmt: Vec<&rust_lex::Token> =
                toks[from..].iter().take_while(|n| n.text != ";").collect();
            let root = stmt.iter().find(|n| n.kind == Kind::Ident);
            if !root.is_some_and(|r| roots.contains(&r.text)) {
                continue;
            }
            for w in stmt.windows(2) {
                if w[0].text == "as" && w[1].kind == Kind::Ident && w[1].text != "_" {
                    roots.insert(w[1].text.clone());
                }
            }
        }
        if roots.len() == before {
            break;
        }
    }
    let mut depth = 0usize;
    // The brace depth inside the innermost open `extern … {` block.
    let mut extern_at: Option<usize> = None;
    // A `#[link_name = "…"]` waiting for the `fn` it renames.
    let mut pending_link: Option<String> = None;
    let mut macro_reported = false;
    let path_sep = |i: usize| {
        toks.get(i).is_some_and(|t| t.text == ":") && toks.get(i + 1).is_some_and(|t| t.text == ":")
    };
    // The first identifier of the path that ends at `toks[i]`, when `toks[i]`
    // is reached through `::` (`libc::connect` → `libc`).
    let path_root = |i: usize| -> Option<&str> {
        if !(i >= 2 && path_sep(i - 2)) {
            return None;
        }
        let mut k = i - 2;
        let mut root = None;
        while k >= 1 && toks[k - 1].kind == Kind::Ident {
            root = Some(toks[k - 1].text.as_str());
            if k >= 3 && path_sep(k - 3) {
                k -= 3;
            } else {
                break;
            }
        }
        root
    };
    for (i, t) in toks.iter().enumerate() {
        match (t.kind, t.text.as_str()) {
            (Kind::Punct, "{") => depth += 1,
            (Kind::Punct, "}") => {
                if extern_at == Some(depth) {
                    extern_at = None;
                    pending_link = None;
                    macro_reported = false;
                }
                depth = depth.saturating_sub(1);
            }
            // `extern {`, `extern "C" {`, `unsafe extern "system" {` — not
            // `extern "C" fn` (a definition, not an import). `extern crate` of a
            // socket layer is red on its own: the crate is the socket API.
            (Kind::Ident, "extern") => {
                if toks.get(i + 1).is_some_and(|n| n.text == "crate")
                    && toks.get(i + 2).is_some_and(|n| {
                        SOCKET_LAYER_CRATES.contains(&n.text.replace('_', "-").as_str())
                    })
                {
                    out.push((t.line, format!("extern crate {}", toks[i + 2].text)));
                }
                let mut j = i + 1;
                if toks.get(j).is_some_and(|n| n.kind == Kind::Str) {
                    j += 1;
                }
                if toks.get(j).is_some_and(|n| n.text == "{") && extern_at.is_none() {
                    extern_at = Some(depth + 1);
                }
            }
            // A `$` inside a foreign block: the block is a macro's template, and
            // the names it declares are whatever the macro is called with.
            (Kind::Punct, "$") if extern_at.is_some() && !macro_reported => {
                macro_reported = true;
                out.push((t.line, "extern block filled in by a macro".to_string()));
            }
            // `#[link_name = "connect"]` names the symbol; anything but one
            // string literal there (`concat!(…)`, a `$x`) hides it.
            (Kind::Ident, "link_name") if toks.get(i + 1).is_some_and(|n| n.text == "=") => {
                let literal = toks.get(i + 2).is_some_and(|n| n.kind == Kind::Str)
                    && toks.get(i + 3).is_some_and(|n| n.text == "]");
                if literal {
                    pending_link = Some(toks[i + 2].text.clone());
                } else {
                    out.push((t.line, "link_name that is not one literal".to_string()));
                }
            }
            (Kind::Ident, "fn") if extern_at.is_some() => {
                let link = pending_link.take();
                if let Some(n) = toks.get(i + 1).filter(|n| n.kind == Kind::Ident) {
                    let symbol = link.unwrap_or_else(|| n.text.clone());
                    // A SOCKET_FFI symbol is reported by its own arm, once.
                    if !SOCKET_FFI.contains(&symbol.as_str())
                        && !EXTERN_FNS.contains(&symbol.as_str())
                    {
                        out.push((n.line, format!("extern fn {symbol} (not in EXTERN_FNS)")));
                    }
                }
            }
            (Kind::Ident, name) if CONNECTORS.contains(&name) || RESOLVERS.contains(&name) => {
                out.push((t.line, name.to_string()));
            }
            // `use libc::{socket, connect};` and `use libc::*;` leave the call
            // site bare, so the import is where the name is: a glob, a socket
            // name, or rustix's whole `net` module anywhere in the braces.
            (Kind::Ident, "use") => {
                let stmt: Vec<&rust_lex::Token> =
                    toks[i + 1..].iter().take_while(|n| n.text != ";").collect();
                let root = stmt.iter().find(|n| n.kind == Kind::Ident);
                if root.is_some_and(|r| roots.contains(&r.text))
                    && stmt.iter().any(|n| {
                        n.text == "*" || n.text == "net" || SOCKET_FFI.contains(&n.text.as_str())
                    })
                {
                    out.push((t.line, format!("use {}", root.map_or("", |r| &r.text))));
                }
            }
            // `rustix::net::connect_v4`, `rustix::net::socket_with`: rustix
            // spells its calls with suffixes no fixed name list keeps up with,
            // and they all live under `net`.
            (Kind::Ident, "net") if path_root(i).is_some_and(|r| roots.contains(r)) => {
                out.push((t.line, format!("{}::net", path_root(i).unwrap_or_default())));
            }
            (Kind::Ident | Kind::Str, name) if SOCKET_FFI.contains(&name) => {
                let declared = extern_at.is_some();
                // `libc::connect`, `nix::sys::socket::connect`, `c::socket`
                // after `use libc as c`.
                let via_crate = path_root(i).is_some_and(|r| roots.contains(r));
                if t.kind == Kind::Ident && (declared || via_crate)
                    || t.kind == Kind::Str && declared
                {
                    out.push((t.line, format!("extern {name}")));
                }
            }
            _ => {}
        }
    }
    out
}

/// The files a crate COMPILES: every `.rs` below its directory, plus every file
/// rustc is pointed at from there, wherever it lives and whatever it is called.
///
/// THE SCOPE WAS `src` UNTIL THAT WASN'T ENOUGH TO SAY SO, then `src`, `tests`,
/// `examples` and `benches` — and the audit of 9 Oct 2026 put a `TcpStream` in
/// `build.rs`, which sits at the crate root, outside all four, and runs on
/// every build of every user who compiles from source. So the scope became the
/// crate directory — and the review of 10 Oct 2026 showed that was a list of
/// places too: `#[path = "../../shared/net.rs"] mod net;` and
/// `include!("net.inc")` both compiled a `TcpStream::connect` the scan never
/// opened, the first from outside the crate, the second from a file that is not
/// `.rs`. The crate is not a directory; it is what rustc reads. So this follows
/// what rustc follows:
///
/// * the manifest's own roots — `[package] build`, and `path` under `[lib]`,
///   `[[bin]]`, `[[test]]`, `[[example]]`, `[[bench]]`;
/// * `mod name;` → `name.rs` / `name/mod.rs`, next to the file or under its
///   stem, through any inline `mod a { … }` around it (only needed once a root
///   lives outside the crate directory, where the directory walk cannot see);
/// * `path = "…"` inside any attribute — `#[path]` and `#[cfg_attr(…, path)]`;
///   a directory there (a `#[path]` on an inline module) is walked whole;
/// * `include!("…")`, relative to the including file.
///
/// What it cannot follow is RED, not skipped (`unfollowable`): an `include!`
/// whose argument is not one literal (`concat!(env!("OUT_DIR"), …)` — code a
/// build script WROTE, which no source scan can read), and a `#[path]` that
/// resolves to nothing. The tree has neither today. A non-compiled fixture can
/// only make this report MORE, which is the safe direction: a false alarm a
/// human reads, never a socket waved through.
///
/// This is also why the agent-session module (RG-33, planned under
/// `tacet-tools/src/sessions/`) needs no line here to be covered;
/// `a_new_sessions_module_is_scanned_without_being_listed` holds that to it.
fn scanned_files(crate_dir: &Path, manifest: &str) -> Vec<PathBuf> {
    crate_scan(crate_dir, manifest).files
}

struct CrateScan {
    files: Vec<PathBuf>,
    /// `file:line: why` for each pointer to code the scan could not follow.
    unfollowable: Vec<String>,
}

fn crate_scan(crate_dir: &Path, manifest: &str) -> CrateScan {
    let mut queue: VecDeque<PathBuf> = rust_files(crate_dir).into();
    if let Ok(rows) = toml_lite::flatten(manifest) {
        for (path, value) in rows {
            let toml_lite::Value::Str(target) = &value else {
                continue;
            };
            let key: Vec<&str> = path.iter().map(String::as_str).collect();
            if matches!(
                key.as_slice(),
                ["package", "build"] | ["lib" | "bin" | "test" | "example" | "bench", "path"]
            ) {
                queue.push_back(crate_dir.join(target));
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut scan = CrateScan {
        files: Vec::new(),
        unfollowable: Vec::new(),
    };
    while let Some(file) = queue.pop_front() {
        let file = normalized(&file);
        if !seen.insert(file.clone()) {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let (more, bad) = compiled_from(&file, &text);
        queue.extend(more);
        scan.unfollowable.extend(bad);
        scan.files.push(file);
    }
    scan.files.sort();
    scan
}

/// `a/b/../c` → `a/c`, without touching the disk: two spellings of one file
/// must be scanned once, and a path that does not exist must still compare.
fn normalized(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// The files `file` makes rustc read (existing ones only), and what it points
/// at that cannot be followed. See `crate_scan` for the forms.
fn compiled_from(file: &Path, text: &str) -> (Vec<PathBuf>, Vec<String>) {
    use rust_lex::Kind;
    let toks = rust_lex::tokens(text);
    let dir = file.parent().unwrap_or(Path::new(""));
    let stem = file.file_stem().map(PathBuf::from).unwrap_or_default();
    let is = |i: usize, kind: Kind, text: &str| {
        toks.get(i)
            .is_some_and(|t| t.kind == kind && t.text == text)
    };
    let mut found = Vec::new();
    let mut bad = Vec::new();
    // Inline `mod a { … }` around the current token, with the brace depth each
    // one closes at.
    let mut inline: Vec<(String, usize)> = Vec::new();
    let mut depth = 0usize;
    // The bracket depth the open attribute (`#[…]`, `#![…]`) closes at.
    let mut attribute: Option<usize> = None;
    let mut brackets = 0usize;
    let bases = |inline: &[(String, usize)]| -> Vec<PathBuf> {
        let nested: PathBuf = inline.iter().map(|(n, _)| n.as_str()).collect();
        vec![dir.join(&nested), dir.join(&stem).join(&nested)]
    };
    for (i, t) in toks.iter().enumerate() {
        match (t.kind, t.text.as_str()) {
            (Kind::Punct, "{") => depth += 1,
            (Kind::Punct, "}") => {
                if inline.last().is_some_and(|&(_, d)| d == depth) {
                    inline.pop();
                }
                depth = depth.saturating_sub(1);
            }
            (Kind::Punct, "[") => {
                brackets += 1;
                let opens_attribute = i >= 1
                    && (is(i - 1, Kind::Punct, "#")
                        || (i >= 2 && is(i - 1, Kind::Punct, "!") && is(i - 2, Kind::Punct, "#")));
                if opens_attribute && attribute.is_none() {
                    attribute = Some(brackets);
                }
            }
            (Kind::Punct, "]") => {
                if attribute == Some(brackets) {
                    attribute = None;
                }
                brackets = brackets.saturating_sub(1);
            }
            (Kind::Ident, "mod") => {
                let Some(name) = toks.get(i + 1).filter(|n| n.kind == Kind::Ident) else {
                    continue;
                };
                if is(i + 2, Kind::Punct, "{") {
                    inline.push((name.text.clone(), depth + 1));
                } else if is(i + 2, Kind::Punct, ";") {
                    for base in bases(&inline) {
                        for c in [
                            normalized(&base.join(format!("{}.rs", name.text))),
                            normalized(&base.join(&name.text).join("mod.rs")),
                        ] {
                            if c.is_file() {
                                found.push(c);
                            }
                        }
                    }
                }
            }
            (Kind::Ident, "path") if attribute.is_some() && is(i + 1, Kind::Punct, "=") => {
                let Some(target) = toks.get(i + 2).filter(|s| s.kind == Kind::Str) else {
                    continue;
                };
                let mut resolved = false;
                for base in bases(&inline) {
                    // Lexically normalized: `src/a/../x` names a real file even
                    // when no directory `a` exists for the OS to walk through.
                    let c = normalized(&base.join(&target.text));
                    if c.is_file() {
                        found.push(c);
                        resolved = true;
                    } else if c.is_dir() {
                        found.extend(rust_files(&c));
                        resolved = true;
                    }
                }
                if !resolved {
                    bad.push(format!(
                        "{}:{}: #[path = {:?}] resolves to no file this scan can read",
                        file.display(),
                        t.line,
                        target.text
                    ));
                }
            }
            (Kind::Ident, "include") if is(i + 1, Kind::Punct, "!") => {
                // `include!("x")` — the literal, then the closing delimiter.
                let literal = toks.get(i + 3).filter(|s| s.kind == Kind::Str).filter(|_| {
                    matches!(
                        toks.get(i + 4).map(|c| c.text.as_str()),
                        Some(")" | "]" | "}" | ",")
                    )
                });
                match literal {
                    Some(target) if dir.join(&target.text).is_file() => {
                        found.push(dir.join(&target.text));
                    }
                    _ => bad.push(format!(
                        "{}:{}: include! of code this scan cannot read",
                        file.display(),
                        t.line
                    )),
                }
            }
            _ => {}
        }
    }
    (found, bad)
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
        /// A string literal of any flavour, with its text decoded as rustc
        /// decodes it (see `unescape`).
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
                        i = unescape(&s, i + 1, &mut text);
                    } else {
                        text.push(s[i]);
                        i += 1;
                    }
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

    /// The escape whose letter is at `s[i]` (just past the `\`), decoded into
    /// `text` the way rustc decodes it; returns the index after it.
    ///
    /// THE FIRST LEXER COPIED THE LETTER THROUGH, so `"cl\u{61}ude"` read as
    /// `clu{61}ude`, `"\x63odex"` as `x63odex`, and a `\`-newline continuation
    /// split `"cl\⏎    aude"` into two words. rustc compiles all three to the
    /// plain name; the review of 10 Oct 2026 fed each to the agent-CLI and
    /// network-program scans and got `[]`. A name written with an escape is
    /// still written, so what is compared here is what the binary will hold.
    /// An escape rustc would reject keeps its letter: the file does not
    /// compile, so nothing it spells can run.
    fn unescape(s: &[char], mut i: usize, text: &mut String) -> usize {
        let Some(&c) = s.get(i) else { return i };
        i += 1;
        match c {
            'n' => text.push('\n'),
            'r' => text.push('\r'),
            't' => text.push('\t'),
            '0' => text.push('\0'),
            'x' => {
                let hex: String = s.iter().skip(i).take(2).collect();
                match u8::from_str_radix(&hex, 16) {
                    Ok(b) if hex.len() == 2 && hex.chars().all(|d| d.is_ascii_hexdigit()) => {
                        text.push(char::from(b));
                        i += 2;
                    }
                    _ => text.push('x'),
                }
            }
            'u' if s.get(i) == Some(&'{') => {
                let close = (i..s.len()).find(|&k| s[k] == '}' || s[k] == '"');
                let hex: String = match close {
                    Some(k) if s[k] == '}' => s[i + 1..k].iter().filter(|&&d| d != '_').collect(),
                    _ => String::new(),
                };
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => {
                        text.push(ch);
                        i = close.map_or(i, |k| k + 1);
                    }
                    None => text.push('u'),
                }
            }
            // STRING_CONTINUE: the newline and every blank after it vanish.
            '\n' | '\r' => {
                while matches!(s.get(i), Some(' ' | '\t' | '\n' | '\r')) {
                    i += 1;
                }
            }
            // `\\`, `\"`, `\'`.
            other => text.push(other),
        }
        i
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
/// gate. This holds the code to that: an agent CLI named as a program in a file
/// not listed in `AGENT_CLI_SPAWNERS`, or listed under a tool that is not in
/// tacet-cli's `EXTERNAL_TOOLS`, is red.
///
/// IT MATCHES THE NAME, NOT THE CALL. The first version looked for the tokens
/// `Command::new("claude")`, and the review of 10 Oct 2026 walked three shapes
/// past it, all with the name written out in full: `use std::process::Command
/// as Cmd; Cmd::new("claude")`; a launcher — `Command::new("npx").args(
/// ["@anthropic-ai/claude-code", "-p"])`, `Command::new("sh").args(["-c",
/// "codex exec resume"])`; and any spawn at all inside `tacet-web` or
/// `tacet-mcp`, which the scan skipped because they own a socket. Owning a
/// socket is not consent to a paid turn (RG-34), so those two are scanned like
/// every other crate. And any string literal in code that names an agent CLI IN
/// PROGRAM POSITION — first word, first word after a shell separator, first
/// word after a launcher such as `env`, `sh -c` or `npx` — or names its npm
/// package anywhere, counts: aliases, wrappers and helper functions
/// (`shell_line("claude", …)`) all have to write the name somewhere — and a
/// name written with an escape (`"cl\u{61}ude"`) is read as rustc reads it.
///
/// NAMES BUILT AT RUN TIME are the limit of a source scan, and are an accepted
/// risk rather than an open item, because they are enumerated:
/// `no_new_spawn_site_takes_its_program_from_a_variable` makes every
/// `Command::new(<not a literal>)` a reviewed row with its reason. A name
/// assembled with `concat!` in a new file is red there; in a listed file, it is
/// what the review of that row has to catch.
#[test]
fn agent_clis_are_spawned_only_behind_the_approval_gate() {
    let main = std::fs::read_to_string(crates_dir().join("tacet-cli/src/main.rs"))
        .expect("tacet-cli's main.rs is readable");
    let external = external_tools(&main);
    assert!(
        external.contains(&"web_search".to_string()),
        "EXTERNAL_TOOLS was not found in main.rs (read {external:?}); this check would be vacuous"
    );
    let problems =
        agent_spawn_problems(&crates_dir(), AGENT_CLI_SPAWNERS, NAMED_NOT_RUN, &external);
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

/// This file names every program it looks for, in its lists and in the
/// fixtures that prove the lists bite. It is the one file the name scans skip;
/// it spawns nothing but `cargo metadata`.
const THIS_FILE: &str = "tacet-cli/tests/network_monopoly.rs";

/// Programs that only START another: the program that matters is the first
/// word after them. `-c`, flags and `VAR=value` are skipped the same way.
const LAUNCHERS: &[&str] = &[
    "env",
    "exec",
    "command",
    "nohup",
    "sudo",
    "doas",
    "time",
    "nice",
    "timeout",
    "stdbuf",
    "xargs",
    "sh",
    "bash",
    "zsh",
    "dash",
    "fish",
    "npx",
    "bunx",
    "pnpx",
    "npm",
    "pnpm",
    "yarn",
    "dlx",
    "cmd",
    "cmd.exe",
    "/c",
    "powershell",
    "pwsh",
    "node",
    "bun",
    "deno",
];

/// The programs a literal names in program position — see
/// `agent_clis_are_spawned_only_behind_the_approval_gate` for what that means.
/// Lowercased file stems: `/opt/bin/Codex.exe` is `codex`, because macOS's
/// default file system and Windows both run `Claude` as `claude`.
fn programs_named(literal: &str) -> Vec<String> {
    let mut out = Vec::new();
    let separators = |c: char| {
        matches!(
            c,
            ';' | '&' | '|' | '(' | ')' | '`' | '\n' | '$' | '{' | '}'
        )
    };
    for segment in literal.split(separators) {
        let program = segment
            .split_whitespace()
            .map(|w| w.trim_matches(|c| matches!(c, '"' | '\'' | '[' | ']' | ',')))
            .find(|w| {
                !w.is_empty()
                    && !w.starts_with('-')
                    && !w.contains('=')
                    && !w
                        .chars()
                        .all(|c| c.is_ascii_digit() || c == '.' || c == 's')
                    && !LAUNCHERS.contains(&w.to_lowercase().as_str())
            });
        if let Some(p) = program {
            out.push(program_stem(p));
        }
    }
    out
}

/// `/opt/bin/Codex.exe` → `codex`. Only an EXECUTABLE extension comes off:
/// `CLAUDE.md` is a file this repository names in its messages, not a program,
/// and the first version of this function stripped `.md` and read it as
/// `claude` (tacet-cli/tests/readme_tool_list.rs).
fn program_stem(word: &str) -> String {
    let name = Path::new(word)
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    for ext in [".exe", ".cmd", ".bat", ".ps1", ".js", ".mjs", ".cjs", ".sh"] {
        if let Some(stem) = name.strip_suffix(ext) {
            return stem.to_string();
        }
    }
    name
}

/// A literal that runs an agent CLI: in program position, or as its package.
fn names_agent_cli(literal: &str) -> Option<String> {
    let lower = literal.to_lowercase();
    if let Some(pkg) = AGENT_CLI_PACKAGES.iter().find(|p| lower.contains(*p)) {
        return Some(pkg.to_string());
    }
    programs_named(literal)
        .into_iter()
        .find(|p| AGENT_CLIS.contains(&p.as_str()))
}

/// Every literal, in every crate — the two that own a socket included — that
/// names an agent CLI and is not covered by a `(file suffix, tool)` row in
/// `spawners` whose tool is in `external`, or by a `(file suffix, literal)` row
/// in `named_not_run`.
fn agent_spawn_problems(
    crates: &Path,
    spawners: &[(&str, &str)],
    named_not_run: &[(&str, &str)],
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
    for (_, file, t) in literals_in_code(crates) {
        let Some(program) = names_agent_cli(&t.text) else {
            continue;
        };
        let shown = file.to_string_lossy().replace('\\', "/");
        let spawner = spawners.iter().any(|(suffix, _)| shown.ends_with(suffix));
        let named = named_not_run
            .iter()
            .any(|(suffix, lit)| shown.ends_with(suffix) && *lit == t.text);
        if !spawner && !named {
            problems.push(format!(
                "{}:{}: {:?} runs `{program}` and is not an external tool",
                file.display(),
                t.line,
                t.text
            ));
        }
    }
    problems
}

/// Every string-literal token in the code of every crate under `crates`, with
/// the file it is in. `THIS_FILE` is left out (see there).
fn literals_in_code(crates: &Path) -> Vec<(String, PathBuf, rust_lex::Token)> {
    let mut out = Vec::new();
    for (name, manifest) in manifests_in(crates) {
        for file in scanned_files(&crates.join(&name), &manifest) {
            if file.ends_with(THIS_FILE) {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            for t in rust_lex::tokens(&text) {
                if t.kind == rust_lex::Kind::Str {
                    out.push((name.clone(), file.clone(), t));
                }
            }
        }
    }
    out
}

/// T4 NAMED SUBPROCESSES AS THE OTHER WAY OUT, and the first rewrite of this
/// file guarded only the agent CLIs. A crate that cannot open a socket can
/// still start a program whose whole job is to open one. Outside the two crates
/// that own the network, naming one of `NETWORK_PROGRAMS` in program position is
/// red unless a `NETWORK_PROGRAM_SPAWNERS` row says which file, which program,
/// and why it is safe there.
#[test]
fn no_crate_outside_the_monopoly_spawns_a_network_program() {
    let problems = network_program_problems(&crates_dir(), NETWORK_PROGRAM_SPAWNERS, NAMED_NOT_RUN);
    assert!(
        problems.is_empty(),
        "a crate outside the network monopoly starts a program that is the network:\n  {}",
        problems.join("\n  ")
    );
}

fn network_program_problems(
    crates: &Path,
    rows: &[(&str, &str, &str)],
    named_not_run: &[(&str, &str)],
) -> Vec<String> {
    let mut problems = Vec::new();
    for (krate, file, t) in literals_in_code(crates) {
        // A crate that owns a socket gains nothing by starting `curl`. The
        // agent CLIs are NOT exempt there (see above): those cost money.
        if ALLOWED.contains(&krate.as_str()) {
            continue;
        }
        let shown = file.to_string_lossy().replace('\\', "/");
        for program in programs_named(&t.text) {
            if !NETWORK_PROGRAMS.contains(&program.as_str()) {
                continue;
            }
            let allowed = rows
                .iter()
                .any(|(suffix, p, _)| shown.ends_with(suffix) && *p == program)
                || named_not_run
                    .iter()
                    .any(|(suffix, lit)| shown.ends_with(suffix) && *lit == t.text);
            if !allowed {
                problems.push(format!(
                    "{}:{}: {:?} runs `{program}`",
                    file.display(),
                    t.line,
                    t.text
                ));
            }
        }
    }
    problems
}

/// THE SPAWNS NO NAME SCAN CAN SEE: `Command::new(x)` where `x` is not a
/// literal. Each file that does it is a row here with where its program comes
/// from, so a NEW one is red until somebody writes that sentence. An alias
/// (`use std::process::Command as Cmd`) is followed.
#[test]
fn no_new_spawn_site_takes_its_program_from_a_variable() {
    let unlisted = runtime_spawn_sites(&crates_dir(), RUNTIME_SPAWN_SITES);
    assert!(
        unlisted.is_empty(),
        "a new process is started from a program name built at run time; add the \
         file to RUNTIME_SPAWN_SITES with where the name comes from:\n  {}",
        unlisted.join("\n  ")
    );
    // A row for a file that no longer spawns is a row nobody will re-read.
    let every = runtime_spawn_sites(&crates_dir(), &[]);
    for (suffix, _) in RUNTIME_SPAWN_SITES {
        assert!(
            every.iter().any(|s| s
                .split(':')
                .next()
                .is_some_and(|f| f.replace('\\', "/").ends_with(suffix))),
            "RUNTIME_SPAWN_SITES lists {suffix}, which no longer spawns from a variable"
        );
    }
}

/// `file:line` of every `Command::new` (or alias) not called with a literal,
/// in a file not covered by `listed`.
fn runtime_spawn_sites(crates: &Path, listed: &[(&str, &str)]) -> Vec<String> {
    use rust_lex::Kind;
    let mut out = Vec::new();
    for (name, manifest) in manifests_in(crates) {
        for file in scanned_files(&crates.join(&name), &manifest) {
            let shown = file.to_string_lossy().replace('\\', "/");
            if listed.iter().any(|(suffix, _)| shown.ends_with(suffix)) {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            let toks = rust_lex::tokens(&text);
            let mut names = BTreeSet::from(["Command".to_string()]);
            for w in toks.windows(3) {
                if w[0].text == "Command" && w[1].text == "as" && w[2].kind == Kind::Ident {
                    names.insert(w[2].text.clone());
                }
            }
            // `type Run = std::process::Command;` is the same alias as `use …
            // as Run`, written another way.
            for (i, t) in toks.iter().enumerate() {
                if t.kind == Kind::Ident
                    && t.text == "type"
                    && toks.get(i + 2).is_some_and(|e| e.text == "=")
                    && toks.get(i + 1).is_some_and(|n| n.kind == Kind::Ident)
                {
                    let rhs: Vec<&rust_lex::Token> =
                        toks[i + 3..].iter().take_while(|n| n.text != ";").collect();
                    if rhs.last().is_some_and(|l| l.text == "Command") {
                        names.insert(toks[i + 1].text.clone());
                    }
                }
            }
            // A LITERAL IS ONE STRING AND THE CLOSING PARENTHESIS. The check
            // used to stop at the first token, so `Command::new("cla".to_owned()
            // + "ude")` — a `claude` spawn assembled at run time in a new,
            // unlisted file — counted as a literal, and neither half names an
            // agent CLI (review of 10 Oct 2026, confirmed in the real tree with
            // the suite green). Anything after the string is run-time work.
            for w in toks.windows(7) {
                let call = w[0].kind == Kind::Ident
                    && names.contains(&w[0].text)
                    && w[1].text == ":"
                    && w[2].text == ":"
                    && w[3].text == "new";
                if !call {
                    continue;
                }
                let literal = w[4].text == "(" && w[5].kind == Kind::Str && w[6].text == ")";
                if !literal {
                    out.push(format!("{}:{}", file.display(), w[0].line));
                }
            }
        }
    }
    out
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
    // The review of 10 Oct 2026: four multi-line strings that each read as
    // something other than `ureq` and walked the rename past this check.
    // The newline after the opening delimiter is not content, in both forms.
    assert_eq!(
        declarers_of("[dependencies]\nnet = { package = \"\"\"\nureq\"\"\" }\n"),
        ["ureq"]
    );
    assert_eq!(
        declarers_of("[dependencies.net]\npackage = '''\nureq'''\n"),
        ["ureq"]
    );
    // A line-ending backslash swallows the newline and the indent after it.
    assert_eq!(
        declarers_of("[dependencies.net]\npackage = \"\"\"ure\\\n   q\"\"\"\n"),
        ["ureq"]
    );
    assert_eq!(
        declarers_of("[dependencies.net]\npackage = \"\"\"ure\\  \r\n\r\n  q\"\"\"\n"),
        ["ureq"]
    );
    // Escapes decode in a multi-line basic string as in a one-line one.
    assert_eq!(
        declarers_of("[dependencies.net]\npackage = \"\"\"\\u0075req\"\"\"\n"),
        ["ureq"]
    );
    // TOML 1.1's `\xHH`, which cargo 1.98.1 resolves to ureq (second review
    // of 10 Oct 2026); the decoder read `x75req`.
    assert_eq!(
        declarers_of("[dependencies]\nnet = { package = \"\\x75req\", version = \"3\" }\n"),
        ["ureq"]
    );
    assert_eq!(
        declarers_of("[dependencies.net]\npackage = \"\"\"\\x75r\\x65q\"\"\"\n"),
        ["ureq"]
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
    // An escape Cargo refuses is not a letter to copy through.
    assert!(dependencies("[dependencies]\nnet = { package = \"\\qureq\" }\n").is_err());
    assert!(dependencies("[dependencies]\nnet = { package = \"\\x7\" }\n").is_err());
}

fn socket_lines(src: &str) -> Vec<usize> {
    raw_network(&rust_lex::tokens(src))
        .into_iter()
        .map(|(line, _)| line)
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
fn a_socket_from_libc_or_a_resolver_is_a_socket() {
    // The review of 10 Oct 2026, second pass, verbatim in spirit: libc is
    // already linked, so this needs no dependency and names no `std::net` type.
    let ffi = r#"unsafe extern "C" {
    fn socket(domain: i32, ty: i32, proto: i32) -> i32;
    fn connect(fd: i32, addr: *const u8, len: u32) -> i32;
}
pub fn f() { unsafe { let fd = socket(2, 1, 0); connect(fd, std::ptr::null(), 16); } }
"#;
    let lines = |src: &str| -> Vec<(usize, String)> { raw_network(&rust_lex::tokens(src)) };
    assert_eq!(
        lines(ffi),
        [
            (2, "extern socket".to_string()),
            (3, "extern connect".to_string())
        ]
    );
    // A `#[link_name]` renames the Rust side, not the symbol.
    assert_eq!(
        lines("extern {\n    #[link_name = \"connect\"]\n    fn tidy(fd: i32) -> i32;\n}\n").len(),
        1
    );
    // The names that defeat a name scan are red to declare at all.
    assert_eq!(
        lines("extern \"C\" { fn syscall(n: i64, ...) -> i64; fn dlsym(h: *mut u8, s: *const u8) -> *mut u8; }")
            .len(),
        2
    );
    // Winsock, the Windows spelling of the same calls.
    assert_eq!(
        lines(
            "#[link(name = \"ws2_32\")]\nunsafe extern \"system\" {\n    fn WSAConnect(s: usize) -> i32;\n}\n"
        ),
        [(3, "extern WSAConnect".to_string())]
    );
    // Through a crate path, or imported bare.
    assert_eq!(
        lines("fn f() { unsafe { libc::connect(fd, a, l) }; }").len(),
        1
    );
    assert_eq!(
        lines("fn f() { nix::sys::socket::bind(fd, &a); }").len(),
        2,
        "both `socket` (the module) and `bind` are under nix::"
    );
    assert_eq!(
        lines("use libc::{c_int, socket};\nfn f() { socket(2, 1, 0); }").len(),
        1
    );
    assert_eq!(lines("use libc::*;\n").len(), 1);
    // DNS is a way out too.
    assert_eq!(
        lines("use std::net::ToSocketAddrs;\nfn f() { \"x.example:80\".to_socket_addrs(); }"),
        [
            (1, "ToSocketAddrs".to_string()),
            (2, "to_socket_addrs".to_string())
        ]
    );
    // Not every `connect` is a socket: a method, a definition, an unrelated
    // extern block, and an `extern "C" fn` that is not an import.
    assert!(lines("fn f() { pool.connect(); db::connect(); x.bind(y); }").is_empty());
    assert!(lines("fn connect() {} extern \"C\" fn socket() {} unsafe extern \"C\" { fn setsid() -> i32; fn killpg(g: i32, s: i32) -> i32; } fn bind() {}").is_empty());
    assert!(lines("extern crate core; fn listen() {}").is_empty());

    // And through the whole scan, in a crate outside the monopoly.
    let ws = FakeWorkspace::new("ffi");
    ws.write("tacet-x/Cargo.toml", "[package]\nname = \"tacet-x\"\n");
    ws.write("tacet-x/src/lib.rs", ffi);
    let offenders = socket_offenders(&ws.0);
    assert_eq!(offenders.len(), 2, "{offenders:#?}");
}

/// The review of 10 Oct 2026, third pass: each of these compiled into
/// tacet-tools (or, for the two extern shapes, under rustc on macOS with a
/// socket fd to show for it) while the suite stayed green.
#[test]
fn a_socket_layer_under_another_name_or_behind_a_macro_is_a_socket() {
    let lines = |src: &str| -> Vec<(usize, String)> { raw_network(&rust_lex::tokens(src)) };
    // A second name for libc, three ways.
    assert!(!lines("use libc as c;\nfn f() { unsafe { c::connect(3, p, 16) }; }").is_empty());
    assert!(!lines("use libc::{self as c};\nfn f() { unsafe { c::socket(2, 1, 0) }; }").is_empty());
    assert!(
        !lines("extern crate libc as sys;\nfn f() { unsafe { sys::connect(3, p, 16) }; }")
            .is_empty()
    );
    // An alias of the alias.
    assert!(
        lines("use libc as c;\nuse c as d;\nfn f() { unsafe { d::socket(2, 1, 0) }; }")
            .iter()
            .any(|(l, _)| *l == 3)
    );
    // rustix, whose calls carry suffixes no list keeps up with.
    assert!(!lines("fn f() { rustix::net::connect(&fd, &a); }").is_empty());
    assert!(!lines("fn f() { let s = rustix::net::socket_with(a, b, c, d); }").is_empty());
    assert!(!lines("use rustix::net as n;\nfn f() { n::connect_v4(&fd, &a); }").is_empty());
    // A name the scan cannot read in a foreign block.
    assert!(!lines(
        "extern \"C\" { #[link_name = concat!(\"soc\", \"ket\")] fn tidy(a: i32, b: i32, c: i32) -> i32; }"
    )
    .is_empty());
    assert!(!lines(
        "macro_rules! imp { ($n:ident) => { extern \"C\" { fn $n(a: i32) -> i32; } } }\nimp!(connect);"
    )
    .is_empty());
    // And any foreign function nobody listed, whatever it is called.
    assert_eq!(
        lines("unsafe extern \"C\" { fn tidy(a: i32) -> i32; }"),
        [(1, "extern fn tidy (not in EXTERN_FNS)".to_string())]
    );
    // NOT EVERYTHING: the tree's own three foreign functions, a libc-free
    // `std::net` address type, and an unrelated alias.
    assert!(lines("unsafe extern \"C\" { fn setsid() -> i32; fn killpg(g: i32, s: i32) -> i32; fn poll(f: *mut u8, n: u64, t: i32) -> i32; }").is_empty());
    assert!(
        lines(
            "use std::net::IpAddr; use std::process::Command as Run; fn f() { Run::new(\"ls\"); }"
        )
        .is_empty()
    );

    // The manifest side: a socket layer declared under any name, inherited or
    // not, is red outside the monopoly and not inside it.
    let found = socket_layer_declarers(
        "[workspace.dependencies]\nsys = { package = \"libc\", version = \"0.2\" }\n",
        &[
            ("tacet-tools".into(), "[dependencies]\nc = { package = \"libc\", version = \"0.2\" }\n".into()),
            ("tacet-eval".into(), "[target.'cfg(unix)'.dependencies]\nrustix = { version = \"1\", features = [\"net\"] }\n".into()),
            ("tacet-zip".into(), "[dependencies]\nsys.workspace = true\n".into()),
            ("tacet-web".into(), "[dependencies]\nlibc = \"0.2\"\n".into()),
        ],
    );
    assert_eq!(
        found,
        [
            "tacet-eval: rustix (as `rustix`)",
            "tacet-tools: libc (as `c`)",
            "tacet-zip: libc (as `sys`)",
        ]
    );

    // The graph side: rustix one crate removed, with and without `net`.
    let meta = |features: serde_json::Value| {
        serde_json::json!({
            "packages": [
                {"id": "cli", "name": "tacet-cli"},
                {"id": "term", "name": "crossterm"},
                {"id": "rx", "name": "rustix"},
            ],
            "workspace_members": ["cli"],
            "resolve": {"nodes": [
                {"id": "cli", "deps": [{"pkg": "term"}], "features": []},
                {"id": "term", "deps": [{"pkg": "rx"}], "features": []},
                {"id": "rx", "deps": [], "features": features},
            ]},
        })
    };
    assert!(socket_feature_offenders(&meta(serde_json::json!(["std", "termios"]))).is_empty());
    assert_eq!(
        socket_feature_offenders(&meta(serde_json::json!(["std", "net"]))),
        ["tacet-cli -> rustix[net]"]
    );
}

#[test]
fn a_crate_outside_crates_is_red() {
    let meta = serde_json::json!({
        "packages": [
            {"id": "cli", "name": "tacet-cli", "source": null,
             "manifest_path": "/w/crates/tacet-cli/Cargo.toml"},
            {"id": "serde", "name": "serde",
             "source": "registry+https://github.com/rust-lang/crates.io-index",
             "manifest_path": "/home/u/.cargo/registry/src/serde-1/Cargo.toml"},
            // A path dependency of a member, outside crates/.
            {"id": "helper", "name": "helper", "source": null,
             "manifest_path": "/w/vendor/helper/Cargo.toml"},
            // A workspace member outside crates/.
            {"id": "xtask", "name": "xtask", "source": null,
             "manifest_path": "/w/xtask/Cargo.toml"},
            // One level too deep: `manifests_in` reads crates/*/Cargo.toml only.
            {"id": "deep", "name": "deep", "source": null,
             "manifest_path": "/w/crates/tacet-cli/fuzz/Cargo.toml"},
        ],
        "workspace_members": ["cli", "xtask"],
    });
    let outside = local_packages_outside(&meta, Path::new("/w/crates"));
    assert_eq!(
        outside,
        [
            "helper (/w/vendor/helper/Cargo.toml)",
            "xtask (/w/xtask/Cargo.toml)",
            "deep (/w/crates/tacet-cli/fuzz/Cargo.toml)",
        ]
    );
}

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

    let unlisted = agent_spawn_problems(&ws.0, &[], &[], &external);
    assert_eq!(unlisted.len(), 2, "{unlisted:#?}");
    assert!(unlisted.iter().any(|p| p.contains("claude.rs:1")));
    assert!(unlisted.iter().any(|p| p.contains("codex.rs:2")));

    let listed = [
        ("sessions/claude.rs", "session_resume"),
        ("sessions/codex.rs", "session_resume"),
    ];
    assert!(agent_spawn_problems(&ws.0, &listed, &[], &external).is_empty());

    // Listed, but under a tool the approval gate does not know.
    let ungated = [
        ("sessions/claude.rs", "session_resume"),
        ("sessions/codex.rs", "codex_bridge"),
    ];
    let problems = agent_spawn_problems(&ws.0, &ungated, &[], &external);
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(problems[0].contains("codex_bridge"));
}

#[test]
fn an_agent_cli_is_red_however_it_is_spelled_and_wherever_it_is() {
    // The review of 10 Oct 2026: each of these got past the `Command::new(`
    // shape with the program name written out in full.
    let ws = FakeWorkspace::new("agentbypass");
    ws.write(
        "tacet-tools/Cargo.toml",
        "[package]\nname = \"tacet-tools\"\n",
    );
    ws.write(
        "tacet-tools/src/alias.rs",
        "use std::process::Command as Cmd;\nfn f() { Cmd::new(\"claude\").arg(\"-p\"); }\n",
    );
    ws.write(
        "tacet-tools/src/npx.rs",
        "fn f() { Command::new(\"npx\")\n    .args([\"-y\", \"@anthropic-ai/claude-code\", \"-p\"]); }\n",
    );
    ws.write(
        "tacet-tools/src/sh.rs",
        "fn f() { Command::new(\"sh\").args([\"-c\", \"cd /w && codex exec resume\"]); }\n",
    );
    ws.write(
        "tacet-tools/src/helper.rs",
        "fn f() { shell_line(\"/usr/local/bin/Gemini\", &[]); env(\"env FOO=1 agy -p\"); }\n",
    );
    // The crates that own a socket are scanned too: a socket is not consent
    // to a paid turn (RG-34).
    ws.write("tacet-mcp/Cargo.toml", "[package]\nname = \"tacet-mcp\"\n");
    ws.write(
        "tacet-mcp/src/lib.rs",
        "fn f() { std::process::Command::new(\"codex\").arg(\"app-server\"); }\n",
    );
    // The review of 10 Oct 2026, second pass: rustc decodes these escapes to
    // `claude`, `codex` and `claude`; the lexer used to read `clu{61}ude`,
    // `x63odex`, and `cl` + `aude`.
    ws.write(
        "tacet-tools/src/escaped.rs",
        r#"fn f() {
    Command::new("cl\u{61}ude").arg("-p");
    Command::new("\x63odex").arg("app-server");
    Command::new("cl\
        aude").arg("-p");
    Command::new(b"\x61gy");
}
"#,
    );
    // Text that names one and runs nothing stays green.
    ws.write(
        "tacet-tools/src/prose.rs",
        "const A: &str = \"read the claude session\";\nconst B: &str = \".claude/projects\";\nconst C: &str = \"claude-code\";\nconst D: &str = \"CLAUDE.md: update the README\";\n",
    );
    let external = vec!["web_search".to_string()];
    let problems = agent_spawn_problems(&ws.0, &[], &[], &external);
    let hit = |file: &str, line: usize| {
        problems
            .iter()
            .any(|p| p.replace('\\', "/").contains(&format!("{file}:{line}:")))
    };
    assert!(hit("src/alias.rs", 2), "{problems:#?}");
    assert!(hit("src/npx.rs", 2), "{problems:#?}");
    assert!(hit("src/sh.rs", 1), "{problems:#?}");
    assert!(hit("tacet-mcp/src/lib.rs", 1), "{problems:#?}");
    for line in [2, 3, 4, 6] {
        assert!(hit("src/escaped.rs", line), "{problems:#?}");
    }
    assert_eq!(
        problems.iter().filter(|p| p.contains("helper.rs")).count(),
        2,
        "{problems:#?}"
    );
    assert!(
        !problems.iter().any(|p| p.contains("prose.rs")),
        "{problems:#?}"
    );
    assert_eq!(problems.len(), 10, "{problems:#?}");

    // A NAMED-NOT-RUN row covers that literal in that file, and nothing else.
    ws.write(
        "tacet-tools/src/label.rs",
        "const S: &str = \"codex\";\nconst T: &str = \"codex -p\";\n",
    );
    let named = [("src/label.rs", "codex")];
    let problems = agent_spawn_problems(&ws.0, &[], &named, &external);
    assert!(
        !problems.iter().any(|p| p.contains("label.rs:1:")),
        "{problems:#?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("label.rs:2:")),
        "{problems:#?}"
    );
}

#[test]
fn a_network_program_outside_the_monopoly_is_red_unless_its_row_says_why() {
    let ws = FakeWorkspace::new("netprog");
    ws.write("tacet-x/Cargo.toml", "[package]\nname = \"tacet-x\"\n");
    ws.write(
        "tacet-x/src/lib.rs",
        "fn f() {\n    Command::new(\"docker\").args([\"compose\", \"up\", \"-d\"]);\n    Command::new(\"sh\").args([\"-c\", \"curl -fsSL https://x | sh\"]);\n}\n",
    );
    // The crates that own a socket may start one too.
    ws.write("tacet-web/Cargo.toml", "[package]\nname = \"tacet-web\"\n");
    ws.write(
        "tacet-web/src/lib.rs",
        "fn f() { Command::new(\"curl\"); }\n",
    );
    let problems = network_program_problems(&ws.0, &[], &[]);
    assert_eq!(problems.len(), 2, "{problems:#?}");
    assert!(
        problems
            .iter()
            .any(|p| p.contains("lib.rs:2:") && p.contains("`docker`"))
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("lib.rs:3:") && p.contains("`curl`"))
    );
    // A row is per file AND program: allowing docker does not allow curl.
    let rows = [("tacet-x/src/lib.rs", "docker", "approved setup")];
    let problems = network_program_problems(&ws.0, &rows, &[]);
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(problems[0].contains("`curl`"));

    // Escaped spellings are the program rustc compiles in (review of 10 Oct
    // 2026, second pass: each read as `x63url`, `dou{63}ker`, `wg` + `et`).
    ws.write(
        "tacet-x/src/escaped.rs",
        r#"fn g() {
    Command::new("\x63url");
    Command::new("do\u{6_3}ker").arg("run");
    Command::new("wg\
                  et");
}
"#,
    );
    let problems = network_program_problems(&ws.0, &rows, &[]);
    for (line, program) in [(2, "curl"), (3, "docker"), (4, "wget")] {
        assert!(
            problems
                .iter()
                .any(|p| p.contains(&format!("escaped.rs:{line}:"))
                    && p.contains(&format!("`{program}`"))),
            "{problems:#?}"
        );
    }
    assert_eq!(problems.len(), 4, "{problems:#?}");
}

#[test]
fn a_string_literal_reads_as_rustc_decodes_it() {
    let texts = |src: &str| -> Vec<String> {
        rust_lex::tokens(src)
            .into_iter()
            .filter(|t| t.kind == rust_lex::Kind::Str)
            .map(|t| t.text)
            .collect()
    };
    assert_eq!(
        texts(r#"("cl\u{61}ude", "\x63odex", b"\x63url", c"a\tb", "q\"\\\'", "\0")"#),
        ["claude", "codex", "curl", "a\tb", "q\"\\'", "\0"]
    );
    // STRING_CONTINUE eats the newline and the indent, not the text after.
    assert_eq!(texts("\"cl\\\n    \t aude\" x"), ["claude"]);
    assert_eq!(texts("\"cl\\\r\n  aude\""), ["claude"]);
    // Raw strings have no escapes, and a malformed escape keeps its letter.
    assert_eq!(texts(r##"r"cl\u{61}ude" "\xZZ""##), [r"cl\u{61}ude", "xZZ"]);
    // And the line count still sees the newlines a continuation swallowed.
    let toks = rust_lex::tokens("\"a\\\n  b\"\nTcpStream");
    assert_eq!(toks.last().map(|t| t.line), Some(3));
}

#[test]
fn a_spawn_from_a_variable_is_red_until_it_is_listed() {
    let ws = FakeWorkspace::new("runtime");
    ws.write("tacet-x/Cargo.toml", "[package]\nname = \"tacet-x\"\n");
    ws.write(
        "tacet-x/src/alias.rs",
        "use std::process::{Command as Run, Stdio};\nfn f(p: &str) { Run::new(p); }\n",
    );
    ws.write(
        "tacet-x/src/concat.rs",
        "fn f() { std::process::Command::new(concat!(\"cla\", \"ude\")); }\n",
    );
    ws.write(
        "tacet-x/src/literal.rs",
        "fn f() { Command::new(\"ls\"); Command::new(r\"ps\"); }\n",
    );
    // The review of 10 Oct 2026: a literal first token is not a literal
    // argument, and a `type` alias is an alias.
    ws.write(
        "tacet-x/src/assembled.rs",
        "fn f() { std::process::Command::new(\"cla\".to_owned() + \"ude\").arg(\"-p\"); }\n",
    );
    ws.write(
        "tacet-x/src/typealias.rs",
        "type Run = std::process::Command;\nfn f(p: &str) { Run::new(p); }\n",
    );
    let sites = runtime_spawn_sites(&ws.0, &[]);
    assert_eq!(sites.len(), 4, "{sites:#?}");
    assert!(
        sites.iter().any(|s| s.ends_with("assembled.rs:1")),
        "{sites:#?}"
    );
    assert!(
        sites.iter().any(|s| s.ends_with("typealias.rs:2")),
        "{sites:#?}"
    );
    assert!(
        sites.iter().any(|s| s.ends_with("alias.rs:2")),
        "{sites:#?}"
    );
    assert!(
        sites.iter().any(|s| s.ends_with("concat.rs:1")),
        "{sites:#?}"
    );
    let listed = [
        ("src/alias.rs", "a test"),
        ("src/concat.rs", "a test"),
        ("src/assembled.rs", "a test"),
        ("src/typealias.rs", "a test"),
    ];
    assert!(runtime_spawn_sites(&ws.0, &listed).is_empty());
}

#[test]
fn code_compiled_from_outside_the_crate_or_from_a_non_rs_file_is_scanned() {
    // The review of 10 Oct 2026, verbatim: both returned no offender.
    let ws = FakeWorkspace::new("pathinclude");
    ws.write("tacet-x/Cargo.toml", "[package]\nname = \"tacet-x\"\n");
    ws.write(
        "tacet-x/src/lib.rs",
        "#[path = \"../../shared/net.rs\"]\nmod net;\ninclude!(\"net.inc\");\n",
    );
    ws.write("shared/net.rs", SOCKET_RS);
    ws.write("tacet-x/src/net.inc", SOCKET_RS);
    // `cfg_attr` carries a path just as well.
    ws.write("tacet-y/Cargo.toml", "[package]\nname = \"tacet-y\"\n");
    ws.write(
        "tacet-y/src/lib.rs",
        "#[cfg_attr(unix, path = \"../../shared_y/u.rs\")]\nmod u;\n",
    );
    ws.write("shared_y/u.rs", SOCKET_RS);
    // A crate root outside the directory, and the plain `mod` it declares.
    ws.write(
        "tacet-z/Cargo.toml",
        "[package]\nname = \"tacet-z\"\n[lib]\npath = \"../shared_z/lib.rs\"\n",
    );
    ws.write("shared_z/lib.rs", "mod net;\n");
    ws.write("shared_z/net.rs", SOCKET_RS);
    // A `#[path]` inside an inline module resolves under that module's name.
    ws.write("tacet-v/Cargo.toml", "[package]\nname = \"tacet-v\"\n");
    ws.write(
        "tacet-v/src/lib.rs",
        "mod a {\n    #[path = \"../../../shared_v/n.rs\"]\n    mod n;\n}\n",
    );
    ws.write("shared_v/n.rs", SOCKET_RS);
    let offenders = socket_offenders(&ws.0);
    for file in [
        "shared/net.rs",
        "tacet-x/src/net.inc",
        "shared_y/u.rs",
        "shared_z/net.rs",
        "shared_v/n.rs",
    ] {
        assert!(
            offenders.iter().any(|o| o
                .replace('\\', "/")
                .contains(&format!("{file}:1: TcpStream"))),
            "{file} was not scanned: {offenders:#?}"
        );
    }
    assert_eq!(offenders.len(), 5, "{offenders:#?}");
}

#[test]
fn a_pointer_to_code_the_scan_cannot_read_is_red() {
    let ws = FakeWorkspace::new("unfollowable");
    ws.write("tacet-x/Cargo.toml", "[package]\nname = \"tacet-x\"\n");
    ws.write(
        "tacet-x/src/lib.rs",
        "include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"));\n#[path = \"nowhere.rs\"]\nmod m;\n",
    );
    let offenders = socket_offenders(&ws.0);
    assert_eq!(offenders.len(), 2, "{offenders:#?}");
    assert!(offenders.iter().any(|o| o.contains("lib.rs:1: include!")));
    assert!(offenders.iter().any(|o| o.contains("lib.rs:2: #[path")));
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
