//! Tool errors: two different audiences, two different texts.
//!
//! The text that goes to the user is a plain human sentence ("File not
//! found."); the text that goes to the model is FIXED and ENGLISH. The split is
//! deliberate, and it survives the user-facing side being localized: even if the
//! model echoes the tool error verbatim in its answer, no localized string
//! leaks, no raw error code, and no personal trace such as a file path. The same
//! rule holds on the Swift side.

use std::path::PathBuf;

/// The fixed error text returned to the model. Deliberately a `const`: letting
/// call sites invent their own variant would break that guarantee.
pub const ERROR_MODEL_TEXT: &str =
    "tool_failed: the action could not be completed; no result was produced";

/// Everything a tool is allowed to fail with.
///
/// A CLOSED SET ON PURPOSE: the text the MODEL is shown for a failure is fixed
/// (`ERROR_MODEL_TEXT`) so a failure cannot become a prompt-injection channel,
/// while the user still sees the real variant. A tool that needs a failure mode
/// not in this list is a tool asking to explain itself to the model.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// The model sent an argument that does not match the schema (missing
    /// field, wrong type).
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// A field the schema marks as required never arrived.
    #[error("required field missing: {0}")]
    MissingField(String),

    /// The path resolved cleanly and there is nothing there.
    #[error("file not found: {0}")]
    FileNotFound(PathBuf),

    /// An attempt to step outside the working directory — a sandbox violation.
    #[error("outside the permitted directory: {0}")]
    SandboxViolation(PathBuf),

    /// The write could not be completed for want of disk.
    #[error("no space left on device")]
    OutOfSpace,

    /// The user said "no" at the approval gate.
    #[error("user did not grant permission: {0}")]
    PermissionDenied(String),

    /// The tool ran out of its own budget — a sandboxed script, a subprocess, a
    /// request that never came back.
    #[error("the operation timed out")]
    Timeout,

    /// A wrapped I/O error.
    #[error("the file operation could not be completed")]
    Io(#[from] std::io::Error),

    /// A wrapped JSON error.
    #[error("the data could not be parsed")]
    Serde(#[from] serde_json::Error),

    /// An error that cannot be classified; the user is still shown a proper
    /// sentence.
    #[error("{0}")]
    Other(String),
}

impl ToolError {
    /// The short human sentence shown on the chip.
    ///
    /// A raw `io::Error` text ("No such file or directory (os error 2)") never
    /// reaches the screen: the user is shown what happened translated into
    /// human language, not a system error.
    pub fn short_error(&self) -> String {
        match self {
            ToolError::InvalidArgument(_) | ToolError::MissingField(_) => {
                "This request could not be understood.".into()
            }
            ToolError::FileNotFound(_) => "File not found.".into(),
            ToolError::SandboxViolation(_) => "There is no access to this location.".into(),
            ToolError::OutOfSpace => "No space left on the device.".into(),
            ToolError::PermissionDenied(_) => "The sharing was not approved.".into(),
            ToolError::Timeout => "The operation took too long.".into(),
            ToolError::Io(_) => "The file operation could not be completed.".into(),
            ToolError::Serde(_) => "The data could not be read.".into(),
            ToolError::Other(m) if !m.is_empty() => m.clone(),
            ToolError::Other(_) => "This step could not be completed.".into(),
        }
    }

    /// The text that will reach the model — always the same, always English.
    pub fn model_text(&self) -> &'static str {
        ERROR_MODEL_TEXT
    }
}

/// What every tool body returns.
pub type ToolResult<T> = Result<T, ToolError>;
