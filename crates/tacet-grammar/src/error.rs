//! Grammar errors.
//!
//! These errors are not meant to be shown TO THE USER: if constrained
//! generation works correctly the model cannot produce an invalid character in
//! the first place, so landing here means either the grammar is off (free
//! decoding) or we are in a debugging scenario. That is why the texts speak to
//! the developer; there is no need for the two-channel setup that `ToolError`
//! has in the tool layer.

/// The cases the grammar rejects.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GrammarError {
    /// This character cannot be produced at this position under the grammar.
    #[error("position {position}: '{character}' is not valid here")]
    UnexpectedCharacter {
        /// The character that arrived.
        character: char,
        /// How many characters had been accepted before it.
        position: usize,
    },

    /// The JSON closed but something (other than whitespace) followed it.
    #[error("position {position}: valid JSON ended, there is trailing input")]
    TrailingInput {
        /// Where the JSON had finished.
        position: usize,
    },

    /// The input ran out while the stack was not empty (an open
    /// object/array/string remains).
    #[error("input left half-open: the structure did not close")]
    Incomplete,

    /// The number syntax is correct but the value is outside the schema's range.
    ///
    /// A separate variant: this is not a SYNTAX error, it is a semantic one.
    /// Baking the range constraint into the automaton digit by digit (for
    /// example "first digit 1-5" for 1..50) would grow the grammar far more
    /// than the constraint's value; opening the gate once, where the number
    /// ends, is the right trade.
    #[error("number out of range: {0}")]
    OutOfRange(String),
}
