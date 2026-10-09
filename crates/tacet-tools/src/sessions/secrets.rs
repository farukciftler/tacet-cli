//! The one door every session reader opens files through.
//!
//! WHY A DOOR AND NOT A LIST EACH READER CONSULTS. The directories these
//! readers walk sit next to the user's credentials: `~/.claude/.credentials.json`
//! (an OAuth token), `~/.claude/sessions/<pid>.key` beside the live-session
//! JSON a reader must look at, `~/.codex/auth.json` (an API key), and
//! Antigravity's `state.vscdb` whose `oauthToken` row is in the same SQLite file
//! as the conversation titles. A denylist that each reader is trusted to check
//! is a denylist one reader forgets — and the files a forgetful reader opens
//! here end up summarised into a prompt and, from there, one approval away from
//! a web request. So the list lives in this file only, the only functions in
//! the module allowed to open a file are below, and
//! `sessions::tests::every_reader_goes_through_the_secrets_gate` fails the
//! build if any other file in `sessions/` opens one directly.
//!
//! TWO LAYERS, BECAUSE NAMES LIE. A name check alone is beaten by a symlink: a
//! file called `transcript_full.jsonl` that points at `auth.json` passes any
//! test on the name it was opened by. So the name is checked on the path as
//! given AND on the canonical path, and on Unix the inode actually opened is
//! compared with the one that was checked, so a link swapped in between the
//! check and the open is caught before a byte is read.
//!
//! ON TOP OF THE DENYLIST, AN ALLOWLIST OF EXTENSIONS. Every file a reader
//! needs is `.jsonl`, `.json` or `.md`; nothing else is opened. The denylist
//! then only has to cover the credential files that share those extensions —
//! and it still covers the ones that do not (`*.key`, `state.vscdb`), so the
//! day someone widens the allowlist the credentials stay shut.

use std::fmt;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Exact file names that hold credentials in the three agents' directories
/// (and the obvious neighbours of the same kind). Compared case-insensitively:
/// the default macOS volume is case-insensitive, so `Auth.JSON` opens
/// `auth.json`.
pub const DENIED_NAMES: &[&str] = &[
    // Claude Code: the OAuth credentials file on Linux/Windows (macOS keeps it
    // in the keychain, but a copied-over home directory carries it).
    ".credentials.json",
    "credentials.json",
    // Codex CLI: `$CODEX_HOME/auth.json` holds the API key or ChatGPT tokens.
    "auth.json",
    // Gemini / Antigravity CLI: OAuth refresh token and the signed-in account.
    "oauth_creds.json",
    "google_accounts.json",
    // Antigravity IDE: the VS Code state database. Its `oauthToken` row
    // (MEASURED, 9 Oct 2026) sits in the same file as the conversation titles,
    // and a SQLite file cannot be read "except one row", so the whole file and
    // its WAL/SHM/backup siblings are shut.
    "state.vscdb",
    "state.vscdb-wal",
    "state.vscdb-shm",
    "state.vscdb.backup",
    ".netrc",
];

/// File extensions that are credentials whatever the name.
/// `~/.claude/sessions/<pid>.key` sits beside the `<pid>.json` a live-session
/// check must read; the brief says NEVER read it, and this is that sentence as
/// code.
pub const DENIED_EXTENSIONS: &[&str] = &["key", "pem", "p12", "pfx", "keychain", "keychain-db"];

/// Substrings that mark a name as a credential. `token` catches
/// `tokens.json`, `refresh_token`, `access-token.json`; `secret`,
/// `credential`, `oauth` and `password` their siblings. No real transcript
/// name contains any of them — every transcript is named by a UUID, a
/// timestamp, or one of a handful of fixed names (`task.md`,
/// `transcript_full.jsonl`) — so the cost of a false positive is zero and the
/// cost of a false negative is a token in a prompt.
pub const DENIED_SUBSTRINGS: &[&str] = &[
    "token",
    "secret",
    "credential",
    "oauth",
    "password",
    "passwd",
    "apikey",
    "api_key",
    "api-key",
];

/// Directory names that are never walked into, wherever they appear in the
/// path.
pub const DENIED_DIRS: &[&str] = &[".ssh", ".gnupg", ".aws", "keychains"];

/// The only extensions a reader needs. `.zst` is refused with its own message
/// (see `Refusal::Compressed`), not lumped in here, because the fix for it is
/// different: the user can decompress it.
pub const ALLOWED_EXTENSIONS: &[&str] = &["jsonl", "json", "md"];

