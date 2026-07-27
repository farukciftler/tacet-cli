//! The SSE half of Streamable HTTP.
//!
//! WHY A SEPARATE MODULE: the parsing is entirely independent of the network —
//! its input is a `BufRead`. That way badly formed streams (a half event, a
//! heartbeat in between, a batch array, an id that is not ours) can be tested
//! WITHOUT GOING ONLINE. The only test that goes online is `#[ignore]`; the real
//! behaviour is proven here.
//!
//! The format (the subset of WHATWG EventSource we need): `data:` lines
//! accumulate, a BLANK LINE completes the event. A line starting with `:` is a
//! comment/heartbeat. The `event:`/`id:`/`retry:` fields carry no meaning in
//! MCP's JSON-RPC transport and are skipped.

use crate::error::{MCPError, MCPResult};
use serde_json::Value;
use std::io::BufRead;

/// Reads the stream and returns the JSON-RPC response belonging to `id`.
///
/// WHY "READ UNTIL FOUND": the server may send progress/log events before our
/// response (the output of a long build, for instance). If we took the first
/// event and returned, every long-running call would say "malformed".
///
/// There is no cap because the `time_limited` layer above (the client) cuts the
/// duration; keeping a second counter here would create two separate truths
/// about "how long do we wait".
pub fn find_event<R: BufRead>(reader: R, id: u64) -> MCPResult<Value> {
    let mut buffer: Vec<String> = Vec::new();

    for line in reader.lines() {
        let Ok(line) = line else {
            // A connection that broke mid-stream: we may be holding a half
            // event, so we look at it first — the server may not have closed
            // the last event with a blank line.
            break;
        };
        // Some servers send `\r\n`; `lines()` only splits on `\n`.
        let line = line.trim_end_matches('\r');

        if line.is_empty() {
            if let Some(event) = resolve_event(&mut buffer, id) {
                return Ok(event);
            }
            continue;
        }
        if line.starts_with(':') {
            continue; // comment / heartbeat
        }
        let Some(part) = line.strip_prefix("data:") else {
            continue; // event:/id:/retry: do not concern us
        };
        // The SINGLE leading space of a field value is part of the format, not
        // of the data.
        buffer.push(part.strip_prefix(' ').unwrap_or(part).to_string());
    }

    // The stream closed: the last event may not have been closed with a blank
    // line.
    resolve_event(&mut buffer, id).ok_or(MCPError::Malformed)
}

/// Treats the accumulated `data:` lines as a single JSON body and looks for our
/// id. An event that is not ours is SILENTLY dropped (the buffer is cleared)
/// and reading continues.
fn resolve_event(buffer: &mut Vec<String>, id: u64) -> Option<Value> {
    if buffer.is_empty() {
        return None;
    }
    let body = buffer.join("\n");
    buffer.clear();
    let parsed: Value = serde_json::from_str(&body).ok()?;
    crate::jsonrpc::select_response(&parsed, id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn read(text: &str, id: u64) -> MCPResult<Value> {
        find_event(Cursor::new(text.to_string()), id)
    }

    #[test]
    fn a_single_event_is_read() {
        let stream = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
        let v = read(stream, 1).expect("event");
        assert_eq!(v["result"]["ok"], serde_json::json!(true));
    }

    #[test]
    fn heartbeats_and_irrelevant_fields_are_skipped() {
        let stream = ": ping\nevent: message\nid: 7\nretry: 100\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":1}\n\n";
        assert_eq!(
            read(stream, 3).expect("event")["result"],
            serde_json::json!(1)
        );
    }

    #[test]
    fn events_that_are_not_ours_are_skipped_and_reading_continues() {
        // The server sends a progress notification first, then another id, and
        // ours last. Returning on the first event would make long jobs always
        // come back "malformed".
        let stream = "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":99,\"result\":\"somebody elses\"}\n\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":5,\"result\":\"mine\"}\n\n";
        assert_eq!(
            read(stream, 5).expect("event")["result"],
            serde_json::json!("mine")
        );
    }

    #[test]
    fn multi_line_data_is_joined() {
        let stream = "data: {\"jsonrpc\":\"2.0\",\n\
                    data: \"id\":2,\n\
                    data: \"result\":{\"a\":1}}\n\n";
        assert_eq!(
            read(stream, 2).expect("event")["result"]["a"],
            serde_json::json!(1)
        );
    }

    #[test]
    fn a_last_event_not_closed_by_a_blank_line_is_still_read() {
        let stream = "data: {\"jsonrpc\":\"2.0\",\"id\":4,\"result\":\"last\"}";
        assert_eq!(
            read(stream, 4).expect("event")["result"],
            serde_json::json!("last")
        );
    }

    #[test]
    fn a_server_sending_crlf_works() {
        let stream = "data: {\"jsonrpc\":\"2.0\",\"id\":8,\"result\":\"crlf\"}\r\n\r\n";
        assert_eq!(
            read(stream, 8).expect("event")["result"],
            serde_json::json!("crlf")
        );
    }

    #[test]
    fn our_id_is_picked_out_of_a_batch_array() {
        let stream = "data: [{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"a\"},\
                    {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":\"b\"}]\n\n";
        assert_eq!(
            read(stream, 2).expect("event")["result"],
            serde_json::json!("b")
        );
    }

    #[test]
    fn if_our_id_never_arrives_it_is_malformed() {
        let stream = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"a\"}\n\n";
        assert_eq!(read(stream, 42).unwrap_err(), MCPError::Malformed);
    }

    #[test]
    fn an_empty_stream_is_malformed() {
        assert_eq!(read("", 1).unwrap_err(), MCPError::Malformed);
    }

    #[test]
    fn a_broken_json_stream_does_not_panic() {
        let stream =
            "data: {this is not json\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":9}\n\n";
        assert_eq!(
            read(stream, 1).expect("event")["result"],
            serde_json::json!(9)
        );
    }
}
