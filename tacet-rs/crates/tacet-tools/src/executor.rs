//! ToolExecutor — the SINGLE crossing point between model output and a `Tool`.
//!
//! Swift's `ToolExecutor` held the chip lifecycle; here that job belongs to
//! core's `Reporter`. What is left to this type is narrower and more
//! critical: the decision to ACCEPT a tool call.
//!
//! FIVE GATES, ALL STRUCTURAL:
//!
//! 1. NAME — if it is not in the catalog the tool does not run. The model
//!    cannot run a signature it made up.
//! 2. SCHEMA — the tool never sees arguments that did not pass
//!    `ArgSchema::validate`. The grammar already forces this, but the grammar
//!    can be turned off (eval, CLI, a direct call); the two layers are deliberate.
//! 3. APPROVAL — in a tainted session a call that would push data to the
//!    outside world does not pass without a user decision.
//! 4. CANCELLATION — if the user cancelled the turn no tool even starts.
//! 5. REPEAT — the same (tool, arguments) pair DOES NOT RUN a second time in a
//!    turn. This gate was added later and the reason is plain: until then the
//!    loop ban was held by a single sentence in SYSTEM_INSTRUCTION, and that
//!    sentence cut RECOVERY while cutting the loop — a model that picked the
//!    wrong tool could no longer call the right one (the user had to say
//!    "search the internet"). Once the instruction was relaxed something else
//!    had to hold the loop; trusting text failed silently once in this project.
//!
//! ERROR DETECTION IS NOT DONE WITH TEXT. On the Swift side someone once
//! silently wrote `to_model.contains("tool_failed")`; an unrelated commit that
//!    changed the tool text killed the recovery path and no test broke.
//! Here the decision comes from `ExecutionOutcome::reason` (a closed enum);
//! `is_error()` and `retryable` derive from it, not from text.
//!
//! IF THE WORLD CHANGED THERE IS NO RETRY. Sending the same prompt a second
//! time in a turn that returned `ToolState::Written` creates a second
//! file/record. The flag is TURN SCOPED and STICKY: a recovery attempt cannot
//! drop it, only a real new turn resets it.

use serde_json::Value;
use tacet_core::{
    ToolState, ToolError, ToolCatalog, ToolOutcome, ERROR_MODEL_TEXT, ToolContext,
};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The FIXED text returned to the model when approval is denied.
///
/// Separate from the tool's own error text: this is not a malfunction, it is a
/// decision. So the model does not slip into a "try again" reflex the sentence
/// is final and gives no reason — the rationale belongs to the user and is not
/// opened to the model.
pub const DENIAL_MODEL_TEXT: &str =
    "permission_denied: the user did not allow this data to leave the device";

/// The fixed text returned to the model in a cancelled turn.
pub const CANCEL_MODEL_TEXT: &str = "cancelled: the user stopped this turn";

/// The fixed text returned to the model when the SAME call arrives a second time in a turn.
///
/// It does two things at once: (1) it cuts the loop STRUCTURALLY — the tool
/// DOES NOT RUN a second time, so neither a network nor a file side effect is
/// repeated; (2) it shows the model the way out. The sentence does not say
/// "stop", it says "do SOMETHING ELSE": had it only said "do not call again"
/// the model would fall silent and no answer would reach the user — that was
/// exactly the ugliest face of the observed loop failure.
/// NO CAPITALS — the same register as the neighbouring constants
/// (`DENIAL_MODEL_TEXT`, `CANCEL_MODEL_TEXT`). In this project capitals used
/// for emphasis were MEASURED to be taken as CONTENT by the model (see
/// `SYSTEM_INSTRUCTION` item 3); if the text enters the model's context the
/// same risk applies.
pub const REPEAT_MODEL_TEXT: &str =
    "duplicate_call: you already made this exact call in this turn and its result is above. \
     Do not repeat it. Either answer the user with what you have, or call a different tool.";

// ---------------------------------------------------------------------------
// Call
// ---------------------------------------------------------------------------

/// A single tool call produced by the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub args: Value,
}

impl ToolCall {
    pub fn new(name: impl Into<String>, args: Value) -> Self {
        Self { name: name.into(), args }
    }

    /// Parses the call out of the model output; `None` if there is no call.
    ///
    /// TWO FORMATS are accepted because with the grammar on the model produces
    /// canonical JSON, while with the grammar off (free generation, eval, hand
    /// typed CLI input) it more often produces the `name({...})` form. Meeting
    /// both here beats growing a separate "maybe this format too" patch at
    /// every call site.
    ///
    /// Returning `None` IS NOT AN ERROR: the model may have given a plain
    /// answer. That distinction is left to the call site.
    pub fn parse(raw: &str) -> Option<Self> {
        let body = strip_code_fence(raw.trim());

        // Format 1 — canonical: {"tool":"name","args":{...}}
        if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(body) {
            let name = object
                .get("tool")
                .or_else(|| object.get("name"))
                .and_then(Value::as_str)?;
            let args = object
                .get("args")
                .or_else(|| object.get("arguments"))
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default()));
            return Some(Self::new(name, args));
        }

        // Format 2 — call form: name({...})
        //
        // THERE MAY BE PLAIN TEXT IN THE PREFIX. The WHOLE text before `(` used
        // to be taken as the tool name; measured on a real model, that turned
        // out to be wrong. Qwen3-4B typically writes its intent first:
        //
        //   "I will run a web search to look up the weather in Istanbul.
        //    web_search({"query":"istanbul weather"})"
        //
        // The old code read the name in that output as "I will ... look up.\n\nweb_search",
        // saw a non-letter character inside it and returned `None` — so even
        // though the model had called the tool CORRECTLY the tool NEVER RAN and
        // the produced call was silently swallowed. The symptom was the most
        // misleading part: it looked like "the model is not calling tools",
        // when in fact it was.
        // The right thing is to take the LAST identifier before `(`. Name
        // validation does not happen here; an unknown name is already rejected
        // at gate 1 (`catalog.find`) and a proper error goes back to the model.
        let closing = body.rfind(')')?;
        // The candidates are tried RIGHT TO LEFT: because constraint production
        // ends at the call's closing parenthesis, the real call is at the END of
        // the text. Parentheses appearing in plain text ("... (mathematics) ...")
        // therefore do not shadow the call.
        for (opening, _) in body[..closing].char_indices().rev().filter(|(_, c)| *c == '(') {
            let name = last_identifier(&body[..opening]);
            if name.is_empty() {
                continue;
            }
            let inner = body[opening + 1..closing].trim();
            let args = if inner.is_empty() {
                Value::Object(Default::default())
            } else {
                match serde_json::from_str(inner) {
                    Ok(v) => v,
                    // This `(` was not the opening of the call; move to the candidate on the left.
                    Err(_) => continue,
                }
            };
            return Some(Self::new(name, args));
        }
        None
    }
}

