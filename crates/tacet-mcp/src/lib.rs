//! tacet-mcp — the MCP (Model Context Protocol) client.
//!
//! **THE NETWORK MONOPOLY.** Together with `tacet-web`, one of the two crates
//! allowed to make a network call. No other crate opens a socket; so that the
//! rule stays auditable, the HTTP dependency also lives only in those two
//! manifests.
//!
//! Hand-written JSON-RPC 2.0 over plain request/response HTTP, speaking the
//! **2026-07-28** revision, with the old one kept frozen beside it for the
//! deprecation window. NO official MCP SDK WAS PULLED IN: the client needs a
//! handful of methods, and the hand-written client IS the audit story — a beta
//! SDK of tens of thousands of lines would replace it with a promise. The one
//! exception is TLS: writing that by hand would be irresponsible
//! (see `Cargo.toml`).
//!
//! ## The promise
//!
//! > Tacet does not go online by itself. If you connect a server, you see what
//! > is sent there every single time.
//!
//! This crate carries the FIRST half of that promise: with no connection,
//! nothing happens (the `config` default is empty). The SECOND half — "nothing
//! goes out unseen" — is NOT here, it is the deterministic approval gate in
//! `tacet-tools::executor`. That split is deliberate: if the network layer held
//! its own gate, whoever changes the network layer could change the gate too.
//!
//! ## Modules
//!
//! | Module | Job | Goes online |
//! | --- | --- | --- |
//! | `jsonrpc` | Request framing, id matching, `result`/`error` | no |
//! | `sse` | `text/event-stream` parsing | no |
//! | `bridge` | MCP JSON Schema -> `ArgSchema`, description truncation | no |
//! | `config` | config directory + `mcp.json` | no |
//! | `elicit` | MRTR questions: parsing, sanitizing, answering | no |
//! | `legacy` | the frozen 2025-06-18 path (sunset 2027-07-28) | no socket of its own |
//! | `transport` | the socket seam + the record/replay test transport | **YES, the only place** |
//! | `client` | protocol logic on top of `transport` | no socket of its own |
//!
//! The only module that goes online is `client`; everything else is pure and
//! tested without the network. The single network test in the tests is
//! `#[ignore]`.

pub mod auth;
pub mod bridge;
pub mod client;
pub mod config;
pub mod elicit;
pub mod error;
pub mod jsonrpc;
pub mod legacy;
pub mod sse;
pub mod tasks;
pub mod transport;

pub use auth::{AuthSetting, Token, TokenStore};
pub use bridge::{
    Conversion, ConversionNotes, UntranslatableReason, choice_is_portable, convert_schema,
    name_is_portable, truncate_description,
};
pub use client::{
    CATALOG_TTL_CAP, MCPClient, PROTOCOL_VERSION, Revision, SpecChoice, TIMEOUT_S, ToolSpec,
};
pub use config::{
    Config, ConnectionSetting, key_file_is_exposed, read_checked, read_default_checked,
};
pub use elicit::{DeclineInput, InputAsk, MAX_INPUT_ROUNDS, Question, QuestionKind};
pub use error::{MCPError, MCPResult, SCREEN_LIMIT, safe_for_screen};
pub use legacy::LEGACY_PROTOCOL_VERSION;
pub use tasks::{SilentWatch, TaskWatch};
pub use transport::{HttpTransport, Request, Transport};
