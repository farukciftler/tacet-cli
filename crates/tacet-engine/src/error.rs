//! Engine errors.
//!
//! A SEPARATE type from `ToolError`: a tool error is something shown to the
//! user as a chip and returned to the model as fixed text; an engine error is
//! the TURN LOOP itself breaking (weights would not load, context overflowed,
//! the constraint contradicted itself). Squeezing both into one enum would make
//! the question "what gets returned to the model" meaningless — when the engine
//! is down there is no model to return to.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The model file (GGUF/safetensors) is missing or unreadable.
    #[error("could not load model file: {0}")]
    ModelNotLoaded(PathBuf),

    /// The tokenizer would not load, or text could not be turned into tokens.
    #[error("tokenization failed: {0}")]
    Tokenization(String),

    /// The prompt did not fit the context budget even AFTER truncation.
    #[error("prompt did not fit the context budget: {measured} > {budget}")]
    BudgetExceeded { measured: usize, budget: usize },

    /// The constrainer rejected the incoming token despite the mask. This is a
    /// logic error (masking and advancing have drifted apart) and must not be
    /// swallowed silently.
    ///
    /// KEPT AS ITS OWN VARIANT even though `ConstraintSession::advance` now
    /// returns the narrower `tacet_kernel::ConstraintError`: call sites match on
    /// this name, and the `From` below means `s.advance(token)?` still widens
    /// into an engine error without any of them changing.
    #[error("constraint rejected the token: {0}")]
    ConstraintViolation(u32),

    /// FakeEngine ran out of script — meaning the test's scenario is incomplete.
    #[error("no steps left in the fake engine script (call {call})")]
    ScriptExhausted { call: usize },

    #[error("inference failed: {0}")]
    Inference(String),

    #[error("could not read model file")]
    Io(#[from] std::io::Error),
}

pub type EngineResult<T> = Result<T, EngineError>;

impl From<tacet_kernel::ConstraintError> for EngineError {
    fn from(e: tacet_kernel::ConstraintError) -> Self {
        match e {
            tacet_kernel::ConstraintError::Violation(token) => {
                EngineError::ConstraintViolation(token)
            }
        }
    }
}
