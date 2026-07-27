//! END TO END: a real socket, real HTTP, real JSON-RPC.
//!
//! WHY NOT `#[ignore]`: the rule is "a test that goes online should be
//! ignored" — this test DOES NOT GO ONLINE. The server is born inside the test
//! itself, on `127.0.0.1`, on a random port, and dies with the test; no DNS, no
//! internet, no external dependency.
//!
//! WHY IT EXISTS: unit tests prove the parsers, they do not prove the client
//! REALLY talks. "The build is green" proves nothing; this file proves that the
//! `initialize -> tools/list -> tools/call` flow works over a socket. Both
//! transport formats are measured: plain `application/json` and SSE.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tacet_mcp::bridge::convert_schema;
use tacet_mcp::client::MCPClient;

/// How the far side SEES US. The product name lands in a third party's log
/// through these two fields; looking at the constants in the code and saying "I
/// changed it" is not enough, what goes over the wire has to be measured.
#[derive(Default)]
struct Identity {
    user_agent: Option<String>,
    client_name: Option<String>,
}

/// The handle for the server thread: a stop flag + a join handle.
///
/// WHY THE FLAG EXISTS (a hanging-test finding): the old version wrote
/// `for _ in 0..4 { accept() }` and called `join()` at the end of the test. But
/// `ureq` uses a CONNECTION POOL — all four JSON-RPC requests go through ONE
/// keep-alive connection, so the second `accept()` never returns. The test
/// passed all its assertions and then hung FOREVER in `join()`; that is why
/// `cargo test --workspace` never finished. The right way: serve REQUEST BY
/// REQUEST over one connection and announce the shutdown explicitly.
struct Server {
    stop: Arc<AtomicBool>,
    job: thread::JoinHandle<()>,
}

impl Server {
    /// Stops listening and reaps the thread.
    fn shutdown(self) {
        self.stop.store(true, Ordering::Relaxed);
        self.job.join().ok();
    }
}

/// A fake MCP server. If `sse` is true it returns the responses as
/// `text/event-stream` — reflecting that real servers can do both.
fn start_server(sse: bool) -> (String, Server, Arc<Mutex<Identity>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("port");
    let port = listener.local_addr().expect("address").port();
    // Non-blocking accept: required in order to see the `stop` flag, otherwise
    // we would sleep inside `accept()` and never learn about the shutdown
    // request.
    listener
        .set_nonblocking(true)
        .expect("non-blocking listener");
    let (notifier, waiter) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_job = Arc::clone(&stop);
    let identity = Arc::new(Mutex::new(Identity::default()));
    let identity_job = Arc::clone(&identity);

    let job = thread::spawn(move || {
        notifier.send(()).ok();
        while !stop_job.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    // Block inside the connection: waiting between requests is
                    // normal.
                    stream.set_nonblocking(false).expect("blocking stream");
                    if serve_connection(stream, sse, &identity_job).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return,
            }
        }
    });

    waiter.recv().expect("the server started");
    (
        format!("http://127.0.0.1:{port}/mcp"),
        Server { stop, job },
        identity,
    )
}

/// Serves request by request over ONE connection until the far side closes it.
fn serve_connection(
    stream: TcpStream,
    sse: bool,
    observed: &Arc<Mutex<Identity>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    while handle_request(&mut reader, &mut writer, sse, observed)? {}
    Ok(())
}

