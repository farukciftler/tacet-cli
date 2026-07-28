//! The connection list: `mcp.json` in the config directory.
//!
//! THE DIRECTORY IS NOT THIS CRATE'S KNOWLEDGE. The path comes from
//! `tacet_kernel::env` — on Unix `$XDG_CONFIG_HOME/tacet` or `~/.tacet`, on
//! Windows `%APPDATA%\Tacet`; the memory and skill layers point at the SAME
//! directory.
//!
//! EMPTY BY DEFAULT. If the file does not exist it means there are no
//! connections, and that IS NOT AN ERROR — spec §2.1's "closed by default"
//! principle: with no connection added, network traffic is zero. The user may
//! have MCP servers running on this machine; Tacet does NOT try to discover
//! them BY ITSELF, the user writes them down by hand.
//!
//! Format:
//!
//! ```json
//! {
//!   "connections": [
//!     {
//!       "name": "home server",
//!       "url": "https://example.com/mcp",
//!       "key": "your token here",
//!       "enabled": true
//!     }
//!   ]
//! }
//! ```
//!
//! `key` and `enabled` are optional (`enabled` defaults to `true`).
//!
//! DISK FORMAT: files written before the English rename use
//! `baglantilar`/`ad`/`anahtar`/`anahtar_ortam`/`etkin`, so every field carries
//! a `serde(alias)` for its old name. Dropping an alias makes an existing
//! `mcp.json` fail to parse, which surfaces as `Malformed` — every connection
//! the user configured disappears at once.
//!
//! KEY STORAGE WARNING: on iOS the token lives in the Keychain (§5.8). Here it
//! is in a plain file; there is no equivalent vault on the desktop. That is why
//! the file must be created with 0600 permissions and why `key_env` can
//! redirect it to an environment variable — so you do not have to keep the
//! token in a file that ends up in a git repository.
//!
//! THE 0600 RULE IS NOW MEASURED, NOT MERELY WRITTEN DOWN. It used to be a
//! sentence in this header and nothing else: nothing stamped the mode when the
//! file was created and nothing looked at it when the file was read. A user who
//! created `mcp.json` in an editor got 0644 from the default umask, and on
//! macOS every local account is in the `staff` group, so a second account on
//! the machine could simply `cat` the token and then connect to the user's
//! server as the user — with the four gates of the tool layer bypassed
//! entirely, because the far side never sees them. `key_file_is_exposed` is the
//! measurement; it is deliberately quiet when the file holds NO plain key
//! (`key_env` users must not be nagged).
//!
//! WHAT IT DOES NOT DO: it does not change the mode and it does not drop the
//! key. Both are behaviour changes that would break a working setup silently,
//! which is worse than the exposure. It reports; the shell shows the warning.

use crate::error::{MCPError, MCPResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionSetting {
    /// The name visible to the user; it opens the chip text ("home server · ...").
    #[serde(alias = "ad")]
    pub name: String,
    pub url: String,
    /// Bearer token — plain text in the file.
    #[serde(default, alias = "anahtar", skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// For those who do not want the token in the file: read the key from this
    /// ENVIRONMENT VARIABLE. If both this and `key` are present, this wins.
    #[serde(
        default,
        alias = "anahtar_ortam",
        skip_serializing_if = "Option::is_none"
    )]
    pub key_env: Option<String>,
    #[serde(default = "yes", alias = "etkin")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl ConnectionSetting {
    /// The key to use — the environment variable first, then the value in the
    /// file.
    pub fn resolved_key(&self) -> Option<String> {
        let from_env = self
            .key_env
            .as_ref()
            .and_then(|name| std::env::var(name).ok())
            .filter(|v| !v.is_empty());
        from_env.or_else(|| self.key.clone().filter(|k| !k.is_empty()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Config {
    #[serde(default, alias = "baglantilar")]
    pub connections: Vec<ConnectionSetting>,
}

impl Config {
    /// Only the enabled connections whose address is accepted.
    ///
    /// The address is filtered here too, so that one bad line does not throw
    /// away the WHOLE file: if one of five connections is broken the others
    /// must keep working.
    pub fn valid(&self) -> Vec<&ConnectionSetting> {
        self.connections
            .iter()
            .filter(|c| c.enabled && !c.name.trim().is_empty())
            .filter(|c| crate::client::validate_url(&c.url).is_ok())
            .collect()
    }
}

/// The variable that points DIRECTLY at the file (so the token does not have to
/// live in a file that falls into the repository).
///
/// RENAMED: this used to be `TACET_MCP_YAPILANDIRMA`. Anyone who exported the
/// old name has to export the new one; nothing falls back to it, because an
/// environment variable is set per run and a silent fallback would hide which
/// of the two is in force.
pub const PATH_VARIABLE: &str = "TACET_MCP_CONFIG";

/// The default config path: `$TACET_MCP_CONFIG`, or `mcp.json` in the config
/// directory.
///
/// THE DIRECTORY IS NOT COMPUTED HERE. This function used to read `HOME` and
/// build `mcp.json` inside a hidden folder; the same expression was also
/// written out in the memory and skill layers, and nothing guaranteed the three
/// stayed the same — changing one would silently separate the others. The path
/// now lives in one place, `tacet_kernel::env`, and that is where the platform
/// difference (XDG / `%APPDATA%`) and the `TACET_HOME` override are known.
pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = tacet_kernel::env_var(PATH_VARIABLE) {
        return Some(PathBuf::from(p));
    }
    tacet_kernel::config_path("mcp.json")
}

/// Reads from the file. **If the file does not exist an empty config is
/// returned — not an error.**
pub fn read(path: &Path) -> MCPResult<Config> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        // It exists but cannot be read (permissions): this MUST NOT pass
        // silently — the user added a connection and must know why it is not
        // showing up.
        Err(_) => return Err(MCPError::InvalidAddress(path.display().to_string())),
    };
    parse(&text)
}

