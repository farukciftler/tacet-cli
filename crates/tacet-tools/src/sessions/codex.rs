//! Codex CLI sessions: discovery (RG-11), rollout parsing (RG-12) and the plan
//! the agent last set with `update_plan` (RG-13).
//!
//! NOT MEASURED ON A REAL INSTALL. Codex was not installed on the machine this
//! was written on (10 Oct 2026). Everything below follows the format as the
//! source writes it — openai/codex @ a06545b: `rollout/src/recorder.rs` (the
//! line envelope), `history/src/rollout_payload.rs` (the item types),
//! `rollout/src/policy.rs` (which items each history mode persists),
//! `rollout/src/ordinal.rs`, `rollout/src/session_index.rs`,
//! `rollout/src/writer_lock.rs`, `protocol/src/plan_tool.rs` and
//! `core/src/tools/handlers/plan.rs` — and is tested against fixtures written
//! from that source. Until the fixture corpus is run against a real
//! `~/.codex`, `version::KNOWN` has no Codex row, so every read carries the
//! "not measured" warning and no write is allowed.
//!
//! THE LAYOUT. `$CODEX_HOME` (else `~/.codex`) holds
//! `sessions/YYYY/MM/DD/rollout-<start time>-<rollout id>.jsonl`, the
//! same names under `archived_sessions/`, `session_index.jsonl` (the names a
//! user gave threads; append-only, the last line for an id wins) and
//! `thread-writer-locks/<thread id>.lock` while a Codex process owns the
//! thread. Codex's own compression worker turns rollouts into `.jsonl.zst`;
//! those are LISTED (so the user learns they exist) and refused on read with
//! the `zstd -d` advice. `auth.json` sits in the same directory: discovery
//! never walks the root itself, only the two session trees and the index, and
//! every file is opened through `secrets`, which refuses it by name and by
//! link.
//!
//! ONE LINE IS `{"timestamp", "ordinal"?, "type", "payload"}`. Two history
//! modes, recorded in `session_meta.history_mode`:
//!
//! * `legacy` (the default): no ordinals; the user-visible transcript is
//!   `event_msg` `user_message`/`agent_message`, the model's context is
//!   `response_item`.
//! * `paginated`: every line carries an `ordinal`, counting up from 0 or from
//!   the end of the `history_base` prefix it was forked from, and
//!   `user_message`/`agent_message` events are NOT persisted —
//!   `item_completed` events carry the visible items instead. A forked
//!   paginated thread does not copy its parent's lines: they stay in the
//!   prefix rollout `history_base` names, and `read` follows that link.
//!
//! WHICH STREAM BECOMES `entries`. The model-visible one (`response_item` plus
//! `compacted`): it is what the agent was working from, it carries the tool
//! calls and their output, and in legacy mode `event_msg` repeats its messages
//! one for one — reading both would show every message twice. The `event_msg`
//! stream is used for the title and, only when a rollout has no
//! `response_item` at all, as the transcript.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::handoff::Status;
use super::model::{
    AgentKind, AgentSession, Entry, EntryKind, PlanItem, Role, SessionError, SessionSource,
    SessionSummary, newest_first, parse_iso_utc, validate_id,
};
use super::secrets::{self, Refusal};

/// Active sessions, `YYYY/MM/DD` below it.
pub const SESSIONS_DIR: &str = "sessions";
/// Sessions the user archived. Read like active ones; a write path must refuse
/// them (Codex's own `external-agent-migration` fails closed there), which is
/// why `RolloutFile::archived` exists.
pub const ARCHIVED_DIR: &str = "archived_sessions";
pub const INDEX_FILE: &str = "session_index.jsonl";
pub const LOCKS_DIR: &str = "thread-writer-locks";

/// The largest rollout read in full. Over it the user gets `TooLarge` with the
/// size, not an allocation the size of the file.
pub const READ_CAP: u64 = 128 << 20;
/// What a listing reads from the front of a rollout: `session_meta` and the
/// first user message. `session_meta` carries the base instructions — tens of
/// kilobytes — so this is generous; a head that ends before the first complete
/// line falls back to a full read.
pub const HEAD_BYTES: u64 = 256 << 10;
/// What a listing reads from the end, for the last timestamp.
pub const TAIL_BYTES: u64 = 64 << 10;
/// `session_index.jsonl` is one short line per rename.
pub const INDEX_CAP: u64 = 8 << 20;
/// A title is a list row, not a paragraph.
const TITLE_CHARS: usize = 100;

/// How deep below `sessions/` and `archived_sessions/` the walk goes.
/// `YYYY/MM/DD` is three; one more for a layout change, and a bound so a
/// directory loop made of real directories cannot be walked forever.
const MAX_DEPTH: usize = 4;

/// How many `history_base` links `read` follows. A fork of a fork of a fork is
/// three; the bound is what stops two rollouts that name each other.
const MAX_PREFIX_DEPTH: usize = 8;

/// What a compressed rollout is called in a listing when the index has no name
/// for it. `read` returns `Refusal::Compressed`, which says how to get it read.
pub const COMPRESSED_TITLE: &str =
    "compressed rollout (.jsonl.zst): run `zstd -d` on it to read it";

/// What Codex's `update_plan` handler answers when it ACCEPTED a plan
/// (`core/src/tools/handlers/plan.rs`, `PLAN_UPDATED_MESSAGE`). Any other
/// output for that call is an error the model was shown — Plan mode refuses
/// the tool — and the plan the user saw did not change.
pub const PLAN_UPDATED: &str = "Plan updated";

/// The openings of text Codex puts into a user-role message itself — the
/// environment block, AGENTS.md, a shell command the user ran with `!`, an
/// interrupted turn — rather than text the user typed. From the contextual
/// fragment matchers in `core/src/context/` and `protocol/src/protocol.rs` at
/// a06545b. Shown as `Role::System`, and never used as a title: the first
/// user-role message of every Codex session is one of these.
pub(crate) const INJECTED_PREFIXES: &[&str] = &[
    "<environment_context>",
    "<environments_instructions>",
    "<user_instructions>",
    "# AGENTS.md instructions",
    "<INSTRUCTIONS>",
    "<user_shell_command>",
    "<turn_aborted>",
    "<subagent_notification>",
    "<apps_instructions>",
    "<skills_instructions>",
    "<plugins_instructions>",
    "<recommended_plugins>",
    "<collaboration_mode>",
    "<model_switch>",
    "<current_time_reminder>",
    "<image_resize_notice>",
    "<user_verification_notice>",
    "<content_filter_guidance>",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryMode {
    #[default]
    Legacy,
    Paginated,
}

/// `session_meta.history_base`: the rollout whose lines below
/// `end_ordinal_exclusive` are the start of this thread's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryBase {
    /// Named `thread_id` in the file; Codex's own comment says to treat it as
    /// a ROLLOUT id — the id in the prefix file's name.
    pub rollout_id: String,
    pub end_ordinal_exclusive: u64,
}