/// THE RECOVERY LAYER — matches a NAMELESS bare JSON object against a schema.
///
/// WHY IT EXISTS: on a real model (Qwen3-4B) in a multi-turn document flow the
/// model prints `{...}` JSON directly without writing the tool NAME —
/// `{"format":"excel","file_name":"plan"}`. That fits neither Format 1 (NO
/// `tool`/`name` key) nor Format 2 (NO parentheses); the call was silently
/// swallowed and NO file was created. The fields (`format`, `file_name`) match
/// the `create_document` schema EXACTLY — the forgotten name is the only thing missing.
///
/// WHY IT IS SAFE — a COMPLETE and UNIQUE schema match. For a tool to count as a candidate:
///  1. ALL keys of the object must be DEFINED in that tool's schema (NO foreign
///     key — if the model carries a field the schema does not have, it is not that tool),
///  2. ALL required fields of the tool must be PRESENT in the object (and not null),
///  3. the tool must have AT LEAST ONE required field (so an arbitrary object
///     does not appear to "fit" a signature-less tool).
///
/// If EXACTLY ONE tool satisfies these conditions it is recovered; with ZERO or
/// MORE THAN ONE candidate `None` is returned and the input counts as PLAIN
/// TEXT. A wrong match breaks irrelevance; NOT recovering under ambiguity is deliberate.
///
/// AN EXTRA STRICT CONDITION — the whole text (after the fence is stripped)
/// must be a SINGLE JSON object. JSON embedded in prose is not recovered: when
/// the model writes its intent and shows an example JSON beside it, that is NOT
/// a call, because the body on its own cannot be parsed as a valid JSON object.
fn recover_nameless_json(raw: &str, catalog: &ToolCatalog) -> Option<ToolCall> {
    let body = strip_code_fence(raw.trim());
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(body) else {
        return None;
    };
    // An empty object points at no tool.
    if object.is_empty() {
        return None;
    }
    // An object carrying a name key was already Format 1's business; only a
    // NAMELESS object should land here. Guarantee it anyway.
    if ["tool", "name"].iter().any(|k| object.contains_key(*k)) {
        return None;
    }

    let mut matched: Option<String> = None;
    for tool in catalog.tools() {
        let schema = tool.schema();
        let fields = schema.fields();

        // (1) Is there a foreign key? Every key in the object must be defined in the schema.
        if !object.keys().all(|k| fields.iter().any(|a| &a.name == k)) {
            continue;
        }

        // (2)+(3) Required fields: there must be at least one and all must be present.
        let mut has_required = false;
        let mut required_complete = true;
        for a in fields.iter().filter(|a| a.required) {
            has_required = true;
            if object.get(&a.name).is_none_or(|v| v.is_null()) {
                required_complete = false;
                break;
            }
        }
        if !has_required || !required_complete {
            continue;
        }

        // A second candidate: the match is AMBIGUOUS, do not recover.
        if matched.is_some() {
            return None;
        }
        matched = Some(tool.name().to_string());
    }

    matched.map(|name| ToolCall::new(name, Value::Object(object)))
}

/// The identity of a call WITHIN A TURN: `name|arguments`.
///
/// Because `serde_json` keeps object keys ordered (BTreeMap), the same
/// arguments written in a different order still produce the same text — so the
/// measure of "sameness" depends on the ARGUMENTS THEMSELVES, not on the order
/// the model wrote them in.
/// IF SERIALIZATION FAILS the key does not collapse to the NAME alone: we fall
/// back to the `Debug` form. The reason is to avoid a silent false match — had
/// we reduced the key to the name, two legitimate calls with different
/// arguments (`read_document` on two different files) would look like a repeat.
fn call_key(call: &ToolCall) -> String {
    let args = serde_json::to_string(&call.args).unwrap_or_else(|_| format!("{:?}", call.args));
    format!("{}|{args}", call.name)
}

/// Returns the identifier at the END of the text (`[A-Za-z0-9_]*`).
///
/// Being ASCII limited is deliberate: tool names are ASCII by contract (see the
/// naming rule — symbols are ASCII). Had we accepted Turkish letters too, the
/// end of a word such as "yapacagim" would look like a valid name and carry
/// needless noise to gate 1.
fn last_identifier(text: &str) -> &str {
    let emit = text
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
        .last()
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    &text[emit..]
}

/// Strips the ```` ```json ... ``` ```` fence. The small model puts it there
/// whether asked to or not; cleaning the fence once here is cheaper than
/// repeating the same check at every call site.
fn strip_code_fence(raw: &str) -> &str {
    let Some(remaining) = raw.strip_prefix("```") else {
        return raw;
    };
    // The first line is the language tag ("json"), the body starts after it.
    let body = remaining.split_once('\n').map(|(_, g)| g).unwrap_or("");
    body.trim().strip_suffix("```").unwrap_or(body).trim()
}

// ---------------------------------------------------------------------------
// Outcome
// ---------------------------------------------------------------------------

