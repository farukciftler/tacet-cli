//! Reading the latest release of a GitHub repository.
//!
//! WHY IT LIVES HERE: the network monopoly. Only `tacet-web` and `tacet-mcp`
//! open sockets, and that rule is what makes the privacy claim auditable —
//! `grep ureq crates/*/Cargo.toml` is the audit. An update check placed in the
//! shell would be the first exception, and the second one would not need an
//! argument.
//!
//! NOTHING HERE RUNS BY ITSELF. There is no start-up check, no timer, no
//! "phone home once a day". A product whose whole promise is that it does not
//! reach the network cannot quietly reach the network to ask about itself. The
//! only caller is the `update` command, which the user types.

use crate::error::{WebError, WebResult};
use std::time::Duration;

/// Anonymous read-only endpoint: no token, no account, and the response carries
/// only what a visitor to the releases page would already see.
const API: &str = "https://api.github.com";

/// GitHub rejects a request with no User-Agent (403). It is sent as the product
/// name and version so a rate-limit investigation on their side can identify
/// what was calling — the same value the MCP client introduces itself with.
fn user_agent() -> String {
    format!("tacet/{}", env!("CARGO_PKG_VERSION"))
}

/// One downloadable file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag as published, e.g. `v0.2.0`.
    pub tag: String,
    /// The human page, for "read the notes" rather than for downloading.
    pub page_url: String,
    pub assets: Vec<ReleaseAsset>,
}

impl Release {
    /// The asset whose name contains `needle` — the host target triple at the
    /// call site.
    ///
    /// Substring, not equality: the file name also carries the version
    /// (`tacet-v0.2.0-aarch64-apple-darwin.tar.gz`), so an exact match would
    /// have to reconstruct the release's own version string and would break the
    /// moment the naming scheme gained a suffix.
    pub fn asset_for(&self, needle: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|a| a.name.contains(needle))
    }
}

/// Reads the latest release of `owner/repo`.
///
/// A pre-release or a draft is NOT returned: `/releases/latest` skips both, and
/// that is the behaviour we want — an unfinished build must not be offered to
/// someone who typed `tacet update`.
pub fn latest(repo: &str, timeout: Duration) -> WebResult<Release> {
    let url = format!("{API}/repos/{repo}/releases/latest");
    let agent = release_agent(timeout);

    let body = match agent
        .get(&url)
        .header("User-Agent", user_agent())
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(mut response) => response
            .body_mut()
            .with_config()
            // 512 KB. A release payload is a few KB; the cap is here so a
            // wrong address cannot stream an unbounded body into memory.
            .limit(512 * 1024)
            .read_to_string()
            .map_err(|e| WebError::Unreachable(e.to_string()))?,
        Err(ureq::Error::StatusCode(code)) => return Err(WebError::ServerCode(code)),
        Err(ureq::Error::Timeout(_)) => return Err(WebError::Timeout),
        Err(other) => return Err(WebError::Unreachable(other.to_string())),
    };

    parse(&body)
}

/// The agent the release check uses. SPLIT OUT SO IT CAN BE ASSERTED ON — see
/// the test at the bottom of this file.
fn release_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(5)
        // THE SCHEME GATE MUST HOLD ON EVERY HOP. `API` is https, but that
        // only covers hop 0: `ureq`'s scheme enforcement lives in one place
        // and only runs when this flag is on, and its default is FALSE. What
        // is read here decides which file gets downloaded and then WRITTEN
        // OVER THE RUNNING BINARY (`tacet-cli::update`), so a chain that drops
        // to plain text lets whoever sits on the path choose that file.
        .https_only(true)
        .build()
        .into()
}

/// Split out from the network call so it can be tested against a recorded
/// payload without a socket. The network test would be the flaky one and it is
/// the one that measures the least.
pub fn parse(body: &str) -> WebResult<Release> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| WebError::InvalidJson(e.to_string()))?;

    let tag = value
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WebError::InvalidJson("no tag_name in the release".into()))?
        .to_string();

    let page_url = value
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let assets = value
        .get("assets")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(ReleaseAsset {
                        name: a.get("name")?.as_str()?.to_string(),
                        url: a.get("browser_download_url")?.as_str()?.to_string(),
                        bytes: a.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Release {
        tag,
        page_url,
        assets,
    })
}

