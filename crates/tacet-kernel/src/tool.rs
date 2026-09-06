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

    /// The work itself. RETURNS NO ERROR: every path produces a `ToolOutcome`,
    /// because an error too must reach the user as a chip and the model as a
    /// fixed text. Implementations convert internal errors with
    /// `ToolOutcome::failed(&error)`.
    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a>;
}
