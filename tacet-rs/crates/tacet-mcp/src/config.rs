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
