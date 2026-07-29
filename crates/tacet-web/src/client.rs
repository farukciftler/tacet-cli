//! `WebSearchClient` — the ONE place that issues a GET to SearXNG.
//!
//! NETWORK MONOPOLY: `ureq` is called nowhere else but this file; no crate
//! outside `tacet-web` opens a socket. The value of the monopoly is that it can
//! be audited: the answer to "where does the query leave from" is a single file.
//!
//! SYNCHRONOUS: `search` blocks. Because `Tool::run` is async this looks wrong
//! at first glance, but Tacet is built so as NOT TO CHOOSE a runtime (the root
//! Cargo.toml says why tokio is absent) and the async signature exists there
//! for dyn-compatibility, not for concurrency. Writing a blocking call in a
//! blocking way is more honest than putting up a fake async front.

use crate::error::{WebError, WebResult};
use crate::outcome::{SearchOutcome, parse};
use crate::text::to_text;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::time::Duration;

/// The environment variable the server address is read from.
pub const ADDRESS_VARIABLE: &str = "TACET_SEARXNG";

/// NO ADDRESS BAKED INTO THE CODE — and for a while there WAS one here.
///
/// There used to be a constant called `DEFAULT_ADDRESS` pointing at the
/// developer's own instance. Its own doc comment said this was wrong ("no
/// address must be PRESET here in an application build... there the default is
/// EMPTY and the search tool does not appear in the catalog at all until a
/// server is configured") — but the code still fell back to that address: there
/// was no addon concept and nowhere else to fall back to.
///
/// Now there is: the address comes either from `TACET_SEARXNG` or from the
/// ADDON REGISTRY (`crate::addon::web_address`). If neither exists the base
/// stays EMPTY and `address_is_valid` rejects it with an EXPLICIT error — no
/// query silently goes to somebody else's server.
const ADDRESS_UNDEFINED: &str = "search server not configured: `tacet addon add web-search`";

/// The timeout. MANDATORY, and not an `Option`: a field whose default is
/// "unlimited" eventually stays unlimited, and a hung connection leaves the
/// user staring at a "searching…" chip forever.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The cap on a body that goes to the STORE, not the model, when fetching a
/// page.
///
/// WHY THERE IS A CAP: if `read_to_string` reads without a limit, a malicious
/// (or merely enormous) page fills memory. 2 MB, once reduced to text, is more
/// than any reasonable article.
const MAX_BODY: u64 = 2 * 1024 * 1024;

/// The redirect cap on the hand-driven chain (see `fetch_public`). The same
/// number `fetch` hands to `ureq`; the two must not disagree about how many
/// hops are reasonable.
const MAX_HOPS: usize = 5;

pub struct WebSearchClient {
    base: String,
    timeout: Duration,
}

impl Default for WebSearchClient {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSearchClient {
    /// A client configured from the environment and the ADDON REGISTRY.
    ///
    /// THE ADDRESS IS NOT BAKED IN: the server belongs to the user and it
    /// changes; a hardcoded address would tie this feature to a single machine.
    /// If no address is found the base stays EMPTY — no request is built and
    /// `search` returns an explicit error.
    pub fn new() -> Self {
        Self::with_address(crate::addon::web_address().unwrap_or_default())
    }

