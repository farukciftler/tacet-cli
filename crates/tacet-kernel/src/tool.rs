//! The tool contract.
//!
//! DECISION — ASYNC SHAPE: `async fn run(...)` (AFIT, edition 2024) WAS TRIED
//! and REJECTED. A trait with AFIT is not dyn-compatible; yet `ToolCatalog` has
//! to hold exactly `Vec<Arc<dyn Tool>>` — tools are looked up by name at run
//! time and live in a heterogeneous list. That is why `run` returns
//! `Pin<Box<dyn Future<Output = ToolOutcome> + Send + '_>>` by hand. The
//! `async-trait` dependency was NOT ADDED; the macro performs exactly this
//! transformation, and we do the same thing dependency-free with a one-line
//! helper (`boxed`).
//!
//! Why the `Send` bound: the engine will run tool calls on a multi-threaded
//! executor.

use crate::context::ToolContext;
use crate::outcome::ToolOutcome;
use crate::schema::ArgSchema;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// How long the executor waits for a tool that did not say otherwise
/// (`Tool::deadline`). After it the call is told to stop and, if it does not,
/// abandoned — the turn goes on without it (audit M30).
///
/// WHY 120 S: it is the longest any built-in waits on a single thing it does
/// not control — the MCP transport's global timeout (`tacet_mcp::TIMEOUT_S`)
/// and the longest web/db budgets (15 s, 60 s) sit under it. The tools that
/// legitimately run longer say so through `deadline`: `shell` (up to 300 s by
/// the user's own cap), an MCP call (a task polled after it, or a question put
/// to the user) and `db_write` (a person reading the effect before saying yes).
/// A number below every honest budget would turn a slow tool into a failed
/// one; this one only catches a tool that is not coming back.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(120);

/// Return type of `run`. The lifetime `'a` is tied to the context and `self`.
pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = ToolOutcome> + Send + 'a>>;

/// Turns an `async` block into a `ToolFuture` — implementations wrap the body
/// of `run` in `boxed(async move { ... })` and keep async/await ergonomics.
pub fn boxed<'a, F>(future: F) -> ToolFuture<'a>
where
    F: Future<Output = ToolOutcome> + Send + 'a,
{
    Box::pin(future)
}

/// Everything a tool must be able to answer about itself.
///
/// The four required methods are the whole contract: a NAME the model calls, a
/// DESCRIPTION saying when to call it, a SCHEMA the model is forced into, and a
/// body. Everything else has a default, and the defaults are the conservative
/// side of each question — see `taints_session` and `world_changed`.
pub trait Tool: Send + Sync {
    /// The name the model calls. ASCII, snake_case, constant for the session.
    fn name(&self) -> &str;

    /// Short description shown to the model: it says WHEN to use this tool.
    fn description(&self) -> &str;

    /// The argument contract. The model is FORCED into this schema (see
    /// tacet-grammar).
    fn schema(&self) -> ArgSchema;

    /// Does this tool read personal data — the approval gate reads this.
    ///
    /// Defaults to `false`: whoever writes a new tool must deliberately say
    /// "yes". The opposite default drags every tool through a pointless
    /// approval prompt, and an approval seen too often stops being read, which
    /// means the gate loses its purpose.
    fn taints_session(&self) -> bool {
        false
    }

    /// Is this tool's RESULT text written by someone outside this machine — a
    /// web page, a remote server, another agent's transcript (audit H2-c).
    ///
    /// A different question from `taints_session`, and a tool can answer yes to
    /// both (an imported transcript is personal AND untrusted, RG-01) or to
    /// either. After a successful call the session is marked, and from then on
    /// every tool that `acts_on_host` asks before it runs: the model's next
    /// command may be the page's.
    ///
    /// Defaults to `false` for the reason `taints_session` does; the tools that
    /// fetch, search, call a remote server or read another agent's files say yes.
    fn external_content(&self) -> bool {
        false
    }

    /// Does a call run with the user's own power on this machine, steered by
    /// the arguments — a program started, a database written (audit H2-c).
    ///
    /// Such a tool asks for approval on EVERY call once `external_content` has
    /// been seen this session: a `git -c alias.x='!cmd' x` lifted from a web page
    /// is one call away otherwise. In a session that read nothing from outside
    /// it runs as before — an approval asked on every `ls` stops being read.
    fn acts_on_host(&self) -> bool {
        false
    }

    /// Ask the user before EVERY call, whatever the session has seen.
    ///
    /// This is the one knob that is configuration rather than nature: a user
    /// can tell Tacet to treat a particular MCP server this way. It can only
    /// ADD caution — nothing in the contract lets configuration switch
    /// `taints_session` or `external_content` off for a tool that has them.
    fn always_asks(&self) -> bool {
        false
    }

    /// The longest the executor waits for one call before telling it to stop
    /// (see `DEFAULT_DEADLINE`). A tool with its own longer, user-capped budget
    /// returns that budget plus a margin, so its own timeout — which reports
    /// better — always fires first.
    fn deadline(&self) -> Duration {
        DEFAULT_DEADLINE
    }

    /// The work itself. RETURNS NO ERROR: every path produces a `ToolOutcome`,
    /// because an error too must reach the user as a chip and the model as a
    /// fixed text. Implementations convert internal errors with
    /// `ToolOutcome::failed(&error)`.
    ///
    /// A PANIC IS STILL CAUGHT, but by the executor, not here: it runs the body
    /// on a worker thread under `catch_unwind` and turns a panic into a
    /// `Failed("internal error in <tool>")` (audit M30). That is a backstop for
    /// parsers of formats nobody here controls (RG-04: the agent-session
    /// readers run inside it), not a licence to `unwrap`.
    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a>;
}
