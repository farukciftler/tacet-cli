//! M3 — authorization, hardened (spec §5).
//!
//! The 2026-07-28 revision tightens OAuth in three ways that matter to a
//! personal, on-device tool, and this module implements exactly those three:
//!
//! 1. **RFC 9207 `iss` validation (SEP-2468).** An authorization response is
//!    NOT redeemed unless it names the issuer we started with. Without it, a
//!    malicious authorization server can hand back a code minted by a
//!    DIFFERENT issuer and have the client trade it in at the honest one.
//! 2. **Issuer binding (SEP-2352).** A stored token remembers which issuer
//!    minted it, and it is never attached to a request going anywhere else. A
//!    token is a bearer credential: sending it to the wrong host gives that
//!    host the user's account.
//! 3. **CIMD over DCR.** Dynamic Client Registration is deprecated, so it is
//!    not implemented at all. `client_id` is whatever the user configured —
//!    including a Client ID Metadata Document URL, which is just a client id
//!    with a URL shape and needs no code of ours.
//!
//! ## The flow is terminal-honest
//!
//! Tacet prints the authorization URL and asks for the redirect URL to be
//! pasted back. THERE IS NO LOCALHOST LISTENER: opening a socket to catch the
//! redirect is a bigger architectural decision than a convenience deserves,
//! even though it would legally live in this crate. If the paste flow proves
//! painful IN PRACTICE, that experience decides the listener — not a
//! prediction made now.
//!
//! ## What is deliberately absent
//!
//! - **EMA (Enterprise-Managed Authorization).** It solves fleet consent for
//!   organisations; this is one person's machine. One sentence, not a half
//!   implementation.
//! - **Implicit flow, password grant, DCR.** Deprecated or dangerous.
//! - **A browser launcher.** The URL is printed. A program whose promise is
//!   that it does nothing behind your back does not open windows for you.

use crate::error::{MCPError, MCPResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Where tokens are kept — beside `mcp.json`, in the same private directory.
pub const TOKEN_FILE: &str = "mcp-tokens.json";

/// A connection's OAuth settings (`mcp.json`, the `auth` block).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthSetting {
    /// The authorization server's issuer identifier. THE ANCHOR OF THE WHOLE
    /// MODULE: `iss` is checked against it and tokens are bound to it.
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    /// A plain client id, or a Client ID Metadata Document URL (CIMD).
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// Where the authorization server sends the user back. It is never
    /// listened on — the user pastes the resulting URL back into the terminal.
    #[serde(default = "default_redirect")]
    pub redirect_uri: String,
}

fn default_redirect() -> String {
    // `application_type: native` (SEP-837) is what makes a localhost redirect
    // acceptable for a CLI. We never listen on it; it is an address the
    // authorization server will happily redirect to and the user can copy out
    // of their browser's address bar.
    "http://127.0.0.1:0/callback".to_string()
}

impl AuthSetting {
    /// The addresses must be `https`, and they must all belong to the issuer's
    /// origin. CHECKED BEFORE ANYTHING IS PRINTED: a config that points the
    /// token endpoint at another host is not a working setup with a typo, it is
    /// the exact shape of a credential-stealing one.
    pub fn validate(&self) -> MCPResult<()> {
        for url in [
            &self.issuer,
            &self.authorization_endpoint,
            &self.token_endpoint,
        ] {
            if !url.starts_with("https://") {
                return Err(MCPError::InvalidAddress(url.clone()));
            }
        }
        let issuer_origin = origin(&self.issuer);
        for url in [&self.authorization_endpoint, &self.token_endpoint] {
            if origin(url) != issuer_origin {
                return Err(MCPError::IssuerMismatch);
            }
        }
        Ok(())
    }
}

/// Scheme + host + port, the part a credential decision may rest on.
fn origin(url: &str) -> String {
    let without_scheme = url
        .split_once("://")
        .map(|(scheme, rest)| format!("{scheme}://{}", rest.split(['/', '?', '#']).next().unwrap_or("")))
        .unwrap_or_default();
    without_scheme.trim_end_matches('/').to_ascii_lowercase()
}

