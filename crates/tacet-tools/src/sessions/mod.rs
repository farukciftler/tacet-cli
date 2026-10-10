//! Other agents' sessions: Claude Code, Codex CLI and Antigravity, read from
//! their local files, continued here, and reported back in a shared handoff.
//!
//! WHAT THIS LAYER IS. The shared ground under the three readers: the session
//! model (`model`), the one door files are opened through (`secrets`), the
//! version gate (`version`) and the handoff record (`handoff`). The readers
//! themselves, and the `agent-sessions` addon that puts them in the catalog,
//! build on this; nothing here is reachable from the model until that addon
//! is installed.
//!
//! WHAT IT NEVER DOES. Open a socket (the network monopoly stays with
//! tacet-web and tacet-mcp), parse a format it would need a new crate for
//! (`.zst`, protobuf, SQLite), read a credential, or write into another
//! agent's storage outside a measured version and an explicit approval. The
//! reasons, and what was measured to reach them, are in
//! docs/adr/0001-agent-sessions.md.

pub mod anonymise;
pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod handoff;
pub mod model;
pub mod secrets;
pub mod version;
pub mod writeback;

pub use handoff::{HandoffRecord, HandoffTask, Status};
pub use model::{
    AgentKind, AgentSession, Entry, EntryKind, PlanItem, Role, SessionError, SessionSource,
    SessionSummary,
};

/// A tool that returns another agent's transcript taints the session (audit
/// H2, RG-01): the text is the user's code, commands and prompts. Stated once
/// so the tool that wraps a reader returns this rather than its own opinion.
pub const TAINTS_SESSION: bool = true;