    pub fn with_address(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, duration: Duration) -> Self {
        self.timeout = duration;
        self
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Validates that the base address is usable.
    fn validate_address(&self) -> WebResult<()> {
        address_is_valid(&self.base)
    }

    /// Builds the request URL. SEPARATE AND PUBLIC: so it can be tested without
    /// going online, and so it can be shown to the user VERBATIM in the chip
    /// detail as "what went out" (spec §3.2, the transparency pattern).
    pub fn request_url(&self, query: &str, language: Option<&str>) -> String {
        let mut u = format!(
            "{}/search?q={}&format=json&safesearch=1",
            self.base,
            escape(query)
        );
        if let Some(l) = language.map(str::trim).filter(|l| !l.is_empty()) {
            u.push_str("&language=");
            u.push_str(&escape(l));
        }
        u
    }

    /// Runs the search. An empty result comes back as an ERROR
    /// (`WebError::EmptyResult`): so the caller is forced to report it to the
    /// model as "no_results" instead of silently seeing an empty list and
    /// starting to make things up.
    pub fn search(&self, query: &str, language: Option<&str>) -> WebResult<Vec<SearchOutcome>> {
        let query = query.trim();
        if query.is_empty() {
            return Err(WebError::InvalidAddress("empty query".into()));
        }
        self.validate_address()?;
        let body = self.fetch(&self.request_url(query, language))?;
        parse(&body)
    }

    /// THE HEALTH QUERY — the verification step of addon installation and of
    /// `tacet addon probe`. Returns: the NUMBER of results that came back.
    ///
    /// WHY A REAL QUERY AND NOT `/` OR `/healthz`: SearXNG can be up while
    /// keeping the JSON format SWITCHED OFF (no `formats: json` inside
    /// `settings.yml`), and in that state the root address returns 200 while
    /// the search silently breaks because HTML comes back. That trap is already
    /// named in this repository (see `WebError::InvalidJson`). Driving THE
    /// PRODUCT'S REAL PATH before declaring the installation "successful" is
    /// the only honest measurement.
    ///
    /// The query is FIXED and innocuous: none of the user's data goes out in
    /// this request.
    ///
    /// NETWORK: this function was NOT MEASURED on this machine (there was no
    /// running SearXNG instance); the path it parses is byte for byte the same
    /// code as `search`, and that path is tested.
    pub fn health(&self) -> WebResult<usize> {
        self.search("tacet connection probe", None).map(|r| r.len())
    }

    /// Pulls the text of a single address (spec v1.1: "fetching a result page").
    ///
    /// Result summaries are truncated to 200 characters and sometimes that is
    /// not enough; this exists so the model can pick ONE address and ask for
    /// the rest. The output is plain text stripped out of the HTML (see
    /// `text.rs` — its simplicity is deliberate).
    /// PLAIN `http://` IS NOT ACCEPTED HERE, and there is no local network
    /// exception either — unlike `search`, whose address the USER TYPED. The
    /// address that arrives here is chosen by the MODEL, or copied out of a
    /// search result, i.e. by the far side; `client.rs` promises "plain http
    /// only on the local network" and `target_is_public` now refuses the local
    /// network on this path, so what is left for `http://` is nothing but an
    /// unencrypted fetch from a remote host.
    pub fn page_text(&self, address: &str) -> WebResult<String> {
        if !address.starts_with("https://") {
            return Err(WebError::InvalidAddress(format!(
                "unsupported scheme: {address}"
            )));
        }
        let raw = self.fetch_public(address)?;
        let text = to_text(&raw);
        if text.trim().is_empty() {
            // A script-rendered page: HTML arrived but there is no text to
            // read. Handing empty text to the model as "the page is empty"
            // would be a wrong fact.
            return Err(WebError::EmptyResult);
        }
        Ok(text)
    }

    /// The single network call TO THE USER'S OWN SERVER. The ONE place HTTP
    /// errors are turned into `WebError`.
    ///
    /// NO TARGET GATE HERE, ON PURPOSE: the address is the base the user
    /// configured with `tacet addon add`, and a SearXNG on `192.168.1.10` is
    /// the normal case. The gate belongs on the path where the address is
    /// chosen by somebody else — see `fetch_public`.
    fn fetch(&self, url: &str) -> WebResult<String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            // A redirect cap: so an endless redirect loop is cut without
            // waiting out the timeout.
            .max_redirects(5)
            // THE SCHEME GATE MUST SURVIVE THE REDIRECT. `address_is_valid`
            // only sees the FIRST address; without this flag a `302 Location:
            // http://…` would silently downgrade the query — and a query is
            // data too. The flag is DERIVED from the already validated address
            // rather than set to a constant `true`, because the local network
            // exception below is deliberate: a local SearXNG has no
            // certificate and its traffic never leaves the machine.
            .https_only(url.starts_with("https://"))
            .build()
            .into();

        match agent.get(url).call() {
            Ok(mut response) => response
                .body_mut()
                .with_config()
                .limit(MAX_BODY)
                .read_to_string()
                .map_err(|e| convert(&e)),
            Err(e) => Err(convert(&e)),
        }
    }