/// Why a path was not opened.
#[derive(Debug)]
pub enum Refusal {
    /// The name, a directory on the way, or what a link resolves to is a
    /// credential.
    Secret {
        path: PathBuf,
        rule: String,
    },
    /// A compressed rollout (`.jsonl.zst`). DECIDED in ADR 0001: no zstd
    /// dependency, so the file is refused with a message that says how to get
    /// it read, rather than skipped as if it did not exist.
    Compressed {
        path: PathBuf,
    },
    /// Not one of `ALLOWED_EXTENSIONS`.
    NotAllowed {
        path: PathBuf,
    },
    /// A directory, FIFO, socket or device. A FIFO named `transcript.jsonl`
    /// would block the reader forever on `open`.
    NotAFile {
        path: PathBuf,
    },
    TooLarge {
        path: PathBuf,
        size: u64,
        cap: u64,
    },
    /// The path changed between the check and the open.
    Swapped {
        path: PathBuf,
    },
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Secret { path, rule } => {
                write!(
                    f,
                    "{} was not read: it is a credential ({rule})",
                    path.display()
                )
            }
            Refusal::Compressed { path } => write!(
                f,
                "{} is zstd-compressed and Tacet does not carry a zstd decoder; \
                 decompress it (`zstd -d`) next to the original to have it read",
                path.display()
            ),
            Refusal::NotAllowed { path } => {
                write!(
                    f,
                    "{} was not read: only .jsonl, .json and .md are",
                    path.display()
                )
            }
            Refusal::NotAFile { path } => write!(f, "{} is not a regular file", path.display()),
            Refusal::TooLarge { path, size, cap } => {
                write!(
                    f,
                    "{} is {size} bytes, over the {cap}-byte cap",
                    path.display()
                )
            }
            Refusal::Swapped { path } => {
                write!(
                    f,
                    "{} changed while it was being opened; not read",
                    path.display()
                )
            }
            Refusal::Io { path, error } => write!(f, "{}: {error}", path.display()),
        }
    }
}

impl std::error::Error for Refusal {}

/// The name rules alone, on one path, without touching the disk.
///
/// Public so a DISCOVERY walk can skip a denied entry without opening it (and
/// without logging its name next to a "refused" that draws the eye to it).
pub fn denied_by_name(path: &Path) -> Option<String> {
    for component in path.components() {
        if let Component::Normal(part) = component {
            let lower = part.to_string_lossy().to_lowercase();
            if DENIED_DIRS.contains(&lower.as_str()) {
                return Some(format!("inside {lower}"));
            }
        }
    }
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if DENIED_NAMES.contains(&name.as_str()) {
        return Some(format!("named {name}"));
    }
    if let Some(ext) = Path::new(&name).extension().and_then(|e| e.to_str())
        && DENIED_EXTENSIONS.contains(&ext)
    {
        return Some(format!("a .{ext} file"));
    }
    if let Some(word) = DENIED_SUBSTRINGS.iter().find(|w| name.contains(*w)) {
        return Some(format!("name contains {word:?}"));
    }
    None
}