/// A stored access token, BOUND to the issuer that minted it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Token {
    /// The issuer this token belongs to. The binding is not decoration: it is
    /// checked on every use.
    pub issuer: String,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix seconds. `None` means the server did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl Token {
    /// Is this token usable for a connection anchored at `issuer`.
    ///
    /// THE ONE FUNCTION THIS MODULE EXISTS FOR. A token minted by issuer A is
    /// never sent to issuer B, whatever the config says today and whoever
    /// edited it since.
    pub fn is_for(&self, issuer: &str) -> bool {
        origin(&self.issuer) == origin(issuer)
    }

    pub fn is_expired(&self, now: u64) -> bool {
        // A minute of slack: a token that expires while the request is in
        // flight is an error the user cannot act on.
        self.expires_at.is_some_and(|at| at <= now + 60)
    }
}

/// The token file: connection name -> token.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TokenStore {
    #[serde(default)]
    pub tokens: std::collections::BTreeMap<String, Token>,
}

impl TokenStore {
    /// The token for a connection, IF it is bound to the issuer that
    /// connection is configured with. A mismatch returns nothing at all —
    /// silently, because the caller's next step is to say "run login again",
    /// which is the truthful advice either way.
    pub fn usable(&self, connection: &str, issuer: &str, now: u64) -> Option<&Token> {
        let token = self.tokens.get(connection)?;
        if !token.is_for(issuer) || token.is_expired(now) {
            return None;
        }
        Some(token)
    }
}

pub fn token_path() -> Option<PathBuf> {
    tacet_kernel::config_path(TOKEN_FILE)
}

pub fn read_tokens(path: &Path) -> MCPResult<TokenStore> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|_| MCPError::Malformed),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TokenStore::default()),
        Err(_) => Err(MCPError::InvalidAddress(path.display().to_string())),
    }
}

/// Writes the token file with the same 0600 stamp `memory.json` gets.
///
/// THE WINDOWS HONESTY NOTE, unchanged from the rest of the product: there is
/// no equivalent of a Unix mode bit here, so on Windows the file is as private
/// as the user's profile directory and no more. Saying so is better than
/// implying a protection that is not there.
pub fn write_tokens(path: &Path, store: &TokenStore) -> MCPResult<()> {
    let text = serde_json::to_string_pretty(store).map_err(|_| MCPError::Malformed)?;
    tacet_kernel::write_private(path, text.as_bytes())
        .map_err(|_| MCPError::InvalidAddress(path.display().to_string()))
}

// ---------------------------------------------------------------------------
// The authorization-code flow
// ---------------------------------------------------------------------------

/// What the user is asked to do, and what has to be remembered while they do it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authorization {
    /// Printed for the user to open.
    pub url: String,
    /// CSRF: it must come back unchanged.
    pub state: String,
    /// PKCE: never leaves this machine until the code is redeemed.
    pub verifier: String,
}

/// Builds the authorization URL with PKCE (S256) and a random state.
pub fn begin(setting: &AuthSetting) -> MCPResult<Authorization> {
    setting.validate()?;
    let verifier = random_token();
    let challenge = base64url(&tacet_kernel::sha256(verifier.as_bytes()));
    let state = random_token();
    let separator = if setting.authorization_endpoint.contains('?') {
        '&'
    } else {
        '?'
    };
    let mut url = format!(
        "{}{separator}response_type=code&client_id={}&redirect_uri={}&state={state}\
         &code_challenge={challenge}&code_challenge_method=S256",
        setting.authorization_endpoint,
        escape(&setting.client_id),
        escape(&setting.redirect_uri),
    );
    if !setting.scopes.is_empty() {
        url.push_str(&format!("&scope={}", escape(&setting.scopes.join(" "))));
    }
    Ok(Authorization {
        url,
        state,
        verifier,
    })
}

/// What the user pasted back, taken apart. PURE — no network, so the checks
/// below are testable without one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Redirect {
    pub code: Option<String>,
    pub state: Option<String>,
    pub issuer: Option<String>,
    pub error: Option<String>,
}

