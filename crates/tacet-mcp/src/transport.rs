//! The socket seam — the ONE place `ureq` is touched, and the reason the
//! protocol can be tested without a network.
//!
//! WHY IT EXISTS: the client used to build `ureq` requests inline, so the only
//! way to test the wire was to open a socket. The 2026-07-28 spec's guarantees
//! are ABOUT THE WIRE ("this header is present", "this header is never sent",
//! "this call is not retried"), and a guarantee that can only be checked by
//! hand is not a guarantee. `Transport` makes the bytes an ordinary value: the
//! test transport (`replay`) records what went out and hands back what a
//! recorded server said, and CI never opens a socket.
//!
//! The network monopoly is UNCHANGED — `ureq` still appears in exactly two
//! manifests, and inside this crate in exactly one file: this one.

use crate::error::{MCPError, MCPResult};
use std::io::Read;
use std::time::Duration;

/// §5.7 — the far side may be doing long work, like a build.
pub const TIMEOUT_S: u64 = 120;

/// One outgoing request, as bytes and header pairs — nothing `ureq`-shaped, so
/// a test can assert on it directly.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The value of a header, case-insensitively — HTTP header names are not
    /// case sensitive and a test must not depend on how we happened to spell
    /// one.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// One reply. The body is a READER, not a buffer: the SSE branch must be able
/// to return the moment its own event arrives, or a server that holds the
/// stream open for progress events would hang every long call until the
/// timeout.
pub struct Reply {
    pub status: u16,
    pub is_sse: bool,
    /// Legacy only. The 2026-07-28 path neither sends nor reads it; it is kept
    /// so the frozen path keeps working during the deprecation window.
    pub session_id: Option<String>,
    pub body: Box<dyn Read>,
}

pub trait Transport: Send + Sync {
    fn post(&self, request: &Request) -> MCPResult<Reply>;
}

/// The real one. The only socket owner in this crate.
pub struct HttpTransport {
    agent: ureq::Agent,
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(TIMEOUT_S)))
            // We want to read the status code OURSELVES: 401 and 500 tell the
            // user different sentences, and ureq's single "status error" erases
            // that distinction. The 2026-07-28 path needs the code for a second
            // reason: a version rejection is recognised by it.
            .http_status_as_error(false)
            // The name that lands in a third party's log: the product name, not
            // the crate name.
            .user_agent("tacet/1.0")
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Transport for HttpTransport {
    fn post(&self, request: &Request) -> MCPResult<Reply> {
        let mut builder = self.agent.post(&request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let response = builder.send(&request.body[..]).map_err(convert_error)?;
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let session_id = header("mcp-session-id").filter(|s| !s.is_empty());
        let is_sse = header("content-type")
            .is_some_and(|t| t.to_ascii_lowercase().contains("text/event-stream"));
        Ok(Reply {
            status,
            is_sse,
            session_id,
            body: Box::new(response.into_body().into_reader()),
        })
    }
}

/// `ureq::Error` -> an `MCPError` that separates in plain language (§3.1).
pub(crate) fn convert_error(error: ureq::Error) -> MCPError {
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

// ---------------------------------------------------------------------------
// The test transport
// ---------------------------------------------------------------------------

/// Record/replay — the transport the protocol tests run on (§9).
///
/// It is PUBLIC on purpose: the guarantees it measures are the product's
/// promises, and the integration tests that assert them live outside this
/// module. It opens no socket and has no `ureq` in sight, so nothing about
/// shipping it widens the network surface.
pub mod replay {
    use super::{Reply, Request, Transport};
    use crate::error::{MCPError, MCPResult};
    use std::sync::Mutex;

    /// What a recorded server said. `status` and `body` are the whole of it —
    /// a fixture is a status line and a JSON body, nothing more.
    #[derive(Debug, Clone)]
    pub struct Canned {
        pub status: u16,
        pub body: String,
    }

    impl Canned {
        pub fn ok(body: impl Into<String>) -> Self {
            Self {
                status: 200,
                body: body.into(),
            }
        }

        pub fn status(status: u16, body: impl Into<String>) -> Self {
            Self {
                status,
                body: body.into(),
            }
        }
    }

    /// Replays canned replies IN ORDER and keeps every request that went out.
    ///
    /// In order, not by method: the ORDER is itself under test (was
    /// `initialize` sent? was the failed call retried?). A map keyed by method
    /// would hide exactly the mistakes these tests exist to catch.
    pub struct ReplayTransport {
        replies: Mutex<std::collections::VecDeque<Canned>>,
        sent: Mutex<Vec<Request>>,
    }

    impl ReplayTransport {
        pub fn new(replies: Vec<Canned>) -> Self {
            Self {
                replies: Mutex::new(replies.into()),
                sent: Mutex::new(Vec::new()),
            }
        }

        /// Every request that went out, in order.
        pub fn sent(&self) -> Vec<Request> {
            self.sent.lock().expect("replay lock").clone()
        }

        pub fn calls(&self) -> usize {
            self.sent.lock().expect("replay lock").len()
        }

        /// The JSON-RPC method of each request, in order — the shorthand most
        /// assertions want.
        pub fn methods(&self) -> Vec<String> {
            self.sent()
                .iter()
                .filter_map(|r| {
                    serde_json::from_slice::<serde_json::Value>(&r.body)
                        .ok()?
                        .get("method")?
                        .as_str()
                        .map(str::to_string)
                })
                .collect()
        }
    }

    fn request_id(body: &[u8]) -> Option<u64> {
        serde_json::from_slice::<serde_json::Value>(body)
            .ok()?
            .get("id")?
            .as_u64()
    }

    impl Transport for ReplayTransport {
        fn post(&self, request: &Request) -> MCPResult<Reply> {
            self.sent.lock().expect("replay lock").push(request.clone());
            // A fixture list that ran out means the client made a call the test
            // did not expect. Reporting that as "unreachable" is honest: from
            // the client's side an absent server IS unreachable, and the test
            // reads the call count to see what really happened.
            let Some(canned) = self.replies.lock().expect("replay lock").pop_front() else {
                return Err(MCPError::Unreachable);
            };
            // A REAL SERVER ECHOES THE ID it was given. A recorded fixture
            // cannot know which counter value the client will be at when it
            // gets replayed (a discovery call ahead of it shifts everything by
            // one), so the id is stamped here rather than frozen into the
            // file. Id MATCHING is still a real guarantee — it is measured in
            // `jsonrpc`'s own tests, where the id is chosen by the test.
            let body = match serde_json::from_str::<serde_json::Value>(&canned.body) {
                Ok(mut parsed) if parsed.get("id").is_some() => {
                    if let Some(id) = request_id(&request.body) {
                        parsed["id"] = serde_json::json!(id);
                    }
                    parsed.to_string()
                }
                _ => canned.body,
            };
            Ok(Reply {
                status: canned.status,
                is_sse: false,
                session_id: None,
                body: Box::new(std::io::Cursor::new(body.into_bytes())),
            })
        }
    }
}
