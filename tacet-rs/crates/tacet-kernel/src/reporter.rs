//! Reporter — the interface through which tools report the chip life cycle.
//!
//! The chip text is produced by the TOOL, not by the model: so that the model
//! cannot hallucinate a step that appears on screen. Every line visible on
//! screen is an event that really happened in the code.
//!
//! It takes `&self` (not `&mut self`): several tools and tasks can feed the
//! same chip stream concurrently; internal synchronization is the concrete
//! implementation's job.

use crate::state::ToolState;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The id of a chip. `u64` — there is no gain worth adding a uuid dependency
/// for, uniqueness within a single process is enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TraceId(pub u64);

/// A chip update. Fields that are not given (None) are PRESERVED — letting a
/// call site reset a field it knows nothing about would let two different
/// stages erase each other's information.
#[derive(Debug, Clone, Default)]
pub struct TraceUpdate {
    pub state: Option<ToolState>,
    pub text: Option<String>,
    pub raw_input: Option<String>,
    pub raw_output: Option<String>,
    pub file_path: Option<PathBuf>,
}

impl TraceUpdate {
    pub fn state(state: ToolState) -> Self {
        Self {
            state: Some(state),
            ..Default::default()
        }
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn raw_input(mut self, v: impl Into<String>) -> Self {
        self.raw_input = Some(v.into());
        self
    }

    pub fn raw_output(mut self, v: impl Into<String>) -> Self {
        self.raw_output = Some(v.into());
        self
    }

    pub fn file_path(mut self, v: impl Into<PathBuf>) -> Self {
        self.file_path = Some(v.into());
        self
    }
}

/// A complete chip record — the view layer reads this.
#[derive(Debug, Clone)]
pub struct ToolTrace {
    pub id: TraceId,
    pub icon: String,
    pub text: String,
    pub state: ToolState,
    pub raw_input: Option<String>,
    pub raw_output: Option<String>,
    pub file_path: Option<PathBuf>,
}

pub trait Reporter: Send + Sync {
    /// Drops a "running" chip and returns its id.
    fn start(&self, icon: &str, text: &str) -> TraceId;
    /// Updates a chip. An unknown id is ignored silently (a notification that
    /// arrives late, after a turn was cancelled, must not be a reason to panic).
    fn update(&self, id: TraceId, update: TraceUpdate);
}

/// The default implementation, which accumulates chips. There is no UI in the
/// header; the engine and eval use this directly, and the view layer reads the
/// traces.
#[derive(Default)]
pub struct TraceCollector {
    counter: AtomicU64,
    traces: Mutex<Vec<ToolTrace>>,
}

impl TraceCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn traces(&self) -> Vec<ToolTrace> {
        self.traces.lock().expect("trace lock").clone()
    }

    /// A new turn — after the previous turn's chips have been carried over.
    pub fn reset(&self) {
        self.traces.lock().expect("trace lock").clear();
    }

    /// Did a `Written` chip drop this turn — the engine's retry gate.
    pub fn world_changed(&self) -> bool {
        self.traces
            .lock()
            .expect("trace lock")
            .iter()
            .any(|t| t.state.changed_world())
    }
}

impl Reporter for TraceCollector {
    fn start(&self, icon: &str, text: &str) -> TraceId {
        let id = TraceId(self.counter.fetch_add(1, Ordering::Relaxed));
        self.traces.lock().expect("trace lock").push(ToolTrace {
            id,
            icon: icon.to_string(),
            text: text.to_string(),
            state: ToolState::Running,
            raw_input: None,
            raw_output: None,
            file_path: None,
        });
        id
    }

    fn update(&self, id: TraceId, u: TraceUpdate) {
        let mut traces = self.traces.lock().expect("trace lock");
        let Some(trace) = traces.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if let Some(s) = u.state {
            trace.state = s;
        }
        if let Some(v) = u.text {
            trace.text = v;
        }
        if let Some(v) = u.raw_input {
            trace.raw_input = Some(v);
        }
        if let Some(v) = u.raw_output {
            trace.raw_output = Some(v);
        }
        if let Some(v) = u.file_path {
            trace.file_path = Some(v);
        }
    }
}

/// An empty reporter for call sites that want no chips (eval, unit tests).
pub struct SilentReporter;

impl Reporter for SilentReporter {
    fn start(&self, _icon: &str, _text: &str) -> TraceId {
        TraceId(0)
    }
    fn update(&self, _id: TraceId, _u: TraceUpdate) {}
}
