//! MCP errors — separated for the user in plain language.
//!
//! WHY SEPARATE VARIANTS: `mcp-connection-spec §3.1` says "if the connection
//! cannot be established, why (timeout, authorization, TLS) is written in plain
//! language". A single "network error" variant teaches the user nothing: wrong
//! key, server down, broken certificate — those need three different actions.
//!
//! The strings here go TO THE USER. This is not the text that goes to the
//! model: the tool bridge turns every error into the fixed English
//! `tool_failed: ...` string via `ToolOutcome::failed` (the two-channel rule).

/// Failures at the wire/protocol level.
///
/// The server's OWN tool error (`isError: true`) does not belong here: that is
/// not a failure, it is the tool's normal outcome and it is told to the model.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MCPError {
    #[error("timed out")]
    Timeout,

    /// 401/403 — no key, or the key was not accepted.
    #[error("the access key was not accepted")]
    Authorization,

    #[error("a secure connection could not be established")]
    Tls,

    /// No network, server down, DNS did not resolve.
    #[error("the server could not be reached")]
    Unreachable,

    /// A JSON-RPC `error` body, or an unexpected HTTP code.
    #[error("the server returned an error{}", if .0.is_empty() { String::new() } else { format!(": {}", .0) })]
    Server(String),

    /// The response does not conform to MCP (not JSON, no `result`, id did not
    /// match).
    #[error("the server response was not understood")]
    Malformed,

    /// The URL scheme was not accepted (see `client::validate_url`).
    #[error("the address was not accepted: {0}")]
    InvalidAddress(String),
}

impl MCPError {
    /// The sentence shown to the user. It does not dramatize, it says what
    /// happened.
    pub fn short_error(&self) -> String {
        self.to_string()
    }
}

/// This crate's Result shorthand.
pub type MCPResult<T> = Result<T, MCPError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_error_with_an_empty_message_does_not_add_a_colon() {
        assert_eq!(
            MCPError::Server(String::new()).short_error(),
            "the server returned an error"
        );
        assert_eq!(
            MCPError::Server("HTTP 500".into()).short_error(),
            "the server returned an error: HTTP 500"
        );
    }

    #[test]
    fn no_error_string_looks_like_the_model_channel() {
        // The user channel and the model channel must not get mixed up: none of
        // these strings may carry model syntax like "tool_failed".
        for e in [
            MCPError::Timeout,
            MCPError::Authorization,
            MCPError::Tls,
            MCPError::Unreachable,
            MCPError::Malformed,
        ] {
            assert!(!e.short_error().contains(':'), "{e:?}");
        }
    }
}
