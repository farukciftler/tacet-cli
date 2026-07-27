//! The outcome of one tool run: what gets written on the chip + what goes back
//! to the model.

use crate::error::ToolError;
use crate::state::ToolState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The single output type of a tool.
///
/// `chip_text` and `to_model` are SEPARATE fields and have to be: the chip is
/// what the user sees (Turkish, short, describing what happened), `to_model` is
/// what enters the context. The heart of the 4096-token channel is here: bulk
/// data is NOT PUT into `to_model`, it is written to the DataStore and pointed
/// at with a `source_ref`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    /// The final text shown on the chip (~5 words, plus " · detail" if needed).
    pub chip_text: String,
    pub state: ToolState,
    /// The SHORT text returned to the model. Bulk data is not written here.
    pub to_model: String,
    /// Raw output for the chip detail view — the second layer of transparency.
    /// It does not go to the model; only the user sees it, and only if they open it.
    pub raw_output: Option<String>,
    /// The path of a file, if one was produced — tapping the chip opens a preview.
    pub file_path: Option<PathBuf>,
}

/// The ONE wire format of the bypass channel.
///
/// WHY A SINGLE FUNCTION: this suffix is written into the text that goes to the
/// model, that is, it is the syntax the model LEARNS. When two call sites write
/// their own `format!` (in one place "full content ready", in another
/// "[source_ref: ...]"), the model sees two different shapes and gets confused
/// about which one to hand back; on top of that one of them once got translated
/// into another language and nobody noticed. The format is defined once, here.
///
/// THE TEXT IS ENGLISH: like every fixed text that goes to the model. The chip
/// text the user sees is a SEPARATE field — the two must not be mixed up, so
/// that localizing the user-facing side can never move the syntax the model
/// learns.
pub fn source_ref_suffix(source_ref: &str) -> String {
    format!("\n(full content ready, source_ref={source_ref})")
}

impl ToolOutcome {
    /// The base constructor for places that need full control; day to day the
    /// short constructors below are preferred.
    pub fn new(
        chip_text: impl Into<String>,
        state: ToolState,
        to_model: impl Into<String>,
    ) -> Self {
        Self {
            chip_text: chip_text.into(),
            state,
            to_model: to_model.into(),
            raw_output: None,
            file_path: None,
        }
    }

    /// Read-only work finished.
    pub fn read_ok(chip_text: impl Into<String>, to_model: impl Into<String>) -> Self {
        Self::new(chip_text, ToolState::Read, to_model)
    }

    /// The world changed — when the engine sees this it closes off retrying.
    pub fn written(chip_text: impl Into<String>, to_model: impl Into<String>) -> Self {
        Self::new(chip_text, ToolState::Written, to_model)
    }

    // NOTE: there used to be a `needs_permission(...)` constructor here. It was
    // removed because it had zero callers (tests included): the approval
    // decision belongs to `ToolExecutor`, not to the tool, and
    // `ToolState::NeedsPermission` is produced by the gate over there. Leaving a
    // second production site was an invitation to write a tool that bypasses
    // the gate.

    /// Builds an outcome from an error. A Turkish sentence for the chip, the
    /// fixed English text for the model — because this is the single crossing
    /// point, call sites cannot break the rule.
    pub fn failed(error: &ToolError) -> Self {
        let reason = error.short_error();
        Self {
            chip_text: reason.clone(),
            state: ToolState::Failed(reason.clone()),
            to_model: error.model_text().to_string(),
            raw_output: Some(reason),
            file_path: None,
        }
    }

    /// Bypass-channel shortcut: the bulk data has been put in the store, only a
    /// summary and a reference go to the model.
    pub fn summarize(
        chip_text: impl Into<String>,
        summary: impl AsRef<str>,
        source_ref: impl AsRef<str>,
    ) -> Self {
        Self::read_ok(
            chip_text,
            format!(
                "{}{}",
                summary.as_ref(),
                source_ref_suffix(source_ref.as_ref())
            ),
        )
    }

    pub fn raw_output(mut self, raw: impl Into<String>) -> Self {
        self.raw_output = Some(raw.into());
        self
    }

    pub fn file_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.file_path = Some(path.into());
        self
    }
}
