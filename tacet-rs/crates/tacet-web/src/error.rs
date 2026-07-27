//! The web layer's error type.
//!
//! WHY A SEPARATE ENUM (instead of using `ToolError` directly): this crate is
//! NOT, and must not be, dependent on `tacet-core`. Making the network code
//! know the tool contract would force tomorrow's non-network caller (a CLI
//! diagnostic command, say) to pull in the core as well. The translation
//! happens on the `tacet-tools` side, at the tool boundary — that side already
//! knows both worlds.
//!
//! EVERY CASE ITS OWN VARIANT: a single "network error" bucket produces nothing
//! beyond telling the user "something went wrong". The server being up while
//! JSON is switched off (SearXNG's well-known trap: `formats: json` inside
//! `settings.yml`) and there being no network at all are two entirely different
//! situations, and what the user should do differs entirely too.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebError {
    /// The server could not be reached at all: DNS did not resolve, the
    /// connection was refused, TLS was not established.
    Unreachable(String),
    /// The connection was established but the response did not arrive within
    /// the timeout.
    Timeout,
    /// HTTP 4xx/5xx. Carries the code: 404 wrong path, 429 rate limit, 5xx server.
    ServerCode(u16),
    /// A response arrived but it is not JSON, or not in the shape we expected.
    ///
    /// On SearXNG the most frequent cause is `formats: json` being switched
    /// off; the server returns HTML in that state and, because it looks like
    /// "200 OK", it goes silently wrong.
    InvalidJson(String),
    /// The query went out, the server answered properly, but `results[]` is
    /// empty. Not an error, but not a success either — the model MUST NOT MAKE
    /// SOMETHING UP here (see `no_results`).
    EmptyResult,
    /// The configured base address is not usable.
    InvalidAddress(String),
}

impl fmt::Display for WebError {
    /// These strings go TO THE USER (chip detail, CLI diagnostics) — they
    /// suggest an action. This is not the text that goes to the model; that one
    /// is turned into a fixed string in `tacet-tools` via `ToolOutcome::failed`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WebError::Unreachable(_) => f.write_str("The search server could not be reached."),
            WebError::Timeout => f.write_str("The search took too long."),
            WebError::ServerCode(c) => write!(f, "The search server did not respond ({c})."),
            WebError::InvalidJson(_) => {
                f.write_str("The server did not return JSON; 'formats: json' must be enabled in its settings.")
            }
            WebError::EmptyResult => f.write_str("The search returned no results."),
            WebError::InvalidAddress(_) => f.write_str("The search server address is invalid."),
        }
    }
}

impl std::error::Error for WebError {}

pub type WebResult<T> = Result<T, WebError>;
