//! `tacet mcp try` must go out with the credential the chat would (audit L10).
//!
//! It built its own client with `setting.resolved_key()` — the STATIC key — so
//! on a connection that logs in with OAuth it sent no token, or a stale one, and
//! reported a failure the shell itself would never have hit. A diagnostic that
//! takes a different path from the thing it diagnoses measures the wrong thing.
//!
//! A FILE OF ITS OWN, AND ONE TEST IN IT, because the token file is found
//! through `TACET_HOME`, which is process-wide: set inside the library's unit
//! tests it would race every other test that reads the configuration
//! directory. Here the process is this test's alone. No socket is opened — the
//! client runs on the replay transport, so nothing leaves the machine and the
//! network-monopoly scan has nothing to find.

use std::sync::Arc;
use tacet_mcp::auth::{Token, TokenStore};
use tacet_mcp::transport::replay::{Canned, ReplayTransport};

#[test]
fn mcp_try_sends_the_oauth_token_the_chat_would_send() {
    let home = std::env::temp_dir().join(format!(
        "tacet-mcp-try-{}-{}",
        std::process::id(),
        tacet_mcp::auth::random_token()
    ));
    std::fs::create_dir_all(&home).expect("home");
    // SAFETY: this binary holds one test, so nothing else reads the
    // environment while it is being changed.
    unsafe {
        std::env::set_var(tacet_kernel::env::HOME_VAR, &home);
        std::env::remove_var(tacet_mcp::config::PATH_VARIABLE);
    }

    // A connection that logs in, AND carries a leftover static key: the token
    // must win, as it does on the chat path.
    let config = serde_json::json!({
        "connections": [{
            "name": "tracker",
            "url": "https://mcp.example.com/mcp",
            "key": "stale-static-key",
            "auth": {
                "issuer": "https://auth.example.com",
                "authorization_endpoint": "https://auth.example.com/authorize",
                "token_endpoint": "https://auth.example.com/token",
                "client_id": "tacet"
            }
        }]
    });
    tacet_kernel::write_private(
        &home.join("mcp.json"),
        serde_json::to_string_pretty(&config)
            .expect("json")
            .as_bytes(),
    )
    .expect("mcp.json");
    let mut store = TokenStore::default();
    store.tokens.insert(
        "tracker".into(),
        Token {
            issuer: "https://auth.example.com".into(),
            access_token: "the-oauth-token".into(),
            refresh_token: None,
            expires_at: None,
            resource: None,
        },
    );
    let token_path = tacet_mcp::auth::token_path().expect("token path");
    assert!(
        token_path.starts_with(&home),
        "the token file must be the test's, not the user's: {}",
        token_path.display()
    );
    tacet_mcp::auth::write_tokens(&token_path, &store).expect("tokens");

    // The server refuses everything; what matters is what was SENT.
    let transport = Arc::new(ReplayTransport::new(vec![
        Canned::status(401, "{}"),
        Canned::status(401, "{}"),
    ]));
    let _ = tacet_tools::mcp::try_connection_over(
        "tracker",
        Arc::new(tacet_mcp::DeclineInput),
        None,
        transport.clone(),
    );

    let sent = transport.sent();
    assert!(!sent.is_empty(), "try never reached the transport");
    for request in &sent {
        assert_eq!(
            request.header("authorization"),
            Some("Bearer the-oauth-token"),
            "mcp try went out with a different credential than the chat path"
        );
    }

    let _ = std::fs::remove_dir_all(&home);
}