/// ...and it is text written OUTSIDE this session (audit H2-c, RG-01): a
/// transcript that says "run make install" is someone else's instruction, so
/// after it every host-acting tool asks. The other half of the rule, pinned
/// beside the first because the trait default for `external_content` is
/// `false` — a reader that forgot to override it would hand a transcript's
/// commands to `shell` with no question asked (review of 10 Oct 2026).
pub const EXTERNAL_CONTENT: bool = true;

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn sessions_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sessions")
    }

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// One token of Rust source, with the line it starts on. Comments, string
    /// literals and char literals are gone before tokens are made, so text
    /// that merely MENTIONS an opener (this module's own fixtures, a doc
    /// comment) is not mistaken for one.
    #[derive(Debug, Clone, PartialEq)]
    struct Tok {
        text: String,
        line: usize,
    }

    /// Rust source with comments and the insides of string and char literals
    /// replaced by spaces, line breaks kept so line numbers survive. Enough
    /// lexer for this guard: block comments nest, raw strings take their
    /// hashes, and `'a` is told apart from `'a'`.
    fn skeleton(src: &str) -> String {
        let c: Vec<char> = src.chars().collect();
        let mut out = String::with_capacity(src.len());
        let blank = |ch: char| if ch == '\n' { '\n' } else { ' ' };
        let mut i = 0;
        while i < c.len() {
            if c[i] == '/' && c.get(i + 1) == Some(&'/') {
                while i < c.len() && c[i] != '\n' {
                    out.push(' ');
                    i += 1;
                }
            } else if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                let mut depth = 0;
                while i < c.len() {
                    if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                        depth += 1;
                        out.push_str("  ");
                        i += 2;
                    } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        out.push_str("  ");
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        out.push(blank(c[i]));
                        i += 1;
                    }
                }
            } else if c[i] == 'r'
                && c.get(i + 1) == Some(&'#')
                && c.get(i + 2).is_some_and(|n| n.is_alphabetic() || *n == '_')
                && (i == 0 || !(c[i - 1].is_alphanumeric() || c[i - 1] == '_'))
            {
                // `r#open` IS `open`. Read as a raw string's start, it became
                // the tokens `r`, `#`, `open`, and `std::fs::File::r#open(p)`
                // and `std::fs::r#read(p)` compiled past the guard (RG-16 third
                // review, rustc 1.98.1).
                out.push_str("  ");
                i += 2;
            } else if c[i] == 'r'
                && (c.get(i + 1) == Some(&'"') || c.get(i + 1) == Some(&'#'))
                && (i == 0 || !(c[i - 1].is_alphanumeric() || c[i - 1] == '_'))
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while c.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if c.get(j) != Some(&'"') {
                    out.push(c[i]);
                    i += 1;
                    continue;
                }
                out.push_str(&" ".repeat(j + 1 - i));
                i = j + 1;
                while i < c.len() {
                    if c[i] == '"' && (1..=hashes).all(|k| c.get(i + k) == Some(&'#')) {
                        out.push_str(&" ".repeat(hashes + 1));
                        i += hashes + 1;
                        break;
                    }
                    out.push(blank(c[i]));
                    i += 1;
                }
            } else if c[i] == '"' {
                out.push(' ');
                i += 1;
                while i < c.len() && c[i] != '"' {
                    if c[i] == '\\' {
                        out.push(' ');
                        i += 1;
                    }
                    if i < c.len() {
                        out.push(blank(c[i]));
                        i += 1;
                    }
                }
                out.push(' ');
                i += 1;
            } else if c[i] == '\'' {
                // `'x'` or `'\n'` is a char literal; `'a` alone is a lifetime.
                let end = if c.get(i + 1) == Some(&'\\') {
                    (i + 2..c.len()).find(|&k| c[k] == '\'')
                } else if c.get(i + 2) == Some(&'\'') {
                    Some(i + 2)
                } else {
                    None
                };
                match end {
                    Some(e) => {
                        out.push_str(&" ".repeat(e + 1 - i));
                        i = e + 1;
                    }
                    None => {
                        out.push(' ');
                        i += 1;
                    }
                }
            } else {
                out.push(c[i]);
                i += 1;
            }
        }
        out
    }

    fn tokens(src: &str) -> Vec<Tok> {
        let c: Vec<char> = skeleton(src).chars().collect();
        let mut out = Vec::new();
        let (mut i, mut line) = (0, 1);
        while i < c.len() {
            let ch = c[i];
            if ch == '\n' {
                line += 1;
                i += 1;
            } else if ch.is_whitespace() {
                i += 1;
            } else if ch.is_alphanumeric() || ch == '_' {
                let start = i;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                    i += 1;
                }
                out.push(Tok {
                    text: c[start..i].iter().collect(),
                    line,
                });
            } else if ch == ':' && c.get(i + 1) == Some(&':') {
                out.push(Tok {
                    text: "::".into(),
                    line,
                });
                i += 2;
            } else {
                out.push(Tok {
                    text: ch.to_string(),
                    line,
                });
                i += 1;
            }
        }
        out
    }

    /// The tokens outside `#[cfg(test)] mod … { … }` blocks. ONLY a test
    /// MODULE is skipped, and only up to its closing brace: the old guard
    /// stopped at the first `#[cfg(test)]` anywhere, so a one-line
    /// `#[cfg(test)] fn x() {}` above production code hid everything after it
    /// (RG-16 review, break B15). A `#[cfg(test)]` on anything else — a
    /// helper function, an `impl` — is scanned like production code.
    fn production(toks: &[Tok]) -> Vec<Tok> {
        let is = |i: usize, t: &str| toks.get(i).is_some_and(|k| k.text == t);
        // `# [ ... ]` starting at i: the index after its `]`.
        let attr_end = |i: usize| -> Option<usize> {
            if !(is(i, "#") && is(i + 1, "[")) {
                return None;
            }
            let mut depth = 0;
            for (k, t) in toks.iter().enumerate().skip(i + 1) {
                match t.text.as_str() {
                    "[" => depth += 1,
                    "]" => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(k + 1);
                        }
                    }
                    _ => {}
                }
            }
            None
        };
        let mut out = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            let cfg_test = ["#", "[", "cfg", "(", "test", ")", "]"]
                .iter()
                .enumerate()
                .all(|(k, t)| is(i + k, t));
            if cfg_test {
                let mut j = i + 7;
                while let Some(next) = attr_end(j) {
                    j = next;
                }
                if is(j, "pub") {
                    j += 1;
                    if is(j, "(") {
                        while j < toks.len() && !is(j, ")") {
                            j += 1;
                        }
                        j += 1;
                    }
                }
                if is(j, "mod") && is(j + 2, "{") {
                    let mut depth = 0;
                    let mut k = j + 2;
                    while k < toks.len() {
                        match toks[k].text.as_str() {
                            "{" => depth += 1,
                            "}" => depth -= 1,
                            _ => {}
                        }
                        k += 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    i = k;
                    continue;
                }
            }
            out.push(toks[i].clone());
            i += 1;
        }
        out
    }

    /// `a :: b` call paths that open a file for reading, as token triples.
    /// Matching tokens rather than text means `std::fs::read::<&Path>(p)` and
    /// `fs :: read (p)` are seen too, and `read_dir`/`metadata` are not.
    ///
    /// `fs::copy` reads its source whole. A copy of `auth.json` named
    /// `x.jsonl` has its own inode and one link, so neither the name rules
    /// nor the hard-link rule can tell it from a transcript once it is read
    /// back through the gate (RG-16 second review: the guard stayed green with
    /// a planted `std::fs::copy`). `fs::rename` is NOT listed: the handoff
    /// writer's atomic temp-then-rename needs it, and it reads nothing.
    const OPENERS: &[(&str, &str)] = &[
        ("fs", "read"),
        ("fs", "read_to_string"),
        ("fs", "copy"),
        ("File", "open"),
        ("File", "options"),
        ("OpenOptions", "new"),
    ];

    /// Names that, imported from an `fs` module, open a file under a name the
    /// call-path check cannot see: `use std::fs::read_to_string;` and then a
    /// bare `read_to_string(p)` (break B14).
    const IMPORTED_OPENERS: &[&str] =
        &["read", "read_to_string", "copy", "File", "OpenOptions", "*"];

    /// Every path a `use` tree brings in, as (segments, alias). Expands
    /// `use std::{fs::{self as f, File}, io};` into its leaves.
    fn use_leaves(
        toks: &[Tok],
        i: &mut usize,
        prefix: Vec<String>,
        out: &mut Vec<(Vec<String>, Option<String>, usize)>,
    ) {
        let mut prefix = prefix;
        let line = toks.get(*i).map_or(0, |t| t.line);
        while let Some(t) = toks.get(*i) {
            match t.text.as_str() {
                "::" => *i += 1,
                "{" => {
                    *i += 1;
                    loop {
                        use_leaves(toks, i, prefix.clone(), out);
                        match toks.get(*i).map(|t| t.text.as_str()) {
                            Some(",") => *i += 1,
                            Some("}") => {
                                *i += 1;
                                break;
                            }
                            _ => break,
                        }
                    }
                    return;
                }
                "*" => {
                    *i += 1;
                    prefix.push("*".into());
                    out.push((prefix, None, line));
                    return;
                }
                "}" | "," | ";" => return,
                word => {
                    *i += 1;
                    if toks.get(*i).is_some_and(|t| t.text == "::") {
                        prefix.push(word.to_string());
                        continue;
                    }
                    let alias = if toks.get(*i).is_some_and(|t| t.text == "as") {
                        *i += 1;
                        let a = toks.get(*i).map(|t| t.text.clone());
                        *i += 1;
                        a
                    } else {
                        None
                    };
                    prefix.push(word.to_string());
                    out.push((prefix, alias, line));
                    return;
                }
            }
        }
    }

    /// Why one production token stream opens a file without the gate, one
    /// entry per finding: (line, what).
    fn openers_in(toks: &[Tok]) -> Vec<(usize, String)> {
        let mut found = Vec::new();
        // THE TYPES THEMSELVES, wherever they are named. Matching call paths
        // was patched one spelling at a time — `File::open`, then
        // `<std::fs::File>::open` — and `type F = std::fs::File; F::open(p)`
        // walked past both (RG-16 third review). Nothing outside the door has
        // a reason to name either type, so naming one is the finding.
        for t in toks {
            if t.text == "File" || t.text == "OpenOptions" {
                found.push((t.line, format!("names {}", t.text)));
            }
        }
        for (k, head) in toks.iter().enumerate() {
            // `<std::fs::File>::open(p)` is the same call as `File::open(p)`
            // with a `>` between the type and the `::`; a plain three-token
            // window never saw it (RG-16 second review, planted in version.rs).
            let mut at = k + 1;
            if toks.get(at).is_some_and(|t| t.text == ">") {
                at += 1;
            }
            let (Some(sep), Some(tail)) = (toks.get(at), toks.get(at + 1)) else {
                continue;
            };
            if sep.text == "::" && OPENERS.contains(&(head.text.as_str(), tail.text.as_str())) {
                found.push((head.line, format!("{}::{}", head.text, tail.text)));
            }
        }
        for (k, t) in toks.iter().enumerate() {
            if t.text != "use" {
                continue;
            }
            let mut i = k + 1;
            let mut leaves = Vec::new();
            use_leaves(toks, &mut i, Vec::new(), &mut leaves);
            for (path, alias, line) in leaves {
                let Some(fs_at) = path.iter().position(|s| s == "fs") else {
                    continue;
                };
                let last = path.last().map(String::as_str).unwrap_or_default();
                let tail = &path[fs_at + 1..];
                // `use std::fs as f;` / `use std::fs::{self as f}` — every
                // call through `f::` dodges the call-path check (break B13).
                let fs_alias = alias.is_some() && (tail.is_empty() || tail == ["self"]);
                if fs_alias || (tail.len() == 1 && IMPORTED_OPENERS.contains(&last)) {
                    let alias = alias.map(|a| format!(" as {a}")).unwrap_or_default();
                    found.push((line, format!("use {}{alias}", path.join("::"))));
                }
            }
        }
        found
    }

    /// Every file under `dir` other than `dir/secrets.rs` that opens a file
    /// for reading, with the line. Walks the directory rather than listing
    /// files, so the next reader added here (claude.rs, codex.rs…) is covered
    /// without anyone remembering to add it. Only the TOP-LEVEL `secrets.rs`
    /// is the door; a `secrets.rs` in a subdirectory is just a file.
    fn bypasses(dir: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let door = dir.join("secrets.rs");
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") || path == door {
                    continue;
                }
                // Source files of this crate, read by a test: not a session file.
                let text = std::fs::read_to_string(&path).unwrap();
                let toks = production(&tokens(&text));
                for (line, what) in openers_in(&toks) {
                    found.push(format!("{}:{line}: {what}", path.display()));
                }
            }
        }
        found.sort();
        found
    }

    /// RG-16: the denylist only protects if nothing walks around it.
    #[test]
    fn every_reader_goes_through_the_secrets_gate() {
        let found = bypasses(&sessions_dir());
        assert!(
            found.is_empty(),
            "these lines open a file without `secrets::open`/`read_text`/`read_tail`, \
             so the credential denylist never sees them:\n{}",
            found.join("\n")
        );
    }

    /// The guard above is only worth having if it can fail: every bypass the
    /// RG-16 review made by hand (B12–B15) IS reported, and the things that
    /// only look like one are not.
    #[test]
    fn the_gate_guard_sees_a_direct_open() {
        let dir = std::env::temp_dir().join(format!("tacet-sessions-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        // Openers are assembled from pieces so this file does not hold them.
        let fs_read = concat!("fs::", "read(");
        let fs_rts = concat!("fs::", "read_to_string");
        let file_open = concat!("File", "::open(");
        let fs_copy = concat!("fs::", "copy(");
        let planted: &[(&str, String, Option<&str>)] = &[
            // The door itself, at the top level: allowed.
            (
                "secrets.rs",
                format!("let f = std::fs::{file_open}\"x\");"),
                None,
            ),
            // B12: fully qualified.
            (
                "nested/codex.rs",
                format!("let t = std::{fs_rts}(p)?;"),
                Some("nested/codex.rs:1:"),
            ),
            // A `secrets.rs` that is not THE door.
            (
                "nested/secrets.rs",
                format!("let t = std::{fs_read}p)?;"),
                Some("nested/secrets.rs:1:"),
            ),
            // B13: an aliased module.
            (
                "alias.rs",
                "use std::fs as f;\nfn g(p: &str) { f::read(p); }".into(),
                Some("alias.rs:1: use std::fs as f"),
            ),
            (
                "self_alias.rs",
                "use std::{fs::{self as f, File}, io};".into(),
                Some("self_alias.rs:1: use std::fs::self as f"),
            ),
            // B14: a bare imported reader, alone and in a group.
            (
                "bare.rs",
                "use std::fs::read_to_string;\nfn g(p: &str) { read_to_string(p); }".into(),
                Some("bare.rs:1: use std::fs::read_to_string"),
            ),
            (
                "glob.rs",
                "use std::fs::*;".into(),
                Some("glob.rs:1: use std::fs::*"),
            ),
            // B15: a `#[cfg(test)]` that is not a test module, above production.
            (
                "early.rs",
                format!("#[cfg(test)]\nfn x() {{}}\nfn f(p: &str) {{ std::{fs_rts}(p); }}"),
                Some("early.rs:3: fs::read_to_string"),
            ),
            // Production code AFTER a test module is still production code.
            (
                "after.rs",
                format!(
                    "#[cfg(test)]\nmod t {{ fn a() {{ std::{fs_read}\"Cargo.lock\"); }} }}\nfn f() {{ std::{fs_read}\"x\"); }}"
                ),
                Some("after.rs:3: fs::read"),
            ),
            // A turbofish and spaces do not hide the call path.
            (
                "spaced.rs",
                "fn f() { std::fs :: read::<&str>(\"x\"); }".into(),
                Some("spaced.rs:1: fs::read"),
            ),
            // Nor does a qualified type: `<std::fs::File>::open` puts a `>`
            // between the type and the `::` (RG-16 second review).
            (
                "qualified.rs",
                "fn f(p: &str) { let _ = <std::fs::File>::open(p); }".into(),
                Some("qualified.rs:1: File::open"),
            ),
            (
                "qualified_new.rs",
                "fn f() { let _ = < std :: fs :: OpenOptions >::new(); }".into(),
                Some("qualified_new.rs:1: OpenOptions::new"),
            ),
            // RG-16 third review: a raw identifier, and a type alias.
            (
                "raw_ident.rs",
                "fn f(p: &str) { let _ = std::fs::r#read(p); }".into(),
                Some("raw_ident.rs:1: fs::read"),
            ),
            (
                "raw_ident_open.rs",
                "fn f(p: &str) { let _ = std::fs::File::r#open(p); }".into(),
                Some("raw_ident_open.rs:1: File::open"),
            ),
            (
                "type_alias.rs",
                "type ZzF = std::fs::File;\nfn f(p: &str) { let _ = ZzF::open(p); }".into(),
                Some("type_alias.rs:1: names File"),
            ),
            // A copy reads its source whole; `auth.json` copied to `x.jsonl`
            // then passes every name rule.
            (
                "copied.rs",
                format!("fn f(p: &str) {{ std::{fs_copy}p, \"x.jsonl\"); }}"),
                Some("copied.rs:1: fs::copy"),
            ),
            (
                "copied_bare.rs",
                "use std::fs::copy;\nfn f(p: &str) { copy(p, \"x.jsonl\"); }".into(),
                Some("copied_bare.rs:1: use std::fs::copy"),
            ),
            // Not openers: a comment, a string, a test module, a directory walk.
            (
                "comment.rs",
                format!("// std::{fs_read}p) in a comment"),
                None,
            ),
            ("string.rs", format!("let s = \"std::{fs_read}p)\";"), None),
            (
                "tested.rs",
                format!(
                    "fn f() {{}}\n#[cfg(test)]\n#[allow(unused)]\nmod tests {{ fn t() {{ let c = '{{'; std::{fs_read}\"Cargo.lock\"); }} }}"
                ),
                None,
            ),
            (
                "walk.rs",
                "use std::fs::{read_dir, metadata};\nfn f() { std::fs::read_dir(\".\"); }".into(),
                None,
            ),
            // `io::copy` moves bytes between handles the gate already opened,
            // and the handoff writer's atomic rename reads nothing.
            (
                "streams.rs",
                "use std::io::copy;\nfn f() { std::io::copy(&mut a, &mut b); std::fs::rename(t, p); }"
                    .into(),
                None,
            ),
        ];
        for (name, text, _) in planted {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let found = bypasses(&dir);
        let expected: Vec<&str> = planted.iter().filter_map(|(_, _, e)| *e).collect();
        for want in &expected {
            assert!(
                found.iter().any(|f| f.contains(want)),
                "the guard missed {want:?}; it reported:\n{}",
                found.join("\n")
            );
        }
        // And nothing else: a file may be reported on more than one line (a
        // `use` of `fs::read_to_string` is also that call path), but a file
        // with no bypass is never reported.
        let reported: std::collections::BTreeSet<&str> = found
            .iter()
            .filter_map(|f| f.strip_prefix(dir.to_str().unwrap()))
            .filter_map(|f| f.trim_start_matches('/').split(':').next())
            .collect();
        let bypassing: std::collections::BTreeSet<&str> = planted
            .iter()
            .filter(|(_, _, e)| e.is_some())
            .map(|(name, _, _)| *name)
            .collect();
        assert_eq!(reported, bypassing, "{found:#?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The crates ADR 0001 decided NOT to add, by the names they would appear
    /// under in `Cargo.lock`. A transitive arrival counts too: a zstd decoder
    /// pulled in by something else would make "refuse .zst" a choice rather
    /// than a constraint, and that is a decision to make on purpose.
    const NOT_ADDED: &[&str] = &[
        "zstd",
        "zstd-sys",
        "zstd-safe",
        "ruzstd",
        "prost",
        "protobuf",
        "rusqlite",
        "libsqlite3-sys",
        "sqlx",
        "sqlite",
    ];

    fn forbidden_in_lock(lock: &str) -> Vec<String> {
        lock.lines()
            .filter_map(|l| l.trim().strip_prefix("name = \""))
            .filter_map(|l| l.strip_suffix('"'))
            .filter(|name| NOT_ADDED.contains(name))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn no_decoder_the_adr_refused_has_entered_the_dependency_graph() {
        let lock = std::fs::read_to_string(workspace_root().join("Cargo.lock")).unwrap();
        assert!(
            lock.contains("name = \"serde_json\""),
            "not the workspace lockfile"
        );
        assert_eq!(forbidden_in_lock(&lock), Vec::<String>::new());
        // And the check can see one when it is there.
        let planted = format!("{lock}\n[[package]]\nname = \"zstd\"\nversion = \"0.13.0\"\n");
        assert_eq!(forbidden_in_lock(&planted), ["zstd"]);
    }

    /// ADR 0001 restates the denylist and the version table for a reader who
    /// will not open the code. A restated number goes stale (CLAUDE.md: a stale
    /// page is a false measurement with a date on it), so every entry the code
    /// enforces must appear in the ADR, and every decision the ADR records must
    /// have its section.
    #[test]
    fn the_adr_states_what_the_code_enforces() {
        let adr = std::fs::read_to_string(workspace_root().join("docs/adr/0001-agent-sessions.md"))
            .unwrap();
        let mut missing = Vec::new();
        for name in super::secrets::DENIED_NAMES {
            if !adr.contains(&format!("`{name}`")) {
                missing.push(format!("denied name `{name}`"));
            }
        }
        for stem in super::secrets::DENIED_STEMS {
            if !adr.contains(&format!("`{stem}`")) {
                missing.push(format!("denied stem `{stem}`"));
            }
        }
        for ext in super::secrets::DENIED_EXTENSIONS {
            if !adr.contains(&format!("`*.{ext}`")) {
                missing.push(format!("denied extension `*.{ext}`"));
            }
        }
        // The substring and directory rules refuse as much as the names do;
        // the RG-05 review added `bearer` and `.kube` to the code and this
        // test stayed green until these two loops existed.
        for word in super::secrets::DENIED_SUBSTRINGS {
            if !adr.contains(&format!("`{word}`")) {
                missing.push(format!("denied substring `{word}`"));
            }
        }
        for dir in super::secrets::DENIED_DIRS {
            if !adr.contains(&format!("`{dir}`")) {
                missing.push(format!("denied directory `{dir}`"));
            }
        }
        for (dir, stem) in super::secrets::DENIED_IN_DIR {
            if !adr.contains(&format!("`{dir}/{stem}*`")) {
                missing.push(format!("denied in a directory `{dir}/{stem}*`"));
            }
        }
        for k in super::version::KNOWN {
            let read = format!("{}–{}", k.read.0, k.read.1);
            if !adr.contains(&read) {
                missing.push(format!("{:?} {:?} read range {read}", k.agent, k.flavor));
            }
            if let Some((lo, hi)) = k.write
                && !adr.contains(&format!("write {lo}–{hi}"))
            {
                missing.push(format!("{:?} write range {lo}–{hi}", k.agent));
            }
        }
        for status in super::Status::ALL {
            if !adr.contains(&format!("`{}`", status.as_str())) {
                missing.push(format!("status `{status}`"));
            }
        }
        // RG-05/RG-23/RG-33, review of 10 Oct 2026: the Scope section said
        // Tacet never runs another agent except as a gated tool, while
        // `tacet handoff note codex --program` starts the Codex binary as
        // `app-server` with the user's login. README and SECURITY send readers
        // here for the threat model, so what the shipped commands start and
        // write is held here too.
        for needle in [
            "`tacet handoff note codex <id> --program <path>`",
            "app-server`, with the user's",
            "`CODEX_HOME`",
            "`tacet handoff note claude <id>`",
            "--experimental-write-task-md",
        ] {
            if !adr.contains(needle) {
                missing.push(format!(
                    "what the handoff commands start or write: {needle}"
                ));
            }
        }
        for heading in [
            "## Scope (RG-05)",
            "## Threat model (RG-05)",
            "## Dependencies (RG-05)",
            "## Antigravity terms of service (RG-05)",
            "## Prior art and licences (RG-35)",
            "## ACP, evaluated and deferred (RG-37)",
        ] {
            if !adr.contains(heading) {
                missing.push(format!("section {heading:?}"));
            }
        }
        assert!(
            missing.is_empty(),
            "docs/adr/0001-agent-sessions.md is missing:\n{}",
            missing.join("\n")
        );
    }
}
