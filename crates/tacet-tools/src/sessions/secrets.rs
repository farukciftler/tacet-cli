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
//! NAMES LIE, THREE WAYS, AND EACH HAS ITS OWN LAYER.
//!
//! * A symlink: `transcript_full.jsonl -> auth.json` passes any test on the
//!   name it was opened by. So the name is checked on the path as given AND on
//!   the canonical path.
//! * A hard link: `ln auth.json rollout-x.jsonl` has no target to
//!   canonicalize — both names are the same inode, equally real. MEASURED by
//!   the RG-16 review on this branch: before this layer, `read_text` on the
//!   innocent name returned the API key. So on Unix a file with more than one
//!   name is refused outright (`Refusal::HardLinked`). No transcript the three
//!   agents write has a second name, so the cost is a refusal nobody meets.
//! * A swap between the check and the open: `check` canonicalizes, then a
//!   process renames something else over the canonical path. So `check` keeps
//!   the metadata of the entry it vetted (`symlink_metadata`, not followed),
//!   `open` refuses to follow a link at the last component (`O_NOFOLLOW`), and
//!   the inode actually opened must be the one that was checked.
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
    // Claude Code: `~/.claude.json` carries `primaryApiKey` on an API-key
    // login and the signed-in account on every login. No reader needs it —
    // transcripts, live sessions and titles are all under `~/.claude/`.
    ".claude.json",
];

