//! LEGACY MCP PATH — frozen. The 2026-07-28 revision gives this a twelve-month
//! offramp; delete this module no earlier than 2027-07-28, and no later than
//! the first release after it.
//!
//! It gets SECURITY FIXES ONLY. No feature added to the current path is added
//! here, and no refactor that is not needed to keep it compiling. It exists for
//! one reason: most public servers will still speak 2025-06-18 for months, and
//! a client that cannot talk to them is a client nobody can use today.
//!
//! What the old revision needed and the new one does not:
//!
//! - an `initialize` request and an `initialized` notification, without which
//!   strict servers reject `tools/list`,
//! - an `Mcp-Session-Id` handed out in a response header and carried on every
//!   later request,
//! - a negotiated protocol version echoed back in a header.
//!
//! Deprecated capabilities (`roots`, `sampling`, `logging`, HTTP+SSE push) were
//! never implemented here and never will be. `sampling` is refused in both
//! revisions — see `MCPClient::handle_sampling`.

use crate::client::{Failure, MCPClient, Revision};
use serde_json::json;

/// The revision this frozen path speaks.
pub const LEGACY_PROTOCOL_VERSION: &str = "2025-06-18";

/// The handshake. Done once; later calls do not touch it.
pub(crate) fn handshake(client: &MCPClient) -> Result<(), Failure> {
    if client.state.lock().expect("mcp state lock").handshaken {
        return Ok(());
    }
    let result = client.send_read(
        Revision::Legacy,
        "initialize",
        None,
        json!({
            "protocolVersion": LEGACY_PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "clientInfo": { "name": "tacet", "version": env!("CARGO_PKG_VERSION") },
        }),
    )?;
    {
        let mut state = client.state.lock().expect("mcp state lock");
        if let Some(version) = result
            .get("protocolVersion")
            .and_then(serde_json::Value::as_str)
            .filter(|v| !v.is_empty())
        {
            state.agreed_version = Some(version.to_string());
        }
        state.handshaken = true;
    }
    // The second half of the handshake: a notification that expects no
    // response. Not a separate "method" but a part of `initialize` — strict
    // servers reject `tools/list` if it is skipped. Its failure is swallowed
    // because some servers answer a notification with something other than 202
    // and that must not stop us.
    let _ = notify(client, "notifications/initialized");
    Ok(())
}

fn notify(client: &MCPClient, method: &str) -> Result<(), Failure> {
    let mut headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        (
            "Accept".to_string(),
            "application/json, text/event-stream".to_string(),
        ),
    ];
    add_headers(client, &mut headers);
    if let Some(key) = client.key.as_deref().filter(|k| !k.is_empty()) {
        headers.push(("Authorization".into(), format!("Bearer {key}")));
    }
    client.transport.post(&crate::transport::Request {
        url: client.url.clone(),
        headers,
        body: crate::jsonrpc::notification_body(method),
    })?;
    Ok(())
}

/// The two headers only the old revision uses.
pub(crate) fn add_headers(client: &MCPClient, headers: &mut Vec<(String, String)>) {
    let state = client.state.lock().expect("mcp state lock");
    if state.handshaken {
        let version = state
            .agreed_version
            .as_deref()
            .unwrap_or(LEGACY_PROTOCOL_VERSION);
        headers.push(("MCP-Protocol-Version".into(), version.to_string()));
    }
    if let Some(session) = &state.session_id {
        headers.push(("Mcp-Session-Id".into(), session.clone()));
    }
}

/// The session id arrives during the handshake and is carried on every later
/// request. The current path never calls this.
pub(crate) fn remember_session(client: &MCPClient, session_id: Option<String>) {
    if let Some(session) = session_id.filter(|s| !s.is_empty()) {
        client.state.lock().expect("mcp state lock").session_id = Some(session);
    }
}
