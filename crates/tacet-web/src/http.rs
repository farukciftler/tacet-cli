//! `http` — the ONE place a request to an API THE USER NAMED leaves from.
//!
//! NETWORK MONOPOLY. `ureq` is called in exactly three files in this repository:
//! `client.rs` (search), `download.rs` (model packages) and this one. The
//! `http` TOOL lives in `tacet-tools/src/http_call.rs` and opens no socket — it
//! calls the primitive here, exactly as `web_search.rs` calls `client.rs`. The
//! value of the monopoly is that it can be audited: the answer to "where can a
//! byte leave this machine from" is a list of three files, not a grep.
//!
//! THIS CRATE DOES NOT KNOW `tacet-kernel`. No `ToolOutcome`, no chip, no
//! `DataStore` here; the translation happens at the tool boundary. That rule is
//! why this file returns `WebError`, whose Display strings all speak about
//! "search" — the http tool therefore writes its OWN user-facing sentences
//! instead of using `Display` (see `http_call::convert`).
//!
//! ---
//!
//! FOUR GATES BEFORE A SINGLE BYTE MOVES, in this order and for this reason:
//!
//! 1. **https only.** No local-network exception, unlike `address_is_valid`.
//!    That exception exists because the SearXNG address is one THE USER TYPED
//!    into their own registry; the address here is composed by THE MODEL, and
//!    the model is reachable by whoever wrote the last page it read. Plain
//!    `http://` would also mean the request body — which may carry whatever the
//!    model scraped out of the session — is readable at every hop.
//! 2. **The allowlist.** The host must be, character for character, one of the
//!    hosts in the user's registry record. Not a prefix, not a suffix, not a
//!    wildcard. `10.evil.com` already defeated a prefix match in this repository
//!    once (`client::is_local`); a suffix match has the same shape of hole
//!    (`evil-api.example.com` against `example.com`). A CLOSED SET is the only
//!    form that cannot be smuggled past.
//! 3. **`target_is_public`.** CALLED, NOT REWRITTEN. The SSRF gate in
//!    `client.rs` was measured against loopback, the `169.254` metadata service,
//!    the RFC1918 blocks and the userinfo trick; a second copy of that reasoning
//!    would drift from the first and only one of the two would get the next fix.
//! 4. **No redirects.** See `call`.
//!
//! THE ALLOWLIST IS CHECKED BEFORE `target_is_public`, AND THE ORDER IS THE
//! POINT. `target_is_public` RESOLVES the name — that is a DNS query, i.e. a
//! packet leaving the machine carrying a string the model chose. A model that
//! wanted to smuggle a secret off the device could encode it in a hostname and
//! never need the request to succeed. Putting the closed set first means a host
//! the user never named is not even LOOKED UP.
//!
//! ---
//!
//! NO MODEL-CHOSEN HEADERS, AND THAT IS A LIMITATION WE ACCEPT OUT LOUD. A
//! header is where a credential lives; letting the model compose one would give
//! it both a place to put a stolen `Authorization` value and a way to override
//! `Host`. Tacet has no store for per-host API credentials yet, so this tool
//! reaches PUBLIC APIs only. That is a smaller tool than "call any API", and it
//! is the honest one: half a credential story is worse than none.
//!
//! CLOSED BY DEFAULT. With no `http` record in the addon registry
//! `allowed_hosts()` is empty, `is_open()` is false, and the tool is not built
//! at all (`HttpCallTool::discover`) — so it never enters the catalog, no
//! grammar is generated for it and the model cannot name it. The "data does not
//! leave the device" default is enforced as the ABSENCE of the tool, not as a
//! runtime check somebody has to remember.

use crate::client::target_is_public;
use crate::error::{WebError, WebResult};
use std::time::Duration;

/// The addon name AND kind in the registry. The same string today; the fields
/// stay separate for the same reason as `WEB_SEARCH` (a user may one day want
/// two records of one kind).
pub const HTTP_ADDON: &str = "http";

