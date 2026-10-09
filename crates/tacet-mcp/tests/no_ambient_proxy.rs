//! THE ROUTE DOES NOT COME FROM THE ENVIRONMENT (audit M12, the MCP half).
//!
//! `ureq`'s default config reads ALL_PROXY / HTTPS_PROXY / HTTP_PROXY, so a
//! variable exported for some other tool sent a LOCAL server's plain-http
//! traffic — bearer token included — to whatever host the variable named. The
//! audit saw `authorization: Bearer BEARER-SECRET` inside a CONNECT tunnel.
//!
//! A FILE OF ITS OWN, WITH ONE TEST, ON PURPOSE: it sets process-wide
//! environment variables, and every test binary is its own process. Sharing a
//! binary would let these variables leak into tests running beside it on
//! other threads. Both the "proxy" and the server are listeners on
//! `127.0.0.1`; nothing leaves the machine.

mod common;

use common::Listener;
use std::thread;
use std::time::Duration;
use tacet_mcp::client::MCPClient;

#[test]
fn a_proxy_variable_in_the_environment_is_not_used() {
    let proxy = Listener::start(|| {
        "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
    });
    let server = Listener::start(|| {
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\
         Connection: close\r\n\r\n{}"
            .to_string()
    });

    let route = format!("http://127.0.0.1:{}", proxy.port);
    // SAFETY: this binary holds exactly one test, so no other thread reads the
    // environment while it is written (see the module comment).
    unsafe {
        for name in [
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ] {
            std::env::set_var(name, &route);
        }
        std::env::remove_var("NO_PROXY");
        std::env::remove_var("no_proxy");
    }

    // The client (and with it the agent) is built AFTER the variables are set,
    // which is when `ureq` would have read them.
    let client = MCPClient::new(
        format!("http://127.0.0.1:{}/mcp", server.port),
        Some("BEARER-SECRET".into()),
    )
    .expect("a local address is accepted");
    // The answer does not matter — `{}` is not a tool list. Where it went does.
    let _ = client.tools();

    thread::sleep(Duration::from_millis(100));
    let (through_proxy, direct) = (proxy.hits(), server.hits());
    proxy.shutdown();
    server.shutdown();

    assert_eq!(
        through_proxy, 0,
        "the request (and its bearer token) went through the proxy the environment named"
    );
    assert!(direct >= 1, "the configured server was reached directly");
}