    /// Fetches an address THE USER DID NOT CONFIGURE: every hop is put through
    /// `target_is_public` first.
    ///
    /// WHY THE REDIRECTS ARE DRIVEN BY HAND: `ureq` follows a `30x` on its own
    /// and the address of the second hop is never seen by us. A page that is
    /// perfectly public can answer `302 Location: http://127.0.0.1:11434/…`
    /// and the gate on the first address would have been pointless — measured
    /// with a local listener before this change. `max_redirects(0)` makes ureq
    /// hand the `30x` response back instead of following it (its
    /// `max_redirects_do_error` is false when the cap is zero), and the loop
    /// below re-runs the gate on every single hop.
    fn fetch_public(&self, address: &str) -> WebResult<String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .max_redirects(0)
            // Belt to the hand-driven braces: even if a hop slipped past the
            // loop, ureq refuses a non-https address by itself.
            .https_only(true)
            .build()
            .into();

        let mut current = address.to_string();
        for _ in 0..MAX_HOPS {
            target_is_public(&current)?;
            let mut response = agent.get(&current).call().map_err(|e| convert(&e))?;
            let status = response.status().as_u16();
            if !(300..400).contains(&status) {
                return response
                    .body_mut()
                    .with_config()
                    .limit(MAX_BODY)
                    .read_to_string()
                    .map_err(|e| convert(&e));
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .ok_or_else(|| {
                    WebError::Unreachable(format!("a {status} redirect with no Location header"))
                })?;
            current = join_location(&current, &location)?;
        }
        Err(WebError::Unreachable("too many redirects".into()))
    }
}

/// May this address be fetched on the say-so of the model or of a search
/// result.
///
/// WHY THIS GATE EXISTS — SSRF. `page_text` used to check the SCHEME ONLY; the
/// host was never looked at. That made three things possible at once, and none
/// of them needed the model to cooperate: (a) reading a cloud instance's
/// credentials off `http://169.254.169.254/latest/meta-data/…`, (b) probing a
/// service listening only on loopback (`http://127.0.0.1:11434/api/tags`), and
/// (c) doing either of those WITHOUT the model asking, because the fetch of a
/// search result page is automatic and the result URLs come from the search
/// server's answer — an address the far side controls.
///
/// THE RULE IS ABOUT THE RESOLVED ADDRESS, NOT THE NAME: an attacker can
/// register a public name whose A record is `127.0.0.1`, so the name is
/// resolved and EVERY address it resolves to has to pass.
///
/// KNOWN LIMIT — DNS REBINDING: `ureq` resolves the name a second time when it
/// connects, and a resolver answering differently the second time would land on
/// an address this function never saw. Closing that needs connecting to the
/// verified IP by hand while keeping the `Host:` header, which `ureq` does not
/// expose. It is written down rather than left silent.
pub fn target_is_public(url: &str) -> WebResult<()> {
    let Some((host, port)) = host_and_port(url) else {
        return Err(WebError::InvalidAddress(format!("no host name: {url}")));
    };
    let refused = || {
        Err(WebError::InvalidAddress(format!(
            "target address is not routable on the public internet: {host}"
        )))
    };

    // A literal address needs no resolver — and MUST NOT get one: a DNS lookup
    // for "127.0.0.1" would be a needless round trip on the hot path.
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if ip_is_off_limits(ip) {
            refused()
        } else {
            Ok(())
        };
    }

    let addresses = (host.as_str(), port).to_socket_addrs().map_err(|e| {
        WebError::InvalidAddress(format!("the target address did not resolve: {host} ({e})"))
    })?;
    let mut seen = false;
    for address in addresses {
        seen = true;
        if ip_is_off_limits(address.ip()) {
            return refused();
        }
    }
    if !seen {
        return Err(WebError::InvalidAddress(format!(
            "the target address did not resolve: {host}"
        )));
    }
    Ok(())
}