/// Reads the file AND measures whether the plain-text key in it is readable by
/// anybody else on this machine.
///
/// A SEPARATE FUNCTION FROM `read`, rather than a field on `Config`: `Config`
/// is serialized back to disk, and a warning flag is not part of the disk
/// format. Callers that do not care keep calling `read`.
pub fn read_checked(path: &Path) -> MCPResult<(Config, bool)> {
    let config = read(path)?;
    let exposed = key_file_is_exposed(path, &config);
    Ok((config, exposed))
}

/// Is there a plain-text key in this file that somebody else on the machine can
/// read.
///
/// TWO CONDITIONS, BOTH REQUIRED. A permissive mode alone is not a finding: a
/// config that only names environment variables carries no secret, and warning
/// about it teaches the user to ignore warnings. A plain `key` alone is not one
/// either: at 0600 it is as safe as a file on a desktop gets.
///
/// `& 0o077` — anything readable, writable or executable by GROUP or OTHER.
/// On macOS the group half is the one that bites: home directories are
/// `drwxr-x---` with group `staff`, and every local account is in `staff`.
#[cfg(unix)]
pub fn key_file_is_exposed(path: &Path, config: &Config) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let carries_a_plain_key = config
        .connections
        .iter()
        .any(|c| c.key.as_deref().is_some_and(|k| !k.is_empty()));
    if !carries_a_plain_key {
        return false;
    }
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o077 != 0)
        .unwrap_or(false)
}

/// ALWAYS FALSE ON WINDOWS. The permission model there is ACLs, not a mode
/// word; `PermissionsExt` does not exist and translating "0600" into an ACL
/// check is a different piece of work. Reporting a made-up answer would be
/// worse than reporting none — the same platform split the memory layer
/// already writes down.
#[cfg(not(unix))]
pub fn key_file_is_exposed(_path: &Path, _config: &Config) -> bool {
    false
}

/// Parses from text — pure, no filesystem needed.
pub fn parse(text: &str) -> MCPResult<Config> {
    if text.trim().is_empty() {
        return Ok(Config::default());
    }
    serde_json::from_str(text).map_err(|_| MCPError::Malformed)
}

/// Reads from the default path; if even the path cannot be found it returns
/// empty.
pub fn read_default() -> MCPResult<Config> {
    match default_path() {
        Some(p) => read(&p),
        None => Ok(Config::default()),
    }
}

