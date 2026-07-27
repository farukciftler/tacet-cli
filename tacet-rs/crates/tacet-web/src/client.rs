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
        Self { base: base.into().trim_end_matches('/').to_string(), timeout: DEFAULT_TIMEOUT }
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
        let mut u = format!("{}/search?q={}&format=json&safesearch=1", self.base, escape(query));
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
    pub fn page_text(&self, address: &str) -> WebResult<String> {
        if !address.starts_with("https://") && !address.starts_with("http://") {
            return Err(WebError::InvalidAddress(format!("unsupported scheme: {address}")));
        }
        let raw = self.fetch(address)?;
        let text = to_text(&raw);
        if text.trim().is_empty() {
            // A script-rendered page: HTML arrived but there is no text to
            // read. Handing empty text to the model as "the page is empty"
            // would be a wrong fact.
            return Err(WebError::EmptyResult);
        }
        Ok(text)
    }

    /// The single network call. The ONE place HTTP errors are turned into
    /// `WebError`.
    fn fetch(&self, url: &str) -> WebResult<String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            // A redirect cap: so an endless redirect loop is cut without
            // waiting out the timeout.
            .max_redirects(5)
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
pub fn is_fetchable(address: &str) -> bool {
    const SKIPPED_EXTENSIONS: [&str; 14] = [
        ".pdf", ".zip", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".jpg", ".jpeg",
        ".png", ".gif", ".mp4", ".mp3",
    ];
    if !address.starts_with("https://") && !address.starts_with("http://") {
        return false;
    }
    let path = address.split(['?', '#']).next().unwrap_or(address).to_ascii_lowercase();
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
    Err(WebError::InvalidAddress("the address must start with https://".into()))
}

/// Is the host a local network address.
fn is_local(host: &str) -> bool {
    host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host.ends_with(".local")
        || host.starts_with("192.168.")
        || host.starts_with("10.")
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
        assert!(c.request_url("a", None).starts_with("https://example.test/searxng/search?"));
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
        assert!(u.contains("%26"), "& must not leak as a query separator: {u}");
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
        let e = WebSearchClient::with_address("http://remote.test").search("a", None).unwrap_err();
        assert!(matches!(e, WebError::InvalidAddress(_)), "{e:?}");
    }

    #[test]
    fn plain_http_is_accepted_on_the_local_network() {
        // It must pass the address gate; a network error is expected after that.
        let e = WebSearchClient::with_address("http://localhost:8888").search("a", None).unwrap_err();
        assert!(!matches!(e, WebError::InvalidAddress(_)), "{e:?}");
    }

    /// WITHOUT AN ADDON THERE IS NO ADDRESS EITHER. In the previous state the
    /// developer's own server sat here and search WORKED without the user
    /// installing anything.
    #[test]
    fn nothing_goes_online_when_the_address_is_undefined() {
        let e = WebSearchClient::with_address("").search("a", None).unwrap_err();
        match e {
            WebError::InvalidAddress(m) => {
                assert!(m.contains("addon add"), "the message must show the way: {m}")
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn address_is_valid_makes_the_same_decision_as_the_client() {
        for address in
            ["https://example.test", "http://localhost:8888", "http://remote.test", "example.test", ""]
        {
            let free = address_is_valid(address).is_ok();
            let client = WebSearchClient::with_address(address).validate_address().is_ok();
            assert_eq!(free, client, "the two rules diverged for '{address}'");
        }
    }

    #[test]
    fn an_address_without_a_scheme_is_rejected() {
        let e = WebSearchClient::with_address("example.test").search("a", None).unwrap_err();
        assert!(matches!(e, WebError::InvalidAddress(_)));
    }

    #[test]
    fn an_empty_query_is_rejected_without_going_online() {
        let c = WebSearchClient::with_address("https://example.test");
        assert!(matches!(c.search("   ", None), Err(WebError::InvalidAddress(_))));
    }

    #[test]
    fn page_text_rejects_an_unsupported_scheme() {
        let c = WebSearchClient::with_address("https://example.test");
        assert!(matches!(c.page_text("file:///etc/passwd"), Err(WebError::InvalidAddress(_))));
        assert!(matches!(c.page_text("javascript:alert(1)"), Err(WebError::InvalidAddress(_))));
    }

    #[test]
    fn the_fetchable_filter_screens_out_binary_files() {
        assert!(is_fetchable("https://a.test/timetable"));
        assert!(is_fetchable("https://a.test/timetable.html?day=1"));
        assert!(!is_fetchable("https://appassets.mvtdev.com/map/169/l/1563/575909.pdf"));
        // The query string must not hide the extension.
        assert!(!is_fetchable("https://a.test/report.PDF?v=2"));
        assert!(!is_fetchable("https://a.test/image.jpg"));
        // The scheme gate applies here too.
        assert!(!is_fetchable("file:///etc/passwd"));
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
        let r = c.search("rust programming language", Some("en")).expect("the search must succeed");
        assert!(!r.is_empty());
        assert!(r.iter().all(|x| !x.url.is_empty() && !x.source.is_empty()));
    }
}
