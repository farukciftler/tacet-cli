//! M4 — the Tasks extension (`io.modelcontextprotocol/tasks`, SEP-2663).
//!
//! Long remote work does not fit in one request/response, and the old answer
//! was a held-open stream. The extension's answer is a TASK: the call returns
//! an id, and the client asks `tasks/get` until it is done. That shape is
//! blocking-HTTP-shaped, so it needs nothing this client refuses to have —
//! which is exactly why this extension is adopted and `subscriptions/listen`
//! (a held stream) is not.
//!
//! Three rules keep polling from becoming a background process:
//!
//! - **The interval is ours to clamp.** A server may suggest one; a suggestion
//!   of 5 ms is a busy loop and a suggestion of an hour is a hang.
//! - **There is a deadline.** When it passes, the WAITING ends — the task may
//!   well still be running on the server, and the sentence the user reads says
//!   so rather than claiming a failure that did not happen.
//! - **The waiting is visible.** Every poll ticks the chip
//!   (`[task] export · running · 12s`), because a program that goes quiet for
//!   two minutes looks broken.

use serde_json::Value;
use std::time::Duration;

/// Below this, polling is a busy loop.
pub const POLL_MIN: Duration = Duration::from_millis(500);

/// Above this, the chip stops feeling alive.
pub const POLL_MAX: Duration = Duration::from_secs(5);

/// Used when the server suggests nothing.
pub const POLL_DEFAULT: Duration = Duration::from_secs(1);

/// How long we are willing to wait in total. The same number as the request
/// timeout: waiting on a task and waiting on a response are the same patience.
pub const DEADLINE: Duration = Duration::from_secs(crate::transport::TIMEOUT_S);

/// Where a task is, as the user sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub id: String,
    /// The server's own word for the state, sanitized.
    pub status: String,
    pub elapsed: Duration,
}

/// Who is told that we are still waiting. The shell paints a chip; eval and
/// scripts use `SilentWatch` and nothing is printed.
pub trait TaskWatch: Send + Sync {
    fn tick(&self, progress: &Progress);
}

pub struct SilentWatch;

impl TaskWatch for SilentWatch {
    fn tick(&self, _progress: &Progress) {}
}

/// The id of the task a `tools/call` result started, if it started one.
///
/// Read defensively: the envelope may name it `task`, or put the id at the top
/// level. ASSUMED until it is measured against a server that returns one.
pub fn task_id(result: &Value) -> Option<String> {
    let says_task = result
        .get("resultType")
        .and_then(Value::as_str)
        .is_some_and(|t| t == "task")
        || result.get("task").is_some();
    if !says_task {
        return None;
    }
    ["taskId", "id"]
        .iter()
        .find_map(|key| {
            result
                .get("task")
                .and_then(|t| t.get(*key))
                .or_else(|| result.get(*key))
                .and_then(Value::as_str)
        })
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// What a `tasks/get` answer says about the state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Still going; the result is not there yet.
    Working(String),
    /// Finished — the payload is the normal `tools/call` result.
    Done(String),
}

impl State {
    pub fn label(&self) -> &str {
        match self {
            Self::Working(status) | Self::Done(status) => status,
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done(_))
    }
}

/// Reads the state out of a `tasks/get` result.
///
/// AN UNKNOWN STATUS IS TREATED AS FINISHED, not as "keep polling". Polling
/// forever on a word we do not recognise is the failure mode that looks like a
/// hang; stopping and handing back whatever content arrived is the one that
/// looks like an answer, and the content is right there in the result.
pub fn state(result: &Value) -> State {
    let status = result
        .get("status")
        .or_else(|| result.get("task").and_then(|t| t.get("status")))
        .and_then(Value::as_str)
        .unwrap_or("completed");
    let status = crate::error::safe_for_screen(status);
    match status.to_ascii_lowercase().as_str() {
        "working" | "running" | "pending" | "queued" | "in_progress" | "submitted" => {
            State::Working(status)
        }
        _ => State::Done(status),
    }
}

/// How long to wait before the next poll, given what the server suggested.
pub fn interval(suggested_ms: Option<u64>) -> Duration {
    suggested_ms
        .map(Duration::from_millis)
        .unwrap_or(POLL_DEFAULT)
        .clamp(POLL_MIN, POLL_MAX)
}

/// The interval a `tasks/get` answer asks for next time, if it asks.
pub fn suggested_ms(result: &Value) -> Option<u64> {
    ["pollIntervalMs", "retryAfterMs"].iter().find_map(|key| {
        result
            .get(*key)
            .or_else(|| result.get("task").and_then(|t| t.get(*key)))
            .and_then(Value::as_u64)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_task_result_is_recognised_and_an_ordinary_one_is_not() {
        assert_eq!(
            task_id(&json!({"resultType": "task", "taskId": "t-1"})),
            Some("t-1".into())
        );
        assert_eq!(
            task_id(&json!({"task": {"id": "t-2", "status": "working"}})),
            Some("t-2".into())
        );
        assert_eq!(
            task_id(&json!({"content": [{"type": "text", "text": "ok"}]})),
            None
        );
        // A task envelope with no id is not a task we can follow.
        assert_eq!(task_id(&json!({"resultType": "task"})), None);
    }

    #[test]
    fn the_interval_is_ours_to_clamp() {
        // A busy loop and a hang, both refused.
        assert_eq!(interval(Some(5)), POLL_MIN);
        assert_eq!(interval(Some(3_600_000)), POLL_MAX);
        assert_eq!(interval(Some(2_000)), Duration::from_secs(2));
        assert_eq!(interval(None), POLL_DEFAULT);
    }

    #[test]
    fn an_unknown_status_stops_the_waiting_rather_than_extending_it() {
        assert!(matches!(
            state(&json!({"status": "working"})),
            State::Working(_)
        ));
        assert!(state(&json!({"status": "completed"})).is_done());
        assert!(state(&json!({"status": "failed"})).is_done());
        assert!(
            state(&json!({"status": "banana"})).is_done(),
            "a word we do not know must not become an infinite loop"
        );
        // No status at all: the answer is the answer.
        assert!(state(&json!({"content": []})).is_done());
    }

    #[test]
    fn a_hostile_status_cannot_paint_the_chip() {
        let painted = state(&json!({"status": "\u{1b}[2Jrunning".to_string()}));
        assert!(!painted.label().contains('\u{1b}'));
    }
}
