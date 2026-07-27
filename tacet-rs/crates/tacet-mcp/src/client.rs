//! The client that talks to a single MCP server — the ONLY socket owner in this
//! crate.
//!
//! Transport: **Streamable HTTP**. JSON-RPC 2.0 bodies go out over HTTP POST;
//! the response may be `application/json` OR `text/event-stream` (SSE). We
//! accept both, and the server chooses which one arrives.
//!
//! ## Why blocking
//!
//! There is NO runtime in this dependency tree: `tacet_engine::wait` drives the
//! futures on a single thread by busy-waiting. Because `MCPClient` is blocking,
//! the call site (`tacet-tools::mcp`) can call it directly from an `async` body
//! and no tokio is imposed on the architecture. The cost of that must be said
//! openly: while an MCP call is in flight, that thread does no other work.
//!
//! ## Why `&self` + `Mutex`
//!
//! `Tool::run` takes `&self` and the catalog holds `Arc<dyn Tool>`; changing
//! state such as the session id has to live behind an internal lock.

use crate::error::{MCPError, MCPResult};
use crate::jsonrpc;
use crate::sse;
use serde_json::{Value, json};
use std::io::BufReader;
use std::sync::Mutex;
use std::time::Duration;

/// The MCP version this client speaks. If the server proposes another one, the
/// server's is followed (the MCP version negotiation).
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// §5.7 — the far side may be doing long work, like a build.
pub const TIMEOUT_S: u64 = 120;

/// The cap on the `tools/list` pagination loop: a misbehaving server must not
/// lock us in forever by returning the same cursor over and over.
const MAX_PAGES: usize = 20;

/// The cap on the number of tools. Only 6-8 tools fit in the 4096 window
/// anyway; loading thousands of tools into memory helps nobody.
const MAX_TOOLS: usize = 200;

/// A single tool definition coming from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    /// The RAW description — it may run 100-500 tokens. It DOES NOT ENTER the
    /// context IN THIS FORM; it is shortened by `bridge::truncate_description`
    /// (§5.3).
    pub description: String,
    /// `inputSchema` (JSON Schema). The bridge converts this into an `ArgSchema`.
    pub schema: Value,
}

/// Server state: the things carried across after the handshake.
#[derive(Default)]
struct State {
    session_id: Option<String>,
    agreed_version: Option<String>,
    handshaken: bool,
    counter: u64,
}

pub struct MCPClient {
    url: String,
    /// The bearer key. In memory only; NEVER written to a log or a chip text.
    key: Option<String>,
    agent: ureq::Agent,
    state: Mutex<State>,
}

