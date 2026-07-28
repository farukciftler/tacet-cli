//! WORKSPACE ROOTS: the file scope is a LIST of directories, not a single one.
//!
//! Until now every file tool was locked to `ToolContext::working_dir`. That is
//! the right default — a fresh session can only see where it was started — but
//! it makes the honest case impossible: the user says "also look at
//! ~/Projects/acme" and there is nowhere to put that answer.
//!
//! So the scope becomes a list: the working directory PLUS the directories the
//! user NAMED at install time. Containment is then "does the real path enter ANY
//! root", and every other guarantee of `sandbox_path` stays exactly as it was.
//!
//! WHY A PROCESS-WIDE LIST AND NOT A FIELD ON `ToolContext`: the context type
//! belongs to tacet-kernel and every file tool constructs its calls through
//! `sandbox_path`. Widening the scope must not force a signature change on
//! call sites that have nothing to do with it — a change like that is how one
//! of them quietly keeps the old, narrower check and the two disagree. The list
//! lives here, the gate reads it here, and there is exactly one place to audit.
//! Moving it into `ToolContext` later changes no semantics.
//!
//! THE LIST IS NOT A FREE-FOR-ALL. `install_roots` is the ONLY way in and it
//! refuses a root that would make the scope meaningless (see `RootRejected`).
//! An empty list means the behaviour is bit-for-bit what it was before this
//! module existed.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Why a directory the user named cannot become a root.
///
/// Each variant carries the path AS THE USER GAVE IT, not the resolved form:
/// the message has to be recognisable to the person who typed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootRejected {
    /// The directory does not exist. Refused AT INSTALL TIME on purpose: a root
    /// that is created later (by whom?) is a root nobody reviewed.
    Missing(PathBuf),
    /// The path exists but is a file, a device, a link to one of those...
    NotADirectory(PathBuf),
    /// The home directory itself, the filesystem root, or anything containing
    /// the home directory. Granting one of these is "see everything", which is
    /// not what an extra workspace root is for — the user who wants a subtree
    /// can name the subtree.
    TooBroad(PathBuf),
}

impl std::fmt::Display for RootRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(p) => write!(f, "workspace root does not exist: {}", p.display()),
            Self::NotADirectory(p) => {
                write!(f, "workspace root is not a directory: {}", p.display())
            }
            Self::TooBroad(p) => write!(
                f,
                "workspace root is too broad (home directory or above): {}",
                p.display()
            ),
        }
    }
}

impl std::error::Error for RootRejected {}

/// The registered extra roots, always CANONICAL (link-free, absolute).
///
/// `const`-constructed so there is no initialisation order to get wrong: before
/// anyone installs anything the list is empty, and an empty list is the old
/// single-root behaviour.
static ROOTS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Poisoned-lock policy: recover the guard instead of panicking.
///
/// A panic while holding this lock cannot leave a half-written list — every
/// write below is a single assignment of an already-built vector — so the data
/// is sound and refusing to read it would only turn an unrelated panic into a
/// dead file layer.
fn read_roots() -> Vec<PathBuf> {
    ROOTS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Checks one directory the user named and returns it in canonical form.
///
/// `canonicalize` here is not a convenience: it is what makes the later
/// containment test meaningful. If the root were stored as typed, a root
/// `~/work` that is itself a link to `/` would look narrow and behave wide.
pub fn validate_root(path: impl AsRef<Path>) -> Result<PathBuf, RootRejected> {
    let given = path.as_ref();
    let real = given
        .canonicalize()
        .map_err(|_| RootRejected::Missing(given.to_path_buf()))?;
    if !real.is_dir() {
        return Err(RootRejected::NotADirectory(given.to_path_buf()));
    }
    if is_too_broad(&real) {
        return Err(RootRejected::TooBroad(given.to_path_buf()));
    }
    Ok(real)
}

/// The filesystem root, the home directory, or any ancestor of the home
/// directory (`/Users`, `/home`, ...).
///
/// The ancestor case is not paranoia: `/Users` contains every account on the
/// machine, so it is strictly WIDER than the home directory the rule already
/// names. One test — "is home inside the candidate" — covers all three, and
/// `Path::starts_with` compares whole components, so `/Users/ada2` is not
/// treated as living under `/Users/ada`.
fn is_too_broad(real: &Path) -> bool {
    // Always refused, and the fallback when the home directory is unknown.
    if real.parent().is_none() {
        return true;
    }
    match home_dir() {
        Some(home) => home.starts_with(real),
        None => false,
    }
}

/// The home directory, canonical. No new dependency for this: the two variables
/// below are what a helper crate would read anyway.
fn home_dir() -> Option<PathBuf> {
    let raw = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()))?;
    PathBuf::from(raw).canonicalize().ok()
}