/// The STRUCTURAL outcome of an execution. Call sites make their decisions from
/// this, not from text (see the file header).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionReason {
    /// The tool ran and returned a successful state.
    Ok,
    /// There is no such tool in the catalog.
    UnknownTool,
    /// The arguments did not fit the schema; the tool NEVER ran.
    InvalidArguments,
    /// The tool ran and returned `Failed`.
    ToolFailed,
    /// The approval gate stayed shut; the tool NEVER ran.
    ApprovalDenied,
    /// The turn was cancelled; the tool NEVER ran.
    Cancelled,
    /// The SAME tool with the SAME arguments had already run this turn; the tool NEVER ran.
    RepeatedCall,
}

impl ExecutionReason {
    /// Is this outcome a malfunction. An approval denial and a cancellation ARE
    /// NOT MALFUNCTIONS: both are decisions the user made, and the recovery path
    /// must not be triggered.
    /// `RepeatedCall` DOES NOT COUNT as a malfunction either, and that is
    /// deliberate: were it counted an error, the call site would open a recovery
    /// turn, the model would produce the same call once more, and the very loop
    /// we are trying to prevent would be built — this time with our encouragement.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            ExecutionReason::UnknownTool
                | ExecutionReason::InvalidArguments
                | ExecutionReason::ToolFailed
        )
    }
}

/// The outcome of executing a tool call.
#[derive(Debug, Clone)]
pub struct ExecutionOutcome {
    pub tool_name: String,
    pub reason: ExecutionReason,
    /// The SHORT text returned to the model. Bulk data never goes here (the bypass channel).
    pub to_model: String,
    /// The chip text visible to the user. May be empty (for unimportant calls).
    pub chip_text: String,
    pub state: ToolState,
    /// Did this call change the outside world (`Written`).
    pub world_changed: bool,
    /// May the SAME PROMPT be sent a second time. FALSE once the world changed
    /// within the turn — otherwise a second file/record is created.
    pub retryable: bool,
    /// The tool's raw output (chip detail, eval evidence). It DOES NOT GO to the model.
    pub raw_output: Option<String>,
}

impl ExecutionOutcome {
    pub fn is_error(&self) -> bool {
        self.reason.is_error()
    }
}

// ---------------------------------------------------------------------------
// The approval gate
// ---------------------------------------------------------------------------

/// The sharing request to be shown to the user. `content` is THE VERY SAME
/// ARGUMENTS that will be sent, not a category summary: the user should see
/// exactly what they are permitting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub source: String,
    pub tool_name: String,
    pub content: String,
}

/// The authority that makes the approval decision. A page in the UI, a fixed
/// answer in eval.
/// `&self`: several concurrent calls may poll the same gate; internal
/// synchronization is the concrete implementation's job (the same call as
/// core's `Reporter`).
pub trait ApprovalGate: Send + Sync {
    fn request(&self, request: &ApprovalRequest) -> bool;
}

/// The headless default: there is nobody to ask, so data DOES NOT LEAVE.
///
/// The default being "deny" is deliberate — letting it through without asking
/// would be the same as having no gate at all.
pub struct SilentDeny;

impl ApprovalGate for SilentDeny {
    fn request(&self, _request: &ApprovalRequest) -> bool {
        false
    }
}

/// For tests and "approved" eval scenarios.
pub struct AlwaysApprove;

impl ApprovalGate for AlwaysApprove {
    fn request(&self, _request: &ApprovalRequest) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// The executor
// ---------------------------------------------------------------------------

/// A turn ticket. `new_turn()` returns one, `execute` takes one.
///
/// WHY A TICKET: had a plain `bool` held the cancellation flag, cleaning up a
/// cancelled OLD turn (a late `clear_cancel`) would drop the NEW turn's flag —
/// the user's second "stop" would be swallowed. A ticket is a number and a
/// cancellation is recorded as "the turn with this number is cancelled"; what
/// an old turn said does not bind the new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnTicket(pub u64);

pub struct ToolExecutor {
    catalog: ToolCatalog,
    /// The names of the tools that can push data to the outside world. Core's
    /// `Tool` contract has NO such flag and none was added: "does it push data
    /// out" is not a property of the tool itself but a DEPLOYMENT decision (the
    /// same tool does not push anything out in an offline install). That is why
    /// the list is kept here.
    external_tools: HashSet<String>,
    gate: Box<dyn ApprovalGate>,
    /// Session scoped. Set once a personal-data tool runs SUCCESSFULLY; a turn
    /// change does not clear it (the context summary may carry personal data).
    session_tainted: AtomicBool,
    /// Turn scoped, STICKY.
    world_changed: AtomicBool,
    /// The active turn number.
    turn: AtomicU64,
    /// The cancelled turn number. 0 = nothing cancelled (turns start at 1).
    cancelled: AtomicU64,
    /// The sources denied in this session — the same source is not asked about
    /// a second time, so the model cannot enter an insistence loop.
    denied: Mutex<HashSet<String>>,
    /// The (tool, arguments) pairs that have run in THIS TURN.
    ///
    /// WHY A STRUCTURAL GUARD IS NEEDED: the loop ban was held until now by a
    /// single sentence in SYSTEM_INSTRUCTION, and in this project text-based
    /// rules died silently once. On top of that, that sentence cut recovery
    /// while cutting the loop — a model that picked the wrong tool could no
    /// longer call the right one. Since the instruction has been relaxed, loop
    /// prevention must rest on code, not text: the same call DOES NOT PASS a
    /// second time in a turn, whatever the model writes.
    ///
    /// TURN SCOPED: it is legitimate for the user to ask "what time is it" a
    /// second time; getting the same result twice within one turn is not.
    turn_calls: Mutex<HashSet<String>>,
}

impl ToolExecutor {
    pub fn new(catalog: ToolCatalog) -> Self {
        Self {
            catalog,
            external_tools: HashSet::new(),
            gate: Box::new(SilentDeny),
            session_tainted: AtomicBool::new(false),
            world_changed: AtomicBool::new(false),
            turn: AtomicU64::new(1),
            cancelled: AtomicU64::new(0),
            denied: Mutex::new(HashSet::new()),
            turn_calls: Mutex::new(HashSet::new()),
        }
    }

