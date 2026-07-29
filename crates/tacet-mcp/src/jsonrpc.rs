//! JSON-RPC 2.0 framing — PURE, independent of the network.
//!
//! WHY SEPARATE: building the request and picking the response is the
//! protocol's only fragile spot (id matching, batch arrays, `error` vs
//! `result`). Split away from the socket, all of it can be tested without going
//! online; the `client` module is left with nothing but "carry bytes".

use crate::error::{MCPError, MCPResult, safe_for_screen};
use serde_json::{Value, json};

/// The body of a request that expects a response.
pub fn request_body(id: u64, method: &str, params: Value) -> Vec<u8> {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
    .to_string()
    .into_bytes()
}

/// The body of a notification that expects NO response (NO `id` — that is the
/// difference in JSON-RPC).
///
/// `notifications/initialized` is its only user: it is the second half of the
/// MCP handshake, and skipping it makes strict servers reject `tools/list`.
/// A JSON-RPC error answer. Written for ONE purpose: refusing `sampling`
/// (see `MCPClient::handle_sampling`). A refusal has to be a real protocol
/// answer, not silence — silence reads as a broken client and gets retried.
pub fn error_body(id: u64, code: i64, message: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }))
    .unwrap_or_default()
}

pub fn notification_body(method: &str) -> Vec<u8> {
    json!({ "jsonrpc": "2.0", "method": method })
        .to_string()
        .into_bytes()
}

/// Picks OUR id's response out of the incoming body.
///
/// The server may return a single object or a batch array. In the array case
/// PICKING ours is mandatory: blindly taking the first would attribute another
/// call's result to our call.
pub fn select_response(body: &Value, id: u64) -> Option<&Value> {
    match body {
        Value::Array(items) => items.iter().find(|i| id_equals(i, id)),
        object if id_equals(object, id) => Some(object),
        _ => None,
    }
}

/// A JSON-RPC id may be a number or a STRING (the spec allows both and real
/// servers send both). We always send a number, but rejecting a server that
/// wraps the reply's id in a string would be needless strictness.
fn id_equals(object: &Value, id: u64) -> bool {
    match object.get("id") {
        Some(Value::Number(n)) => n.as_u64() == Some(id),
        Some(Value::String(s)) => s.parse::<u64>().ok() == Some(id),
        _ => false,
    }
}

/// Extracts the `result`; if there is an `error` it becomes a server error.
pub fn extract_result(response: &Value) -> MCPResult<Value> {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        // THE BOUNDARY WHERE THE FAR SIDE'S BYTES BECOME A STRING SHOWN TO THE
        // USER. This message goes into the chip and from there onto the tty
        // unchanged, so the filter belongs HERE — one place, no call site left
        // to forget it. See `safe_for_screen` for what it stops.
        return Err(MCPError::Server(safe_for_screen(message)));
    }
    response.get("result").cloned().ok_or(MCPError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_body_carries_the_jsonrpc_fields() {
        let bytes = request_body(7, "tools/list", json!({"cursor": "a"}));
        let v: Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(v["jsonrpc"], json!("2.0"));
        assert_eq!(v["id"], json!(7));
        assert_eq!(v["method"], json!("tools/list"));
        assert_eq!(v["params"]["cursor"], json!("a"));
    }

    #[test]
    fn a_notification_has_no_id() {
        let v: Value =
            serde_json::from_slice(&notification_body("notifications/initialized")).expect("json");
        assert!(v.get("id").is_none(), "a notification must have no id: {v}");
        assert_eq!(v["method"], json!("notifications/initialized"));
    }

    #[test]
    fn the_right_id_is_picked_out_of_a_batch() {
        let body = json!([
            {"jsonrpc":"2.0","id":1,"result":"a"},
            {"jsonrpc":"2.0","id":2,"result":"b"},
        ]);
        assert_eq!(
            select_response(&body, 2).expect("response")["result"],
            json!("b")
        );
        assert!(select_response(&body, 3).is_none());
    }

    #[test]
    fn a_string_id_matches_too() {
        let body = json!({"jsonrpc":"2.0","id":"12","result":1});
        assert!(select_response(&body, 12).is_some());
    }

    #[test]
    fn an_error_body_becomes_a_server_error() {
        let response = json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"missing"}});
        assert_eq!(
            extract_result(&response).unwrap_err(),
            MCPError::Server("missing".into())
        );
    }

    /// A JSON-RPC `error.message` is text the FAR SIDE wrote. It reaches the
    /// user's terminal, so it is filtered at the point where it becomes a
    /// string — not at the place that happens to print it today.
    #[test]
    fn a_hostile_error_message_is_cleaned_where_it_enters() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -1, "message": "\u{1b}[2J\u{1b}[1mall files deleted\u{1b}[0m\n  ⏺ fake chip"},
        });
        let MCPError::Server(message) = extract_result(&response).unwrap_err() else {
            panic!("a server error was expected");
        };
        assert!(!message.contains('\u{1b}'), "{message:?}");
        assert!(!message.contains('\n'), "{message:?}");
        assert!(message.contains("all files deleted"), "{message:?}");

        // An unbounded message must not be able to scroll the chips away.
        let long = json!({"id":1,"error":{"message":"x".repeat(5000)}});
        let MCPError::Server(message) = extract_result(&long).unwrap_err() else {
            panic!("a server error was expected");
        };
        assert!(message.chars().count() <= crate::error::SCREEN_LIMIT + 1);
    }

    #[test]
    fn neither_result_nor_error_is_malformed() {
        assert_eq!(
            extract_result(&json!({"jsonrpc":"2.0","id":1})).unwrap_err(),
            MCPError::Malformed
        );
    }
}
