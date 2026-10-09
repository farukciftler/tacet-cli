//! A CONNECTED SERVER CANNOT SEND THIS MACHINE SOMEWHERE ELSE (audit L2).
//!
//! `ureq` follows ten redirects unless told otherwise, so an MCP server could
//! answer `302 Location: http://127.0.0.1:…/internal-admin` and turn the
//! client into a blind request-forger against whatever listens on the user's
//! own machine or network — an address the user never configured and that
//! never went through `validate_url`. The audit's harness saw exactly that:
//! `GET /internal-admin?x=1` arriving with `mcp-method: tools/call`.
//!
//! Real sockets on `127.0.0.1` and random ports, as in `hostile_server.rs` —
//! the "elsewhere" is a second local listener on another port, i.e. another
//! origin. Nothing leaves the machine.

mod common;

use common::Listener;
use std::thread;
use std::time::Duration;
use tacet_mcp::client::MCPClient;

#[test]
fn a_redirect_to_another_origin_is_not_followed() {
    let elsewhere = Listener::start(|| {
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\
         Connection: close\r\n\r\n{}"
            .to_string()
    });
    let target = format!("http://127.0.0.1:{}/internal-admin?x=1", elsewhere.port);
    let server = Listener::start(move || {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\n\
             Connection: close\r\n\r\n"
        )
    });

    let client = MCPClient::new(
        format!("http://127.0.0.1:{}/mcp", server.port),
        Some("BEARER-SECRET".into()),
    )
    .expect("a local address is accepted");
    let outcome = client.tools();

    // ureq follows a redirect before `post` returns, so by now the second
    // listener has either been dialled or it never will be; the pause only
    // covers its accept loop noticing.
    thread::sleep(Duration::from_millis(100));
    let (asked, followed) = (server.hits(), elsewhere.hits());
    server.shutdown();
    elsewhere.shutdown();

    assert!(asked >= 1, "the configured server was asked");
    assert_eq!(
        followed, 0,
        "the redirect was followed to an address the user never configured"
    );
    assert!(
        outcome.is_err(),
        "a 302 is not a tool list, and must not be reported as one"
    );
}
