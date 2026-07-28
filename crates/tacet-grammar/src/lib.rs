//! tacet-grammar — turns an `ArgSchema` into a constrained-decoding grammar.
//!
//! WHY IT EXISTS: on the Swift side Apple FoundationModels, via
//! `DynamicGenerationSchema`, FORCED the model to produce valid JSON. When we
//! moved to pure Rust we lost that forcing; a small model (3B), left free,
//! starts explaining "shall I do this?" instead of calling a tool — this
//! regression happened once already. So this is not a comfort layer, it is the
//! operating condition of tool calling.
//!
//! FLOW:
//!   ArgSchema --compile--> Grammar (immutable, shared via Arc)
//!   Grammar --state()--> GrammarState (push-down automaton, cheap to clone)
//!   GrammarState + vocab --TokenMask--> Vec<bool> (allowed tokens)
//!
//! Example:
//! ```
//! use tacet_kernel::{ArgSchema, Field};
//! use tacet_grammar::Grammar;
//!
//! let schema = ArgSchema::object(vec![
//!     Field::new("query", ArgSchema::text()).required(),
//!     Field::new("scope", ArgSchema::choice(["all", "near"])),
//! ]);
//! let grammar = Grammar::compile(&schema);
//! let mut state = grammar.state();
//! state.advance(r#"{"query":"report"}"#).unwrap();
//! assert!(state.is_done());
//! ```
//!
//! NO NETWORK, NO FILES: this crate is pure computation.

mod call;
mod compile;
mod error;
mod mask;
mod permit;
mod state;

pub use call::CallConstraint;
pub use compile::Grammar;
pub use error::GrammarError;
pub use mask::TokenMask;
pub use permit::AllowedSet;
pub use state::GrammarState;

#[cfg(test)]
mod tests;