/// The stems of the credential files above, plus SSH private keys (which have
/// no extension to deny). A name that IS a stem, or starts with one followed by
/// anything but a letter or digit, is a credential too: `auth copy.json`
/// (Finder's Duplicate), `auth (1).json` (a browser download), `auth-backup.json`,
/// `auth.json.md`, `state.vscdb.json`, `id_rsa.json`. MEASURED by the RG-16
/// second review: every one of those spellings returned the key through
/// `read_text` while only the exact names were denied. The separator is what
/// keeps `author.md` and `stateful.jsonl` readable.
pub const DENIED_STEMS: &[&str] = &[
    "auth",
    ".credentials",
    "credentials",
    "oauth_creds",
    "google_accounts",
    "state.vscdb",
    ".netrc",
    ".claude",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
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

/// `(directory, stem)`: a stem that is a credential only in that directory,
/// copies included (the `DENIED_STEMS` rule, scoped to one parent).
///
/// Claude Code's `settings.json` / `settings.local.json` — under `~/.claude/`
/// and under a project's `.claude/` — commonly hold `env.ANTHROPIC_API_KEY`,
/// `ANTHROPIC_AUTH_TOKEN` or an `apiKeyHelper` command. No reader needs them
/// (transcripts, live sessions and titles live elsewhere); the RG-21 hook
/// installer writes there and has its own path. Scoped to `.claude` because
/// `settings.json` elsewhere is an ordinary editor file. The RG-16 third review
/// found the test asserting `~/.claude/settings.json` as the READABLE
/// neighbour of a credential.
pub const DENIED_IN_DIR: &[(&str, &str)] = &[(".claude", "settings")];

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
    /// The file has more than one name on disk. One of the others may be a
    /// credential's, and a hard link cannot be told apart from it by name.
    HardLinked {
        path: PathBuf,
        links: u64,
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
            Refusal::HardLinked { path, links } => write!(
                f,
                "{} was not read: it has {links} names on disk, and another of them \
                 may be a credential's",
                path.display()
            ),
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
    // A copy keeps the credential's name and adds to one end of it. A stem
    // followed by a separator covers the suffixes (`auth.json.md`,
    // `auth copy.json`); a denied name preceded by one covers the prefixes
    // (`Copy of auth.json`, `old-auth.json`).
    // A DIGIT IS A COPY TOO: `auth2.json`, `id_rsa2.json` are what a person
    // types for a second copy, and the old rule (anything but a letter OR
    // DIGIT) returned the key for both (RG-16 third review). A letter is
    // still a different word: `author.md`.
    let separated = |rest: &str| {
        rest.chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() || c.is_ascii_digit())
    };
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .map(|p| p.to_string_lossy().to_lowercase());
    if let Some((dir, stem)) = DENIED_IN_DIR.iter().find(|(dir, stem)| {
        parent.as_deref() == Some(*dir) && name.strip_prefix(*stem).is_some_and(separated)
    }) {
        return Some(format!("{stem} inside {dir}"));
    }
    if let Some(stem) = DENIED_STEMS
        .iter()
        .find(|stem| name.strip_prefix(*stem).is_some_and(separated))
    {
        return Some(format!("a copy of {stem}"));
    }
    if let Some(denied) = DENIED_NAMES.iter().find(|denied| {
        name.strip_suffix(*denied).is_some_and(|rest| {
            rest.chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric())
        })
    }) {
        return Some(format!("a copy of {denied}"));
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

/// Every rule that can be answered before a file is opened, on the path as
/// given and on what it resolves to. Returns the canonical path.
///
/// The rules only an open handle can answer — the link count, and that the
/// file opened is the file checked — are `open`'s; read through `open`,
/// `read_text` or `read_tail`, never through `check` and a `File::open` of
/// one's own.
pub fn check(path: &Path) -> Result<PathBuf, Refusal> {
    check_entry(path).map(|(real, _)| real)
}

/// `check`, keeping the metadata of the entry it vetted for `open` to compare
/// against.
fn check_entry(path: &Path) -> Result<(PathBuf, std::fs::Metadata), Refusal> {
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
    // NOT FOLLOWED. `canonicalize` resolved every link, so a link at `real`
    // now is one put there since; following it here would vet the target and
    // then compare `open`'s result against the target — a check of nothing.
    let meta = std::fs::symlink_metadata(&real).map_err(io)?;
    if meta.file_type().is_symlink() {
        return Err(Refusal::Swapped {
            path: path.to_path_buf(),
        });
    }
    if !meta.is_file() {
        return Err(Refusal::NotAFile {
            path: path.to_path_buf(),
        });
    }
    Ok((real, meta))
}

/// Open flags, spelled per platform because there is no libc crate here
/// (zero-dependency identity, as in `run_code.rs`). The Linux values are NOT
/// the same on every architecture: x86 has `O_NOFOLLOW` at `0o400000`, arm and
/// aarch64 at `0o100000` (their `0o400000` is `O_LARGEFILE`). An
/// architecture not listed gets `0` — no extra flag — and the inode
/// comparison in `open` is then the only belt.
///
/// `O_NONBLOCK` is here for a FIFO swapped in after the check: a blocking
/// read-only open of a FIFO waits for a writer that never comes, and the
/// reader with it. Non-blocking, the open returns at once and the handle's
/// metadata says it is not a regular file. On a regular file the flag changes
/// nothing.
#[cfg(any(target_os = "macos", target_os = "ios"))]
const OPEN_FLAGS: i32 = 0x0100 /* O_NOFOLLOW */ | 0x0004 /* O_NONBLOCK */;
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(target_arch = "x86", target_arch = "x86_64")
))]
const OPEN_FLAGS: i32 = 0o400000 /* O_NOFOLLOW */ | 0o4000 /* O_NONBLOCK */;
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(target_arch = "arm", target_arch = "aarch64")
))]
const OPEN_FLAGS: i32 = 0o100000 /* O_NOFOLLOW */ | 0o4000 /* O_NONBLOCK */;
#[cfg(all(
    unix,
    not(any(
        target_os = "macos",
        target_os = "ios",
        all(
            any(target_os = "linux", target_os = "android"),
            any(
                target_arch = "x86",
                target_arch = "x86_64",
                target_arch = "arm",
                target_arch = "aarch64"
            )
        )
    ))
))]
const OPEN_FLAGS: i32 = 0;

/// Opens without following a link at the last component: for reading, or —
/// `append` — for appending to a file that already exists (never created
/// here: a write-back that finds no transcript has nothing to add to).
fn open_no_follow(real: &Path, append: bool) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    if append {
        options.append(true);
    } else {
        options.read(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(OPEN_FLAGS);
    }
    options.open(real)
}

