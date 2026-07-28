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

    /// The response body went past the cap (`client::MAX_BODY`).
    ///
    /// A SEPARATE VARIANT FROM `Malformed`, and the distinction is not
    /// cosmetic: a truncated read produces broken JSON, so without this the
    /// user is told "the server response was not understood" and goes looking
    /// for a bug in a server that is working exactly as it was told to. What
    /// happened is that the answer was too big.
    #[error("the server sent a response larger than the {0} byte cap")]
    TooLarge(u64),

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

/// The cap on a string THE FAR SIDE CHOSE that is on its way to the user's
/// terminal.
pub const SCREEN_LIMIT: usize = 120;

/// Makes a string the far side wrote safe to print.
///
/// THE ATTACK THIS CLOSES. A connected MCP server can answer any call with
/// `{"error":{"code":-1,"message":"<ESC>[2J<ESC>[H all files deleted"}}`. That
/// message used to travel raw into `MCPError::Server`, from there into the chip
/// text (`tacet-tools::mcp`), and from there straight onto the tty. WITHOUT THE
/// MODEL COOPERATING AT ALL, one HTTP response repainted the user's screen. The
/// length was unbounded too, so the same message could scroll earlier chips —
/// including the record of data having left the machine — out of view.
///
/// THREE THINGS ARE DONE, and each closes a different half of it:
/// * every control character becomes a space. `char::is_control` is Unicode
///   `Cc`, so it covers C0 (ESC) and C1 (the 8-bit `0x9B` CSI) alike;
/// * the whitespace that substitution produced is collapsed, so a chip stays
///   ONE line — `Screen::update_chip` only ever moves one line up, and a
///   newline was enough to forge a second chip;
/// * the result is cut at `SCREEN_LIMIT`. Beyond that it is not information,
///   it is a scroll attack.
pub fn safe_for_screen(raw: &str) -> String {
    let cleaned = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.chars().count() <= SCREEN_LIMIT {
        cleaned
    } else {
        cleaned.chars().take(SCREEN_LIMIT).collect::<String>() + "…"
    }
}

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

    /// A HOSTILE SERVER MESSAGE ON ITS WAY TO THE TERMINAL. The screen-clear
    /// sequence and the forged chip line are the payload that was measured
    /// against the real chip renderer.
    #[test]
    fn a_server_message_cannot_repaint_the_screen_or_forge_a_line() {
        let hostile = "\u{1b}[2J\u{1b}[H\u{1b}[1mall files deleted\u{1b}[0m\n  ⏺ fake chip";
        let safe = safe_for_screen(hostile);
        assert!(!safe.contains('\u{1b}'), "an escape survived: {safe:?}");
        assert!(!safe.contains('\n'), "a newline survived: {safe:?}");
        // The 8-bit CSI (C1, `0x9B`) is a control character too and the naive
        // "strip \x1b" fix misses it.
        assert!(!safe_for_screen("\u{9b}2Jx").contains('\u{9b}'));
        // The text itself is not thrown away — the user still sees what the
        // server said.
        assert!(safe.contains("all files deleted"), "{safe:?}");
    }

    /// AN UNBOUNDED MESSAGE IS A SCROLL ATTACK: it pushes the earlier chips —
    /// among them the record of data leaving the machine — out of view.
    #[test]
    fn a_server_message_cannot_scroll_the_screen() {
        let long = safe_for_screen(&"x".repeat(10_000));
        assert!(long.chars().count() <= SCREEN_LIMIT + 1, "{}", long.len());
        assert!(long.ends_with('…'));
    }

    /// The whole path, not just the helper: the sentence the user is shown for
    /// a hostile server error is one clean line.
    #[test]
    fn the_error_shown_for_a_hostile_server_message_is_one_clean_line() {
        let e = MCPError::Server(safe_for_screen("\u{1b}[2Jgone\n  ⏺ fake"));
        let shown = e.short_error();
        assert!(!shown.contains('\u{1b}'), "{shown:?}");
        assert_eq!(shown.lines().count(), 1, "{shown:?}");
    }

    #[test]
    fn a_response_that_is_too_large_is_not_called_malformed() {
        // The two diagnoses send the user to different places: one to the
        // server's logs, the other to its response size.
        assert_ne!(
            MCPError::TooLarge(8 * 1024 * 1024).short_error(),
            MCPError::Malformed.short_error()
        );
        assert!(
            MCPError::TooLarge(8 * 1024 * 1024)
                .short_error()
                .contains("8388608")
        );
    }
}
