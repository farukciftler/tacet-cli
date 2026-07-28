//! tacet-web — web search through the user's own SearXNG instance.
//!
//! THE NETWORK MONOPOLY (an architectural rule): in Tacet a network call exists
//! ONLY in this crate and in `tacet-mcp`. No other crate opens a socket, builds
//! an HTTP client, or pulls in `ureq`/`reqwest`. The value of the rule is in its
//! auditability: the answer to "where does the user's query leave from" is a
//! single file (`client.rs`).
//!
//! THIS CRATE DOES NOT KNOW `tacet-kernel`. The tool contract, chips,
//! `ToolOutcome`, `DataStore` — none of them occur here. The translation is done
//! at the `tacet-tools/src/web_search.rs` boundary. That way the network layer
//! does not change when the tool architecture does; and a caller that never
//! goes online (a diagnostic command) does not pull in the core.
//!
//! CLOSED BY DEFAULT. This crate's network surface cannot be used unless the
//! user installs an ADDON: the address is no longer baked into the code, and the
//! `web_search` tool does not appear in the catalog at all unless the addon is
//! open (see the `addon` module).
//!
//! LAYERS:
//! - `addon`     — the addon registry (`addons.json`): what is installed, which
//!   one is open, what the address is. NO NETWORK, local file only. The source
//!   of the catalog gate.
//! - `client`    — the network surface for search (`ureq`), URL construction,
//!   timeout, the address gate.
//! - `download`  — downloading a large file (a model package): the approval
//!   gate, resuming with Range, atomic completion, SHA-256. A SEPARATE file from
//!   search because the timeout regimes are opposite (see the rationale there).
//!   Within this crate `ureq` occurs only in those two files.
//! - `outcome`   — the interpretation of SearXNG's JSON. NO NETWORK, its input
//!   is a `&str`, all of it testable.
//! - `text`      — HTML → plain text. Deliberately simple; not a DOM parser.
//! - `relevance` — "does this text answer the query": result ranking and cutting
//!   the right WINDOW out of a page. NO NETWORK (see the measurement there).
//! - `error`     — every failure case is its OWN variant (see the rationale
//!   there).
//!
//! PRIVACY: a query is data too. This crate DOES NOT RECORD the query anywhere,
//! prints no log and writes nothing to disk; it only builds the request and
//! returns the response. Putting the query through user approval in a tainted
//! session is the caller's job (`tacet-tools` + the `ToolExecutor` approval
//! gate).

pub mod addon;
pub mod client;
pub mod download;
pub mod error;
pub mod outcome;
pub mod release;
pub mod relevance;
pub mod text;

pub use addon::{Addon, Record as AddonRecord, WEB_SEARCH, web_search_is_open};
pub use client::{
    ADDRESS_VARIABLE, DEFAULT_TIMEOUT, WebSearchClient, address_is_valid, is_fetchable,
    target_is_public,
};
pub use download::{
    AlwaysDeny, DownloadApproval, DownloadError, DownloadOutcome, DownloadPlan, DownloadResult,
    Progress, SilentProgress, TEMP_EXTENSION, digest, digest_from_file, download, temp_path,
};
pub use error::{WebError, WebResult};
pub use outcome::{SearchOutcome, domain, parse, truncate_at_word};
pub use relevance::{
    clock_count, data_density, keywords, relevance_score, relevant_section, simplify,
    strong_data_density,
};
pub use text::to_text;
