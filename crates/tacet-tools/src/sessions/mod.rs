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

pub mod handoff;
pub mod model;
pub mod secrets;
pub mod version;

pub use handoff::{HandoffRecord, HandoffTask, Status};
pub use model::{
    AgentKind, AgentSession, Entry, EntryKind, PlanItem, Role, SessionError, SessionSource,
    SessionSummary,
};

/// A tool that returns another agent's transcript taints the session (audit
/// H2, RG-01): the text is the user's code, commands and prompts. Stated once
/// so the tool that wraps a reader returns this rather than its own opinion.
pub const TAINTS_SESSION: bool = true;

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn sessions_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sessions")
    }

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The ways Rust code opens a file for reading. Spelled in pieces so this
    /// test does not find itself.
    const OPENERS: &[&str] = &[
        concat!("File", "::open("),
        concat!("fs::", "read("),
        concat!("fs::", "read_to_string("),
        concat!("OpenOptions", "::new("),
        concat!("File", "::options("),
    ];

    /// Every file in `sessions/` other than `secrets.rs` that opens a file for
    /// reading, with the line. Walks the directory rather than listing files,
    /// so the next reader added here (claude.rs, codex.rs…) is covered without
    /// anyone remembering to add it.
    fn bypasses(dir: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs")
                    || path.file_name().unwrap() == "secrets.rs"
                {
                    continue;
                }
                // Source files of this crate, read by a test: not a session file.
                let text = std::fs::read_to_string(&path).unwrap();
                // Production code only. A test that reads back the source, the
                // lockfile or the ADR is not a session reader, and the module
                // convention puts `#[cfg(test)]` last.
                let production = text
                    .lines()
                    .take_while(|l| !l.trim_start().starts_with("#[cfg(test)]"));
                for (n, line) in production.enumerate() {
                    let code = line.split("//").next().unwrap_or_default();
                    if OPENERS.iter().any(|o| code.contains(o)) {
                        found.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                }
            }
        }
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

    /// The guard above is only worth having if it can fail: a file that opens
    /// directly IS reported.
    #[test]
    fn the_gate_guard_sees_a_direct_open() {
        let dir = std::env::temp_dir().join(format!("tacet-sessions-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(
            dir.join("secrets.rs"),
            format!("let f = std::fs::{}\"x\");", "File::open("),
        )
        .unwrap();
        std::fs::write(
            dir.join("nested/codex.rs"),
            format!("let t = std::{}p)?;", "fs::read_to_string("),
        )
        .unwrap();
        std::fs::write(
            dir.join("ok.rs"),
            format!("// std::{}p) in a comment", "fs::read("),
        )
        .unwrap();
        std::fs::write(
            dir.join("tested.rs"),
            format!(
                "fn f() {{}}\n#[cfg(test)]\nmod tests {{ fn t() {{ std::{}\"Cargo.lock\"); }} }}",
                "fs::read("
            ),
        )
        .unwrap();
        let found = bypasses(&dir);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("codex.rs:1"), "{found:?}");
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
        for ext in super::secrets::DENIED_EXTENSIONS {
            if !adr.contains(&format!("`*.{ext}`")) {
                missing.push(format!("denied extension `*.{ext}`"));
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