/// Is this IP one that must never be reached on somebody else's say-so.
fn ip_is_off_limits(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => ipv4_is_off_limits(v4),
        IpAddr::V6(v6) => {
            let parts = v6.segments();
            // `::ffff:127.0.0.1` IS loopback, but `Ipv6Addr::is_loopback` says
            // false for it — the v4 hidden in the last four bytes has to be
            // judged by the v4 rules. The IPv4-compatible form (`::a.b.c.d`)
            // is deprecated but still parses, so it is covered too. `::1` and
            // `::` fall in here as well and come out refused (0.0.0.1 / 0.0.0.0).
            if parts[..5] == [0, 0, 0, 0, 0] && (parts[5] == 0 || parts[5] == 0xffff) {
                let o = v6.octets();
                return ipv4_is_off_limits(Ipv4Addr::new(o[12], o[13], o[14], o[15]));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // fc00::/7 unique local
                || (parts[0] & 0xfe00) == 0xfc00
                // fe80::/10 link local
                || (parts[0] & 0xffc0) == 0xfe80
        }
    }
}

fn ipv4_is_off_limits(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_loopback()            // 127.0.0.0/8
        || ip.is_private()      // 10/8, 172.16/12, 192.168/16
        || ip.is_link_local()   // 169.254/16 — THE CLOUD METADATA SERVICE
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || a == 0               // 0.0.0.0/8 — "this network"; some stacks route it to loopback
        || (a == 100 && (64..128).contains(&b)) // 100.64/10 carrier grade NAT
        || (a == 192 && b == 0 && c == 0) // 192.0.0/24 protocol assignments
        || (a == 198 && (18..20).contains(&b)) // 198.18/15 benchmarking
        || a >= 240 // 240/4 reserved
}