impl MCPClient {
    /// If `url` is not accepted it stops here — nothing goes online at all.
    pub fn new(url: impl Into<String>, key: Option<String>) -> MCPResult<Self> {
        let url = url.into();
        validate_url(&url)?;
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(TIMEOUT_S)))
            // We want to read the status code OURSELVES: 401 and 500 tell the
            // user different sentences, and ureq's single "status error" erases
            // that distinction.
            .http_status_as_error(false)
            // The name that lands in a third party's log: the product name, not
            // the crate name. The crate stays `tacet-mcp` (internal identity),
            // the network identity is `tacet`.
            .user_agent("tacet/1.0")
            .build();
        Ok(Self {
            url,
            key,
            agent: config.into(),
            state: Mutex::new(State::default()),
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    // --- The three methods (v1 scope) ---

    /// The handshake. Done once; later calls do not touch it.
    pub fn handshake(&self) -> MCPResult<()> {
        if self.state.lock().expect("mcp state lock").handshaken {
            return Ok(());
        }
        let result = self.call(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "clientInfo": { "name": "tacet", "version": "1.0" },
            }),
        )?;
        {
            let mut s = self.state.lock().expect("mcp state lock");
            if let Some(v) = result
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
            {
                s.agreed_version = Some(v.to_string());
            }
            s.handshaken = true;
        }
        // The second half of the handshake: a notification that expects no
        // response. Not a separate "method" but a part of `initialize` — strict
        // servers reject `tools/list` if it is skipped. We swallow its failure
        // because some servers return something other than 202 to a
        // notification and that must not stop us.
        let _ = self.notify("notifications/initialized");
        Ok(())
    }

    /// The server's tools. Pagination (`nextCursor`) is followed to the end.
    pub fn tools(&self) -> MCPResult<Vec<ToolSpec>> {
        self.handshake()?;
        let mut total: Vec<ToolSpec> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page = 0usize;

        loop {
            page += 1;
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let result = self.call("tools/list", params)?;
            total.extend(extract_specs(&result));

            let next = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            // A server handing back the same cursor must not keep us in a loop.
            cursor = if next == cursor { None } else { next };
            if cursor.is_none() || page >= MAX_PAGES || total.len() >= MAX_TOOLS {
                break;
            }
        }

        total.truncate(MAX_TOOLS);
        Ok(total)
    }

    /// Calls the remote tool and reduces the content to plain text.
    ///
    /// IMPORTANT: the approval gate is passed BEFORE this, in `ToolExecutor`.
    /// Everything that gets here is what the user saw and approved.
    ///
    /// The returned `is_error` is the SERVER's own tool error (`isError`): not a
    /// transport failure but the tool's normal outcome, and it is told to the
    /// model.
    pub fn call_tool(&self, name: &str, arguments: &Value) -> MCPResult<(String, bool)> {
        self.handshake()?;
        let result = self.call(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )?;
        Ok(flatten_content(&result))
    }

    // --- Transport ---

    fn next_id(&self) -> u64 {
        let mut s = self.state.lock().expect("mcp state lock");
        s.counter += 1;
        s.counter
    }

    fn build_request(&self) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        let s = self.state.lock().expect("mcp state lock");
        let mut request = self
            .agent
            .post(&self.url)
            .header("Content-Type", "application/json")
            // We accept both formats; the server picks which one to use.
            .header("Accept", "application/json, text/event-stream");
        if s.handshaken {
            let version = s.agreed_version.as_deref().unwrap_or(PROTOCOL_VERSION);
            request = request.header("MCP-Protocol-Version", version);
        }
        if let Some(session) = &s.session_id {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(key) = self.key.as_deref().filter(|k| !k.is_empty()) {
            request = request.header("Authorization", &format!("Bearer {key}"));
        }
        request
    }

    /// A notification that expects no response.
    fn notify(&self, method: &str) -> MCPResult<()> {
        let body = jsonrpc::notification_body(method);
        self.build_request()
            .send(&body[..])
            .map_err(convert_error)?;
        Ok(())
    }

    /// A single JSON-RPC call — returns the `result` object.
    fn call(&self, method: &str, params: Value) -> MCPResult<Value> {
        let id = self.next_id();
        let body = jsonrpc::request_body(id, method, params);
        let mut response = self
            .build_request()
            .send(&body[..])
            .map_err(convert_error)?;

        // The session id arrives during the handshake and is carried on every
        // later request.
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        {
            self.state.lock().expect("mcp state lock").session_id = Some(session);
        }

        let code = response.status().as_u16();
        match code {
            200..=299 => {}
            401 | 403 => return Err(MCPError::Authorization),
            _ => return Err(MCPError::Server(format!("HTTP {code}"))),
        }

        let is_sse = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.to_ascii_lowercase().contains("text/event-stream"));

        let reader = BufReader::new(response.body_mut().as_reader());
        let body = if is_sse {
            sse::find_event(reader, id)?
        } else {
            let parsed: Value = serde_json::from_reader(reader).map_err(|_| MCPError::Malformed)?;
            jsonrpc::select_response(&parsed, id)
                .cloned()
                .ok_or(MCPError::Malformed)?
        };

        jsonrpc::extract_result(&body)
    }
}