/// Is `candidate` newer than `current`? Both may carry a leading `v`.
///
/// Numeric comparison per component, NOT string comparison: `"0.10.0" >
/// "0.9.0"` is false as text and true as a version, and that is exactly the
/// comparison that decides whether a user is told an update exists.
///
/// An unparsable component makes the whole comparison return `false`: refusing
/// to claim an update is the safe direction, since the other one nags a user
/// into replacing a working binary.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(value: &str) -> Option<Vec<u64>> {
        let cleaned = value.trim().trim_start_matches('v');
        // A pre-release suffix (`0.2.0-rc1`) is cut: the numeric part decides.
        let core = cleaned.split(['-', '+']).next().unwrap_or(cleaned);
        core.split('.').map(|p| p.parse::<u64>().ok()).collect()
    }
    let (Some(a), Some(b)) = (parts(candidate), parts(current)) else {
        return false;
    };
    let width = a.len().max(b.len());
    for index in 0..width {
        let left = a.get(index).copied().unwrap_or(0);
        let right = b.get(index).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &str = r#"{
        "tag_name": "v0.2.0",
        "html_url": "https://github.com/o/r/releases/tag/v0.2.0",
        "assets": [
            {"name": "tacet-v0.2.0-aarch64-apple-darwin.tar.gz",
             "browser_download_url": "https://example.invalid/a.tar.gz", "size": 4096},
            {"name": "SHA256SUMS", "browser_download_url": "https://example.invalid/s", "size": 200}
        ]
    }"#;

    #[test]
    fn parses_tag_and_assets() {
        let release = parse(PAYLOAD).expect("payload parses");
        assert_eq!(release.tag, "v0.2.0");
        assert_eq!(release.assets.len(), 2);
        assert_eq!(release.assets[0].bytes, 4096);
    }

    #[test]
    fn finds_the_asset_for_the_host_triple() {
        let release = parse(PAYLOAD).unwrap();
        let asset = release
            .asset_for("aarch64-apple-darwin")
            .expect("asset present");
        assert_eq!(asset.url, "https://example.invalid/a.tar.gz");
        assert!(release.asset_for("x86_64-pc-windows-msvc").is_none());
    }

    #[test]
    fn a_release_with_no_assets_is_not_an_error() {
        // A source-only release is legitimate; the caller reports "nothing to
        // install for this platform" rather than failing the check.
        let release = parse(r#"{"tag_name":"v1.0.0","html_url":"","assets":[]}"#).unwrap();
        assert!(release.assets.is_empty());
    }

    #[test]
    fn a_body_without_a_tag_is_rejected() {
        assert!(parse(r#"{"html_url":"x"}"#).is_err());
    }

    #[test]
    fn versions_compare_numerically_not_as_text() {
        // The case that string comparison gets wrong.
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.9.0", "0.10.0"));
    }

    #[test]
    fn the_leading_v_and_a_prerelease_suffix_do_not_change_the_answer() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.2.0-rc1", "v0.1.9"));
    }

    #[test]
    fn the_same_version_is_not_newer() {
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
    }

    #[test]
    fn a_shorter_version_is_padded_with_zeros() {
        assert!(!is_newer("1.0", "1.0.0"));
        assert!(is_newer("1.0.1", "1.0"));
    }

    /// The release JSON decides which asset the updater downloads and installs
    /// over the running binary. A `30x` that drops the chain onto plain http
    /// would put that choice in the hands of whoever is on the path; the only
    /// thing standing in the way is this flag, and a flag has no observable
    /// behaviour until the day it matters.
    #[test]
    fn the_release_agent_refuses_to_leave_https() {
        assert!(release_agent(Duration::from_secs(5)).config().https_only());
    }

    #[test]
    fn an_unparsable_version_never_claims_an_update() {
        // The safe direction: silence beats nagging someone into replacing a
        // binary that works.
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("0.2.0", "unknown"));
    }
}