pub fn parse_redirect(url: &str) -> Redirect {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or(url);
    let mut out = Redirect::default();
    for pair in query.split(['&', '#']) {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = unescape(value);
        match key {
            "code" => out.code = Some(value),
            "state" => out.state = Some(value),
            "iss" => out.issuer = Some(value),
            "error" => out.error = Some(value),
            _ => {}
        }
    }
    out
}

/// Everything that must be true BEFORE a code is worth anything.
///
/// The order matters: an error response is reported as itself rather than as
/// "no code", and the `iss` check runs before the code is ever put in a
/// request body.
pub fn check_redirect(
    setting: &AuthSetting,
    started: &Authorization,
    redirect: &Redirect,
) -> MCPResult<String> {
    if let Some(error) = &redirect.error {
        return Err(MCPError::Server(crate::error::safe_for_screen(error)));
    }
    if redirect.state.as_deref() != Some(started.state.as_str()) {
        return Err(MCPError::StateMismatch);
    }
    // RFC 9207 (SEP-2468). MANDATORY, not "when present": a mixed-up authorization
    // response is exactly what an attacker produces, and it produces it by
    // leaving the parameter out.
    match redirect.issuer.as_deref() {
        Some(issuer) if origin(issuer) == origin(&setting.issuer) => {}
        _ => return Err(MCPError::IssuerMismatch),
    }
    redirect
        .code
        .clone()
        .filter(|c| !c.is_empty())
        .ok_or(MCPError::Malformed)
}

/// The body of the token request. Kept separate from the sending so the shape
/// is testable and so nothing about a secret has to be reconstructed in a test.
pub fn token_request_body(
    setting: &AuthSetting,
    started: &Authorization,
    code: &str,
) -> String {
    format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        escape(code),
        escape(&setting.redirect_uri),
        escape(&setting.client_id),
        escape(&started.verifier),
    )
}

pub fn refresh_request_body(setting: &AuthSetting, refresh_token: &str) -> String {
    format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}",
        escape(refresh_token),
        escape(&setting.client_id),
    )
}

/// Turns a token endpoint's answer into a bound token.
pub fn token_from_response(setting: &AuthSetting, body: &Value, now: u64) -> MCPResult<Token> {
    if let Some(error) = body.get("error").and_then(Value::as_str) {
        return Err(MCPError::Server(crate::error::safe_for_screen(error)));
    }
    let access = body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or(MCPError::Malformed)?;
    Ok(Token {
        // THE BINDING IS MADE HERE, from the configured issuer — never from
        // anything the response says it is.
        issuer: setting.issuer.clone(),
        access_token: access.to_string(),
        refresh_token: body
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_string),
        expires_at: body
            .get("expires_in")
            .and_then(Value::as_u64)
            .map(|seconds| now + seconds),
    })
}

// ---------------------------------------------------------------------------
// The two calls that go on the wire
// ---------------------------------------------------------------------------

/// Trades the code for a token. **GOES ON THE NETWORK** — through the same
/// `Transport` seam everything else uses, so it is testable without one.
///
/// Everything checkable is checked BEFORE the code is sent: the settings, the
/// state, the issuer. A code that fails any of them never leaves the machine.
pub fn redeem(
    transport: &dyn crate::transport::Transport,
    setting: &AuthSetting,
    started: &Authorization,
    redirect: &Redirect,
    now: u64,
) -> MCPResult<Token> {
    setting.validate()?;
    let code = check_redirect(setting, started, redirect)?;
    let body = token_request_body(setting, started, &code);
    let response = post_form(transport, &setting.token_endpoint, body)?;
    token_from_response(setting, &response, now)
}

/// Trades a refresh token for a fresh access token. The new token is bound to
/// the SAME issuer — a refresh cannot move a credential to another host.
pub fn refresh(
    transport: &dyn crate::transport::Transport,
    setting: &AuthSetting,
    token: &Token,
    now: u64,
) -> MCPResult<Token> {
    setting.validate()?;
    if !token.is_for(&setting.issuer) {
        return Err(MCPError::IssuerMismatch);
    }
    let refresh_token = token.refresh_token.as_deref().ok_or(MCPError::NotAuthorized)?;
    let body = refresh_request_body(setting, refresh_token);
    let response = post_form(transport, &setting.token_endpoint, body)?;
    let mut fresh = token_from_response(setting, &response, now)?;
    // A server that does not rotate the refresh token expects the old one to
    // keep working; dropping it would log the user out on the next expiry.
    if fresh.refresh_token.is_none() {
        fresh.refresh_token = token.refresh_token.clone();
    }
    Ok(fresh)
}