/// A rollout, parsed. Richer than `AgentSession`: a write-back path (RG-24)
/// needs the mode, the last ordinal and the lines that did not parse, and a
/// listing needs the first user message before the index name is known.
#[derive(Debug, Clone, Default)]
pub struct Rollout {
    /// `session_meta.id` of the FIRST `session_meta` in this file. A legacy
    /// fork repeats its parent's meta further down; the first one is this
    /// thread's. After a `thread/revert` it can differ from the id in the file
    /// name (Codex: "a reverted thread's filename carries a distinct rollout
    /// ID"), so it is kept apart from the id a listing shows.
    pub meta_id: Option<String>,
    pub forked_from: Option<String>,
    /// The latest `turn_context.cwd`, else `session_meta.cwd`: where the work
    /// was going on when the session stopped.
    pub cwd: Option<PathBuf>,
    pub cli_version: Option<String>,
    pub history_mode: HistoryMode,
    pub history_base: Option<HistoryBase>,
    /// The prefix rollout `history_base` names could not be read (missing,
    /// compressed, refused, or a chain longer than `MAX_PREFIX_DEPTH`): the
    /// entries start at the fork, and the caller should say so.
    pub missing_prefix: Option<String>,
    pub started: Option<i64>,
    /// The LATEST timestamp in this file, not the last line's: nothing
    /// promises the lines are in clock order.
    pub updated: Option<i64>,
    /// The first message the user typed (not an injected context block).
    pub first_user_message: Option<String>,
    pub entries: Vec<Entry>,
    pub plan: Vec<PlanItem>,
    /// 1-based numbers of lines in THIS file that were not a rollout line: not
    /// JSON, no `type`, no `payload`, or — in paginated mode — no `ordinal`.
    pub skipped: Vec<usize>,
    /// The highest ordinal in this file (paginated mode). An append continues
    /// after it — with `checked_add`: a damaged file can hold `u64::MAX`, and
    /// then there is no next ordinal to write.
    pub last_ordinal: Option<u64>,
    /// Ordinals missing between the lowest and highest in this file.
    pub ordinal_gaps: u64,
    /// Lines of this file that parsed as rollout lines, of any type.
    pub valid_lines: usize,
}

/// One rollout line that has the shape, before its payload is looked at.
#[derive(Debug, Clone)]
struct Line {
    number: usize,
    ordinal: Option<u64>,
    ts: Option<i64>,
    kind: String,
    payload: Value,
}

/// The envelope check, line by line. Codex's own decoder requires `timestamp`
/// too; a line without one is kept here (with no time) because the payload is
/// still the session's, and losing a message over a missing clock is worse
/// than showing it without one.
fn split_lines(text: &str, skipped: &mut Vec<usize>) -> Vec<Line> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let number = i + 1;
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let Ok(Value::Object(mut obj)) = serde_json::from_str::<Value>(raw) else {
            skipped.push(number);
            continue;
        };
        let Some(kind) = obj.get("type").and_then(Value::as_str).map(str::to_string) else {
            skipped.push(number);
            continue;
        };
        let Some(payload) = obj.remove("payload").filter(|p| !p.is_null()) else {
            skipped.push(number);
            continue;
        };
        out.push(Line {
            number,
            ordinal: obj.get("ordinal").and_then(Value::as_u64),
            ts: obj
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_iso_utc),
            kind,
            payload,
        });
    }
    out
}