/// Opens a file for reading, after `check`. THE ONLY OPEN IN THIS MODULE.
pub fn open(path: &Path) -> Result<std::fs::File, Refusal> {
    open_checked(path, false)
}

/// Opens an EXISTING file for appending, after every check `open` makes.
///
/// WHY THE WRITE-BACK GOES THROUGH THE SAME DOOR (RG-22, RG-24). An append is
/// aimed at a path another program chose — a transcript name read out of a
/// directory listing — and the three ways names lie hold for a write as they
/// do for a read: a planted `rollout-x.jsonl -> ~/.codex/auth.json` link, a
/// hard link, a swap between the check and the open. A note appended to
/// `auth.json` breaks the user's login; so the same denylist, the same
/// extension allowlist, `O_NOFOLLOW`, the inode comparison and the link count
/// all apply. The handle is also what a rollback truncates through
/// (`set_len`), so the file cut back is the file that was checked.
pub fn open_append(path: &Path) -> Result<std::fs::File, Refusal> {
    open_checked(path, true)
}

fn open_checked(path: &Path, append: bool) -> Result<std::fs::File, Refusal> {
    let (real, checked) = check_entry(path)?;
    let io = |error| Refusal::Io {
        path: path.to_path_buf(),
        error,
    };
    #[cfg(test)]
    swap_seam::between_check_and_open(&real);
    let file = match open_no_follow(&real, append) {
        Ok(file) => file,
        // `O_NOFOLLOW` on a link fails the open (ELOOP); name it for what it
        // is rather than as an I/O error the user would go looking for.
        Err(_)
            if std::fs::symlink_metadata(&real)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false) =>
        {
            return Err(Refusal::Swapped {
                path: path.to_path_buf(),
            });
        }
        Err(error) => return Err(io(error)),
    };
    let opened = file.metadata().map_err(io)?;
    if !opened.is_file() {
        return Err(Refusal::NotAFile {
            path: path.to_path_buf(),
        });
    }
    // The inode opened must be the inode CHECKED — the metadata `check_entry`
    // took, not a fresh lookup now: a fresh lookup follows whatever is at the
    // path by now, and so matches the swapped-in file every time (the RG-16
    // review disabled the old comparison and every test stayed green). Unix
    // only: std exposes no stable file id on Windows, and there the window is
    // left open (NOT MEASURED there).
    #[cfg(unix)]
    if !same_inode(&checked, &opened) {
        return Err(Refusal::Swapped {
            path: path.to_path_buf(),
        });
    }
    // On the HANDLE, after the open: a link added after `check` is seen, and
    // the count is the inode's, whichever name it was opened by. Unix only —
    // the Windows link count is not on stable std.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.nlink() > 1 {
            return Err(Refusal::HardLinked {
                path: path.to_path_buf(),
                links: opened.nlink(),
            });
        }
    }
    #[cfg(not(unix))]
    let _ = checked;
    Ok(file)
}

#[cfg(unix)]
fn same_inode(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (a.dev(), a.ino()) == (b.dev(), b.ino())
}

/// The window between `check` and the open, held open on demand. A race
/// cannot be won reliably from outside, so the tests below run their swap
/// HERE, inside `open` itself — the comparison is then tested as `open`
/// uses it, not as a helper someone could stop calling.
#[cfg(test)]
pub(crate) mod swap_seam {
    use std::cell::RefCell;
    use std::path::Path;

    type Hook = Box<dyn Fn(&Path)>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(crate) fn between_check_and_open(real: &Path) {
        HOOK.with(|h| {
            if let Some(hook) = h.borrow().as_ref() {
                hook(real);
            }
        });
    }