/// The name rules, the extension rules and the zstd rule on one path, without
/// touching the disk.
fn check_name(path: &Path) -> Result<(), Refusal> {
    if let Some(rule) = denied_by_name(path) {
        return Err(Refusal::Secret {
            path: path.to_path_buf(),
            rule,
        });
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if ext == "zst" {
        return Err(Refusal::Compressed {
            path: path.to_path_buf(),
        });
    }
    if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(Refusal::NotAllowed {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// Every rule, on the path as given and on what it resolves to. Returns the
/// canonical path, which is the one `open` then opens.
pub fn check(path: &Path) -> Result<PathBuf, Refusal> {
    check_name(path)?;
    let io = |error| Refusal::Io {
        path: path.to_path_buf(),
        error,
    };
    let real = std::fs::canonicalize(path).map_err(io)?;
    // The canonical path is checked as a WHOLE, not just its file name: a link
    // to a directory (`brain/x -> ~/.ssh`) changes a middle component.
    check_name(&real).map_err(|refusal| match refusal {
        Refusal::Secret { rule, .. } => Refusal::Secret {
            path: path.to_path_buf(),
            rule: format!("resolves to {}: {rule}", real.display()),
        },
        other => other,
    })?;
    let meta = std::fs::metadata(&real).map_err(io)?;
    if !meta.is_file() {
        return Err(Refusal::NotAFile {
            path: path.to_path_buf(),
        });
    }
    Ok(real)
}

/// Opens a file for reading, after `check`. THE ONLY `File::open` IN THIS
/// MODULE.
pub fn open(path: &Path) -> Result<std::fs::File, Refusal> {
    let real = check(path)?;
    let io = |error| Refusal::Io {
        path: path.to_path_buf(),
        error,
    };
    #[cfg(unix)]
    let checked = std::fs::metadata(&real).map_err(io)?;
    let file = std::fs::File::open(&real).map_err(io)?;
    let opened = file.metadata().map_err(io)?;
    if !opened.is_file() {
        return Err(Refusal::NotAFile {
            path: path.to_path_buf(),
        });
    }
    // The inode opened must be the inode checked. Without this, renaming a link
    // to `auth.json` over the canonical path between `check` and `open` would
    // hand the caller the key — `canonicalize` has already run, so nothing
    // after it looks at names again. Unix only: std exposes no stable file id
    // on Windows, and there the window is left open (NOT MEASURED there).
    #[cfg(unix)]
    if !same_inode(&checked, &opened) {
        return Err(Refusal::Swapped {
            path: path.to_path_buf(),
        });
    }
    Ok(file)
}

#[cfg(unix)]
fn same_inode(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (a.dev(), a.ino()) == (b.dev(), b.ino())
}

/// Reads a whole file as text, refusing one over `cap` bytes.
///
/// Lossy on invalid UTF-8: one bad byte in a 40 MB transcript must not hide
/// the session, and the replacement character is visible to whoever reads the
/// result. The cap is checked on the open handle AND enforced while reading, so
/// a file that grows between the two is still cut at the cap.
pub fn read_text(path: &Path, cap: u64) -> Result<String, Refusal> {
    let file = open(path)?;
    let io = |error| Refusal::Io {
        path: path.to_path_buf(),
        error,
    };
    let size = file.metadata().map_err(io)?.len();
    if size > cap {
        return Err(Refusal::TooLarge {
            path: path.to_path_buf(),
            size,
            cap,
        });
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(cap + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > cap {
        return Err(Refusal::TooLarge {
            path: path.to_path_buf(),
            size: bytes.len() as u64,
            cap,
        });
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Reads at most the last `bytes` bytes, starting at the first full line.
///
/// FOR LISTING. A Claude transcript's title (`ai-title`/`custom-title`) and
/// last activity are near its end, and the files run to tens of megabytes
/// (MEASURED: 340 MB across the 30 newest); reading each in full to draw a list
/// would make `tacet sessions` take seconds per row.
pub fn read_tail(path: &Path, bytes: u64) -> Result<String, Refusal> {
    use std::io::{Seek, SeekFrom};
    let mut file = open(path)?;
    let io = |error| Refusal::Io {
        path: path.to_path_buf(),
        error,
    };
    let size = file.metadata().map_err(io)?.len();
    let start = size.saturating_sub(bytes);
    file.seek(SeekFrom::Start(start)).map_err(io)?;
    let mut buf = Vec::with_capacity(bytes.min(size) as usize);
    file.take(bytes).read_to_end(&mut buf).map_err(io)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if start == 0 {
        return Ok(text);
    }
    // The first line is almost certainly cut in half; a half line of JSON is a
    // parse error the caller would have to special-case.
    Ok(match text.find('\n') {
        Some(i) => text[i + 1..].to_string(),
        None => String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tacet-secrets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Each credential the brief names, at the path the agent really keeps it
    /// (under a temp root), is refused — and its innocent neighbour is not, so
    /// the test cannot pass by refusing everything.
    #[test]
    fn every_known_credential_is_refused_and_its_neighbour_is_read() {
        let root = scratch("named");
        let cases = [
            (".claude/.credentials.json", ".claude/settings.json"),
            (".claude/sessions/4242.key", ".claude/sessions/4242.json"),
            (".codex/auth.json", ".codex/session_index.jsonl"),
            (
                ".gemini/oauth_creds.json",
                ".gemini/antigravity-cli/brain/c1/task.md",
            ),
            (
                ".gemini/antigravity-ide/state.vscdb",
                ".gemini/antigravity-ide/brain/c1/task.md",
            ),
            (
                ".gemini/antigravity-ide/state.vscdb-wal",
                ".gemini/antigravity-ide/brain/c1/walkthrough.md",
            ),
            (".codex/tokens.json", ".codex/history.jsonl"),
            (".claude/AUTH.JSON", ".claude/projects/p/a035ad6d.jsonl"),
            (".ssh/config.json", ".claude/sessions/4243.json"),
        ];
        for (secret, neighbour) in cases {
            for rel in [secret, neighbour] {
                let path = root.join(rel);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, "{\"oauthToken\":\"ya29.secret\"}\n").unwrap();
            }
            match read_text(&root.join(secret), 1 << 20) {
                Err(Refusal::Secret { .. }) => {}
                other => panic!("{secret} was not refused as a secret: {other:?}"),
            }
            assert!(
                read_text(&root.join(neighbour), 1 << 20).is_ok(),
                "{neighbour} was refused"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_credential_is_refused_by_what_it_points_at() {
        let root = scratch("link");
        let codex = root.join(".codex");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("auth.json"), "{\"OPENAI_API_KEY\":\"sk-x\"}").unwrap();
        let brain = root.join("brain/c1/.system_generated/logs");
        std::fs::create_dir_all(&brain).unwrap();
        let link = brain.join("transcript_full.jsonl");
        std::os::unix::fs::symlink(codex.join("auth.json"), &link).unwrap();
        match read_text(&link, 1 << 20) {
            Err(Refusal::Secret { rule, .. }) => assert!(rule.contains("auth.json"), "{rule}"),
            other => panic!("a link named transcript_full.jsonl reached auth.json: {other:?}"),
        }
        // And a link to a DIRECTORY that is denied, entered through an innocent name.
        let ssh = root.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::write(ssh.join("known.json"), "{}").unwrap();
        std::os::unix::fs::symlink(&ssh, root.join("brain/c2")).unwrap();
        assert!(matches!(
            read_text(&root.join("brain/c2/known.json"), 1 << 20),
            Err(Refusal::Secret { .. })
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The race itself cannot be won on demand in a test, so the comparison
    /// that catches it is held to its job directly: two different files are
    /// not the same file, even when their contents are identical.
    #[cfg(unix)]
    #[test]
    fn a_file_swapped_in_after_the_check_is_told_apart() {
        let root = scratch("swap");
        let (a, b) = (root.join("a.jsonl"), root.join("b.jsonl"));
        std::fs::write(&a, "{}").unwrap();
        std::fs::write(&b, "{}").unwrap();
        let meta = |p: &Path| std::fs::metadata(p).unwrap();
        assert!(same_inode(&meta(&a), &meta(&a)));
        assert!(
            !same_inode(&meta(&a), &meta(&b)),
            "two files compared as one"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_compressed_rollout_is_refused_with_a_way_out() {
        let root = scratch("zst");
        let path = root.join("rollout-2026-10-01T10-00-00-abc.jsonl.zst");
        std::fs::write(&path, [0x28, 0xb5, 0x2f, 0xfd]).unwrap();
        let refusal = read_text(&path, 1 << 20).unwrap_err();
        assert!(matches!(refusal, Refusal::Compressed { .. }), "{refusal:?}");
        assert!(refusal.to_string().contains("zstd -d"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_three_text_formats_are_opened() {
        let root = scratch("ext");
        for name in ["c1.db", "c1.pb", "notes.txt", "noext"] {
            std::fs::write(root.join(name), "x").unwrap();
            assert!(
                matches!(
                    read_text(&root.join(name), 64),
                    Err(Refusal::NotAllowed { .. })
                ),
                "{name}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_with_a_transcript_name_is_not_opened() {
        let root = scratch("dir");
        std::fs::create_dir_all(root.join("x.jsonl")).unwrap();
        assert!(matches!(
            read_text(&root.join("x.jsonl"), 64),
            Err(Refusal::NotAFile { .. })
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_cap_is_enforced_and_the_tail_starts_on_a_line() {
        let root = scratch("cap");
        let path = root.join("t.jsonl");
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n").unwrap();
        assert!(matches!(
            read_text(&path, 10),
            Err(Refusal::TooLarge { .. })
        ));
        assert_eq!(read_tail(&path, 12).unwrap(), "{\"c\":3}\n");
        assert_eq!(
            read_tail(&path, 1000).unwrap(),
            "{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