/// Text out of the shapes Codex uses for content: a string, an array of
/// `{type, text}` parts, an object with `text` or `content`. Images, encrypted
/// content and anything else without text contribute nothing.
fn content_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(content_text)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(o) => {
            if let Some(t) = o.get("text").and_then(Value::as_str) {
                t.to_string()
            } else if let Some(c) = o.get("content") {
                content_text(c)
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

fn injected(text: &str) -> bool {
    let t = text.trim_start();
    INJECTED_PREFIXES.iter().any(|p| t.starts_with(p))
}

/// Codex calls a user message contextual when ANY of its parts is a fragment
/// (`is_contextual_user_fragment` is applied with `any`), so a typed message
/// sent in the same item as the environment block is still the block's.
fn injected_message(content: &Value) -> bool {
    match content {
        Value::Array(parts) => parts.iter().any(|p| injected(&content_text(p))),
        other => injected(&content_text(other)),
    }
}

/// RG-13: the plan an `update_plan` call's `arguments` set, or `None` when
/// Codex would have REJECTED the call.
///
/// MIRRORS `UpdatePlanArgs` EXACTLY. Codex parses the arguments with
/// `#[serde(deny_unknown_fields)]` on both the call and each step, a required
/// `plan`, and a three-value status enum; a call that fails any of that gets
/// "failed to parse function arguments" and the plan on screen does not
/// change. Dropping only the bad step — what a lenient reader would do — would
/// show a plan the user never saw.
///
/// Every accepted call REPLACES the whole plan (the tool takes the full list
/// each time), so only the last accepted call matters.
pub fn plan_from_update(arguments: &str) -> Option<Vec<PlanItem>> {
    let Value::Object(args) = serde_json::from_str::<Value>(arguments).ok()? else {
        return None;
    };
    if args.keys().any(|k| k != "explanation" && k != "plan") {
        return None;
    }
    if !matches!(
        args.get("explanation"),
        None | Some(Value::Null) | Some(Value::String(_))
    ) {
        return None;
    }
    let steps = args.get("plan")?.as_array()?;
    steps
        .iter()
        .map(|s| {
            let s = s.as_object()?;
            if s.len() != 2 {
                return None;
            }
            Some(PlanItem {
                title: s.get("step")?.as_str()?.to_string(),
                status: Status::from_codex_plan(s.get("status")?.as_str()?)?,
            })
        })
        .collect()
}

/// The plan as the user saw it: set by an accepted `update_plan`, put back if
/// the call's output says Codex refused it.
#[derive(Default)]
struct PlanTracker {
    current: Vec<PlanItem>,
    /// The call that set `current`, and the plan before it.
    pending: Option<(String, Vec<PlanItem>)>,
}

impl PlanTracker {
    fn call(&mut self, call_id: &str, arguments: &str) {
        if let Some(plan) = plan_from_update(arguments) {
            let before = std::mem::replace(&mut self.current, plan);
            self.pending = Some((call_id.to_string(), before));
        }
    }

    /// Arguments that parse can still be refused: Plan mode answers
    /// "update_plan is a TODO/checklist tool and is not allowed in Plan mode".
    /// Only the output says which happened. A call with no output yet (the
    /// session stopped mid-call) keeps its plan: the event that redraws the
    /// plan is sent before the output is written.
    fn output(&mut self, call_id: &str, text: &str) {
        if self.pending.as_ref().is_some_and(|(id, _)| id == call_id)
            && let Some((_, before)) = self.pending.take()
            && text.trim() != PLAN_UPDATED
        {
            self.current = before;
        }
    }
}

/// The normalised name of a `TurnItem` type: the source serialises them as
/// `UserMessage`; `user_message` and `userMessage` are accepted as the same.
fn item_kind(v: &Value) -> String {
    v.get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .filter(|c| *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn entry(
    role: Role,
    kind: EntryKind,
    text: String,
    tool: Option<String>,
    ts: Option<i64>,
) -> Entry {
    Entry {
        role,
        kind,
        text,
        tool,
        ts,
    }
}

/// Paginated lines in ordinal order, the LAST line written for an ordinal
/// kept. Codex's writer gives each line the next ordinal, so file order and
/// ordinal order agree on a healthy file; sorting is what keeps a damaged or
/// hand-edited one readable, and a repeated ordinal can only be a rewrite
/// later in the file (inferred from the append-only writer; not measured).
fn by_ordinal(mut lines: Vec<Line>) -> Vec<Line> {
    lines.sort_by_key(|l| (l.ordinal, l.number));
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    for l in lines {
        match out.last_mut() {
            Some(prev) if prev.ordinal == l.ordinal => *prev = l,
            _ => out.push(l),
        }
    }
    out
}

fn history_base(meta: &Value) -> Option<HistoryBase> {
    let base = meta.get("history_base")?;
    let rollout_id = base.get("thread_id")?.as_str()?.to_string();
    validate_id(&rollout_id).ok()?;
    Some(HistoryBase {
        rollout_id,
        end_ordinal_exclusive: base.get("end_ordinal_exclusive")?.as_u64()?,
    })
}

/// RG-12: parses a whole rollout (or a head of one). Never fails: a line that
/// is not a rollout line is skipped and counted, so one corrupt line costs one
/// line, not the session. A paginated fork's prefix is NOT followed here (it is
/// another file); `CodexSource::read_rollout` does that.
pub fn parse_rollout(text: &str) -> Rollout {
    let mut skipped = Vec::new();
    let own = split_lines(text, &mut skipped);
    assemble(Vec::new(), own, skipped)
}

/// `prefix`: lines inherited from `history_base` rollouts, already cut and in
/// order. `own`: this file's lines.
fn assemble(prefix: Vec<Line>, own: Vec<Line>, skipped: Vec<usize>) -> Rollout {
    let mut r = Rollout {
        skipped,
        valid_lines: own.len(),
        ..Rollout::default()
    };

    // The meta comes from FILE order: it is the first line Codex writes, and
    // it carries the mode that decides how every other line is ordered.
    let meta = own.iter().find(|l| l.kind == "session_meta");
    let mut meta_cwd = None;
    if let Some(m) = meta {
        let p = &m.payload;
        let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
        r.meta_id = s("id").or_else(|| s("session_id"));
        r.forked_from = s("forked_from_id");
        r.cli_version = s("cli_version");
        r.history_base = history_base(p);
        meta_cwd = s("cwd").map(PathBuf::from);
        r.started = p
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_utc)
            .or(m.ts);
    }
    let declared = meta.and_then(|m| m.payload.get("history_mode").and_then(Value::as_str));
    r.history_mode = match declared {
        Some("paginated") => HistoryMode::Paginated,
        Some("legacy") => HistoryMode::Legacy,
        // No meta in view (a head cut short, a damaged file) or a mode this
        // reader has not seen: the lines themselves say whether they are
        // numbered.
        _ if own.iter().any(|l| l.ordinal.is_some()) => HistoryMode::Paginated,
        _ => HistoryMode::Legacy,
    };
    r.updated = own.iter().filter_map(|l| l.ts).max();

    let own: Vec<Line> = match r.history_mode {
        HistoryMode::Legacy => own,
        HistoryMode::Paginated => {
            // Codex itself refuses to append to a paginated rollout whose
            // last line has no ordinal; a reader that must not lose the other
            // lines skips such a line instead and reports it.
            let (numbered, unnumbered): (Vec<Line>, Vec<Line>) =
                own.into_iter().partition(|l| l.ordinal.is_some());
            r.skipped.extend(unnumbered.iter().map(|l| l.number));
            r.skipped.sort_unstable();
            let numbered = by_ordinal(numbered);
            if let (Some(lo), Some(hi)) = (
                numbered.first().and_then(|l| l.ordinal),
                numbered.last().and_then(|l| l.ordinal),
            ) {
                r.last_ordinal = Some(hi);
                // Not `hi - lo + 1 - len`: the span 0..=u64::MAX has 2^64
                // ordinals, one more than a u64 holds, and an ordinal that
                // large is one line of an untrusted file away. `+ 1` panicked
                // in a debug build and wrapped the count to 0 in a release one
                // (corpus case 0199e000-0047-…-003). `by_ordinal` leaves one
                // line per ordinal, so `len - 1 <= hi - lo` and this is exact.
                r.ordinal_gaps = (hi - lo) - (numbered.len() as u64 - 1);
            }
            numbered
        }
    };
    if r.started.is_none() {
        r.started = prefix.iter().chain(&own).find_map(|l| l.ts);
    }

    let mut model: Vec<Entry> = Vec::new();
    let mut ui: Vec<Entry> = Vec::new();
    let mut saw_response_item = false;
    let mut first_typed_in_model: Option<String> = None;
    let mut first_typed_in_events: Option<String> = None;
    let mut turn_cwd: Option<PathBuf> = None;
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut plan = PlanTracker::default();

    // A fork's title is ITS first question, not its parent's: the listing
    // reads only this file's head, and `read` must name the session the same
    // way or the two disagree about which session the user picked.
    let inherited = prefix.len();
    for (n, l) in prefix.iter().chain(&own).enumerate() {
        let own_line = n >= inherited;
        let p = &l.payload;
        let ts = l.ts;
        match l.kind.as_str() {
            "turn_context" => {
                if let Some(c) = p.get("cwd").and_then(Value::as_str) {
                    turn_cwd = Some(PathBuf::from(c));
                }
            }
            "compacted" => {
                // A remote compaction can leave `message` empty and put the
                // kept history in `replacement_history` instead.
                let mut text = p
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if text.trim().is_empty()
                    && let Some(Value::Array(items)) = p.get("replacement_history")
                {
                    text = items
                        .iter()
                        .filter(|i| i.get("type").and_then(Value::as_str) == Some("message"))
                        .map(|i| content_text(i.get("content").unwrap_or(&Value::Null)))
                        .filter(|t| !t.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                }
                let e = entry(Role::System, EntryKind::Summary, text, None, ts);
                model.push(e.clone());
                ui.push(e);
            }
            "response_item" => {
                saw_response_item = true;
                let s = |k: &str| p.get(k).and_then(Value::as_str);
                match s("type").unwrap_or_default() {
                    "message" => {
                        let content = p.get("content").unwrap_or(&Value::Null);
                        let text = content_text(content);
                        let role = match s("role").unwrap_or_default() {
                            "user" if injected_message(content) => Role::System,
                            "user" => {
                                if own_line
                                    && first_typed_in_model.is_none()
                                    && !text.trim().is_empty()
                                {
                                    first_typed_in_model = Some(text.clone());
                                }
                                Role::User
                            }
                            "assistant" => Role::Assistant,
                            // `developer` and `system`: Codex's own instructions.
                            _ => Role::System,
                        };
                        model.push(entry(role, EntryKind::Text, text, None, ts));
                    }
                    // A message from another Codex agent (multi-agent mode):
                    // input to this one, written by neither the user nor it.
                    "agent_message" => {
                        let text = content_text(p.get("content").unwrap_or(&Value::Null));
                        if !text.is_empty() {
                            model.push(entry(Role::System, EntryKind::Text, text, None, ts));
                        }
                    }
                    "reasoning" => {
                        // `encrypted_content` is opaque by design; only the
                        // summary and any raw content are text.
                        let text = [
                            content_text(p.get("summary").unwrap_or(&Value::Null)),
                            content_text(p.get("content").unwrap_or(&Value::Null)),
                        ]
                        .into_iter()
                        .filter(|t| !t.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                        if !text.is_empty() {
                            model.push(entry(Role::Assistant, EntryKind::Thinking, text, None, ts));
                        }
                    }
                    kind @ ("function_call" | "custom_tool_call" | "tool_search_call") => {
                        let name = match (kind, s("name")) {
                            ("tool_search_call", _) => "tool_search".to_string(),
                            (_, Some(n)) => match s("namespace") {
                                Some(ns) => format!("{ns}.{n}"),
                                None => n.to_string(),
                            },
                            (_, None) => String::new(),
                        };
                        let call_id = s("call_id").unwrap_or_default();
                        if !call_id.is_empty() {
                            tool_names.insert(call_id.to_string(), name.clone());
                        }
                        // `arguments` is a JSON STRING for a function call
                        // (`models.rs`: "the Responses API returns the function
                        // call arguments as a *string*"); `input` for a custom
                        // tool; any JSON for a tool search.
                        let args = p.get("arguments").or_else(|| p.get("input"));
                        let args_text = match args {
                            Some(Value::String(a)) => a.clone(),
                            Some(other) => other.to_string(),
                            None => String::new(),
                        };
                        // Only a PLAIN `update_plan` function call reaches the
                        // plan handler: a namespaced one is some MCP server's
                        // tool, and a custom-tool payload is refused as
                        // "unsupported payload".
                        if kind == "function_call"
                            && name == "update_plan"
                            && let Some(Value::String(a)) = args
                        {
                            plan.call(call_id, a);
                        }
                        model.push(entry(
                            Role::Assistant,
                            EntryKind::ToolCall,
                            args_text,
                            Some(name),
                            ts,
                        ));
                    }
                    "local_shell_call" => {
                        if let Some(id) = s("call_id") {
                            tool_names.insert(id.to_string(), "local_shell".into());
                        }
                        let cmd = p
                            .pointer("/action/command")
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .unwrap_or_default();
                        model.push(entry(
                            Role::Assistant,
                            EntryKind::ToolCall,
                            cmd,
                            Some("local_shell".into()),
                            ts,
                        ));
                    }
                    "web_search_call" => {
                        let query = p
                            .pointer("/action/query")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        model.push(entry(
                            Role::Assistant,
                            EntryKind::ToolCall,
                            query,
                            Some("web_search".into()),
                            ts,
                        ));
                    }
                    "function_call_output" | "custom_tool_call_output" | "tool_search_output" => {
                        let call_id = s("call_id").unwrap_or_default();
                        let tool = tool_names.get(call_id).cloned();
                        let text = content_text(p.get("output").unwrap_or(&Value::Null));
                        plan.output(call_id, &text);
                        model.push(entry(Role::Tool, EntryKind::ToolResult, text, tool, ts));
                    }
                    // A Responses-API item type this reader does not know is a
                    // VALID line with nothing to show, not a malformed one.
                    _ => {}
                }
            }
            "event_msg" => {
                let s = |k: &str| p.get(k).and_then(Value::as_str);
                let (role, kind, text) = match s("type").unwrap_or_default() {
                    "user_message" => (
                        Role::User,
                        EntryKind::Text,
                        s("message").map(str::to_string),
                    ),
                    "agent_message" => (
                        Role::Assistant,
                        EntryKind::Text,
                        s("message").map(str::to_string),
                    ),
                    "agent_reasoning" => (
                        Role::Assistant,
                        EntryKind::Thinking,
                        s("text").map(str::to_string),
                    ),
                    // Paginated mode persists these instead of the three above.
                    "item_completed" => {
                        let item = p.get("item").unwrap_or(&Value::Null);
                        let (role, kind) = match item_kind(item).as_str() {
                            "usermessage" => (Role::User, EntryKind::Text),
                            "agentmessage" => (Role::Assistant, EntryKind::Text),
                            "reasoning" => (Role::Assistant, EntryKind::Thinking),
                            _ => continue,
                        };
                        let text = match item.get("content") {
                            Some(c) => content_text(c),
                            None => content_text(item.get("summary_text").unwrap_or(&Value::Null)),
                        };
                        (role, kind, Some(text))
                    }
                    _ => continue,
                };
                let text = text.unwrap_or_default();
                if role == Role::User
                    && own_line
                    && first_typed_in_events.is_none()
                    && !text.trim().is_empty()
                    && !injected(&text)
                {
                    first_typed_in_events = Some(text.clone());
                }
                ui.push(entry(role, kind, text, None, ts));
            }
            // session_meta was read above; token_count records, world_state,
            // realtime items and types added after a06545b carry nothing a
            // transcript shows.
            _ => {}
        }
    }

    r.cwd = turn_cwd.or(meta_cwd);
    // The user-visible event is what the user typed, verbatim; the model's
    // copy of it can carry Codex's own wrapping.
    r.first_user_message = first_typed_in_events.or(first_typed_in_model);
    r.entries = if saw_response_item { model } else { ui };
    r.plan = plan.current;
    r
}

/// First line, at most `TITLE_CHARS` characters.
fn title_of(text: &str) -> String {
    let line = text.trim().lines().next().unwrap_or_default().trim();
    let mut out: String = line.chars().take(TITLE_CHARS).collect();
    if line.chars().nth(TITLE_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// A UUID as Codex writes thread and rollout ids: 8-4-4-4-12 hex digits.
fn is_uuid(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// `rollout-<YYYY-MM-DDThh-mm-ss>-<uuid>.jsonl[.zst]` → (id, compressed).
///
/// STRICT, as Codex's own `RolloutFileName::parse` is: the timestamp shape and
/// a UUID, or the file is not a rollout. A looser match listed
/// `rollout-notes.jsonl` as a session with the id `notes`.
pub fn rollout_id(name: &str) -> Option<(String, bool)> {
    let (stem, compressed) = match name.strip_suffix(".jsonl.zst") {
        Some(s) => (s, true),
        None => (name.strip_suffix(".jsonl")?, false),
    };
    let rest = stem.strip_prefix("rollout-")?;
    let ts = rest.get(..20)?;
    let shaped = ts.bytes().enumerate().all(|(i, b)| match i {
        10 => b == b'T',
        4 | 7 | 13 | 16 | 19 => b == b'-',
        _ => b.is_ascii_digit(),
    });
    let id = rest.get(20..)?;
    (shaped && is_uuid(id)).then(|| (id.to_string(), compressed))
}

/// A rollout file found on disk.
#[derive(Debug, Clone)]
struct Found {
    id: String,
    path: PathBuf,
    compressed: bool,
    archived: bool,
}

fn walk(dir: &Path, depth: usize, archived: bool, out: &mut Vec<Found>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        // Skipped without opening, and without a "refused" line that would
        // draw attention to the name.
        if secrets::denied_by_name(&path).is_some() {
            continue;
        }
        let Ok(kind) = e.file_type() else { continue };
        // `DirEntry::file_type` does not follow links: a linked directory is
        // not walked, so a link to `/` or to the root itself cannot turn a
        // listing into a crawl. A linked FILE is listed and then vetted by the
        // gate on read, which resolves it.
        if kind.is_dir() {
            walk(&path, depth + 1, archived, out);
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some((id, compressed)) = rollout_id(name) {
            out.push(Found {
                id,
                path,
                compressed,
                archived,
            });
        }
    }
}

/// Every rollout under `root`, one per id: an active session before an
/// archived copy, and a plain `.jsonl` before a `.jsonl.zst` of the same id —
/// the refusal tells the user to decompress NEXT TO the original, and once
/// they have, the readable file must be the one used. (Codex keeps both for a
/// moment while it compresses, and prefers the plain one too.)
fn rollouts(root: &Path) -> Vec<Found> {
    let mut found = Vec::new();
    walk(&root.join(SESSIONS_DIR), 1, false, &mut found);
    walk(&root.join(ARCHIVED_DIR), 1, true, &mut found);
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<Found> = Vec::new();
    for f in found {
        match by_id.get(&f.id) {
            Some(&at) if out[at].compressed && !f.compressed => out[at] = f,
            Some(_) => {}
            None => {
                by_id.insert(f.id.clone(), out.len());
                out.push(f);
            }
        }
    }
    out
}

/// `session_index.jsonl`: id → the name the user gave the thread
/// (`{"id","thread_name","updated_at"}`). Append-only, so the LAST line for an
/// id is its current name.
pub fn index_names(root: &Path) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let Ok(text) = secrets::read_text(&root.join(INDEX_FILE), INDEX_CAP) else {
        return names;
    };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let (Some(id), Some(name)) = (
            v.get("id").and_then(Value::as_str),
            v.get("thread_name").and_then(Value::as_str),
        ) else {
            continue;
        };
        if validate_id(id).is_ok() && !name.trim().is_empty() {
            names.insert(id.to_string(), name.to_string());
        }
    }
    names
}

/// Is a Codex process writing this thread? `thread-writer-locks/<id>.lock`
/// exists.
///
/// WHY PRESENCE AND NOT THE LOCK. Codex holds an advisory lock on the file
/// while it owns the thread and deletes the file when it lets go
/// (`writer_lock.rs`); taking the lock to test it would need the file opened
/// for writing, and this module opens nothing but transcripts through the
/// gate. A Codex that crashed leaves the file behind until the next Codex
/// start sweeps stale locks, so a dead session can read as live — the safe
/// side for every write path; the price is a wrong "live" in a listing.
pub fn is_live(root: &Path, id: &str) -> bool {
    validate_id(id).is_ok()
        && std::fs::symlink_metadata(root.join(LOCKS_DIR).join(format!("{id}.lock"))).is_ok()
}

/// The first `bytes` bytes, cut after the last complete line, and whether
/// anything was cut. Opened through the gate like everything else; the gate
/// has no head reader because only this reader needs one.
fn read_head(path: &Path, bytes: u64) -> Result<(String, bool), Refusal> {
    let handle = secrets::open(path)?;
    let mut buf = Vec::new();
    handle
        .take(bytes + 1)
        .read_to_end(&mut buf)
        .map_err(|error| Refusal::Io {
            path: path.to_path_buf(),
            error,
        })?;
    let cut = buf.len() as u64 > bytes;
    if cut {
        buf.truncate(bytes as usize);
        let end = buf.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        buf.truncate(end);
    }
    Ok((String::from_utf8_lossy(&buf).into_owned(), cut))
}

fn mtime(path: &Path) -> Option<i64> {
    let modified = std::fs::symlink_metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// The roots Codex itself would use: `$CODEX_HOME`, else `<home>/.codex`.
///
/// A `CODEX_HOME` that is set but relative names a directory relative to
/// whatever directory Codex was started in, which this process cannot know;
/// reading `~/.codex` instead would list another configuration's sessions as
/// if they were the ones the user pointed Codex at. So it yields NO root
/// rather than a guess.
pub fn roots_from(codex_home: Option<OsString>, home: Option<OsString>) -> Vec<PathBuf> {
    if let Some(dir) = codex_home.filter(|v| !v.is_empty()) {
        let dir = PathBuf::from(dir);
        return if dir.is_absolute() {
            vec![dir]
        } else {
            Vec::new()
        };
    }
    home.filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .map(|h| vec![h.join(".codex")])
        .unwrap_or_default()
}

/// `roots_from` over named environment variables, the lookup passed in.
///
/// WHY THE LOOKUP IS AN ARGUMENT. Which variable names are read is the part
/// worth testing, and a test that sets `CODEX_HOME` with `set_var` races every
/// other test thread; starting the test binary again with its own environment
/// is a runtime spawn site, which `network_monopoly` (rightly) refuses to
/// leave unreviewed. So `default_roots` passes the real lookup and the tests
/// pass a table.
pub fn roots_from_env(var: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let home = var("HOME");
    #[cfg(windows)]
    let home = home.or_else(|| var("USERPROFILE"));
    roots_from(var("CODEX_HOME"), home)
}

/// A rollout as `read_rollout` returns it: the parse, and where it came from.
#[derive(Debug, Clone)]
pub struct RolloutFile {
    pub path: PathBuf,
    /// Under `archived_sessions/`. Readable; never a write target.
    pub archived: bool,
    pub rollout: Rollout,
}

/// The Codex CLI reader.
#[derive(Debug, Clone, Copy, Default)]
pub struct CodexSource;

impl CodexSource {
    /// `read`, keeping what `AgentSession` has no field for: the history mode,
    /// the last ordinal, the lines that were skipped, the prefix that could
    /// not be found, and the file it came from. A write-back path starts here.
    pub fn read_rollout(&self, root: &Path, id: &str) -> Result<RolloutFile, SessionError> {
        validate_id(id)?;
        let all = rollouts(root);
        let found = all
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| SessionError::NotFound {
                agent: AgentKind::Codex,
                id: id.to_string(),
            })?;
        // A `.jsonl.zst` goes to the gate too, and comes back as
        // `Refusal::Compressed` with the `zstd -d` advice: one message, one
        // place.
        let text = secrets::read_text(&found.path, READ_CAP)?;
        let mut skipped = Vec::new();
        let own = split_lines(&text, &mut skipped);
        if own.is_empty() {
            return Err(SessionError::Format {
                path: found.path.clone(),
                line: skipped.first().copied(),
                why: "no line of this file is a Codex rollout line".into(),
            });
        }
        let base = own
            .iter()
            .find(|l| l.kind == "session_meta")
            .and_then(|m| history_base(&m.payload));
        let (prefix, missing) = match &base {
            Some(b) => match prefix_lines(&all, b, 1) {
                Ok(lines) => (lines, None),
                Err(why) => (Vec::new(), Some(why)),
            },
            None => (Vec::new(), None),
        };
        let mut rollout = assemble(prefix, own, skipped);
        rollout.missing_prefix = missing;
        Ok(RolloutFile {
            path: found.path.clone(),
            archived: found.archived,
            rollout,
        })
    }

    fn summarize(
        &self,
        root: &Path,
        f: &Found,
        names: &HashMap<String, String>,
    ) -> Option<SessionSummary> {
        let base = |title: String, rollout: Option<&Rollout>| SessionSummary {
            agent: AgentKind::Codex,
            id: f.id.clone(),
            title,
            cwd: rollout.and_then(|r| r.cwd.clone()),
            started: rollout.and_then(|r| r.started),
            updated: rollout.and_then(|r| r.updated).or_else(|| mtime(&f.path)),
            live: is_live(root, &f.id),
            version: rollout.and_then(|r| r.cli_version.clone()),
            path: f.path.clone(),
        };
        let name = names.get(&f.id).map(|n| title_of(n));
        if f.compressed {
            // Not opened: the gate would refuse it, and the listing's job is
            // to say it exists. A compressed file under a denied name was
            // already skipped by the walk.
            return Some(base(name.unwrap_or_else(|| COMPRESSED_TITLE.into()), None));
        }
        let (head, cut) = read_head(&f.path, HEAD_BYTES).ok()?;
        let mut rollout = parse_rollout(&head);
        if rollout.valid_lines == 0 {
            if !cut {
                return None;
            }
            // A first line longer than the head: only a full read can tell
            // whether this is a rollout at all.
            rollout = parse_rollout(&secrets::read_text(&f.path, READ_CAP).ok()?);
            if rollout.valid_lines == 0 {
                return None;
            }
        } else if cut {
            let tail = secrets::read_tail(&f.path, TAIL_BYTES).ok();
            let last = tail.as_deref().map(parse_rollout).and_then(|t| t.updated);
            rollout.updated = rollout.updated.max(last);
        }
        let title = name
            .or_else(|| rollout.first_user_message.as_deref().map(title_of))
            .unwrap_or_default();
        Some(base(title, Some(&rollout)))
    }
}

/// The lines a `history_base` link inherits: the prefix file's own lines below
/// the cut (its `session_meta` excluded — the child has its own), after that
/// file's own prefix, oldest first. `Err` names what could not be read.
fn prefix_lines(all: &[Found], base: &HistoryBase, depth: usize) -> Result<Vec<Line>, String> {
    if depth > MAX_PREFIX_DEPTH {
        return Err(format!(
            "the fork chain is longer than {MAX_PREFIX_DEPTH} rollouts"
        ));
    }
    let found = all
        .iter()
        .find(|f| f.id == base.rollout_id)
        .ok_or_else(|| format!("prefix rollout {} is not on disk", base.rollout_id))?;
    let text = secrets::read_text(&found.path, READ_CAP).map_err(|r| r.to_string())?;
    let lines = split_lines(&text, &mut Vec::new());
    let grand = match lines
        .iter()
        .find(|l| l.kind == "session_meta")
        .and_then(|m| history_base(&m.payload))
    {
        Some(b) => prefix_lines(all, &b, depth + 1)?,
        None => Vec::new(),
    };
    let own = by_ordinal(
        lines
            .into_iter()
            .filter(|l| {
                l.kind != "session_meta"
                    && l.ordinal.is_some_and(|o| o < base.end_ordinal_exclusive)
            })
            .collect(),
    );
    Ok(grand.into_iter().chain(own).collect())
}

impl SessionSource for CodexSource {
    fn agent(&self) -> AgentKind {
        AgentKind::Codex
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        roots_from_env(tacet_kernel::env::env_var)
    }

    fn discover(&self, root: &Path) -> Vec<SessionSummary> {
        let names = index_names(root);
        let mut out: Vec<SessionSummary> = rollouts(root)
            .iter()
            .filter_map(|f| self.summarize(root, f, &names))
            .collect();
        newest_first(&mut out);
        out
    }

    fn read(&self, root: &Path, id: &str) -> Result<AgentSession, SessionError> {
        let r = self.read_rollout(root, id)?.rollout;
        let title = index_names(root)
            .get(id)
            .map(|n| title_of(n))
            .or_else(|| r.first_user_message.as_deref().map(title_of))
            .unwrap_or_default();
        Ok(AgentSession {
            agent: AgentKind::Codex,
            id: id.to_string(),
            title,
            cwd: r.cwd,
            started: r.started,
            updated: r.updated,
            live: is_live(root, id),
            version: r.cli_version,
            entries: r.entries,
            plan: r.plan,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ts: &str, kind: &str, payload: &str) -> String {
        format!("{{\"timestamp\":\"{ts}\",\"type\":\"{kind}\",\"payload\":{payload}}}\n")
    }

    fn numbered(ord: u64, kind: &str, payload: &str) -> String {
        format!(
            "{{\"timestamp\":\"2026-10-02T10:00:{:02}Z\",\"ordinal\":{ord},\"type\":\"{kind}\",\"payload\":{payload}}}\n",
            ord % 60
        )
    }

    fn plan_call(call_id: &str, args: &str) -> String {
        let quoted = serde_json::to_string(args).unwrap();
        format!(
            "{{\"type\":\"function_call\",\"name\":\"update_plan\",\"arguments\":{quoted},\"call_id\":\"{call_id}\"}}"
        )
    }

    fn plan_output(call_id: &str, output: &str) -> String {
        format!(
            "{{\"type\":\"function_call_output\",\"call_id\":\"{call_id}\",\"output\":{}}}",
            serde_json::to_string(output).unwrap()
        )
    }

    fn titles(plan: &[PlanItem]) -> Vec<(&str, Status)> {
        plan.iter().map(|p| (p.title.as_str(), p.status)).collect()
    }

    const UUID: &str = "0199d3a1-7c2e-7a10-9b55-3f2c1d0e4a01";

    #[test]
    fn a_rollout_name_yields_its_id_and_whether_it_is_compressed() {
        assert_eq!(
            rollout_id(&format!("rollout-2026-10-01T09-15-00-{UUID}.jsonl")),
            Some((UUID.to_string(), false))
        );
        assert_eq!(
            rollout_id(&format!("rollout-2026-10-01T09-15-00-{UUID}.jsonl.zst")),
            Some((UUID.to_string(), true))
        );
        for not in [
            format!("{UUID}.jsonl"),
            format!("rollout-2026-10-01T09-15-00-{UUID}.json"),
            format!("rollout-{UUID}.jsonl"),
            format!("rollout-2026-10-01 09-15-00-{UUID}.jsonl"),
            "rollout-2026-10-01T09-15-00-.jsonl".to_string(),
            "rollout-2026-10-01T09-15-00-..jsonl".to_string(),
            "rollout-2026-10-01T09-15-00-notes.jsonl".to_string(),
            "rollout-notes.jsonl".to_string(),
            "history.jsonl".to_string(),
        ] {
            assert_eq!(rollout_id(&not), None, "{not}");
        }
    }

    #[test]
    fn codex_home_wins_and_a_relative_one_is_not_guessed_at() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(
            roots_from(os("/srv/codex-home"), os("/home/u")),
            [PathBuf::from("/srv/codex-home")]
        );
        assert_eq!(
            roots_from(None, os("/home/u")),
            [PathBuf::from("/home/u/.codex")]
        );
        assert_eq!(
            roots_from(os(""), os("/home/u")),
            [PathBuf::from("/home/u/.codex")]
        );
        assert!(roots_from(os("rel/codex-home"), os("/home/u")).is_empty());
        assert!(roots_from(None, os("rel")).is_empty());
        assert!(roots_from(None, None).is_empty());
    }

    /// RG-11: the variable names `default_roots` reads — `CODEX_HOME` first,
    /// then `HOME` — through a table, so no test touches the real environment.
    #[test]
    fn the_roots_come_from_codex_home_then_home() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(name, _)| *name == k)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        assert_eq!(
            roots_from_env(env(&[
                ("CODEX_HOME", "/srv/codex-home"),
                ("HOME", "/home/u")
            ])),
            [PathBuf::from("/srv/codex-home")]
        );
        assert_eq!(
            roots_from_env(env(&[("HOME", "/home/u")])),
            [PathBuf::from("/home/u/.codex")]
        );
        assert!(roots_from_env(env(&[])).is_empty());
    }

    /// RG-12, legacy mode: file order, response items as the transcript (the
    /// event copies are NOT shown twice), injected context as system text, a
    /// tool result named after its call, a compaction summary, malformed lines
    /// skipped and counted.
    #[test]
    fn a_legacy_rollout_reads_the_model_stream_once() {
        let mut t = String::new();
        t += &line(
            "2026-10-01T09:15:00Z",
            "session_meta",
            r#"{"id":"s1","cwd":"/a","cli_version":"0.46.0","timestamp":"2026-10-01T09:14:59Z"}"#,
        );
        t += &line(
            "2026-10-01T09:15:01Z",
            "response_item",
            r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}"#,
        );
        t += &line(
            "2026-10-01T09:15:02Z",
            "response_item",
            r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"fix it"}]}"#,
        );
        t += &line(
            "2026-10-01T09:15:02Z",
            "event_msg",
            r#"{"type":"user_message","message":"fix it"}"#,
        );
        t += "not json\n";
        t += &line(
            "2026-10-01T09:15:03Z",
            "response_item",
            r#"{"type":"function_call","name":"shell","arguments":"{\"command\":[\"ls\"]}","call_id":"k1"}"#,
        );
        t += &line(
            "2026-10-01T09:15:04Z",
            "response_item",
            r#"{"type":"function_call_output","call_id":"k1","output":"a.rs"}"#,
        );
        t += "{\"type\":\"response_item\"}\n";
        t += &line(
            "2026-10-01T09:15:05Z",
            "compacted",
            r#"{"message":"so far: listed"}"#,
        );
        t += &line(
            "2026-10-01T09:15:06Z",
            "turn_context",
            r#"{"cwd":"/b","model":"gpt-5-codex"}"#,
        );
        t += &line(
            "2026-10-01T09:15:07Z",
            "response_item",
            r#"{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}"#,
        );
        t += &line(
            "2026-10-01T09:15:07Z",
            "event_msg",
            r#"{"type":"agent_message","message":"done"}"#,
        );
        let r = parse_rollout(&t);
        assert_eq!(r.history_mode, HistoryMode::Legacy);
        assert_eq!(r.meta_id.as_deref(), Some("s1"));
        assert_eq!(r.cli_version.as_deref(), Some("0.46.0"));
        assert_eq!(r.cwd, Some(PathBuf::from("/b")), "the latest turn's cwd");
        assert_eq!(r.started, parse_iso_utc("2026-10-01T09:14:59Z"));
        assert_eq!(r.updated, parse_iso_utc("2026-10-01T09:15:07Z"));
        assert_eq!(r.skipped, [5, 8]);
        assert_eq!(r.first_user_message.as_deref(), Some("fix it"));
        let shape: Vec<_> = r
            .entries
            .iter()
            .map(|e| (e.role, e.kind, e.text.as_str(), e.tool.as_deref()))
            .collect();
        assert_eq!(
            shape,
            [
                (
                    Role::System,
                    EntryKind::Text,
                    "<environment_context>cwd</environment_context>",
                    None
                ),
                (Role::User, EntryKind::Text, "fix it", None),
                (
                    Role::Assistant,
                    EntryKind::ToolCall,
                    "{\"command\":[\"ls\"]}",
                    Some("shell")
                ),
                (Role::Tool, EntryKind::ToolResult, "a.rs", Some("shell")),
                (Role::System, EntryKind::Summary, "so far: listed", None),
                (Role::Assistant, EntryKind::Text, "done", None),
            ]
        );
    }

    /// A user-role message is Codex's own when ANY part is a context block —
    /// the typed text riding along in the same item does not make it the
    /// user's, and must not become the title.
    #[test]
    fn a_context_block_with_a_typed_part_is_still_context() {
        let t = line(
            "2026-10-01T09:15:01Z",
            "response_item",
            r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"},{"type":"input_text","text":"<user_instructions>be terse</user_instructions>"}]}"#,
        ) + &line(
            "2026-10-01T09:15:02Z",
            "response_item",
            r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"the real ask"}]}"#,
        );
        let r = parse_rollout(&t);
        assert_eq!(r.entries[0].role, Role::System);
        assert_eq!(r.first_user_message.as_deref(), Some("the real ask"));
    }

    /// RG-12, paginated mode: the ordinal is the order, the last write of an
    /// ordinal wins, a line with no ordinal is skipped, and the highest
    /// ordinal and the gaps are reported for whoever appends next.
    #[test]
    fn a_paginated_rollout_is_ordered_by_ordinal_not_by_file_position() {
        let msg = |role: &str, text: &str| {
            format!(
                r#"{{"type":"message","role":"{role}","content":[{{"type":"input_text","text":"{text}"}}]}}"#
            )
        };
        let mut t = String::new();
        t += &numbered(
            0,
            "session_meta",
            r#"{"id":"s2","history_mode":"paginated","cli_version":"0.47.0"}"#,
        );
        t += &numbered(1, "response_item", &msg("user", "one"));
        t += &numbered(3, "response_item", &msg("assistant", "three"));
        t += &numbered(2, "response_item", &msg("assistant", "two-first"));
        t += &line(
            "2026-10-02T10:00:30Z",
            "response_item",
            &msg("user", "unnumbered"),
        );
        t += &numbered(2, "response_item", &msg("assistant", "two"));
        t += &numbered(6, "response_item", &msg("user", "six"));
        let r = parse_rollout(&t);
        assert_eq!(r.history_mode, HistoryMode::Paginated);
        let texts: Vec<_> = r.entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["one", "two", "three", "six"]);
        assert_eq!(r.skipped, [5]);
        assert_eq!(r.last_ordinal, Some(6));
        assert_eq!(r.ordinal_gaps, 2, "4 and 5 are missing");
    }

    /// The same lines with no `history_mode` and no meta: numbered lines say
    /// paginated themselves; unnumbered ones are legacy and keep file order.
    #[test]
    fn without_a_meta_the_lines_say_which_mode_they_are() {
        let m = |text: &str| {
            format!(
                r#"{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{text}"}}]}}"#
            )
        };
        let t = numbered(2, "response_item", &m("b")) + &numbered(1, "response_item", &m("a"));
        let r = parse_rollout(&t);
        assert_eq!(r.history_mode, HistoryMode::Paginated);
        assert_eq!(
            r.entries
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        let legacy = line("2026-10-02T10:00:00Z", "response_item", &m("b"))
            + &line("2026-10-02T10:00:01Z", "response_item", &m("a"));
        let r = parse_rollout(&legacy);
        assert_eq!(r.history_mode, HistoryMode::Legacy);
        assert_eq!(
            r.entries
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
    }

    /// When no `response_item` was written, the event stream is the
    /// transcript, in both modes' event shapes.
    #[test]
    fn with_no_model_stream_the_events_are_the_transcript() {
        let mut t = String::new();
        t += &numbered(
            0,
            "session_meta",
            r#"{"id":"s3","history_mode":"paginated"}"#,
        );
        t += &numbered(
            1,
            "event_msg",
            r#"{"type":"item_completed","item":{"type":"UserMessage","id":"u1","content":[{"type":"text","text":"rename the crate"}]}}"#,
        );
        t += &numbered(
            2,
            "event_msg",
            r#"{"type":"item_completed","item":{"type":"AgentMessage","id":"a1","content":[{"type":"Text","text":"renamed"}]}}"#,
        );
        t += &numbered(3, "event_msg", r#"{"type":"token_count","info":null}"#);
        let r = parse_rollout(&t);
        assert_eq!(r.first_user_message.as_deref(), Some("rename the crate"));
        let shape: Vec<_> = r
            .entries
            .iter()
            .map(|e| (e.role, e.text.as_str()))
            .collect();
        assert_eq!(
            shape,
            [
                (Role::User, "rename the crate"),
                (Role::Assistant, "renamed")
            ]
        );

        let legacy = line(
            "2026-10-01T00:00:00Z",
            "event_msg",
            r#"{"type":"user_message","message":"hi"}"#,
        ) + &line(
            "2026-10-01T00:00:01Z",
            "event_msg",
            r#"{"type":"agent_message","message":"hello"}"#,
        );
        let r = parse_rollout(&legacy);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[1].role, Role::Assistant);
    }

    /// A remote compaction leaves `message` empty; the kept history is the
    /// summary then, not an empty entry.
    #[test]
    fn an_empty_compaction_message_falls_back_to_the_replacement_history() {
        let t = line(
            "2026-10-01T00:00:00Z",
            "compacted",
            r#"{"message":"","replacement_history":[{"type":"message","role":"user","content":[{"type":"input_text","text":"kept ask"}]},{"type":"reasoning","summary":[],"encrypted_content":"x"}]}"#,
        );
        let r = parse_rollout(&t);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].kind, EntryKind::Summary);
        assert_eq!(r.entries[0].text, "kept ask");
    }

    /// RG-13: the last ACCEPTED `update_plan` is the plan. Codex rejects a
    /// whole call over one bad step or an unknown field, and Plan mode refuses
    /// a well-formed one; either way the previous plan stands.
    #[test]
    fn the_plan_is_the_last_update_plan_codex_accepted() {
        let ts = "2026-10-01T10:00:00Z";
        let mut t = String::new();
        t += &line(
            ts,
            "response_item",
            &plan_call(
                "c1",
                r#"{"explanation":"x","plan":[{"step":"Reproduce","status":"in_progress"},{"step":"Fix","status":"pending"}]}"#,
            ),
        );
        t += &line(ts, "response_item", &plan_output("c1", PLAN_UPDATED));
        t += &line(
            ts,
            "response_item",
            &plan_call(
                "c2",
                r#"{"plan":[{"step":"Reproduce","status":"completed"},{"step":"Fix","status":"in_progress"}]}"#,
            ),
        );
        t += &line(ts, "response_item", &plan_output("c2", PLAN_UPDATED));
        let r = parse_rollout(&t);
        let accepted = [
            ("Reproduce", Status::Completed),
            ("Fix", Status::InProgress),
        ];
        assert_eq!(titles(&r.plan), accepted);

        for rejected in [
            // Not JSON.
            "{not json",
            // No `plan`.
            r#"{"explanation":"only"}"#,
            // One step with a status Codex does not have rejects the CALL.
            r#"{"plan":[{"step":"Ship","status":"completed"},{"step":"Odd","status":"paused"}]}"#,
            // `deny_unknown_fields`, on the call and on a step.
            r#"{"plan":[{"step":"Ship","status":"completed"}],"extra":1}"#,
            r#"{"plan":[{"step":"Ship","status":"completed","note":"n"}]}"#,
            r#"{"explanation":3,"plan":[{"step":"Ship","status":"completed"}]}"#,
        ] {
            let more = t.clone() + &line(ts, "response_item", &plan_call("c3", rejected));
            assert_eq!(titles(&parse_rollout(&more).plan), accepted, "{rejected}");
        }

        // Well-formed, but refused by Plan mode: the output says so.
        let refused = t.clone()
            + &line(
                ts,
                "response_item",
                &plan_call("c4", r#"{"plan":[{"step":"Ship","status":"completed"}]}"#),
            )
            + &line(
                ts,
                "response_item",
                &plan_output(
                    "c4",
                    "update_plan is a TODO/checklist tool and is not allowed in Plan mode",
                ),
            );
        assert_eq!(titles(&parse_rollout(&refused).plan), accepted);

        // Accepted, and a call with no output yet keeps its plan.
        let pending = t.clone()
            + &line(
                ts,
                "response_item",
                &plan_call("c5", r#"{"plan":[{"step":"Ship","status":"completed"}]}"#),
            );
        assert_eq!(
            titles(&parse_rollout(&pending).plan),
            [("Ship", Status::Completed)]
        );
    }

    /// Only Codex's own tool sets the plan: another tool whose arguments
    /// happen to carry `plan`, an MCP tool named `update_plan` in a namespace,
    /// and a custom-tool payload (which the handler refuses) do not.
    #[test]
    fn only_the_plain_update_plan_function_sets_the_plan() {
        let ts = "2026-10-01T10:00:00Z";
        let args = serde_json::to_string(r#"{"plan":[{"step":"Not a plan","status":"pending"}]}"#)
            .unwrap();
        let t = line(
            ts,
            "response_item",
            &format!(
                r#"{{"type":"function_call","name":"shell","arguments":{args},"call_id":"d"}}"#
            ),
        ) + &line(
            ts,
            "response_item",
            &format!(
                r#"{{"type":"function_call","name":"update_plan","namespace":"mcp__todo","arguments":{args},"call_id":"e"}}"#
            ),
        ) + &line(
            ts,
            "response_item",
            &format!(
                r#"{{"type":"custom_tool_call","name":"update_plan","input":{args},"call_id":"f"}}"#
            ),
        );
        let r = parse_rollout(&t);
        assert!(r.plan.is_empty(), "{:?}", r.plan);
        assert_eq!(r.entries[1].tool.as_deref(), Some("mcp__todo.update_plan"));
    }

    #[test]
    fn a_long_first_message_is_cut_to_one_line_of_a_listing() {
        let long = format!("{}\nsecond line", "é".repeat(150));
        let t = title_of(&long);
        assert_eq!(t.chars().count(), TITLE_CHARS + 1);
        assert!(t.ends_with('…'));
        assert_eq!(title_of("  short\nmore"), "short");
    }
}