    /// Marks this tool as "pushes data to the outside world".
    pub fn external_tool(mut self, name: impl Into<String>) -> Self {
        self.external_tools.insert(name.into());
        self
    }

    pub fn with_gate(mut self, gate: impl ApprovalGate + 'static) -> Self {
        self.gate = Box::new(gate);
        self
    }

    pub fn catalog(&self) -> &ToolCatalog {
        &self.catalog
    }

    pub fn session_tainted(&self) -> bool {
        self.session_tainted.load(Ordering::SeqCst)
    }

    pub fn world_changed(&self) -> bool {
        self.world_changed.load(Ordering::SeqCst)
    }

    /// May the same prompt be sent a second time — the SINGLE decision point of
    /// the recovery path.
    pub fn retry_safe(&self) -> bool {
        !self.world_changed()
    }

    pub fn active_turn(&self) -> TurnTicket {
        TurnTicket(self.turn.load(Ordering::SeqCst))
    }

    /// A real new turn: the side-effect flag is reset, a new ticket is produced.
    pub fn new_turn(&self) -> TurnTicket {
        self.world_changed.store(false, Ordering::SeqCst);
        self.turn_calls.lock().expect("turn calls lock").clear();
        TurnTicket(self.turn.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// A recovery attempt — NOT a real new turn. The ticket, the side-effect
    /// flag and the TURN CALLS are preserved: whether a side effect happened
    /// must not be lost exactly at the moment the retry decision is made; in the
    /// same way a recovery turn must not become an excuse to repeat a failed call verbatim.
    pub fn recovery_attempt(&self) -> TurnTicket {
        self.active_turn()
    }

    /// Cancels the active turn.
    pub fn cancel(&self) {
        self.cancelled.store(self.turn.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    pub fn is_cancelled(&self, ticket: TurnTicket) -> bool {
        self.cancelled.load(Ordering::SeqCst) == ticket.0
    }

    /// End of session — everything ends, including the taint and the denial cache.
    pub fn reset_chat(&self) {
        self.session_tainted.store(false, Ordering::SeqCst);
        self.world_changed.store(false, Ordering::SeqCst);
        self.denied.lock().expect("denial cache lock").clear();
        self.turn_calls.lock().expect("turn calls lock").clear();
        self.turn.fetch_add(1, Ordering::SeqCst);
    }

    /// Parses the call out of raw model output and executes it.
    ///
    /// `None` = the model did not call a tool (a plain answer). That is not an error.
    ///
    /// The two canonical formats are tried FIRST; if neither holds, the RECOVERY
    /// layer (matching nameless JSON against a schema, see
    /// `recover_nameless_json`) is tried. Recovery sees the catalog, so it lives
    /// here, outside `parse` — `parse` only takes a string and is responsible
    /// for the two canonical formats.
    pub async fn execute_raw(
        &self,
        raw: &str,
        ticket: TurnTicket,
        ctx: &mut ToolContext,
    ) -> Option<ExecutionOutcome> {
        let call = ToolCall::parse(raw)
            .or_else(|| recover_nameless_json(raw, &self.catalog))?;
        Some(self.execute(&call, ticket, ctx).await)
    }

    /// Runs a single tool call through the gates.
    ///
    /// It DOES NOT RETURN a `Result` (the same rationale as core decision 4):
    /// every path produces an `ExecutionOutcome`, so call sites cannot find an
    /// option called "forget the error path".
    pub async fn execute(
        &self,
        call: &ToolCall,
        ticket: TurnTicket,
        ctx: &mut ToolContext,
    ) -> ExecutionOutcome {
        // GATE 4 — cancellation. First of all: in a cancelled turn no tool
        // should start, not even a cheap one.
        if self.is_cancelled(ticket) {
            return self.outcome(
                &call.name,
                ExecutionReason::Cancelled,
                CANCEL_MODEL_TEXT.to_string(),
                "Cancelled".to_string(),
                ToolState::Failed("cancelled".into()),
                None,
                false,
            );
        }

        // GATE 1 — the name.
        let Some(tool) = self.catalog.find(&call.name) else {
            let error = ToolError::InvalidArgument(format!("unknown tool: {}", call.name));
            return self.outcome(
                &call.name,
                ExecutionReason::UnknownTool,
                ERROR_MODEL_TEXT.to_string(),
                error.short_error(),
                ToolState::Failed(error.short_error()),
                None,
                false,
            );
        };

        // GATE 2 — the schema. The tool validates internally too; this layer
        // guarantees the tool NEVER RUNS (which matters for tools with side effects).
        if let Err(error) = tool.schema().validate(&call.args) {
            return self.outcome(
                &call.name,
                ExecutionReason::InvalidArguments,
                ERROR_MODEL_TEXT.to_string(),
                error.short_error(),
                ToolState::Failed(error.short_error()),
                None,
                false,
            );
        }

        // GATE 5 — REPEAT. AFTER the name and the schema: a malformed call
        // should first learn that it is malformed, not "you already did that".
        //
        // THE QUERY IS HERE, THE RECORD IS AFTER APPROVAL — the two were split deliberately:
        //
        //  * The query must come BEFORE approval, otherwise a repeat of an
        //    approved call would ask the user for approval A SECOND TIME. Someone
        //    who opened the gate once must not be disturbed again just because
        //    the model reproduced the same request.
        //  * The record must come AFTER approval, otherwise a DENIED call is
        //    marked "done" and its repeat tells the model "duplicate_call". The
        //    right answer is "permission_denied": the model must know it was
        //    denied, not that it repeated itself.
        let key = call_key(call);
        if self.turn_calls.lock().expect("turn calls lock").contains(&key) {
            return self.outcome(
                &call.name,
                ExecutionReason::RepeatedCall,
                REPEAT_MODEL_TEXT.to_string(),
                // THE CHIP TEXT IS EMPTY: the user must not see this as an
                // "operation". The tool did not run, nothing happened in the
                // world; showing a second line on screen would present work that
                // was never done as if it had been.
                String::new(),
                ToolState::Read,
                None,
                false,
            );
        }

        // GATE 3 — approval. Only in a tainted session and only for external
        // tools. It is not asked in a clean session: what is rare gets read,
        // what is frequent stops being read and the gate loses its function.
        if self.external_tools.contains(&call.name) && self.session_tainted() {
            let content = serde_json::to_string(&call.args).unwrap_or_default();
            let request = ApprovalRequest {
                source: call.name.clone(),
                tool_name: call.name.clone(),
                content,
            };
            if !self.ask_approval(&request) {
                let trace = ctx.start_chip("approval", &format!("{} · not sent", request.source));
                ctx.update_chip(
                    trace,
                    tacet_core::TraceUpdate::state(ToolState::NeedsPermission)
                        .raw_input(request.content.clone()),
                );
                return self.outcome(
                    &call.name,
                    ExecutionReason::ApprovalDenied,
                    DENIAL_MODEL_TEXT.to_string(),
                    format!("{} · not sent", request.source),
                    ToolState::NeedsPermission,
                    None,
                    false,
                );
            }
        }

        // The record half of gate 5: the call really runs FROM HERE on.
        self.turn_calls.lock().expect("turn calls lock").insert(key);

        let outcome: ToolOutcome = tool.run(call.args.clone(), ctx).await;

        // The cancellation may have arrived WHILE the tool was running. We still
        // return the outcome, but if a write happened the flag is preserved —
        // "I cancelled it and the file was created anyway" must not be hidden.
        let world = outcome.state.changed_world();
        if world {
            self.world_changed.store(true, Ordering::SeqCst);
            ctx.taint();
        }

        // TAINTING: only Read/Written. NeedsPermission and Failed DO NOT TAINT
        // — no data was read, and tainting the session would tighten the gate
        // for no reason.
        let succeeded = matches!(outcome.state, ToolState::Read | ToolState::Written);
        if succeeded && tool.taints_session() {
            self.session_tainted.store(true, Ordering::SeqCst);
            ctx.taint();
        }

        let reason = match &outcome.state {
            ToolState::Failed(_) => ExecutionReason::ToolFailed,
            ToolState::NeedsPermission => ExecutionReason::ApprovalDenied,
            _ => ExecutionReason::Ok,
        };

        self.outcome(
            &call.name,
            reason,
            outcome.to_model,
            outcome.chip_text,
            outcome.state,
            outcome.raw_output,
            world,
        )
    }

    fn ask_approval(&self, request: &ApprovalRequest) -> bool {
        {
            let denial = self.denied.lock().expect("denial cache lock");
            if denial.contains(&request.source) {
                return false;
            }
        }
        let accept = self.gate.request(request);
        if !accept {
            self.denied
                .lock()
                .expect("denial cache lock")
                .insert(request.source.clone());
        }
        accept
    }

    #[allow(clippy::too_many_arguments)]
    fn outcome(
        &self,
        name: &str,
        reason: ExecutionReason,
        to_model: String,
        chip_text: String,
        state: ToolState,
        raw_output: Option<String>,
        world_changed: bool,
    ) -> ExecutionOutcome {
        ExecutionOutcome {
            tool_name: name.to_string(),
            reason,
            to_model,
            chip_text,
            state,
            world_changed,
            // It is checked at TURN level, not at this call's level: if a file
            // was written in an earlier step of the turn, this call's innocence
            // does not make a retry safe.
            retryable: !self.world_changed(),
            raw_output,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tacet_core::{
        Field, Tool, ToolFuture, ArgSchema, InMemoryDataStore, SilentReporter, boxed,
    };
    use std::sync::Arc;

    // --- Helpers ---

    /// A tool that reads personal data: it taints the session.
    struct PersonalTool;
    impl Tool for PersonalTool {
        fn name(&self) -> &str {
            "personal_read"
        }
        fn description(&self) -> &str {
            "Reads personal data."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![Field::new("what", ArgSchema::text()).required()])
        }
        fn taints_session(&self) -> bool {
            true
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::read_ok("read", "data: 42") })
        }
    }

    /// A tool that sends data out.
    struct ExternalTool;
    impl Tool for ExternalTool {
        fn name(&self) -> &str {
            "send_out"
        }
        fn description(&self) -> &str {
            "Sends the data out."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![Field::new("body", ArgSchema::text()).required()])
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::read_ok("sent", "sent") })
        }
    }

