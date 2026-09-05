//! The constraint contract, which now lives in `tacet-kernel`.
//!
//! IT MOVED, AND THE MOVE IS THE POINT. Every signature in it is over
//! `&mut [f32]` and `u32` — no model, no tokenizer, no device — so keeping it
//! here made a runtime-independent guarantee look like a property of this
//! engine, and forced anyone who wanted constrained generation to depend on
//! GGUF loading, prompt budgeting and candle to get three method signatures.
//!
//! Re-exported rather than deleted so `tacet_engine::Constrainer` keeps
//! resolving: the names are load-bearing in the CLI and the eval crate, and a
//! rename would have been churn with no reader benefit.

pub use tacet_kernel::{Constrainer, ConstraintError, ConstraintSession, FreeConstraint};