fn post_form(
    transport: &dyn crate::transport::Transport,
    url: &str,
    body: String,
) -> MCPResult<Value> {
    let request = crate::transport::Request {
        url: url.to_string(),
        headers: vec![
            (
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            ),
            ("Accept".into(), "application/json".into()),
        ],
        body: body.into_bytes(),
    };
    let reply = transport.post(&request)?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(reply.body, 64 * 1024), &mut text)
        .map_err(|_| MCPError::Malformed)?;
    let parsed: Value = serde_json::from_str(&text).map_err(|_| MCPError::Malformed)?;
    // An OAuth error is a JSON body with an `error` field, whatever the status
    // line says; `token_from_response` turns it into the user's sentence.
    if !(200..=299).contains(&reply.status) && parsed.get("error").is_none() {
        return Err(MCPError::Server(format!("HTTP {}", reply.status)));
    }
    Ok(parsed)
}

pub fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Small pure helpers
// ---------------------------------------------------------------------------

/// A random, URL-safe token for `state` and the PKCE verifier.
///
/// NO RANDOM CRATE. The entropy comes from the operating system's own
/// randomness (`/dev/urandom` on Unix, `RtlGenRandom`'s file-less equivalent is
/// not available to us on Windows, where the fallback is used); the fallback
/// mixes the clock, the process id and the address of a fresh allocation. The
/// fallback is WEAKER and it is named as such — it protects a CSRF `state` and
/// a PKCE verifier that live for one minute, not a long-lived secret.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_err()
    {
        let clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seed = format!(
            "{clock}:{}:{:p}",
            std::process::id(),
            &bytes as *const [u8; 32]
        );
        bytes = tacet_kernel::sha256(seed.as_bytes());
    }
    base64url(&bytes)
}

/// base64url without padding (RFC 4648 §5) — what PKCE asks for.
pub fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let packed = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        let count = chunk.len() + 1;
        for i in 0..count {
            let index = (packed >> (18 - 6 * i)) & 0x3f;
            out.push(ALPHABET[index as usize] as char);
        }
    }
    out
}