/// Handles one request. `false` = the far side closed the connection.
fn handle_request(
    reader: &mut BufReader<TcpStream>,
    stream: &mut TcpStream,
    sse: bool,
    // THE NAME `id` IS NOT FREE HERE: the JSON-RPC id uses that name below.
    observed: &Arc<Mutex<Identity>>,
) -> std::io::Result<bool> {
    // Read the headers, learn the body length.
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(false);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            length = v.trim().parse().unwrap_or(0);
        }
        // A header's NAME is case-insensitive, its VALUE is not: the comparison
        // is done in lowercase but the raw value is what gets recorded.
        if lower.starts_with("user-agent:") {
            let value = trimmed["user-agent:".len()..].trim().to_string();
            observed.lock().expect("identity lock").user_agent = Some(value);
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let request: serde_json::Value =
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);

    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = request.get("id").cloned();

    // A notification (no id): return 202, no body.
    let Some(id) = id else {
        stream.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")?;
        stream.flush()?;
        return Ok(true);
    };

    let result = match method {
        "initialize" => {
            if let Some(name) = request
                .get("params")
                .and_then(|p| p.get("clientInfo"))
                .and_then(|c| c.get("name"))
                .and_then(|n| n.as_str())
            {
                observed.lock().expect("identity lock").client_name = Some(name.to_string());
            }
            serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
                "serverInfo": {"name": "fake", "version": "0.1"},
            })
        }
        "tools/list" => serde_json::json!({"tools": [
            {
                "name": "run",
                "description": "Runs a shell command on the remote server and returns its output. \
                                This description is deliberately long; to show that it must be \
                                truncated on import it repeats the same thing again and again, \
                                again and again, and it still goes on.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "shell command"},
                        "mode": {"type": "string", "enum": ["fast", "safe"]}
                    },
                    "required": ["command"]
                }
            },
            {
                // UNTRANSLATABLE: a multi-branch `oneOf`. MUST NOT be accepted
                // silently.
                "name": "flexible",
                "description": "Accepts any type.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "target": {"oneOf": [{"type": "string"}, {"type": "integer"}]}
                    }
                }
            }
        ]}),
        "tools/call" => serde_json::json!({
            "content": [{"type": "text", "text": "2 files updated"}],
            "isError": false,
        }),
        _ => serde_json::json!({}),
    };

    let response = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();

    if sse {
        // A heartbeat and an event that is not ours are sprinkled in between:
        // the client has to skip them and find its own id.
        let body = format!(
            ": heartbeat\n\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}}\n\n\
             data: {response}\n\n"
        );
        stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                 Mcp-Session-Id: session-42\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )?;
    } else {
        stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Mcp-Session-Id: session-42\r\nContent-Length: {}\r\n\r\n{response}",
                response.len()
            )
            .as_bytes(),
        )?;
    }
    stream.flush()?;
    Ok(true)
}

/// The whole flow: handshake, tool list, schema bridge, tool call.
fn full_flow(sse: bool) {
    let (url, server, observed) = start_server(sse);
    let client = MCPClient::new(url, Some("secret-token".into())).expect("client");

    let tools = client.tools().expect("tools/list");
    assert_eq!(tools.len(), 2, "the server declared two tools");

    // 1) The translatable tool: the schema must REALLY become usable.
    let run = tools.iter().find(|t| t.name == "run").expect("run");
    let conversion = convert_schema(&run.schema).expect("must convert");
    assert!(
        conversion
            .schema
            .validate(&serde_json::json!({"command": "ls"}))
            .is_ok()
    );
    assert!(
        conversion
            .schema
            .validate(&serde_json::json!({"command": "ls", "mode": "none"}))
            .is_err(),
        "a value outside the enum set must not pass"
    );
    assert!(
        conversion.schema.validate(&serde_json::json!({})).is_err(),
        "required field"
    );

    // 2) The untranslatable tool IS NOT ACCEPTED SILENTLY.
    let flexible = tools
        .iter()
        .find(|t| t.name == "flexible")
        .expect("flexible");
    let reason = convert_schema(&flexible.schema)
        .err()
        .expect("should have been rejected");
    assert_eq!(
        reason,
        tacet_mcp::UntranslatableReason::CompositeSchema("oneOf".into())
    );

    // 3) A long description does not enter the context RAW.
    let truncated = tacet_mcp::truncate_description(&run.description);
    assert!(truncated.chars().count() < run.description.chars().count());
    assert!(truncated.chars().count() <= tacet_mcp::bridge::DESCRIPTION_LIMIT + 1);

    // 4) The tool call.
    let (text, is_error) = client
        .call_tool("run", &serde_json::json!({"command": "git pull"}))
        .expect("tools/call");
    assert_eq!(text, "2 files updated");
    assert!(!is_error);

    // 5) BRAND: what lands in the far side's log. If a wrong name leaks, the
    //    third-party server the user connected RECORDS it and that cannot be
    //    taken back; that is why the assertion looks not at the code but at the
    //    value read off the wire.
    {
        let o = observed.lock().expect("identity lock");
        assert_eq!(
            o.user_agent.as_deref(),
            Some("tacet/1.0"),
            "the User-Agent header must carry the product name"
        );
        assert_eq!(
            o.client_name.as_deref(),
            Some("tacet"),
            "initialize.clientInfo.name must carry the product name"
        );
    }

    // Dropping the client closes the pooled connection; the server thread's
    // read sees EOF and the loop ends. The flag ends the listening too.
    drop(client);
    server.shutdown();
}

#[test]
fn end_to_end_over_the_plain_json_transport() {
    full_flow(false);
}

#[test]
fn end_to_end_over_the_sse_transport() {
    full_flow(true);
}
