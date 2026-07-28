//! Chip state: the single source of truth the user sees in the stream.

use serde::{Deserialize, Serialize};

/// The state of a tool chip in its life cycle.
///
/// The `Read`/`Written` split is not cosmetic: the engine answers the question
/// "did the world change this turn" by looking ONLY at `Written`, and it bases
/// retry safety after an error on that. Picking the wrong state means a double
/// side effect.
///
/// SERIALIZED FORM CHANGED WITH THE RENAME. With `rename_all = "snake_case"`
/// the variant names ARE the on-disk format; the Turkish build wrote
/// `izin_gerekli`. No migration was written, because no build with the old
/// names was ever shipped — the same argument recorded in `env.rs`: the app was
/// never released, so no record with the old spelling exists on any machine.
/// This is a deliberate decision, not an oversight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    /// Work in progress; the chip shows a spinner.
    Running,
    /// Read-only work finished — there is nothing to undo.
    Read,
    /// The world changed (a file was written, a record created). Retry forbidden.
    Written,
    /// Waiting on a user decision — the gate is in the code, not in the model.
    NeedsPermission,
    /// Failed; the text it carries is shown TO THE USER, which is why it is a
    /// plain human sentence. The text that goes to the model is not this one,
    /// it is `ERROR_MODEL_TEXT`.
    Failed(String),
}

impl ToolState {
    /// Did this state "change the world" — the engine's retry gate.
    pub fn changed_world(&self) -> bool {
        matches!(self, ToolState::Written)
    }

    /// Is the work over (the chip is no longer live).
    pub fn is_done(&self) -> bool {
        !matches!(self, ToolState::Running | ToolState::NeedsPermission)
    }
}
