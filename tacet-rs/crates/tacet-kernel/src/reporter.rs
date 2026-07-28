//! Reporter — the interface through which tools report the chip life cycle.
//!
//! The chip text is produced by the TOOL, not by the model: so that the model
//! cannot hallucinate a step that appears on screen. Every line visible on
//! screen is an event that really happened in the code.
//!
//! THAT SENTENCE IS ABOUT THE STRUCTURE, NOT THE BYTES. The tool writes the
//! shape of the line, but it interpolates model-written arguments into it
//! (`searching · {query}`), and the model is reachable by whoever wrote the
//! page it just read. So every chip text is sanitised on the way in — see
//! `single_line`. The transparency surface must not be writable by the thing it
//! is watching.
//!
//! It takes `&self` (not `&mut self`): several tools and tasks can feed the
//! same chip stream concurrently; internal synchronization is the concrete
//! implementation's job.

use crate::state::ToolState;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Forces a chip text to be exactly what its name says: ONE LINE, no commands.
///
/// WHY THIS IS NOT COSMETIC. A chip is the TRANSPARENCY SURFACE — it is the
/// line that shows the user what is leaving the machine, and the approval gate
/// leans on the user having read it. The text is not written by the model
/// directly, but it INTERPOLATES model-written arguments (`searching ·
/// {query}`), and the model is reachable by whoever wrote the page it just
/// fetched. The tool-call grammar rejects a raw control byte in a JSON string
/// body but accepts the six-character escape form (backslash-u-0-0-1-b), and
/// `serde_json` turns that straight back into a real ESC when the tool reads
/// the argument. From there the chip is a COMMAND, not a
/// label: `ESC[2K\r` erases the line the user was meant to read and rewrites
/// it, so the query printed on screen and the query sent to the network can be
/// two different things — the transparency gate defeated without ever failing a
/// check. `\n` is just as bad in a different way: extra lines can imitate the
/// approval prompt itself.
///
/// DONE HERE, IN THE ONE FUNNEL EVERY CHIP PASSES THROUGH. The live reporter,
/// the batch render and eval all read the same store; sanitising in the drawing
/// layer would leave the other two, and the next reader of the store would be a
/// third hole. The RAW record (`raw_input`/`raw_output`) is deliberately NOT
/// touched — it exists for diagnosis and must stay faithful; the sanitising
/// belongs on the copy that is drawn.
fn single_line(text: &str) -> String {
    // `is_control()` covers C0, DEL and the C1 range (U+009B is a
    // single-character CSI on a UTF-8 terminal, as good as `ESC [`).
    text.chars()
        .map(|c| if c.is_control() { '·' } else { c })
        .collect()
}

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
            // SANITISED AT THE FUNNEL — see `single_line`.
            text: single_line(text),
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
            trace.text = single_line(&v);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A HOSTILE CHIP TEXT CANNOT REPAINT THE TERMINAL.
    ///
    /// The premise is asserted too, because it is the whole point: the
    /// tool-call grammar refuses a RAW control byte inside a JSON string, but
    /// it accepts the escaped form, and `serde_json` hands the tool a REAL ESC
    /// back. A tool then interpolates that into its chip and all four gates
    /// (name, schema, approval, cancel) have been passed — `web_search` does
    /// not taint the session, so no approval is even asked. `ESC[2K` followed
    /// by a carriage return would then erase the line the user was meant to
    /// read and print a harmless-looking query in its place, while a different
    /// one goes to the network.
    ///
    /// IT GOES THROUGH `TraceCollector::start`, NOT A HAND-BUILT `ToolTrace`:
    /// a test that builds the struct directly would skip the place the fix
    /// lives and stay green through a regression.
    #[test]
    fn a_hostile_chip_text_cannot_repaint_the_terminal() {
        let decoded: String = serde_json::from_str(r#""secret\u001b[2K\rweather""#).unwrap();
        assert!(
            decoded.contains('\u{1b}'),
            "the premise: serde_json really decodes the escape"
        );

        let collector = TraceCollector::new();
        let id = collector.start("globe", &format!("searching · {decoded}"));
        let stored = collector.traces().into_iter().next().unwrap().text;
        for bad in ['\u{1b}', '\r', '\n', '\u{9b}', '\u{7}'] {
            assert!(
                !stored.contains(bad),
                "{bad:?} survived into the chip: {stored:?}"
            );
        }
        // WHAT IS LEAVING THE MACHINE IS STILL SHOWN. Neutralising the commands
        // must not hide the query — that would defeat the same gate from the
        // other side.
        assert!(stored.contains("secret"), "{stored:?}");
        assert!(stored.contains("weather"), "{stored:?}");

        // The update path is a second entrance to the same field.
        collector.update(id, TraceUpdate::default().text("done · a\u{9b}2Kb\nc"));
        let updated = collector.traces().into_iter().next().unwrap().text;
        for bad in ['\u{1b}', '\r', '\n', '\u{9b}'] {
            assert!(!updated.contains(bad), "update path: {updated:?}");
        }

        // The RAW record stays faithful: it exists for diagnosis, and it is the
        // DRAWN copy that has to be safe.
        collector.update(id, TraceUpdate::default().raw_input(decoded.clone()));
        let raw = collector.traces().into_iter().next().unwrap().raw_input;
        assert_eq!(raw.as_deref(), Some(decoded.as_str()));
    }
}