/// Extracts the specs out of a `tools/list` result. PURE — its test needs no
/// network.
///
/// An item without a name is SILENTLY skipped: we cannot show a nameless tool
/// in the catalog, it has no signature the model could call.
pub fn extract_specs(result: &Value) -> Vec<ToolSpec> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| {
                    let name = i.get("name").and_then(Value::as_str)?;
                    if name.is_empty() {
                        return None;
                    }
                    Some(ToolSpec {
                        name: name.to_string(),
                        description: i
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        schema: i.get("inputSchema").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reduces a `tools/call` result's `content` array to plain text.
///
/// Non-text content (image/audio) is not carried in v1 but IT IS NOT SWALLOWED
/// SILENTLY: a marker is put in its place, otherwise the model thinks "the tool
/// returned nothing" and retries the job.
pub fn flatten_content(result: &Value) -> (String, bool) {
    let parts: Vec<String> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| {
                    if let Some(t) = i.get("text").and_then(Value::as_str) {
                        return Some(t.to_string());
                    }
                    match i.get("type").and_then(Value::as_str) {
                        Some(kind) if kind != "text" => Some(format!("[{kind} content omitted]")),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    (parts.join("\n"), is_error)
}

/// The address acceptance rule (§3.1): `https://` everywhere, plain `http://`
/// ONLY on local network addresses.
///
/// WHY HERE: the rule must live in the network layer itself. If it lives in the
/// settings screen (or in the head of the user hand-writing the config file), a
/// second call path skips it and the token goes out in plain text.
pub fn validate_url(url: &str) -> MCPResult<()> {
    let error = || MCPError::InvalidAddress(url.to_string());
    if let Some(rest) = url.strip_prefix("https://") {
        return if rest.is_empty() {
            Err(error())
        } else {
            Ok(())
        };
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return Err(error());
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or_else(|| rest.split(['/', '?', '#']).next().unwrap_or_default())
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();

    if is_local(&host) {
        Ok(())
    } else {
        Err(error())
    }
}

/// Is it a local network address. The list is kept narrow: every address we
/// cannot be sure about is forced to ask for `https`.
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
        [10, ..] => true,
        [192, 168, ..] => true,
        [172, second, ..] if (16..=31).contains(second) => true,
        [127, ..] => true,
        _ => false,
    }
}

/// `ureq::Error` -> an `MCPError` that separates in plain language (§3.1).
fn convert_error(error: ureq::Error) -> MCPError {
    match error {
        ureq::Error::Timeout(_) => MCPError::Timeout,
        ureq::Error::Tls(_) | ureq::Error::Pem(_) => MCPError::Tls,
        ureq::Error::StatusCode(401) | ureq::Error::StatusCode(403) => MCPError::Authorization,
        ureq::Error::StatusCode(code) => MCPError::Server(format!("HTTP {code}")),
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => MCPError::Timeout,
        // Everything else is the same thing in the user's eyes: the server
        // could not be reached. Showing the raw `ureq` text helps nobody.
        _ => MCPError::Unreachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn https_is_always_accepted() {
        assert!(validate_url("https://example.com/mcp").is_ok());
    }

    #[test]
    fn plain_http_only_on_the_local_network() {
        assert!(validate_url("http://localhost:8080/mcp").is_ok());
        assert!(validate_url("http://127.0.0.1:3000").is_ok());
        assert!(validate_url("http://192.168.1.20/mcp").is_ok());
        assert!(validate_url("http://10.0.0.5/mcp").is_ok());
        assert!(validate_url("http://172.20.0.3/mcp").is_ok());
        assert!(validate_url("http://server.local/mcp").is_ok());

        assert!(validate_url("http://example.com/mcp").is_err());
        assert!(
            validate_url("http://172.32.0.1/mcp").is_err(),
            "172.32 is outside the private range"
        );
        assert!(validate_url("http://8.8.8.8/mcp").is_err());
    }

    #[test]
    fn non_http_schemes_are_rejected() {
        for u in [
            "ws://localhost/mcp",
            "file:///etc/passwd",
            "example.com",
            "",
        ] {
            assert!(
                validate_url(u).is_err(),
                "{u} should not have been accepted"
            );
        }
    }

    #[test]
    fn an_invalid_address_does_not_let_the_client_be_built() {
        // The rule lives in the network layer: with a bad address not even a
        // client OBJECT comes into existence, so there is no path left to
        // accidentally send a request to that address.
        assert!(MCPClient::new("http://example.com/mcp", None).is_err());
    }

    #[test]
    fn specs_are_extracted_and_a_nameless_item_is_skipped() {
        let result = json!({"tools": [
            {"name": "run", "description": "command", "inputSchema": {"type": "object"}},
            {"description": "nameless"},
            {"name": ""},
            {"name": "plain"},
        ]});
        let specs = extract_specs(&result);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "run");
        assert_eq!(specs[1].name, "plain");
        assert_eq!(
            specs[1].schema,
            Value::Null,
            "a schema-less tool arrives with a null schema"
        );
    }

    #[test]
    fn the_content_is_flattened() {
        let result = json!({"content": [
            {"type": "text", "text": "first"},
            {"type": "text", "text": "second"},
        ]});
        assert_eq!(flatten_content(&result), ("first\nsecond".into(), false));
    }

    #[test]
    fn non_text_content_is_not_swallowed_silently() {
        let result = json!({"content": [{"type": "image", "data": "..."}]});
        let (text, is_error) = flatten_content(&result);
        assert_eq!(text, "[image content omitted]");
        assert!(!is_error);
    }

    #[test]
    fn the_servers_own_tool_error_is_marked() {
        let result = json!({"content": [{"type":"text","text":"exit 1"}], "isError": true});
        assert_eq!(flatten_content(&result), ("exit 1".into(), true));
    }

    /// GOES ONLINE — deliberately `#[ignore]`. Runs with
    /// `cargo test -p tacet-mcp -- --ignored` when TACET_MCP_TEST_URL is given.
    /// The variables are read through `tacet_core::env_var`: an empty value
    /// counts as "undefined" and the test is skipped silently.
    #[test]
    #[ignore = "goes online; needs TACET_MCP_TEST_URL"]
    fn handshake_with_a_real_server() {
        let Some(url) = tacet_core::env_var("TACET_MCP_TEST_URL") else {
            panic!("TACET_MCP_TEST_URL is not defined");
        };
        let url = url.to_string_lossy().into_owned();
        let key =
            tacet_core::env_var("TACET_MCP_TEST_KEY").map(|k| k.to_string_lossy().into_owned());
        let client = MCPClient::new(url, key).expect("client");
        client.handshake().expect("handshake");
        let tools = client.tools().expect("tools/list");
        println!("{} tools found", tools.len());
        for t in &tools {
            println!(
                "  {} — {}",
                t.name,
                crate::bridge::truncate_description(&t.description)
            );
        }
    }
}