/// Replaces the root list with the directories the user named at install time.
///
/// ALL-OR-NOTHING: one bad directory refuses the whole call and the previous
/// list stays untouched. A partial install is the worst outcome — the user
/// believes a root is in scope, the assistant says "no such file", and the
/// obvious next move is to widen something else.
///
/// Returns the canonical roots that were installed.
pub fn install_roots<I, P>(paths: I) -> Result<Vec<PathBuf>, RootRejected>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut checked: Vec<PathBuf> = Vec::new();
    for path in paths {
        let real = validate_root(path)?;
        // Exact duplicates only. Collapsing nested roots would be wrong: they
        // are not equivalent to the outer one for anything but containment,
        // and containment already accepts either.
        if !checked.contains(&real) {
            checked.push(real);
        }
    }
    let installed = checked.clone();
    *ROOTS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = checked;
    Ok(installed)
}

/// Drops every extra root — the scope goes back to the working directory alone.
///
/// This is what "the plugin was switched off" must call. A disabled plugin whose
/// roots are still live would be a scope the user cannot see in the catalogue.
pub fn clear_roots() {
    ROOTS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

/// The registered roots, canonical. EXCLUDES the working directory: that one
/// comes from `ToolContext` and is always in scope whether or not this list has
/// anything in it.
pub fn extra_roots() -> Vec<PathBuf> {
    read_roots()
}

/// True when no extra root is registered, i.e. the file layer behaves exactly
/// as it did before this module existed. The gate branches on this.
pub fn has_no_extra_roots() -> bool {
    ROOTS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty()
}

/// TEST SUPPORT: the root list is process-wide, so two tests that install or
/// clear it cannot run at the same time. Every test that touches the list — in
/// this module and in `sandbox_path` — takes this first.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-roots-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path.canonicalize().expect("resolved")
    }

    #[test]
    fn a_named_directory_is_stored_canonical() {
        let _guard = test_lock();
        clear_roots();
        let dir = temp_dir("ok");
        let installed = install_roots([&dir]).expect("must install");
        assert_eq!(installed, vec![dir.clone()]);
        assert_eq!(extra_roots(), vec![dir]);
        assert!(!has_no_extra_roots());
        clear_roots();
        assert!(has_no_extra_roots());
    }

    /// A root that does not exist is refused AT INSTALL TIME, not on first use.
    #[test]
    fn a_missing_directory_is_refused() {
        let _guard = test_lock();
        clear_roots();
        let missing = temp_dir("missing").join("nope");
        assert!(matches!(
            validate_root(&missing),
            Err(RootRejected::Missing(_))
        ));
    }

    #[test]
    fn a_file_is_not_a_root() {
        let _guard = test_lock();
        clear_roots();
        let dir = temp_dir("file");
        let file = dir.join("notes.txt");
        std::fs::write(&file, "hello").expect("write");
        assert!(matches!(
            validate_root(&file),
            Err(RootRejected::NotADirectory(_))
        ));
    }

    /// THE RULE THAT KEEPS THIS FEATURE FROM BECOMING "SEE EVERYTHING": the home
    /// directory, its ancestors and the filesystem root are not roots.
    #[test]
    fn home_and_the_filesystem_root_are_refused() {
        let _guard = test_lock();
        clear_roots();
        assert!(matches!(
            validate_root(std::path::Path::new("/")),
            Err(RootRejected::TooBroad(_))
        ));
        if let Some(home) = home_dir() {
            assert!(matches!(
                validate_root(&home),
                Err(RootRejected::TooBroad(_))
            ));
            if let Some(parent) = home.parent() {
                assert!(matches!(
                    validate_root(parent),
                    Err(RootRejected::TooBroad(_))
                ));
            }
        }
    }

    /// One bad entry refuses the WHOLE install: no half-configured scope.
    #[test]
    fn a_bad_entry_leaves_the_previous_list_untouched() {
        let _guard = test_lock();
        clear_roots();
        let good = temp_dir("keep");
        install_roots([&good]).expect("first install");
        let missing = good.join("gone");
        assert!(install_roots([good.clone(), missing]).is_err());
        assert_eq!(extra_roots(), vec![good]);
        clear_roots();
    }

    /// A root named twice (or via a link) is stored once.
    #[test]
    fn duplicates_collapse() {
        let _guard = test_lock();
        clear_roots();
        let dir = temp_dir("dup");
        install_roots([&dir, &dir]).expect("install");
        assert_eq!(extra_roots().len(), 1);
        clear_roots();
    }
}