/// The registry setting that carries the allowlist. A comma-separated list of
/// bare host names — `api.github.com, api.open-meteo.com`.
///
/// WHY A STRING AND NOT A JSON ARRAY: `addon::Addon::settings` is a
/// `BTreeMap<String, String>` and that shape is not mine to change from this
/// file. A comma-separated list parses in four lines and cannot be
/// mis-escaped; a nested format inside a string value would be the worse of the
/// two options.
pub const HOSTS_KEY: &str = "hosts";

/// The timeout. MANDATORY and not an `Option`, for the same reason as
/// `client::DEFAULT_TIMEOUT`: a field whose default is "unlimited" eventually
/// stays unlimited, and a hung connection leaves the user staring at a chip.
///
/// LONGER THAN SEARCH'S 10s: an API call is not a page fetch that we can give
/// up on and still answer from summaries — if it times out the tool has nothing
/// at all. 15s is still short enough that a user does not think the shell hung.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// The cap on the response body kept in memory (256 KiB).
///
/// It bounds the STORE record and the chip detail, not the model's window — the
/// model never sees more than the tool's own summary budget. An API answer
/// larger than this is a data dump, not an answer, and the truncation is
/// reported rather than hidden (`HttpResponse::truncated`).
pub const MAX_BODY: u64 = 256 * 1024;

/// The cap on the REQUEST body the model may send (32 KiB).
///
/// The response cap protects memory; this one protects the USER. Every byte
/// here leaves the device, and the approval chip shows what leaves — a request
/// body of megabytes cannot be read by a human before they press yes, so an
/// approval gate in front of it would be theatre. Bounding the payload is what
/// keeps the gate meaningful.
pub const MAX_REQUEST_BODY: usize = 32 * 1024;

/// The content type of a POST. FIXED, not model-chosen — see the header note at
/// the top of the file.
pub const POST_CONTENT_TYPE: &str = "application/json";

// ---------------------------------------------------------------------------
// The method
// ---------------------------------------------------------------------------

/// The closed set of verbs. `DELETE`, `PUT` and `PATCH` are not "blocked", they
/// are UNWRITABLE: adding one means adding an enum variant and a schema choice,
/// which is a decision somebody makes on purpose rather than a filter somebody
/// forgets to update. The same shape as `git::Action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl Method {
    /// The single source of the schema's choice list and of the parser below.
    /// Written out twice, the two would drift and the model would be offered a
    /// value the parser rejects.
    pub const ALL: [Method; 2] = [Method::Get, Method::Post];

    pub fn name(&self) -> &'static str {
        match self {
            Method::Get => "get",
            Method::Post => "post",
        }
    }

    pub fn parse(raw: &str) -> Option<Method> {
        let raw = raw.trim().to_ascii_lowercase();
        Method::ALL.into_iter().find(|m| m.name() == raw)
    }
}

// ---------------------------------------------------------------------------
// The response
// ---------------------------------------------------------------------------

/// What came back. A 4xx/5xx IS NOT AN ERROR HERE — see `call`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    /// Did the body hit `MAX_BODY`. Reported, never silently swallowed: a model
    /// handed a body cut in half without being told invents the rest.
    pub truncated: bool,
    /// The `Location` of a redirect we did NOT follow. Carried so the tool can
    /// tell the user which host they would have to allowlist, instead of
    /// returning a bare "it did not work".
    pub location: Option<String>,
}

impl HttpResponse {
    /// Did the server call this a success. Kept next to the type so the tool
    /// layer does not re-invent the range.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status)
    }
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

