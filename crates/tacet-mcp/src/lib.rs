//! tacet-mcp — the MCP (Model Context Protocol) client.
//!
//! **THE NETWORK MONOPOLY.** Together with `tacet-web`, one of the two crates
//! allowed to make a network call. No other crate opens a socket; so that the
//! rule stays auditable, the HTTP dependency also lives only in those two
//! manifests.
//!
//! Hand-written JSON-RPC 2.0 + Streamable HTTP + SSE. NO official MCP SDK WAS
//! PULLED IN: v1 needs three methods (`initialize`, `tools/list`,
//! `tools/call`), and Tacet's identity is built on not pulling in ready-made
//! crates. The exception is TLS: writing that by hand would be irresponsible
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
//! | `client` | HTTP POST + session state | **YES, the only place** |
//!
//! The only module that goes online is `client`; everything else is pure and
//! tested without the network. The single network test in the tests is
//! `#[ignore]`.

pub mod bridge;
pub mod client;
pub mod config;
pub mod error;
pub mod jsonrpc;
pub mod sse;

pub use bridge::{
    Conversion, ConversionNotes, UntranslatableReason, convert_schema, truncate_description,
};
pub use client::{MCPClient, PROTOCOL_VERSION, TIMEOUT_S, ToolSpec};
pub use config::{Config, ConnectionSetting};
pub use error::{MCPError, MCPResult};