    /// A tool that changes the world.
    struct WritingTool;
    impl Tool for WritingTool {
        fn name(&self) -> &str {
            "write_file"
        }
        fn description(&self) -> &str {
            "Writes a file."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![])
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::written("written", "file_created") })
        }
    }

    /// A tool that always fails.
    struct CrashingTool;
    impl Tool for CrashingTool {
        fn name(&self) -> &str {
            "crashing"
        }
        fn description(&self) -> &str {
            "Always crashes."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![])
        }
        fn taints_session(&self) -> bool {
            true
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::failed(&ToolError::OutOfSpace) })
        }
    }

    fn context() -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            std::env::temp_dir(),
            Arc::new(SilentReporter),
        )
    }

    fn executor() -> ToolExecutor {
        let mut k = ToolCatalog::new();
        k.add(Arc::new(PersonalTool))
            .add(Arc::new(ExternalTool))
            .add(Arc::new(WritingTool))
            .add(Arc::new(CrashingTool));
        ToolExecutor::new(k).external_tool("send_out")
    }

    /// A minimal poll loop — this crate MUST NOT PICK an executor (the same
    /// rationale as in core), so there is no tokio in the test either.
    fn run<F: Future<Output = T>, T>(f: F) -> T {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn empty(_: *const ()) {}
        fn clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(clone, empty, empty, empty);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = std::pin::pin!(f);
        loop {
            if let Poll::Ready(v) = std::pin::Pin::as_mut(&mut f).poll(&mut cx) {
                return v;
            }
        }
    }

    use std::future::Future;

    // --- Parsing ---

    #[test]
    fn canonical_json_is_parsed() {
        let c = ToolCall::parse(r#"{"tool":"calculate","args":{"expression":"2+2"}}"#).unwrap();
        assert_eq!(c.name, "calculate");
        assert_eq!(c.args["expression"], "2+2");
    }

    #[test]
    fn the_call_format_is_parsed() {
        let c = ToolCall::parse(r#"calculate({"expression":"2+2"})"#).unwrap();
        assert_eq!(c.name, "calculate");
        assert_eq!(c.args["expression"], "2+2");
    }

    #[test]
    fn the_code_fence_is_stripped() {
        let c = ToolCall::parse("```json\n{\"tool\":\"time\",\"args\":{}}\n```").unwrap();
        assert_eq!(c.name, "time");
    }

    #[test]
    fn a_plain_answer_is_not_a_call() {
        assert!(ToolCall::parse("Hello, how can I help you?").is_none());
        assert!(ToolCall::parse("").is_none());
    }

    /// REAL MODEL OUTPUT (Qwen3-4B, verbatim shape; the model's preamble was
    /// Turkish in the captured run and is rendered in English here). The model
    /// writes its intent first and produces the call after it; the old parser
    /// swallowed this silently.
    #[test]
    fn a_call_with_a_plain_text_prefix_is_parsed() {
        let raw = "I will run a web search to look up the weather in Istanbul.\n\n\
                   web_search({\"query\":\"istanbul weather\"})";
        let c = ToolCall::parse(raw).expect("the prefixed call could not be parsed");
        assert_eq!(c.name, "web_search");
        assert_eq!(c.args["query"], "istanbul weather");
    }

    /// Parentheses in the prefix must not shadow the call: candidates are tried FROM THE RIGHT.
    #[test]
    fn a_parenthesis_in_the_prefix_does_not_shadow_the_call() {
        let raw = "First I need a calculation (simple arithmetic): calculate({\"expression\":\"2+2\"})";
        let c = ToolCall::parse(raw).expect("the call was not found");
        assert_eq!(c.name, "calculate");
        assert_eq!(c.args["expression"], "2+2");
    }

    /// There is a parenthesis but no identifier BEFORE it — that is not a call.
    #[test]
    fn a_parenthesis_without_an_identifier_does_not_count_as_a_call() {
        assert!(ToolCall::parse("This is an explanation (in parentheses).").is_none());
        assert!(ToolCall::parse("({\"a\":1})").is_none());
    }

    // --- Recovery (nameless JSON -> schema match) ---

    /// A REAL FAILURE: the model forgets the name and prints a bare `{...}`.
    /// `personal_read` makes 'what' required; the object carries only that -> a
    /// UNIQUE and clear match.
    #[test]
    fn nameless_json_is_recovered_when_it_fits_exactly_one_schema() {
        let y = executor();
        let c = recover_nameless_json(r#"{"what":"calendar"}"#, y.catalog()).expect("must be recovered");
        assert_eq!(c.name, "personal_read");
        assert_eq!(c.args["what"], "calendar");
    }

    /// Nameless JSON wrapped in a code fence must be recovered too (the small model adds the fence).
    #[test]
    fn nameless_json_is_recovered_even_inside_a_code_fence() {
        let y = executor();
        let c = recover_nameless_json("```json\n{\"body\":\"hello\"}\n```", y.catalog())
            .expect("must be recovered");
        assert_eq!(c.name, "send_out");
        assert_eq!(c.args["body"], "hello");
    }

    /// If a required field is missing NO recovery happens: there is no 'what',
    /// and 'title' is not required by any tool -> no candidate.
    #[test]
    fn a_missing_required_field_is_not_recovered() {
        let y = executor();
        assert!(recover_nameless_json(r#"{"title":"x"}"#, y.catalog()).is_none());
    }

    /// A foreign key breaks the match: 'what' belongs to personal_read, 'body'
    /// to send_out; together they fit NO single schema.
    #[test]
    fn a_foreign_key_is_not_recovered() {
        let y = executor();
        assert!(recover_nameless_json(r#"{"ne":"x","body":"y"}"#, y.catalog()).is_none());
    }

    /// An empty object, plain text and JSON inside prose: none of them count as
    /// a call (irrelevance is preserved).
    #[test]
    fn an_empty_object_plain_text_and_embedded_json_are_not_recovered() {
        let y = executor();
        assert!(recover_nameless_json("{}", y.catalog()).is_none());
        assert!(recover_nameless_json("Hello, how are you?", y.catalog()).is_none());
        assert!(recover_nameless_json("Thanks!", y.catalog()).is_none());
        // The whole text is NOT a single JSON object -> unparseable -> rejected.
        assert!(recover_nameless_json("Example: {\"what\":\"x\"} is written like this.", y.catalog()).is_none());
    }

    /// If two tools share the same single required field the match is AMBIGUOUS -> rejected.
    #[test]
    fn an_ambiguous_match_is_not_recovered() {
        struct A;
        impl Tool for A {
            fn name(&self) -> &str {
                "tool_a"
            }
            fn description(&self) -> &str {
                "a"
            }
            fn schema(&self) -> ArgSchema {
                ArgSchema::object(vec![Field::new("x", ArgSchema::text()).required()])
            }
            fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
                boxed(async move { ToolOutcome::read_ok("a", "a") })
            }
        }
        struct B;
        impl Tool for B {
            fn name(&self) -> &str {
                "tool_b"
            }
            fn description(&self) -> &str {
                "b"
            }
            fn schema(&self) -> ArgSchema {
                ArgSchema::object(vec![Field::new("x", ArgSchema::text()).required()])
            }
            fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
                boxed(async move { ToolOutcome::read_ok("b", "b") })
            }
        }
        let mut k = ToolCatalog::new();
        k.add(Arc::new(A)).add(Arc::new(B));
        assert!(recover_nameless_json(r#"{"x":"1"}"#, &k).is_none());
    }

    /// End to end: nameless JSON really runs the tool when it goes through `execute_raw`.
    #[test]
    fn nameless_json_runs_the_tool_when_it_goes_through_execute_raw() {
        let y = executor();
        let mut ctx = context();
        let s = run(y.execute_raw(r#"{"what":"calendar"}"#, y.active_turn(), &mut ctx))
            .expect("must be recovered and run");
        assert_eq!(s.reason, ExecutionReason::Ok);
        assert_eq!(s.to_model, "data: 42");
    }

    // --- The gates ---

    #[test]
    fn an_unknown_tool_does_not_run_and_returns_the_fixed_text() {
        let y = executor();
        let mut ctx = context();
        let s = run(y.execute(&ToolCall::new("yok_boyle", serde_json::json!({})), y.active_turn(), &mut ctx));
        assert_eq!(s.reason, ExecutionReason::UnknownTool);
        assert!(s.is_error());
        assert_eq!(s.to_model, ERROR_MODEL_TEXT);
    }

    #[test]
    fn on_a_schema_violation_the_tool_never_runs() {
        let y = executor();
        let mut ctx = context();
        // "what" is required; it is not given.
        let s = run(y.execute(&ToolCall::new("personal_read", serde_json::json!({})), y.active_turn(), &mut ctx));
        assert_eq!(s.reason, ExecutionReason::InvalidArguments);
        // Since the tool did not run the session was not tainted either.
        assert!(!y.session_tainted());
    }

    #[test]
    fn a_successful_personal_tool_taints_the_session() {
        let y = executor();
        let mut ctx = context();
        assert!(!y.session_tainted());
        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "calendar"})),
            y.active_turn(),
            &mut ctx,
        ));
        assert!(y.session_tainted());
        assert!(ctx.session_tainted());
    }

    #[test]
    fn a_failed_tool_does_not_taint_the_session() {
        // CrashingTool has taints_session()==true but returns Failed: no data
        // was read, so there is no reason to tighten the gate.
        let y = executor();
        let mut ctx = context();
        let s = run(y.execute(&ToolCall::new("crashing", serde_json::json!({})), y.active_turn(), &mut ctx));
        assert_eq!(s.reason, ExecutionReason::ToolFailed);
        assert!(!y.session_tainted());
    }

    #[test]
    fn in_a_clean_session_an_external_tool_passes_without_asking() {
        let y = executor(); // the default gate: SilentDeny
        let mut ctx = context();
        let s = run(y.execute(
            &ToolCall::new("send_out", serde_json::json!({"body": "hello"})),
            y.active_turn(),
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::Ok);
    }

    #[test]
    fn in_a_tainted_session_an_external_tool_hits_the_gate() {
        let y = executor();
        let mut ctx = context();
        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "calendar"})),
            y.active_turn(),
            &mut ctx,
        ));
        let s = run(y.execute(
            &ToolCall::new("send_out", serde_json::json!({"body": "data: 42"})),
            y.active_turn(),
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::ApprovalDenied);
        assert_eq!(s.to_model, DENIAL_MODEL_TEXT);
        // A denial is NOT a malfunction: the recovery path must not be triggered.
        assert!(!s.is_error());
    }

    #[test]
    fn if_approval_is_given_it_passes_even_in_a_tainted_session() {
        let mut k = ToolCatalog::new();
        k.add(Arc::new(PersonalTool)).add(Arc::new(ExternalTool));
        let y = ToolExecutor::new(k).external_tool("send_out").with_gate(AlwaysApprove);
        let mut ctx = context();
        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        let s = run(y.execute(
            &ToolCall::new("send_out", serde_json::json!({"body": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::Ok);
    }

    #[test]
    fn a_source_denied_once_is_never_asked_again() {
        /// A gate that counts HOW MANY TIMES IT WAS ASKED — the only evidence of the denial cache.
        struct CountingGate(Arc<std::sync::atomic::AtomicUsize>);
        impl ApprovalGate for CountingGate {
            fn request(&self, _i: &ApprovalRequest) -> bool {
                self.0.fetch_add(1, Ordering::SeqCst);
                false
            }
        }
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let mut k = ToolCatalog::new();
        k.add(Arc::new(PersonalTool)).add(Arc::new(ExternalTool));
        let y = ToolExecutor::new(k)
            .external_tool("send_out")
            .with_gate(CountingGate(Arc::clone(&counter)));
        let mut ctx = context();
        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        for _ in 0..3 {
            run(y.execute(
                &ToolCall::new("send_out", serde_json::json!({"body": "x"})),
                y.active_turn(),
                &mut ctx,
            ));
        }
        // Three attempts, ONE question: the model cannot enter an insistence loop.
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    // --- Side effect / retry ---

    #[test]
    fn once_the_world_changes_retry_is_forbidden() {
        let y = executor();
        let mut ctx = context();
        assert!(y.retry_safe());
        let s = run(y.execute(&ToolCall::new("write_file", serde_json::json!({})), y.active_turn(), &mut ctx));
        assert!(s.world_changed);
        assert!(!s.retryable);
        assert!(!y.retry_safe());
    }

    #[test]
    fn even_an_innocent_call_after_a_side_effect_cannot_be_retried() {
        let y = executor();
        let mut ctx = context();
        run(y.execute(&ToolCall::new("write_file", serde_json::json!({})), y.active_turn(), &mut ctx));
        let s = run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        assert!(!s.world_changed);
        // The call is innocent but the TURN is not.
        assert!(!s.retryable);
    }

    #[test]
    fn a_recovery_attempt_does_not_drop_the_side_effect_flag() {
        let y = executor();
        let mut ctx = context();
        run(y.execute(&ToolCall::new("write_file", serde_json::json!({})), y.active_turn(), &mut ctx));
        let ticket = y.recovery_attempt();
        assert_eq!(ticket, y.active_turn());
        assert!(!y.retry_safe());
        // A real new turn resets it.
        y.new_turn();
        assert!(y.retry_safe());
    }

    #[test]
    fn a_new_turn_does_not_clear_the_taint_a_chat_reset_does() {
        let y = executor();
        let mut ctx = context();
        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        y.new_turn();
        assert!(y.session_tainted(), "the context summary may carry personal data");
        y.reset_chat();
        assert!(!y.session_tainted());
    }

    // --- Cancellation ---

    #[test]
    fn in_a_cancelled_turn_no_tool_runs() {
        let y = executor();
        let mut ctx = context();
        let ticket = y.active_turn();
        y.cancel();
        let s = run(y.execute(&ToolCall::new("write_file", serde_json::json!({})), ticket, &mut ctx));
        assert_eq!(s.reason, ExecutionReason::Cancelled);
        assert!(!s.world_changed);
        assert!(!y.world_changed(), "the cancelled tool never ran");
    }

    #[test]
    fn cancelling_an_old_turn_does_not_bind_the_new_turn() {
        let y = executor();
        let mut ctx = context();
        let old = y.active_turn();
        y.cancel();
        assert!(y.is_cancelled(old));

        let new = y.new_turn();
        assert!(!y.is_cancelled(new), "cancelling an old turn must not drop the new one");
        let s = run(y.execute(&ToolCall::new("write_file", serde_json::json!({})), new, &mut ctx));
        assert_eq!(s.reason, ExecutionReason::Ok);
    }

    // --- Raw execution ---

    #[test]
    fn plain_text_is_not_executed() {
        let y = executor();
        let mut ctx = context();
        assert!(run(y.execute_raw("Hello!", y.active_turn(), &mut ctx)).is_none());
    }

    // --- GATE 5: the repeated call ---

    /// A tool that counts HOW MANY TIMES IT RAN. The claim is not "what result
    /// came back" but "did the tool really run": the cost of the loop failure is
    /// exactly the tool that RUNS AGAIN (a second file, a second network request).
    struct CountingTool(Arc<std::sync::atomic::AtomicUsize>);
    impl Tool for CountingTool {
        fn name(&self) -> &str {
            "say"
        }
        fn description(&self) -> &str {
            "Increments the call counter."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![Field::new("x", ArgSchema::text()).required()])
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            self.0.fetch_add(1, Ordering::SeqCst);
            boxed(async move { ToolOutcome::read_ok("counted", "ok") })
        }
    }

    fn counting() -> (ToolExecutor, Arc<std::sync::atomic::AtomicUsize>) {
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let catalog: ToolCatalog =
            [Arc::new(CountingTool(Arc::clone(&counter))) as Arc<dyn Tool>].into_iter().collect();
        (ToolExecutor::new(catalog), counter)
    }

    /// A REGRESSION TEST — the loop failure. The instruction's "do not call any
    /// more tools" sentence was relaxed; what holds the loop now is THIS gate.
    #[test]
    fn the_same_call_does_not_run_a_second_time_in_a_turn() {
        let (y, counter) = counting();
        let mut ctx = context();
        let ticket = y.new_turn();
        let call = ToolCall::new("say", serde_json::json!({"x": "a"}));

        let first = run(y.execute(&call, ticket, &mut ctx));
        assert_eq!(first.reason, ExecutionReason::Ok);

        for _ in 0..3 {
            let s = run(y.execute(&call, ticket, &mut ctx));
            assert_eq!(s.reason, ExecutionReason::RepeatedCall);
            // The recovery path MUST NOT OPEN: had it counted as an error the
            // call site would give the model one more turn and the loop we are
            // trying to prevent would be built.
            assert!(s.to_model.contains("duplicate_call"));
        }
        assert_eq!(counter.load(Ordering::SeqCst), 1, "the tool should have run only once");
    }

    /// The gate is ARGUMENT sensitive: calling the same tool with different
    /// arguments is legitimate (like reading two separate files) and must not be blocked.
    #[test]
    fn a_call_with_different_arguments_does_not_count_as_a_repeat() {
        let (y, counter) = counting();
        let mut ctx = context();
        let ticket = y.new_turn();
        for x in ["a", "b", "c"] {
            let s = run(y.execute(&ToolCall::new("say", serde_json::json!({"x": x})), ticket, &mut ctx));
            assert_eq!(s.reason, ExecutionReason::Ok);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    /// The gate is TURN SCOPED: it is legitimate for the user to ask the same
    /// thing again in a second message. What is forbidden is getting the same
    /// result twice within a single message.
    #[test]
    fn a_new_turn_clears_the_repeat_record() {
        let (y, counter) = counting();
        let mut ctx = context();
        let call = ToolCall::new("say", serde_json::json!({"x": "a"}));

        run(y.execute(&call, y.new_turn(), &mut ctx));
        run(y.execute(&call, y.new_turn(), &mut ctx));
        assert_eq!(counter.load(Ordering::SeqCst), 2, "it must run twice in separate turns");

        // A recovery attempt is NOT a real new turn: it must not be an excuse to
        // repeat a failed call verbatim.
        let s = run(y.execute(&call, y.recovery_attempt(), &mut ctx));
        assert_eq!(s.reason, ExecutionReason::RepeatedCall);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    /// Argument ORDER must not break sameness — the model may write the same
    /// call in two different orders and that does not make it a new call.
    #[test]
    fn argument_order_does_not_break_sameness() {
        let a = ToolCall::new("say", serde_json::json!({"x": "1", "y": "2"}));
        let b = ToolCall::new("say", serde_json::json!({"y": "2", "x": "1"}));
        assert_eq!(call_key(&a), call_key(&b));
        // A different value, a different key.
        let c = ToolCall::new("say", serde_json::json!({"x": "9", "y": "2"}));
        assert_ne!(call_key(&a), call_key(&c));
    }

    #[test]
    fn raw_json_is_executed() {
        let y = executor();
        let mut ctx = context();
        let s = run(y.execute_raw(
            r#"{"tool":"personal_read","args":{"what":"calendar"}}"#,
            y.active_turn(),
            &mut ctx,
        ))
        .unwrap();
        assert_eq!(s.reason, ExecutionReason::Ok);
        assert_eq!(s.to_model, "data: 42");
    }
}