/// The host and the port of an address. IPv6 literals are bracketed
/// (`https://[::1]:8443/x`) and the brackets are NOT part of the host.
fn host_and_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let default_port = match scheme {
        "https" => 443u16,
        "http" => 80,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // `user:password@host` — the host is what comes AFTER the last '@'.
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.is_empty() {
        return None;
    }
    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, tail) = bracketed.split_once(']')?;
        let port = tail
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .unwrap_or(default_port);
        return Some((host.to_ascii_lowercase(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => Some((
            host.to_ascii_lowercase(),
            port.parse().unwrap_or(default_port),
        )),
        _ => Some((authority.to_ascii_lowercase(), default_port)),
    }
}

/// Resolves a `Location` header against the address it came from.
///
/// Deliberately small: an absolute address is taken as it is (and then has to
/// pass the gate again), `//host/…`, `/path` and a relative path are the three
/// remaining shapes. A wrong join here cannot open a hole — whatever comes out
/// goes through `target_is_public` before a single byte is sent.
fn join_location(base: &str, location: &str) -> WebResult<String> {
    let location = location.trim();
    if location.is_empty() {
        return Err(WebError::Unreachable("an empty Location header".into()));
    }
    if location.contains("://") {
        return Ok(location.to_string());
    }
    let (scheme, rest) = base
        .split_once("://")
        .ok_or_else(|| WebError::InvalidAddress(base.to_string()))?;
    if let Some(without_scheme) = location.strip_prefix("//") {
        return Ok(format!("{scheme}://{without_scheme}"));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if location.starts_with('/') {
        return Ok(format!("{scheme}://{authority}{location}"));
    }
    let path = rest[authority.len()..]
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    let directory = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    Ok(format!("{scheme}://{authority}{directory}{location}"))
}

/// Converts a `ureq` error into a `WebError`.
///
/// THE SINGLE TRANSLATION POINT: if the call sites wrote their own mapping, one
/// of them would count a timeout as "unreachable" and another would not; the
/// user would see two different sentences for the same failure.
fn convert(error: &ureq::Error) -> WebError {
    match error {
        ureq::Error::StatusCode(code) => WebError::ServerCode(*code),
        ureq::Error::Timeout(_) => WebError::Timeout,
        // Everything else, `BodyExceedsLimit` included, is a transport layer
        // problem; the detail goes to the chip detail, not to the user.
        other => WebError::Unreachable(other.to_string()),
    }
}

/// Does fetching this address have any chance of producing text.
///
/// WHY A FILTER: `to_text` is an HTML stripper. Handing it a PDF or a JPEG
/// means downloading 2 MB of binary data and extracting a meaningless soup of
/// bytes out of it — it eats the timeout budget and the resulting junk is put
/// in front of the model as "page text". Measured: three of the first ten
/// results for the ferry query were `appassets.mvtdev.com/....pdf` and all
/// three had zero readable times.
///
/// IT IS CHECKED WITH THE QUERY STRING DROPPED: `.../report.pdf?v=2` is a PDF too.
///
/// `https://` ONLY, and that is not cosmetic: `page_text` refuses plain http,
/// so a `http://` result has no chance of producing text either. The filter
/// says so up front, which turns "the fetch failed" into "it was never tried".
pub fn is_fetchable(address: &str) -> bool {
    const SKIPPED_EXTENSIONS: [&str; 14] = [
        ".pdf", ".zip", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".jpg", ".jpeg", ".png",
        ".gif", ".mp4", ".mp3",
    ];
    if !address.starts_with("https://") {
        return false;
    }
    let path = address
        .split(['?', '#'])
        .next()
        .unwrap_or(address)
        .to_ascii_lowercase();
    !SKIPPED_EXTENSIONS.iter().any(|e| path.ends_with(e))
}

/// Is the base address acceptable.
///
/// A FREE FUNCTION, because two separate sides have to ask the same rule: (1)
/// the client itself, before a request is built, and (2) `tacet addon add` —
/// BEFORE writing the address into the registry. If the second side wrote its
/// own rule, the two would silently drift apart and an address that looks valid
/// to the registry but is rejected by the search would get in.
///
/// PLAIN `http://` ONLY ON THE LOCAL NETWORK: a query going unencrypted to a
/// remote server is readable at every hop in between — the "a query is data
/// too" principle (spec §2.2) forbids that. Expecting encryption on a local
/// address, on the other hand, would force the user to install a certificate on
/// their own machine; on loopback the traffic never leaves the machine, so
/// there is no listener in between either. The exception is NARROW:
/// `localhost`, `127.0.0.1`, `::1`, `.local` and the private network blocks.
pub fn address_is_valid(base: &str) -> WebResult<()> {
    let base = base.trim();
    if base.is_empty() {
        return Err(WebError::InvalidAddress(ADDRESS_UNDEFINED.into()));
    }
    if let Some(rest) = base.strip_prefix("https://") {
        return if rest.is_empty() {
            Err(WebError::InvalidAddress("no host name".into()))
        } else {
            Ok(())
        };
    }
    if let Some(rest) = base.strip_prefix("http://") {
        let host = crate::outcome::domain(rest);
        return if is_local(&host) {
            Ok(())
        } else {
            Err(WebError::InvalidAddress(format!(
                "plain http is only accepted on the local network: {host}"
            )))
        };
    }
    Err(WebError::InvalidAddress(
        "the address must start with https://".into(),
    ))
}

/// Is the host a local network address.
///
/// A HOST NAME IS NOT AN IP, and this function used to treat it as one:
/// `starts_with("10.")` also matches `10.evil.com` — a remote domain anybody
/// can register — and `192.168.evil.com` came out local too. The consequence
/// was not theoretical: an address accepted here is an address every later
/// query goes to IN PLAIN TEXT, readable at every hop in between. The private
/// blocks are decided by PARSING THE OCTETS: four parts, each a valid `u8`.
/// (`tacet-mcp::client::is_local` already made the decision this way; the two
/// had silently drifted apart and this is that drift.)
fn is_local(host: &str) -> bool {
    if matches!(host, "localhost" | "127.0.0.1" | "::1") || host.ends_with(".local") {
        return true;
    }
    let octets: Vec<u8> = host
        .split('.')
        .filter_map(|p| p.parse::<u8>().ok())
        .collect();
    if octets.len() != 4 || host.split('.').count() != 4 {
        return false;
    }
    match octets.as_slice() {
        [10, ..] | [192, 168, ..] | [127, ..] => true,
        [172, second, ..] if (16..=31).contains(second) => true,
        _ => false,
    }
}

/// Percent escaping (every byte outside RFC 3986 `unreserved` is escaped).
///
/// ZERO DEPENDENCY: no `urlencoding`/`percent-encoding` crate WAS ADDED — the
/// rule is ten lines and its input is a single query string. It works BYTE BY
/// BYTE, not character by character: multi-byte UTF-8 (Turkish "ç", "ğ") must
/// have each byte escaped separately, otherwise the server receives a corrupt
/// query.
fn escape(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    for b in raw.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                output.push(*b as char)
            }
            _ => output.push_str(&format!("%{b:02X}")),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_url_is_built_correctly() {
        let c = WebSearchClient::with_address("https://example.test/searxng");
        assert_eq!(
            c.request_url("rust async", None),
            "https://example.test/searxng/search?q=rust%20async&format=json&safesearch=1"
        );
    }

    #[test]
    fn a_trailing_slash_in_the_base_does_not_produce_a_double_slash() {
        let c = WebSearchClient::with_address("https://example.test/searxng/");
        assert!(
            c.request_url("a", None)
                .starts_with("https://example.test/searxng/search?")
        );
    }

    #[test]
    fn a_language_is_appended_when_given_and_not_when_blank() {
        let c = WebSearchClient::with_address("https://example.test");
        assert!(c.request_url("a", Some("tr")).ends_with("&language=tr"));
        assert!(!c.request_url("a", Some("  ")).contains("language"));
        assert!(!c.request_url("a", None).contains("language"));
    }

    #[test]
    fn non_ascii_and_special_characters_escape_byte_by_byte() {
        let c = WebSearchClient::with_address("https://example.test");
        let u = c.request_url("çay & kahve?", None);
        // "ç" is two bytes (C3 A7) and both must be escaped separately.
        assert!(u.contains("%C3%A7ay"), "{u}");
        assert!(
            u.contains("%26"),
            "& must not leak as a query separator: {u}"
        );
        assert!(u.contains("%3F"), "? must be escaped: {u}");
    }

    #[test]
    fn a_query_injection_cannot_become_a_url_parameter() {
        // Even if the model produces a query like "x&format=html" it CANNOT
        // add a parameter.
        let c = WebSearchClient::with_address("https://example.test");
        let u = c.request_url("x&format=html", None);
        assert_eq!(u.matches("format=").count(), 1, "{u}");
    }

    #[test]
    fn plain_http_is_rejected_for_a_remote_server() {
        let e = WebSearchClient::with_address("http://remote.test")
            .search("a", None)
            .unwrap_err();
        assert!(matches!(e, WebError::InvalidAddress(_)), "{e:?}");
    }

    #[test]
    fn plain_http_is_accepted_on_the_local_network() {
        // It must pass the address gate (not fail with InvalidAddress).
        let res = WebSearchClient::with_address("http://localhost:8888").search("a", None);
        if let Err(e) = res {
            assert!(!matches!(e, WebError::InvalidAddress(_)), "{e:?}");
        }
    }

    /// WITHOUT AN ADDON THERE IS NO ADDRESS EITHER. In the previous state the
    /// developer's own server sat here and search WORKED without the user
    /// installing anything.
    #[test]
    fn nothing_goes_online_when_the_address_is_undefined() {
        let e = WebSearchClient::with_address("")
            .search("a", None)
            .unwrap_err();
        match e {
            WebError::InvalidAddress(m) => {
                assert!(
                    m.contains("addon add"),
                    "the message must show the way: {m}"
                )
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn address_is_valid_makes_the_same_decision_as_the_client() {
        for address in [
            "https://example.test",
            "http://localhost:8888",
            "http://remote.test",
            "example.test",
            "",
        ] {
            let free = address_is_valid(address).is_ok();
            let client = WebSearchClient::with_address(address)
                .validate_address()
                .is_ok();
            assert_eq!(free, client, "the two rules diverged for '{address}'");
        }
    }

    #[test]
    fn an_address_without_a_scheme_is_rejected() {
        let e = WebSearchClient::with_address("example.test")
            .search("a", None)
            .unwrap_err();
        assert!(matches!(e, WebError::InvalidAddress(_)));
    }

    #[test]
    fn an_empty_query_is_rejected_without_going_online() {
        let c = WebSearchClient::with_address("https://example.test");
        assert!(matches!(
            c.search("   ", None),
            Err(WebError::InvalidAddress(_))
        ));
    }

    #[test]
    fn page_text_rejects_an_unsupported_scheme() {
        let c = WebSearchClient::with_address("https://example.test");
        assert!(matches!(
            c.page_text("file:///etc/passwd"),
            Err(WebError::InvalidAddress(_))
        ));
        assert!(matches!(
            c.page_text("javascript:alert(1)"),
            Err(WebError::InvalidAddress(_))
        ));
        // Plain http is refused HERE even though `search` accepts it on the
        // local network: the address on this path is not the user's.
        assert!(matches!(
            c.page_text("http://example.test/a"),
            Err(WebError::InvalidAddress(_))
        ));
    }

    /// A REMOTE HOST THAT MERELY LOOKS PRIVATE IS NOT PRIVATE. These are
    /// ordinary registrable domains; the old prefix match called them local and
    /// let every query leave in plain text.
    #[test]
    fn a_host_name_that_only_looks_private_does_not_get_the_plain_http_exception() {
        for address in [
            "http://10.evil.com/searxng",
            "http://192.168.evil.com",
            "http://10.attacker.tr",
            "http://172.16.evil.com",
            "http://127.0.0.1.evil.com",
            "http://localhost.evil.com",
            "http://10.0.0.5.evil.com",
        ] {
            assert!(
                address_is_valid(address).is_err(),
                "plain http must not be accepted for {address}"
            );
        }
        // The genuine private blocks keep working — 172.16/12 included, which
        // the old rule did not even know about.
        for address in [
            "http://10.0.0.5/searxng",
            "http://192.168.1.10:8888",
            "http://172.20.0.3",
            "http://127.0.0.1:8888",
            "http://localhost:8888",
            "http://searx.local",
        ] {
            assert!(
                address_is_valid(address).is_ok(),
                "a genuinely local address was rejected: {address}"
            );
        }
    }

    /// THE SSRF GATE. Every one of these is an address the model — or a page,
    /// or a search result the far side chose — could hand to `page_text`.
    /// NOTHING GOES ONLINE: the gate runs before the agent is even built, and
    /// every address here is a literal so no resolver is consulted either.
    #[test]
    fn a_target_that_is_not_on_the_public_internet_is_refused() {
        for address in [
            "https://127.0.0.1/x",
            "https://127.1.2.3:9999/x",
            // The cloud metadata service — the reason this gate exists.
            "https://169.254.169.254/latest/meta-data/iam/security-credentials/",
            "https://10.0.0.5/x",
            "https://192.168.1.1/",
            "https://172.20.0.3/admin",
            "https://100.64.0.1/",
            "https://0.0.0.0/",
            "https://[::1]/x",
            // A plain `is_loopback` on the v6 address says false for this one.
            "https://[::ffff:127.0.0.1]/x",
            "https://[fd00::1]/x",
            "https://[fe80::1]/x",
            "https://255.255.255.255/",
            "https://198.18.0.1/",
        ] {
            assert!(
                matches!(target_is_public(address), Err(WebError::InvalidAddress(_))),
                "the gate let {address} through"
            );
        }
        // A public address passes without a lookup.
        for address in [
            "https://8.8.8.8/x",
            "https://[2001:4860:4860::8888]/x",
            "https://93.184.216.34/",
        ] {
            assert!(target_is_public(address).is_ok(), "{address}");
        }
    }

    /// `page_text` runs the gate itself — the caller cannot forget it.
    #[test]
    fn page_text_refuses_a_target_that_is_not_public() {
        let c = WebSearchClient::with_address("https://example.test");
        for address in [
            "https://127.0.0.1:11434/api/tags",
            "https://169.254.169.254/latest/meta-data/",
            "https://[::1]:8080/",
        ] {
            assert!(
                matches!(c.page_text(address), Err(WebError::InvalidAddress(_))),
                "{address}"
            );
        }
    }

    /// A REDIRECT IS AN ADDRESS TOO. The chain is driven by hand exactly so
    /// the gate can be re-run on the second hop; this measures the piece that
    /// produces that second address.
    #[test]
    fn a_redirect_target_is_resolved_and_then_faces_the_same_gate() {
        let base = "https://public.test/a/b?q=1";
        assert_eq!(
            join_location(base, "http://127.0.0.1:11434/api/tags").unwrap(),
            "http://127.0.0.1:11434/api/tags"
        );
        assert_eq!(
            join_location(base, "/root").unwrap(),
            "https://public.test/root"
        );
        assert_eq!(join_location(base, "c").unwrap(), "https://public.test/a/c");
        assert_eq!(
            join_location(base, "//169.254.169.254/latest").unwrap(),
            "https://169.254.169.254/latest"
        );
        // And the thing that comes out is refused, whichever shape it took.
        for location in [
            "http://127.0.0.1:11434/api/tags",
            "//169.254.169.254/latest",
            "//[::1]/x",
        ] {
            let next = join_location(base, location).expect("resolves");
            assert!(
                target_is_public(&next).is_err(),
                "the second hop was let through: {next}"
            );
        }
    }

    #[test]
    fn the_host_is_read_out_of_the_address_without_being_fooled() {
        assert_eq!(
            host_and_port("https://user:pw@example.test:8443/a"),
            Some(("example.test".into(), 8443))
        );
        // A userinfo part that CONTAINS a public looking host must not be
        // mistaken for the host: the host is what follows the last '@'.
        assert_eq!(
            host_and_port("https://example.test@127.0.0.1/a"),
            Some(("127.0.0.1".into(), 443))
        );
        assert_eq!(
            host_and_port("https://[::1]:8080/x"),
            Some(("::1".into(), 8080))
        );
        assert_eq!(
            host_and_port("https://a.test/x"),
            Some(("a.test".into(), 443))
        );
        assert_eq!(
            host_and_port("http://a.test/x"),
            Some(("a.test".into(), 80))
        );
        assert_eq!(host_and_port("file:///etc/passwd"), None);
    }

    /// The userinfo trick end to end: the gate must judge the REAL host.
    #[test]
    fn a_public_looking_userinfo_does_not_smuggle_a_local_target_through() {
        assert!(target_is_public("https://example.test@127.0.0.1/x").is_err());
        assert!(target_is_public("https://example.test@169.254.169.254/x").is_err());
    }

    #[test]
    fn the_fetchable_filter_screens_out_binary_files() {
        assert!(is_fetchable("https://a.test/timetable"));
        assert!(is_fetchable("https://a.test/timetable.html?day=1"));
        assert!(!is_fetchable(
            "https://appassets.mvtdev.com/map/169/l/1563/575909.pdf"
        ));
        // The query string must not hide the extension.
        assert!(!is_fetchable("https://a.test/report.PDF?v=2"));
        assert!(!is_fetchable("https://a.test/image.jpg"));
        // The scheme gate applies here too — and plain http has no chance
        // either, because `page_text` refuses it.
        assert!(!is_fetchable("file:///etc/passwd"));
        assert!(!is_fetchable("http://a.test/timetable"));
    }

    #[test]
    fn the_default_timeout_is_not_infinite() {
        let c = WebSearchClient::with_address("https://example.test");
        assert_eq!(c.timeout, DEFAULT_TIMEOUT);
        assert!(c.timeout <= Duration::from_secs(15));
    }

    /// REAL NETWORK — run by hand: `cargo test -p tacet-web -- --ignored`.
    /// `#[ignore]` so CI does not depend on the network; here so it does not
    /// get deleted.
    #[test]
    #[ignore = "requires the real network"]
    fn smoke_connects_to_the_real_server() {
        let c = WebSearchClient::new();
        let r = c
            .search("rust programming language", Some("en"))
            .expect("the search must succeed");
        assert!(!r.is_empty());
        assert!(r.iter().all(|x| !x.url.is_empty() && !x.source.is_empty()));
    }
}