pub struct HttpClient {
    /// The allowlist, already lowercased. Empty = nothing may be called; there
    /// is no "empty means everything" reading anywhere in this file.
    hosts: Vec<String>,
    timeout: Duration,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient {
    /// A client configured from the ADDON REGISTRY. No address and no host is
    /// baked into the code.
    pub fn new() -> Self {
        Self::with_hosts(allowed_hosts())
    }

    /// The allowlist supplied from outside — for tests and for the diagnostic
    /// command. A separate constructor rather than a mutable field: the list is
    /// a security boundary and must not be reachable for edit after the client
    /// exists.
    pub fn with_hosts(hosts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            hosts: hosts
                .into_iter()
                .map(|h| h.into().trim().to_ascii_lowercase())
                .filter(|h| !h.is_empty())
                .collect(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Every gate, and NOT ONE BYTE OF NETWORK — with one stated exception.
    ///
    /// SEPARATE AND PUBLIC so the tool can pre-flight a call before it draws a
    /// chip that says data is leaving, and so the gates are testable without a
    /// server. The exception: step 4 (`target_is_public`) resolves the name,
    /// which is a DNS query. It runs LAST, after the closed set has already
    /// approved the host — see the ordering note at the top of the file.
    pub fn check(&self, url: &str) -> WebResult<()> {
        let url = url.trim();
        if !url.starts_with("https://") {
            return Err(WebError::InvalidAddress(format!(
                "the address must start with https:// — {url}"
            )));
        }
        let Some(host) = host_of(url) else {
            return Err(WebError::InvalidAddress(format!("no host name: {url}")));
        };
        if self.hosts.is_empty() {
            return Err(WebError::InvalidAddress(
                "no API host has been allowed: `tacet addon add http`".into(),
            ));
        }
        // THE CLOSED SET. `contains`, i.e. whole-string equality — NOT
        // `starts_with`, NOT `ends_with`. See gate 2 at the top of the file.
        if !self.hosts.contains(&host) {
            return Err(WebError::InvalidAddress(format!(
                "{host} is not in the allowed API host list"
            )));
        }
        target_is_public(url)
    }

    /// Makes the call.
    ///
    /// A 4xx/5xx COMES BACK AS A RESPONSE, NOT AS AN ERROR
    /// (`http_status_as_error(false)`). An API's 404 body says WHICH resource is
    /// missing and its 429 body says how long to wait; turning both into one
    /// fixed "tool_failed" throws away the only useful part and pushes the model
    /// into guessing. A transport failure — DNS, TLS, timeout — is a different
    /// thing and does stay an error.
    ///
    /// REDIRECTS ARE NOT FOLLOWED (`max_redirects(0)`, and `ureq` hands the 30x
    /// back rather than erroring when the cap is zero). `client::fetch_public`
    /// drives its redirect chain by hand precisely so the SSRF gate can re-run
    /// on every hop, and that is right for reading a PUBLIC page. It is the
    /// wrong trade here: the hop address is chosen by THE FAR SIDE, while the
    /// thing being carried is the USER'S OWN REQUEST BODY. Following it would
    /// deliver that payload to a host the user never put on the allowlist —
    /// re-running the gate would not help, because the gate would have to be the
    /// allowlist and the allowlist is exactly what the redirect is trying to
    /// leave. The `Location` is reported instead, so the user can decide to add
    /// that host themselves.
    pub fn call(&self, method: Method, url: &str, body: Option<&str>) -> WebResult<HttpResponse> {
        let url = url.trim();
        // THE PAYLOAD CAP COMES FIRST, BEFORE `check`, and the order is the same
        // argument as the allowlist-before-resolver one above: `check` ends in a
        // DNS lookup, and a request that is going to be refused for its size must
        // not put a model-chosen name on the wire on its way to being refused.
        // It is also the cheapest test we have — a local `len()`.
        if let Some(b) = body
            && b.len() > MAX_REQUEST_BODY
        {
            return Err(WebError::InvalidAddress(format!(
                "the request body is too large ({} bytes, the limit is {MAX_REQUEST_BODY})",
                b.len()
            )));
        }
        self.check(url)?;

        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            // See the redirect note above.
            .max_redirects(0)
            // Belt to the `check` braces: even if a gate were reordered one day,
            // ureq refuses a non-https address by itself.
            .https_only(true)
            // See the 4xx/5xx note above.
            .http_status_as_error(false)
            .build()
            .into();

        let sent = match (method, body) {
            (Method::Get, _) => agent.get(url).call(),
            (Method::Post, Some(b)) => agent.post(url).content_type(POST_CONTENT_TYPE).send(b),
            // A POST with no body is a legitimate shape (a trigger endpoint);
            // `send_empty` sets `Content-Length: 0` instead of leaving the
            // request half-written.
            (Method::Post, None) => agent.post(url).send_empty(),
        };

        let mut response = sent.map_err(|e| convert(&e))?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        // READ THROUGH `take`, NOT THROUGH `limit` ALONE. `limit` makes ureq
        // ERROR when the body is longer, which would throw away the part we
        // already have — and the useful half of an over-long API answer is
        // usually its beginning. `take` stops at the cap and hands back what
        // arrived; `limit` stays as a second wall in case `take` is ever
        // refactored away.
        let mut reader = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .lossy_utf8(true)
            .reader();
        let mut buffer: Vec<u8> = Vec::new();
        {
            use std::io::Read;
            reader
                .by_ref()
                .take(MAX_BODY)
                .read_to_end(&mut buffer)
                .map_err(|e| WebError::Unreachable(e.to_string()))?;
        }
        let truncated = buffer.len() as u64 >= MAX_BODY;

        Ok(HttpResponse {
            status,
            // LOSSY, NOT AN ERROR: an API that returns one malformed byte in a
            // 200 response must not turn into "the tool is broken". The body is
            // data to be shown, not a format we depend on.
            body: String::from_utf8_lossy(&buffer).into_owned(),
            truncated,
            location,
        })
    }
}

// ---------------------------------------------------------------------------
// The registry — NO NETWORK below this line
// ---------------------------------------------------------------------------

/// The hosts the user has allowed. An empty list means "no API may be called",
/// never "any API may be called".
///
/// A CORRUPT OR CLOSED REGISTRY IS AN EMPTY LIST — the same fail-closed reading
/// as `addon::web_search_is_open`: an unreadable configuration must not be
/// interpreted as "it was probably open", because the cost of guessing wrong is
/// the user's data going to a host they never named.
pub fn allowed_hosts() -> Vec<String> {
    let Ok(record) = crate::addon::read() else {
        return Vec::new();
    };
    let Some(addon) = record.find(HTTP_ADDON) else {
        return Vec::new();
    };
    if !addon.open {
        return Vec::new();
    }
    parse_hosts(addon.setting(HOSTS_KEY).unwrap_or_default())
}

/// THE CATALOG GATE. Whether the `http` tool is built at all depends on this.
pub fn is_open() -> bool {
    !allowed_hosts().is_empty()
}

/// Splits the setting into hosts. SEPARATE AND PUBLIC so the install command can
/// use the same rule it will later be judged by — the `address_is_valid`
/// pattern. Two copies of this rule would let an entry into the registry that
/// the client then refuses, and the user would have no way to see why.
///
/// IT SPLITS ON BOTH SEPARATORS, and that is a measured fix rather than
/// generosity. A many-valued setting is STORED newline-separated
/// (`addon::VALUE_SEPARATOR`), because a comma is legal inside a directory name;
/// this function used to split on the comma alone, so a registry written by the
/// installer came back as ONE junk host and every call was refused while the
/// tool still stood in the catalog — a tool that fails every time, which is the
/// trap this repository refuses elsewhere. Comma stays accepted because that is
/// what a person types into `--value "a.example.com,b.example.com"`. Neither
/// character can occur INSIDE a host name, so accepting both loses nothing.
pub fn parse_hosts(raw: &str) -> Vec<String> {
    raw.split([',', crate::addon::VALUE_SEPARATOR])
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        // A user who writes `https://api.example.com/v1` means the host; taking
        // the setting literally would silently allow nothing at all.
        .map(|h| host_of(&h).unwrap_or(h))
        .collect()
}

/// The host of an address, lowercased and without the port.
///
/// A DELIBERATE SECOND COPY of the parsing inside `client::host_and_port`, and
/// the reason is stated rather than hidden: that function is private and
/// returns a `(host, port)` pair this file has no use for, and making it public
/// means editing `client.rs`. WHAT IS NOT COPIED is the SECURITY DECISION —
/// whether an address may be reached at all is still `target_is_public`'s
/// answer alone. This function only answers "which name did the user have to
/// have allowed", and the one rule it must not get wrong is the userinfo one
/// below, which is tested.
///
/// `user:password@host` — THE HOST IS WHAT FOLLOWS THE LAST `@`. Reading the
/// front instead would let `https://api.allowed.test@evil.test/` pass an
/// allowlist that contains `api.allowed.test` while the bytes go to
/// `evil.test`. That exact trick is already recorded against this repository's
/// SSRF gate.
pub fn host_of(url: &str) -> Option<String> {
    let rest = match url.split_once("://") {
        Some(("https", rest)) => rest,
        Some(("http", rest)) => rest,
        Some(_) => return None,
        // A bare `api.example.com` — the shape a user types into the registry.
        None => url,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.is_empty() {
        return None;
    }
    // An IPv6 literal is bracketed and the brackets are not part of the host.
    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, _) = bracketed.split_once(']')?;
        return Some(host.to_ascii_lowercase());
    }
    let host = match authority.rsplit_once(':') {
        Some((host, _)) if !host.is_empty() => host,
        _ => authority,
    };
    Some(host.to_ascii_lowercase())
}

/// `ureq::Error` → `WebError`.
///
/// A SECOND TRANSLATION POINT, and it is not the same one as `client::convert`
/// even though the arms line up: that function is private, and the two paths
/// disagree on purpose about what a status code means — `client` turns a 503
/// into `ServerCode`, while this file never reaches this function for a status
/// at all (`http_status_as_error(false)`). Sharing one function would hide that
/// difference behind a shape that looks identical.
fn convert(error: &ureq::Error) -> WebError {
    match error {
        // Reached only if `http_status_as_error` is ever turned back on; kept so
        // the arm does not fall into the catch-all and lose the code.
        ureq::Error::StatusCode(code) => WebError::ServerCode(*code),
        ureq::Error::Timeout(_) => WebError::Timeout,
        other => WebError::Unreachable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> HttpClient {
        HttpClient::with_hosts(["api.example.test", "data.example.test"])
    }

    /// THE HAPPY PATH STILL PASSES THE GATES — a security check that also
    /// refuses the legitimate case is the one that gets deleted six months
    /// later. `example.test` does not resolve, so this asserts on the shape of
    /// the failure rather than on success: everything BEFORE the resolver is
    /// what this file owns.
    #[test]
    fn an_allowed_host_gets_past_the_closed_set() {
        let c = client();
        match c.check("https://api.example.test/v1/things?a=1") {
            // The resolver refused it — the allowlist did not.
            Err(WebError::InvalidAddress(m)) => assert!(
                !m.contains("not in the allowed"),
                "the allowlist rejected an allowed host: {m}"
            ),
            other => assert!(other.is_ok(), "{other:?}"),
        }
    }

    #[test]
    fn a_host_that_is_not_on_the_list_is_refused_before_anything_else() {
        let c = client();
        let e = c.check("https://evil.test/steal").unwrap_err();
        match e {
            WebError::InvalidAddress(m) => assert!(m.contains("not in the allowed"), "{m}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// THE PREFIX/SUFFIX HOLE, WRITTEN DOWN AS A TEST. `client::is_local` was
    /// fooled by `10.evil.com` because it matched a prefix; an allowlist matched
    /// by suffix has the identical shape. None of these may pass.
    #[test]
    fn a_host_that_merely_looks_allowed_is_not_allowed() {
        let c = client();
        for url in [
            "https://api.example.test.evil.test/x",
            "https://evil-api.example.test/x",
            "https://api.example.testX/x",
            "https://notapi.example.test/x",
            "https://api.example.test.evil/x",
        ] {
            let e = c.check(url).unwrap_err();
            assert!(
                matches!(&e, WebError::InvalidAddress(m) if m.contains("not in the allowed")),
                "the closed set let {url} through: {e:?}"
            );
        }
    }

    /// THE USERINFO TRICK. The allowlist must judge the host the bytes actually
    /// go to, not the decoration in front of the `@`.
    #[test]
    fn a_public_looking_userinfo_does_not_smuggle_a_host_past_the_list() {
        let c = client();
        for url in [
            "https://api.example.test@evil.test/x",
            "https://api.example.test:pw@evil.test/x",
            "https://user@api.example.test@evil.test/x",
        ] {
            let e = c.check(url).unwrap_err();
            assert!(
                matches!(&e, WebError::InvalidAddress(m) if m.contains("not in the allowed")),
                "userinfo smuggled a host past the list: {url} -> {e:?}"
            );
        }
        assert_eq!(
            host_of("https://api.example.test@evil.test/x").as_deref(),
            Some("evil.test")
        );
    }

    /// THE SSRF GATE IS REALLY CALLED — proved with a host that IS on the
    /// allowlist and IS off limits. Without the `target_is_public` call the
    /// closed set alone would happily let a user allow `127.0.0.1` and turn the
    /// tool into a port scanner for whoever poisons the model next.
    #[test]
    fn an_allowed_but_unroutable_host_is_still_refused() {
        let c = HttpClient::with_hosts(["127.0.0.1", "169.254.169.254", "[::1]"]);
        for url in [
            "https://127.0.0.1:11434/api/tags",
            "https://169.254.169.254/latest/meta-data/iam/security-credentials/",
            "https://[::1]:8080/x",
        ] {
            assert!(
                matches!(c.check(url), Err(WebError::InvalidAddress(_))),
                "the SSRF gate did not run for {url}"
            );
        }
    }

    #[test]
    fn plain_http_and_every_other_scheme_are_refused() {
        let c = HttpClient::with_hosts(["api.example.test", "localhost"]);
        for url in [
            "http://api.example.test/x",
            // No local-network exception: this is not the user's own configured
            // address, it is one the model composed.
            "http://localhost:8080/x",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "api.example.test/x",
        ] {
            assert!(c.check(url).is_err(), "{url} passed the scheme gate");
        }
    }

    /// AN EMPTY ALLOWLIST MEANS NOTHING, NEVER EVERYTHING. This is the exact
    /// place a fail-open default would hide.
    #[test]
    fn an_empty_allowlist_refuses_everything() {
        let c = HttpClient::with_hosts(Vec::<String>::new());
        assert!(c.check("https://api.example.test/x").is_err());
        assert!(c.hosts().is_empty());
        // A whitespace-only setting is an empty list too — the shape a
        // half-finished `tacet addon add http` would leave behind.
        assert!(HttpClient::with_hosts([" ", ""]).hosts().is_empty());
    }

    #[test]
    fn the_request_body_cap_is_enforced_before_the_socket() {
        let c = client();
        let big = "x".repeat(MAX_REQUEST_BODY + 1);
        let e = c
            .call(Method::Post, "https://api.example.test/v1", Some(&big))
            .unwrap_err();
        assert!(
            matches!(&e, WebError::InvalidAddress(m) if m.contains("too large")),
            "{e:?}"
        );
    }

    #[test]
    fn the_host_setting_is_parsed_forgivingly_but_stored_strictly() {
        let hosts = parse_hosts(" API.Example.Test , https://data.example.test/v1 ,, ");
        assert_eq!(hosts, vec!["api.example.test", "data.example.test"]);
        assert!(parse_hosts("").is_empty());
        assert!(parse_hosts("  ,  ").is_empty());
    }

    /// THE STORED FORM IS NEWLINE-SEPARATED AND THIS USED TO MISS IT ENTIRELY.
    /// The installer joins many values with `addon::VALUE_SEPARATOR`; this
    /// function split on the comma alone, so a two-host registry came back as
    /// ONE junk host, every call was refused, and the tool still sat in the
    /// catalog offering itself. Measured before the fix: installing
    /// `api.a.test,api.b.test` and then calling `https://api.a.test/…` answered
    /// "api.a.test is not in the allowed API host list".
    #[test]
    fn hosts_stored_the_way_the_installer_writes_them_come_back_whole() {
        let stored = crate::addon::join_values(&["api.a.test", "api.b.test"]);
        assert!(stored.contains(crate::addon::VALUE_SEPARATOR), "{stored:?}");
        assert_eq!(parse_hosts(&stored), vec!["api.a.test", "api.b.test"]);
        // Both separators, mixed, in one setting — neither character can occur
        // inside a host name, so accepting both costs nothing.
        assert_eq!(
            parse_hosts("api.a.test\napi.b.test, api.c.test"),
            vec!["api.a.test", "api.b.test", "api.c.test"]
        );
    }

    #[test]
    fn the_host_is_read_out_of_an_address_without_being_fooled() {
        assert_eq!(host_of("https://a.test/x").as_deref(), Some("a.test"));
        assert_eq!(host_of("https://A.TEST:8443/x").as_deref(), Some("a.test"));
        assert_eq!(
            host_of("https://[2001:db8::1]:443/x").as_deref(),
            Some("2001:db8::1")
        );
        assert_eq!(host_of("a.test").as_deref(), Some("a.test"));
        assert_eq!(host_of("ftp://a.test/x"), None);
        assert_eq!(host_of("https:///x"), None);
    }

    #[test]
    fn the_method_set_is_closed() {
        assert_eq!(Method::parse("GET"), Some(Method::Get));
        assert_eq!(Method::parse(" post "), Some(Method::Post));
        for verb in ["delete", "put", "patch", "head", "connect", ""] {
            assert_eq!(Method::parse(verb), None, "{verb} must not be writable");
        }
        assert_eq!(Method::ALL.len(), 2);
    }

    #[test]
    fn the_default_timeout_is_not_infinite() {
        assert_eq!(
            HttpClient::with_hosts(["a.test"]).timeout(),
            DEFAULT_TIMEOUT
        );
        assert!(DEFAULT_TIMEOUT <= Duration::from_secs(30));
    }

    #[test]
    fn the_response_helpers_agree_with_the_status_ranges() {
        let r = |status| HttpResponse {
            status,
            body: String::new(),
            truncated: false,
            location: None,
        };
        assert!(r(200).is_success() && !r(200).is_redirect());
        assert!(r(301).is_redirect() && !r(301).is_success());
        assert!(!r(404).is_success() && !r(404).is_redirect());
    }

    /// REAL NETWORK — run by hand:
    /// `cargo test -p tacet-web http -- --ignored --nocapture`. `#[ignore]` so
    /// CI does not depend on the network; here so it does not get deleted.
    #[test]
    #[ignore = "requires the real network"]
    fn smoke_calls_a_real_public_api() {
        let c = HttpClient::with_hosts(["api.open-meteo.com"]);
        let r = c
            .call(
                Method::Get,
                "https://api.open-meteo.com/v1/forecast?latitude=41&longitude=29&current=temperature_2m",
                None,
            )
            .expect("the call must succeed");
        println!("status={} truncated={}", r.status, r.truncated);
        println!("{}", r.body);
        assert!(r.is_success());
        assert!(r.body.contains("temperature_2m"));
    }
}
