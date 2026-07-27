//! The environment contract: WHERE the configuration lives and under WHAT NAME
//! environment variables are read.
//!
//! WHY IN THE CORE: the "no work is done in this crate" rule still holds, and
//! this file does no work either — it reads not a single line from a file and
//! creates not a single directory. What lives here is an AGREEMENT: memory,
//! skills, MCP and the shell MUST point at the same directory. That agreement
//! used to be written as three separate hidden-folder expressions in three
//! separate crates, and nothing guaranteed the three stayed equal; changing one
//! would silently separate it from the others. Just as types are defined in one
//! place, so is the path.
//!
//! PLATFORM. The path is no longer "home directory + hidden folder" but the
//! operating system's own configuration location:
//!
//! | platform | directory |
//! | --- | --- |
//! | Unix (XDG set) | `$XDG_CONFIG_HOME/tacet` |
//! | Unix (no XDG) | `~/.tacet` |
//! | Windows | `%APPDATA%\Tacet` |
//!
//! On Windows, writing `~/.tacet` compiles and runs, but it is the wrong place:
//! it sits at the profile root as a folder that is not hidden in the user's
//! explorer, and it does not travel with a roaming profile.
//!
//! NO MIGRATION CODE, NO BACKWARD COMPATIBILITY. The app was never released: a
//! configuration directory under the old brand name was never created on any
//! machine, and an environment variable under the old name was never written
//! into any shell profile. For a while there was a bridge that read both names
//! (and printed a warning for the old one); it was removed because what it read
//! never existed. ONE NAME IS READ: `TACET_*`.

use std::ffi::OsString;
use std::path::PathBuf;

/// The variable that overrides the configuration directory (also used by tests
/// and the developer shell: run without polluting the real settings directory).
pub const HOME_VAR: &str = "TACET_HOME";

/// Reads an environment variable.
///
/// WHY A SEPARATE FUNCTION: the `filter` in its one-line body is a CONTRACT. An
/// empty value is treated as ABSENT — a user who writes `export TACET_MODEL=`
/// has not defined the variable, they have CLEARED it, and must fall back to
/// the default. Had this rule been written out at every call site, the day
/// someone forgot it an empty string would travel on as a valid path and the
/// error would blow up much further downstream.
pub fn env_var(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

/// The configuration directory. The directory is NOT CREATED; creating it is
/// the job of the layer that writes into it.
///
/// `None` is returned only when neither `TACET_HOME`, nor `APPDATA` (Windows),
/// nor `HOME` (Unix) can be resolved — in that case the caller never touches
/// the disk and stays in memory (see `MemoryStore::in_memory`).
pub fn config_dir() -> Option<PathBuf> {
    if let Some(home) = env_var(HOME_VAR) {
        return Some(PathBuf::from(home));
    }
    platform_dir()
}

#[cfg(windows)]
fn platform_dir() -> Option<PathBuf> {
    // `%APPDATA%` = ...\AppData\Roaming. Configuration must travel with the
    // roaming profile; `LOCALAPPDATA` is for caches, not for settings.
    std::env::var_os("APPDATA")
        .filter(|a| !a.is_empty())
        .map(|a| PathBuf::from(a).join("Tacet"))
}

#[cfg(not(windows))]
fn platform_dir() -> Option<PathBuf> {
    // The XDG rule: if the variable is not ABSOLUTE it is ignored. A relative
    // value would tie the configuration to the user's current working directory.
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        let x = PathBuf::from(x);
        if x.is_absolute() {
            return Some(x.join("tacet"));
        }
    }
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".tacet"))
}

/// The full path of a file in the configuration directory (`mcp.json`,
/// `memory.json`).
pub fn config_path(file: &str) -> Option<PathBuf> {
    config_dir().map(|d| d.join(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Environment variables are PROCESS-WIDE: the tests of this module run in
    /// ONE test, in sequence, so they do not step on each other.
    #[test]
    fn home_variable_overrides_the_path() {
        // SAFETY: single-threaded test; no other test reads this variable.
        unsafe { std::env::remove_var(HOME_VAR) };

        // 1) The variable overrides the path.
        let target = std::env::temp_dir().join("tacet-env-test");
        unsafe { std::env::set_var(HOME_VAR, &target) };
        assert_eq!(config_dir().unwrap(), target);
        assert_eq!(
            config_path("memory.json").unwrap(),
            target.join("memory.json")
        );

        // 2) An empty value counts as "undefined" — the user who cleared it falls back.
        unsafe { std::env::set_var(HOME_VAR, "") };
        assert_ne!(config_dir(), Some(target.clone()));

        unsafe { std::env::remove_var(HOME_VAR) };
    }

    /// Without an override the path is the PLATFORM's configuration location
    /// and its LEAF is the brand. On Windows that guarantee goes through
    /// `%APPDATA%\Tacet`; it was NOT RUN here (this machine is Unix).
    #[test]
    fn the_leaf_of_the_platform_path_carries_the_brand_name() {
        let Some(d) = platform_dir() else {
            return; // run without an environment (container): nothing to prove
        };
        let leaf = d
            .file_name()
            .expect("the path cannot be empty")
            .to_string_lossy()
            .to_lowercase();
        assert!(
            leaf == "tacet" || leaf == ".tacet",
            "unexpected leaf for the configuration directory: {leaf}"
        );
    }
}