    /// Runs `body` with `hook` in the window, on this thread only (tests run
    /// in parallel threads).
    // Every swap test is Unix-only (links, FIFOs, inodes); on Windows nothing
    // calls this, and `clippy -D warnings` would not be the one to say so.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn with<T>(hook: impl Fn(&Path) + 'static, body: impl FnOnce() -> T) -> T {
        HOOK.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
        let out = body();
        HOOK.with(|h| *h.borrow_mut() = None);
        out
    }
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
            (".claude/.credentials.json", ".claude/history.jsonl"),
            // Claude Code's settings may carry `env.ANTHROPIC_API_KEY`; only
            // under `.claude` (RG-16 third review).
            (".claude/settings.json", ".claude/projects/p/a035ad6e.jsonl"),
            (".claude/settings.local.json", "proj/settings.json"),
            ("proj/.claude/settings.local.json", "proj/.claude/agents.md"),
            // A digit after a stem is a copy (RG-16 third review).
            (".codex/auth2.json", ".codex/author2.md"),
            ("home/id_rsa2.json", "home/idea2.md"),
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
            // Copies of a credential under a name a person or a tool gives a
            // copy (RG-16 second review: each of these returned the key).
            (".codex/auth copy.json", ".codex/author.md"),
            (".codex/auth (1).json", ".codex/sessions/rollout-1.jsonl"),
            (".codex/auth-backup.json", ".codex/hooks.json"),
            (".codex/auth.json.json", ".codex/session_index.jsonl"),
            (".codex/auth.json.md", ".codex/AGENTS.md"),
            (".codex/Copy of auth.json", ".codex/copy of notes.md"),
            (
                ".gemini/antigravity-ide/state.vscdb.json",
                ".gemini/antigravity-ide/brain/c1/task.md.metadata.json",
            ),
            (
                ".gemini/google_accounts (1).json",
                ".gemini/antigravity-cli/brain/c1/implementation_plan.md",
            ),
            ("home/id_rsa.json", "home/idea.md"),
            (
                "home/.claude.json",
                ".claude/projects/p/s1/custom-title.json",
            ),
            (
                "home/.claude.json.backup.json",
                ".claude/projects/p/s1/subagents/agent-a1.meta.json",
            ),
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

    /// The comparison `open` relies on, on its own: two different files are
    /// not the same file, even when their contents are identical. The swaps
    /// themselves are driven through `open` by the tests that follow.
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

    /// RG-16 review, hostile probe: `ln auth.json sessions/rollout-x.jsonl`
    /// returned the API key, because a hard link has no target to resolve and
    /// every name rule ran on the innocent name.
    #[cfg(unix)]
    #[test]
    fn a_hard_link_to_a_credential_is_refused() {
        let root = scratch("hardlink");
        let codex = root.join(".codex");
        std::fs::create_dir_all(codex.join("sessions")).unwrap();
        std::fs::write(
            codex.join("auth.json"),
            "{\"OPENAI_API_KEY\":\"sk-hardlinked\"}",
        )
        .unwrap();
        let link = codex.join("sessions/rollout-x.jsonl");
        std::fs::hard_link(codex.join("auth.json"), &link).unwrap();
        for result in [read_text(&link, 1 << 20), read_tail(&link, 1 << 20)] {
            match result {
                Err(Refusal::HardLinked { links: 2, .. }) => {}
                other => panic!("a hard link to auth.json was not refused: {other:?}"),
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RG-22/RG-24: the write-back opens through the same rules. A note
    /// appended to `auth.json` — reached by name, by a planted link or by a
    /// hard link under a rollout's name — would break the user's login; an
    /// append to a transcript that is not there creates nothing.
    #[cfg(unix)]
    #[test]
    fn the_append_door_refuses_what_the_read_door_refuses() {
        use std::io::Write;
        let root = scratch("append");
        let codex = root.join(".codex");
        std::fs::create_dir_all(codex.join("sessions")).unwrap();
        let auth = codex.join("auth.json");
        std::fs::write(&auth, "{\"OPENAI_API_KEY\":\"sk-append\"}").unwrap();
        let hard = codex.join("sessions/rollout-h.jsonl");
        std::fs::hard_link(&auth, &hard).unwrap();
        let soft = codex.join("sessions/rollout-s.jsonl");
        std::os::unix::fs::symlink(&auth, &soft).unwrap();
        assert!(matches!(open_append(&auth), Err(Refusal::Secret { .. })));
        assert!(matches!(open_append(&soft), Err(Refusal::Secret { .. })));
        assert!(matches!(
            open_append(&hard),
            Err(Refusal::HardLinked { links: 2, .. })
        ));
        let missing = codex.join("sessions/rollout-m.jsonl");
        assert!(matches!(open_append(&missing), Err(Refusal::Io { .. })));
        assert!(std::fs::symlink_metadata(&missing).is_err());
        assert_eq!(
            std::fs::read_to_string(&auth).unwrap(),
            "{\"OPENAI_API_KEY\":\"sk-append\"}"
        );
        // And a real transcript is appended to, at its end.
        let ok = codex.join("sessions/rollout-ok.jsonl");
        std::fs::write(&ok, "{\"a\":1}\n").unwrap();
        open_append(&ok).unwrap().write_all(b"{\"b\":2}\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&ok).unwrap(),
            "{\"a\":1}\n{\"b\":2}\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Something else renamed over the checked path between `check` and the
    /// open: a regular file, so `O_NOFOLLOW` has nothing to refuse and only
    /// the inode comparison stands between the reader and the key.
    #[cfg(unix)]
    #[test]
    fn a_file_renamed_over_the_checked_path_is_refused() {
        let root = scratch("rename-swap");
        let path = root.join("t.jsonl");
        std::fs::write(&path, "{\"type\":\"user\"}\n").unwrap();
        let planted = root.join("planted");
        std::fs::write(&planted, "{\"OPENAI_API_KEY\":\"sk-swapped\"}").unwrap();
        let result = swap_seam::with(
            move |real| std::fs::rename(&planted, real).unwrap(),
            || read_text(&path, 1 << 20),
        );
        match result {
            Err(Refusal::Swapped { .. }) => {}
            other => panic!("the file renamed in after the check was read: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The reviewer's exact race: the checked path becomes a link to
    /// `auth.json` after `check`. Refused by `O_NOFOLLOW` first and by the
    /// inode comparison behind it.
    #[cfg(unix)]
    #[test]
    fn a_link_swapped_in_after_the_check_is_not_followed() {
        let root = scratch("link-swap");
        std::fs::create_dir_all(root.join(".codex")).unwrap();
        let auth = root.join(".codex/auth.json");
        std::fs::write(&auth, "{\"OPENAI_API_KEY\":\"sk-linked\"}").unwrap();
        let path = root.join("t.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        let result = swap_seam::with(
            move |real| {
                std::fs::remove_file(real).unwrap();
                std::os::unix::fs::symlink(&auth, real).unwrap();
            },
            || read_text(&path, 1 << 20),
        );
        match result {
            Err(Refusal::Swapped { .. }) => {}
            other => panic!("a link swapped in after the check was followed: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `O_NOFOLLOW` itself, so a wrong per-platform constant (the arm64 Linux
    /// value differs from x86's) fails here rather than silently becoming
    /// "no flag".
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn the_open_does_not_follow_a_link() {
        let root = scratch("nofollow");
        std::fs::write(root.join("target.json"), "{}").unwrap();
        std::os::unix::fs::symlink(root.join("target.json"), root.join("link.json")).unwrap();
        assert!(open_no_follow(&root.join("target.json"), false).is_ok());
        assert!(
            open_no_follow(&root.join("link.json"), false).is_err(),
            "the open followed a link"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A FIFO swapped in after the check would hang a blocking open forever.
    /// The open runs on its own thread so a regression fails the test on the
    /// timeout instead of hanging the suite.
    #[cfg(unix)]
    #[test]
    fn a_fifo_swapped_in_after_the_check_does_not_hang_the_reader() {
        let root = scratch("fifo-swap");
        let path = root.join("t.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        let fifo = root.join("fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo");
        assert!(made.success());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = swap_seam::with(
                move |real| std::fs::rename(&fifo, real).unwrap(),
                || read_text(&path, 1 << 20),
            );
            let _ = tx.send(result);
        });
        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Err(Refusal::NotAFile { .. })) => {}
            Ok(other) => panic!("a FIFO swapped in was not refused: {other:?}"),
            Err(_) => panic!("the open blocked on a FIFO swapped in after the check"),
        }
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
