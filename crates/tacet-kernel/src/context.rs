//! ToolContext — the running state tools share.
//!
//! Tools are handed a single `&mut ToolContext`; there is no global state. That
//! way eval runs the same tool with a fake store and a silent reporter while
//! the app runs it with the real ones, and the tool cannot tell them apart.

use crate::data_store::{DataStore, SourceRef};
use crate::error::{ToolError, ToolResult};
use crate::reporter::{Reporter, TraceId, TraceUpdate};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What a tool is handed when it runs: the store to put bulk data in, the
/// directory it may not leave, the reporter it draws its chip on, and the
/// session's taint.
pub struct ToolContext {
    /// The bypass channel. Bulk data goes here, a short summary goes to the model.
    pub data_store: Arc<dyn DataStore>,
    /// The sandbox for file operations. Cannot be escaped.
    pub working_dir: PathBuf,
    /// Where a tool announces what it is doing. The shell draws chips from it;
    /// a test collects them instead.
    pub reporter: Arc<dyn Reporter>,
    /// Has a personal-data tool run SUCCESSFULLY at least once this session.
    ///
    /// One-way (see `taint`) and NOT RESET between turns: when the context is
    /// summarized the summary itself may carry personal data, so the taint
    /// carries over with it. Only a real chat reset clears it.
    session_tainted: bool,
}

impl ToolContext {
    /// A fresh, untainted context.
    pub fn new(
        data_store: Arc<dyn DataStore>,
        working_dir: impl Into<PathBuf>,
        reporter: Arc<dyn Reporter>,
    ) -> Self {
        Self {
            data_store,
            working_dir: working_dir.into(),
            reporter,
            session_tainted: false,
        }
    }

    /// Has a personal-data tool run successfully this session. The approval
    /// gate in front of every outbound tool reads this.
    pub fn session_tainted(&self) -> bool {
        self.session_tainted
    }

    /// Called after a SUCCESSFUL call of a personal-data tool. There is no way back.
    pub fn taint(&mut self) {
        self.session_tainted = true;
    }

    /// A real chat reset — anything with session lifetime ends here.
    pub fn reset_chat(&mut self) {
        self.session_tainted = false;
        self.data_store.clear();
    }

    // --- Bypass channel shortcuts ---

    /// Puts bulk data into the store; a reference goes to the model instead.
    pub fn store(&self, kind: &str, summary: &str, body: String) -> SourceRef {
        self.data_store.put(kind, summary, body)
    }

    /// Retrieves data stored in an earlier step.
    pub fn from_store(&self, source_ref: &SourceRef) -> Option<crate::data_store::Record> {
        self.data_store.take(source_ref)
    }

    // --- Chip shortcuts ---

    /// Opens a chip for work that is starting. Finish it with `update_chip`.
    pub fn start_chip(&self, icon: &str, text: &str) -> TraceId {
        self.reporter.start(icon, text)
    }

    /// Fills in what an open chip turned out to be — state, text, the file it
    /// wrote, the raw exchange behind it.
    pub fn update_chip(&self, id: TraceId, update: TraceUpdate) {
        self.reporter.update(id, update);
    }

    // --- Sandbox ---

    /// Resolves the given path against the working directory and verifies that
    /// it does not escape.
    ///
    /// The check runs component by component instead of waiting for the file to
    /// exist: `canonicalize` fails on a file that does not exist, yet on the
    /// WRITE path the target does not exist yet — and that is exactly where the
    /// gate is needed most.
    pub fn resolve_path(&self, path: impl AsRef<Path>) -> ToolResult<PathBuf> {
        use std::path::Component;
        let path = path.as_ref();
        let mut resolved = self.working_dir.clone();
        let relative = path.strip_prefix(&self.working_dir).unwrap_or(path);
        for component in relative.components() {
            match component {
                Component::Normal(p) => resolved.push(p),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !resolved.pop() || !resolved.starts_with(&self.working_dir) {
                        return Err(ToolError::SandboxViolation(path.to_path_buf()));
                    }
                }
                // An absolute path root or a Windows prefix: an attempt to
                // disable the sandbox from the outset.
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolError::SandboxViolation(path.to_path_buf()));
                }
            }
        }
        if resolved.starts_with(&self.working_dir) {
            Ok(resolved)
        } else {
            Err(ToolError::SandboxViolation(path.to_path_buf()))
        }
    }
}