/// Percent-encoding for a query value. Everything outside the unreserved set
/// is escaped — a client id or scope with a `&` in it must not be able to add
/// a parameter.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn setting() -> AuthSetting {
        AuthSetting {
            issuer: "https://auth.example.com".into(),
            authorization_endpoint: "https://auth.example.com/authorize".into(),
            token_endpoint: "https://auth.example.com/token".into(),
            client_id: "https://tacet.example/client.json".into(),
            scopes: vec!["mcp:tools".into()],
            redirect_uri: default_redirect(),
        }
    }

    #[test]
    fn base64url_matches_the_rfc_vectors() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foob"), "Zm9vYg");
        assert_eq!(base64url(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url(b"foobar"), "Zm9vYmFy");
        // The two characters that make it URL-safe.
        assert_eq!(base64url(&[251, 255]), "-_8");
    }

    #[test]
    fn the_authorization_url_carries_pkce_and_state() {
        let started = begin(&setting()).expect("built");
        assert!(started.url.starts_with("https://auth.example.com/authorize?"));
        assert!(started.url.contains("code_challenge_method=S256"));
        assert!(started.url.contains(&format!("state={}", started.state)));
        // The verifier NEVER travels with the authorization request; only its
        // hash does.
        assert!(!started.url.contains(&started.verifier));
        let expected = base64url(&tacet_kernel::sha256(started.verifier.as_bytes()));
        assert!(started.url.contains(&format!("code_challenge={expected}")));
        // A scope with a space is escaped, not left to split the query.
        assert!(started.url.contains("scope=mcp%3Atools"));
    }

    #[test]
    fn a_response_without_iss_is_not_redeemed() {
        // RFC 9207. THE CASE THAT MATTERS: an attacker's authorization server
        // simply omits the parameter, and a client that treats `iss` as
        // optional trades the code in at the honest server.
        let started = begin(&setting()).expect("built");
        let redirect = parse_redirect(&format!(
            "http://127.0.0.1/callback?code=abc&state={}",
            started.state
        ));
        assert!(matches!(
            check_redirect(&setting(), &started, &redirect),
            Err(MCPError::IssuerMismatch)
        ));
    }

    #[test]
    fn a_response_from_another_issuer_is_not_redeemed() {
        let started = begin(&setting()).expect("built");
        let redirect = parse_redirect(&format!(
            "http://127.0.0.1/callback?code=abc&state={}&iss=https%3A%2F%2Fevil.example.com",
            started.state
        ));
        assert!(matches!(
            check_redirect(&setting(), &started, &redirect),
            Err(MCPError::IssuerMismatch)
        ));
    }

    #[test]
    fn a_matching_response_yields_the_code() {
        let started = begin(&setting()).expect("built");
        let redirect = parse_redirect(&format!(
            "http://127.0.0.1/callback?code=the-code&state={}&iss=https%3A%2F%2Fauth.example.com",
            started.state
        ));
        assert_eq!(
            check_redirect(&setting(), &started, &redirect).expect("valid"),
            "the-code"
        );
    }

    #[test]
    fn a_replayed_state_is_refused() {
        let started = begin(&setting()).expect("built");
        let redirect = parse_redirect(
            "http://127.0.0.1/callback?code=abc&state=someone-elses&iss=https%3A%2F%2Fauth.example.com",
        );
        assert!(matches!(
            check_redirect(&setting(), &started, &redirect),
            Err(MCPError::StateMismatch)
        ));
        // And the error response is reported as itself.
        let denied = parse_redirect("http://127.0.0.1/callback?error=access_denied");
        assert!(matches!(
            check_redirect(&setting(), &started, &denied),
            Err(MCPError::Server(_))
        ));
    }

    #[test]
    fn a_token_is_bound_to_its_issuer_and_never_travels() {
        let token = token_from_response(
            &setting(),
            &json!({"access_token": "at-1", "refresh_token": "rt-1", "expires_in": 3600}),
            1_000,
        )
        .expect("token");
        assert_eq!(token.issuer, "https://auth.example.com");
        assert_eq!(token.expires_at, Some(4_600));
        assert!(token.is_for("https://auth.example.com/"));
        assert!(
            !token.is_for("https://evil.example.com"),
            "a token minted by A is never sent to B"
        );

        let mut store = TokenStore::default();
        store.tokens.insert("home".into(), token);
        assert!(store.usable("home", "https://auth.example.com", 1_000).is_some());
        assert!(
            store.usable("home", "https://evil.example.com", 1_000).is_none(),
            "the store enforces the binding too, not just the token"
        );
        assert!(
            store.usable("home", "https://auth.example.com", 4_600).is_none(),
            "an expired token is not usable"
        );
    }

    #[test]
    fn endpoints_must_belong_to_the_issuer() {
        let mut bad = setting();
        bad.token_endpoint = "https://evil.example.com/token".into();
        assert!(matches!(bad.validate(), Err(MCPError::IssuerMismatch)));

        let mut plain = setting();
        plain.authorization_endpoint = "http://auth.example.com/authorize".into();
        assert!(matches!(plain.validate(), Err(MCPError::InvalidAddress(_))));
    }

    #[test]
    fn the_token_request_carries_the_verifier_and_nothing_extra() {
        let started = begin(&setting()).expect("built");
        let body = token_request_body(&setting(), &started, "the code&injected=1");
        assert!(body.contains("grant_type=authorization_code"));
        assert!(body.contains(&format!("code_verifier={}", escape(&started.verifier))));
        // The `&` in the code cannot add a parameter.
        assert!(!body.contains("injected=1"));
        assert!(body.contains("%26injected%3D1"));
    }

    #[test]
    fn two_tokens_are_not_the_same_token() {
        assert_ne!(random_token(), random_token());
        assert!(random_token().len() >= 40);
    }
}