/// `read_default` PLUS the permission measurement.
///
/// WHY THIS EXISTS: `key_file_is_exposed` and `read_checked` were written and
/// tested, but nothing on the live path called either of them — the shell loads
/// through `read_default`, so the warning could never reach a user. A check that
/// only runs from a test is not a fix; it is a closed door painted on a wall.
/// This is the one entry point the shell actually uses, so the measurement is
/// attached HERE rather than left for callers to remember.
///
/// `false` when the path cannot be resolved: there is no file, so there is no
/// exposed key.
pub fn read_default_checked() -> MCPResult<(Config, bool)> {
    match default_path() {
        Some(p) => read_checked(&p),
        None => Ok((Config::default(), false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_gives_an_empty_config() {
        assert_eq!(parse("").expect("empty"), Config::default());
        assert!(parse("   \n").expect("empty").connections.is_empty());
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let missing = Path::new("/tmp/definitely-missing-tacet-mcp-file.json");
        assert!(
            read(missing)
                .expect("must not error")
                .connections
                .is_empty()
        );
    }

    #[test]
    fn a_full_record_parses() {
        let c = parse(
            r#"{"connections":[{"name":"home","url":"https://example.com/mcp","key":"abc"}]}"#,
        )
        .expect("must parse");
        assert_eq!(c.connections.len(), 1);
        assert_eq!(c.connections[0].name, "home");
        assert!(c.connections[0].enabled, "enabled must default to true");
        assert_eq!(c.connections[0].resolved_key().as_deref(), Some("abc"));
    }

    /// DISK FORMAT record: this is what `mcp.json` looked like before the
    /// English rename. Without the `serde(alias)`es every configured connection
    /// would vanish at once.
    #[test]
    fn a_config_written_with_the_old_turkish_keys_still_parses() {
        let c = parse(
            r#"{"baglantilar":[{"name":"home","url":"https://example.com/mcp",
                "anahtar":"abc","etkin":false}]}"#,
        )
        .expect("must parse");
        assert_eq!(c.connections.len(), 1);
        assert_eq!(c.connections[0].name, "home");
        assert!(
            !c.connections[0].enabled,
            "the old `enabled` must carry across"
        );
        assert_eq!(c.connections[0].resolved_key().as_deref(), Some("abc"));
    }

    #[test]
    fn broken_json_is_malformed() {
        assert_eq!(
            parse("{this job note json").unwrap_err(),
            MCPError::Malformed
        );
    }

    /// THE 0600 RULE, MEASURED. The header of this module has promised it
    /// since the beginning; until now nothing checked it, so a token sitting
    /// in a world-readable file looked exactly like a token in a locked one.
    #[cfg(unix)]
    #[test]
    fn a_plain_key_in_a_readable_file_is_reported_and_a_locked_one_is_not() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join("tacet-mcp-key-mode-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("directory");

        let write = |name: &str, body: &str, mode: u32| {
            let path = dir.join(name);
            std::fs::write(&path, body).expect("write");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .expect("permissions");
            path
        };

        let with_key =
            r#"{"connections":[{"name":"home","url":"https://a.test/mcp","key":"secret"}]}"#;
        let with_env =
            r#"{"connections":[{"name":"home","url":"https://a.test/mcp","key_env":"TOKEN"}]}"#;

        // The finding: an editor's default umask gives 0644 and the token is
        // readable by every other account on the machine.
        let exposed = write("exposed.json", with_key, 0o644);
        let (config, warned) = read_checked(&exposed).expect("reads");
        assert!(warned, "a plain key at 0644 must be reported");
        assert!(key_file_is_exposed(&exposed, &config));

        // The documented state.
        let locked = write("locked.json", with_key, 0o600);
        assert!(!read_checked(&locked).expect("reads").1);

        // NO NOISE: a file that names an environment variable holds no secret,
        // and a warning nobody needs is a warning everybody learns to skip.
        let env_only = write("env.json", with_env, 0o644);
        assert!(!read_checked(&env_only).expect("reads").1);

        // Group-only readability counts too — on macOS every local account is
        // in `staff`, so 0640 is not private.
        let group = write("group.json", with_key, 0o640);
        assert!(read_checked(&group).expect("reads").1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_carries_no_warning() {
        let missing = Path::new("/tmp/definitely-missing-tacet-mcp-file.json");
        let (config, warned) = read_checked(missing).expect("must not error");
        assert!(config.connections.is_empty());
        assert!(!warned);
    }

    #[test]
    fn the_invalid_ones_are_filtered_and_the_rest_remain() {
        let c = parse(
            r#"{"connections":[
                {"name":"good","url":"https://a.com/mcp"},
                {"name":"disabled","url":"https://b.com/mcp","enabled":false},
                {"name":"bad-address","url":"http://remote.com/mcp"},
                {"name":"","url":"https://c.com/mcp"}
            ]}"#,
        )
        .expect("must parse");
        let valid = c.valid();
        assert_eq!(valid.len(), 1);
        assert_eq!(valid[0].name, "good");
    }
}
