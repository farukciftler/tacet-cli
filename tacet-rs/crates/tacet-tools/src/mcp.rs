//! The MCP bridge: presents a remote MCP tool as if it were a local `Tool`.
//!
//! THE SWIFT LESSON: over there `MCPTool` was written, compiled, passed its tests
//! and was NEVER INSTANTIATED — the mechanism was built and nobody wired it up. So
//! as not to repeat that mistake, the WIRING functions live here next to the
//! bridge (`feed_catalog`, `bind_executor`) and both are genuinely tested: not "I
//! built it" but "it appears in the catalog and it triggers the approval gate".
//!
//! ## This file DOES NOT GO ON THE NETWORK
//!
//! The socket is in `tacet-mcp`. What happens here is translation: a schema
//! converted into an `ArgSchema`, a response converted into a `ToolOutcome`, a
//! network error converted into a `ToolError`. The network monopoly is preserved
//! by that separation.
//!
//! ## The security model (spec §3.3, §5.4) — WHAT IS and IS NOT here
//!
//! The B-permission model has three defences:
//!
//! - **(a) profile separation:** personal-data tools DO NOT SHARE a profile with
//!   MCP. `Router` establishes that; the contribution here is
//!   `MCPTool::taints_session()` returning `false` — an MCP tool is not a SOURCE of
//!   personal data but an EXIT for it.
//! - **(b) the approval gate on a tainted session:** in `ToolExecutor`. This file
//!   BINDS the tools' names into that gate's `external_tools` list
//!   (`bind_executor`). Unbound, the gate has no input — exactly the mistake of
//!   the first round.
//! - **(c) showing the real content at approval time:** `ToolExecutor` puts
//!   `call.args` verbatim into the approval request. There is no work for this
//!   file there, but we test that it is not broken.
//!
//! "Always allow" (spec §3.6) is DELIBERATELY ABSENT in v1: whatever the
//! `ApprovalGate` implementation says goes, and no path bypassing the gate was
//! opened here.
//!
//! ## Blocking
//!
//! `tacet-mcp` is a blocking client (the reasoning is there). The `run` body is
//! `async` but there is a waiting socket inside it; since there is no runtime in
//! this work tree and `tacet_engine::wait` drives futures on a single thread, the
//! behaviour is correct. The price must be stated openly: a 120-second MCP call
//! keeps that thread busy for that long.

use serde_json::Value;
use tacet_core::{
    ArgSchema, Tool, ToolCatalog, ToolContext, ToolError, ToolOutcome, TraceUpdate, boxed,
};
use tacet_mcp::{
    ConnectionSetting, Config, MCPClient, MCPError, convert_schema, truncate_description,
};
use std::sync::Arc;

/// The cap on raw output returned to the model (characters).
///
/// Output above this goes into the `DataStore`, and the model gets a summary + a
/// `source_ref` (§5.5, the 4096 bypass channel). The number was chosen to be about
/// 200 tokens: a file listing or a short command output should pass as is, a
/// ten-page build log should NOT.
pub const SHORT_OUTPUT_LIMIT: usize = 800;

/// The tail of long output handed to the model (lines).
///
/// WHY THE TAIL, NOT THE HEAD: in command/log style output the error lives AT THE
/// END. Showing the head would give the model the impression "everything is fine".
pub const TAIL_LINES: usize = 30;

// ---------------------------------------------------------------------------
// The bridge tool
// ---------------------------------------------------------------------------

/// The local tool representing a single remote MCP tool.
pub struct MCPTool {
    /// The name in the catalog. `<connection>_<remote_name>` — see
    /// `build_tool_name`.
    name: String,
    /// The real name on the server; `tools/call` wants this.
    remote_name: String,
    /// The user-friendly connection name shown at the head of the chip text. The
    /// user reads "this happened off the device" from the chip (§3.2).
    connection_name: String,
    description: String,
    schema: ArgSchema,
    client: Arc<MCPClient>,
}

impl MCPTool {
    pub fn new(
        name: impl Into<String>,
        remote_name: impl Into<String>,
        connection_name: impl Into<String>,
        description: impl Into<String>,
        schema: ArgSchema,
        client: Arc<MCPClient>,
    ) -> Self {
        Self {
            name: name.into(),
            remote_name: remote_name.into(),
            connection_name: connection_name.into(),
            description: description.into(),
            schema,
            client,
        }
    }

    pub fn remote_name(&self) -> &str {
        &self.remote_name
    }

    pub fn connection_name(&self) -> &str {
        &self.connection_name
    }
}

impl Tool for MCPTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> ArgSchema {
        self.schema.clone()
    }

    /// An MCP tool DOES NOT TAINT the session.
    ///
    /// Taint means "personal data WAS READ off the device"; an MCP tool does not
    /// read data, it SENDS IT OUT. Returning `true` here would close the gate on
    /// itself: the first MCP call would taint the session and the second would ask
    /// for approval — even though nothing was read off the device in between. That
    /// is how approval fatigue starts, and it would kill §2.4's "an approval that
    /// is rare gets read" principle.
    fn taints_session(&self) -> bool {
        false
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let trace =
                ctx.start_chip("connection", &format!("{} · running…", self.connection_name));
            ctx.update_chip(
                trace,
                TraceUpdate::default()
                    // The first layer of transparency: the outgoing content stands
                    // in the chip. Even the "always allow" mode of §3.6 does not
                    // drop it.
                    .raw_input(serde_json::to_string_pretty(&args).unwrap_or_default()),
            );

            let result = self.client.call_tool(&self.remote_name, &args);

            match result {
                Err(error) => {
                    // The two channels: a Turkish sentence to the chip, FIXED
                    // English text to the model. The translation goes through the
                    // single exit point of `ToolOutcome::failed`; raw `ureq`/server
                    // text DOES NOT LEAK from here.
                    let converted = convert_error(&error);
                    let chip = format!("{} · {}", self.connection_name, error.short_error());
                    ctx.update_chip(
                        trace,
                        TraceUpdate::state(tacet_core::ToolState::Failed(chip.clone()))
                            .text(chip.clone()),
                    );
                    let mut o = ToolOutcome::failed(&converted);
                    o.chip_text = chip;
                    o
                }
                Ok((text, server_error)) => {
                    let outcome = self.process_output(ctx, text, server_error);
                    ctx.update_chip(
                        trace,
                        TraceUpdate::state(outcome.state.clone())
                            .text(outcome.chip_text.clone())
                            .raw_output(outcome.raw_output.clone().unwrap_or_default()),
                    );
                    outcome
                }
            }
        })
    }
}

impl MCPTool {
    /// §5.5 — MCP output NEVER enters the context in its raw form.
    fn process_output(
        &self,
        ctx: &ToolContext,
        text: String,
        server_error: bool,
    ) -> ToolOutcome {
        let label = if server_error { "failed" } else { "ok" };
        let chip = format!("{} · {} {}", self.connection_name, self.remote_name, label);

        if text.trim().is_empty() {
            // Empty output DOES NOT mean "nothing happened" (a command may have
            // succeeded silently); we say that to the model explicitly, otherwise it
            // will try to repeat the tool call.
            return ToolOutcome::read_ok(chip, "tool returned no output").raw_output(text);
        }

        // Short output passes as is: using the bypass channel needlessly costs the
        // model another turn, and in a small model that turn is wasted.
        if text.chars().count() <= SHORT_OUTPUT_LIMIT {
            let to_model = if server_error {
                // THE SERVER'S tool error is not a malfunction but the tool's
                // result; the model must be able to read it and explain it to the
                // user. That is why `ERROR_MODEL_TEXT` IS NOT USED.
                format!("tool_error: {text}")
            } else {
                text.clone()
            };
            return ToolOutcome::read_ok(chip, to_model).raw_output(text);
        }

        // Long output: all of it to the store, the last lines + a source_ref to the
        // model.
        let lines: Vec<&str> = text.lines().collect();
        let tail = lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n");
        let summary = format!(
            "{} lines of output; last {} lines:\n{}",
            lines.len(),
            lines.len().min(TAIL_LINES),
            truncate(&tail, SHORT_OUTPUT_LIMIT),
        );
        let source_ref = ctx.store(
            "mcp",
            &format!("{} lines of output", lines.len()),
            text.clone(),
        );
        ToolOutcome::summarize(chip, summary, source_ref.as_str()).raw_output(text)
    }
}

/// Truncates long text at a character limit (respecting UTF-8 boundaries).
fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect::<String>() + "…"
}

/// `MCPError` -> `ToolError`.
///
/// The text going to the model will be `ERROR_MODEL_TEXT` in any case (a core
/// decision); the distinction only determines the sentence the USER sees.
fn convert_error(error: &MCPError) -> ToolError {
    match error {
        MCPError::Timeout => ToolError::Timeout,
        other => ToolError::Other(other.short_error()),
    }
}

// ---------------------------------------------------------------------------
// Name generation
// ---------------------------------------------------------------------------

/// The catalog name: `<connection>_<remote_name>`, ASCII snake_case.
///
/// WHY A PREFIX: two servers may both offer a tool called `run`. Without a prefix
/// the second shadows the first and the model cannot know which server it went to;
/// on top of that the approval gate's denial cache uses the tool name as the
/// `source`, so a denial given to one server would silence the other too.
///
/// WHY SIMPLIFICATION: the model has to produce the name verbatim in the grammar
/// and `ToolCall::parse` accepts only `[A-Za-z0-9_]`. A connection name with spaces
/// such as "home server" would produce an uncallable tool.
pub fn build_tool_name(connection_name: &str, remote_name: &str) -> String {
    format!("{}_{}", simplify_name(connection_name), simplify_name(remote_name))
}

fn simplify_name(raw: &str) -> String {
    let mut output = String::new();
    let mut underscore_pending = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            if underscore_pending && !output.is_empty() {
                output.push('_');
            }
            underscore_pending = false;
            output.extend(c.to_lowercase());
        } else {
            // Everything non-ASCII, Turkish letters and spaces included, counts as
            // a separator; consecutive separators collapse into a single
            // underscore.
            underscore_pending = true;
        }
    }
    output
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// A tool that could not be imported — NOT SWALLOWED SILENTLY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedTool {
    pub connection: String,
    pub remote_name: String,
    pub reason: String,
}

/// The load outcome of one connection.
#[derive(Default)]
pub struct LoadOutcome {
    pub tools: Vec<Arc<dyn Tool>>,
    pub skipped: Vec<SkippedTool>,
    /// If the connection could not be established, why (shown to the user).
    pub connection_errors: Vec<(String, String)>,
    /// Constraints dropped during conversion: "tool: field: `pattern` not
    /// enforced".
    pub notes: Vec<String>,
}

impl LoadOutcome {
    /// The names that will appear in the catalog. This is exactly the input of
    /// `ToolExecutor::external_tool`.
    pub fn names(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.name().to_string()).collect()
    }

    fn merge(&mut self, other: LoadOutcome) {
        self.tools.extend(other.tools);
        self.skipped.extend(other.skipped);
        self.connection_errors.extend(other.connection_errors);
        self.notes.extend(other.notes);
    }
}

/// A single connection: `initialize` + `tools/list` + the schema bridge.
/// **GOES ON THE NETWORK.**
pub fn load_connection(setting: &ConnectionSetting) -> LoadOutcome {
    let mut outcome = LoadOutcome::default();

    let client = match MCPClient::new(setting.url.clone(), setting.resolved_key()) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            outcome.connection_errors.push((setting.name.clone(), e.short_error()));
            return outcome;
        }
    };

    let specs = match client.tools() {
        Ok(s) => s,
        Err(e) => {
            outcome.connection_errors.push((setting.name.clone(), e.short_error()));
            return outcome;
        }
    };

    for spec in specs {
        match convert_schema(&spec.schema) {
            Ok(conversion) => {
                for note in &conversion.notes.0 {
                    outcome.notes.push(format!("{}: {note}", spec.name));
                }
                outcome.tools.push(Arc::new(MCPTool::new(
                    build_tool_name(&setting.name, &spec.name),
                    spec.name.clone(),
                    setting.name.clone(),
                    truncate_description(&spec.description),
                    conversion.schema,
                    Arc::clone(&client),
                )) as Arc<dyn Tool>);
            }
            // SILENTLY ACCEPTING SOMETHING UNCONVERTIBLE IS FORBIDDEN: a wrongly
            // narrowed schema means the grammar forces the model into a shape the
            // server will reject, and nobody can see it. The tool is skipped and the
            // reason is recorded.
            Err(reason) => outcome.skipped.push(SkippedTool {
                connection: setting.name.clone(),
                remote_name: spec.name,
                reason: reason.short(),
            }),
        }
    }

    outcome
}

/// Every valid connection in the configuration. **GOES ON THE NETWORK.**
///
/// With no connections it returns an empty outcome — NOT AN ERROR (§2.1 "off by
/// default").
pub fn load_from_config(config: &Config) -> LoadOutcome {
    let mut total = LoadOutcome::default();
    for setting in config.valid() {
        total.merge(load_connection(setting));
    }
    total
}

/// Loads from `mcp.json` in the configuration directory (the path is decided by
/// `tacet_core::env`, not by `tacet_mcp`). If the file is missing it stays quietly
/// empty. **GOES ON THE NETWORK.**
pub fn load_from_default() -> LoadOutcome {
    let mut outcome = LoadOutcome::default();
    match tacet_mcp::config::read_default() {
        Ok(c) => outcome.merge(load_from_config(&c)),
        Err(e) => outcome
            .connection_errors
            .push(("configuration".into(), e.short_error())),
    }
    outcome
}

/// Adds the loaded tools to the catalog and returns their names.
///
/// The returned names must be given to `bind_executor` — OTHERWISE the approval
/// gate has no input and the Swift mistake at the head of this file repeats.
#[must_use = "the returned names must be bound into ToolExecutor's external_tools list"]
pub fn feed_catalog(catalog: &mut ToolCatalog, outcome: &LoadOutcome) -> Vec<String> {
    for tool in &outcome.tools {
        catalog.add(Arc::clone(tool));
    }
    outcome.names()
}

/// Binds the MCP tools into the approval gate's `external_tools` list.
///
/// Since `ToolExecutor::external_tool` is a builder that consumes `self`, the loop
/// has to be written outside; it lives here once so the same loop is not repeated
/// at every call site.
pub fn bind_executor(
    mut executor: crate::executor::ToolExecutor,
    names: &[String],
) -> crate::executor::ToolExecutor {
    for name in names {
        executor = executor.external_tool(name.clone());
    }
    executor
}

use tacet_core::ToolFuture;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{
        AlwaysApprove, DENIAL_MODEL_TEXT, ExecutionReason, SilentDeny, ToolCall, ToolExecutor,
    };
    use serde_json::json;
    use tacet_core::{
        ERROR_MODEL_TEXT, Field, InMemoryDataStore, SilentReporter, ToolState, boxed,
    };
    use tacet_engine::wait;

    fn context() -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            std::env::temp_dir(),
            Arc::new(SilentReporter),
        )
    }

    /// A fake that TAKES THE PLACE of an MCP tool without going on the network: it
    /// fills the `Tool` contract exactly as the real `MCPTool` does (the same name
    /// generation, the same `taints_session`), so the approval gate tests measure
    /// the real path.
    struct FakeMCPTool {
        name: String,
    }

    impl Tool for FakeMCPTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "Runs a command on a remote server."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![Field::new("command", ArgSchema::text()).required()])
        }
        fn taints_session(&self) -> bool {
            false
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::read_ok("sent", "ok") })
        }
    }

    /// A personal-data tool — it taints the session.
    struct PersonalTool;
    impl Tool for PersonalTool {
        fn name(&self) -> &str {
            "calendar_read"
        }
        fn description(&self) -> &str {
            "Reads the calendar."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::empty()
        }
        fn taints_session(&self) -> bool {
            true
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::read_ok("read", "3 events") })
        }
    }

    fn mcp_catalog() -> (ToolCatalog, Vec<String>) {
        let name = build_tool_name("home server", "run_command");
        let mut catalog = ToolCatalog::new();
        catalog.add(Arc::new(PersonalTool));
        catalog.add(Arc::new(FakeMCPTool { name: name.clone() }));
        (catalog, vec![name])
    }

    // --- Name generation ---

    #[test]
    fn the_tool_name_is_produced_in_a_callable_form() {
        let name = build_tool_name("home server", "run-command");
        assert_eq!(name, "home_server_run_command");
        // The grammar and `ToolCall::parse` accept only this set.
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{name}");
    }

    #[test]
    fn turkish_and_consecutive_separators_collapse_into_one_underscore() {
        assert_eq!(build_tool_name("Work  Net", "get::info"), "work_net_get_info");
    }

    #[test]
    fn the_same_tool_name_on_two_servers_does_not_collide() {
        assert_ne!(build_tool_name("ev", "run"), build_tool_name("is", "run"));
    }

    // --- The approval gate: is the mechanism REALLY triggered ---

    #[test]
    fn an_mcp_call_passes_unasked_in_a_clean_session() {
        // §3.3: in a session where no personal-data tool was touched, approval IS
        // NOT ASKED. An approval seen often stops being read; the gate's value is in
        // its rarity.
        let (catalog, names) = mcp_catalog();
        let executor = bind_executor(ToolExecutor::new(catalog), &names).with_gate(SilentDeny);
        let mut ctx = context();
        let ticket = executor.active_turn();

        let outcome = wait(executor.execute(
            &ToolCall::new(&names[0], json!({"command": "git pull"})),
            ticket,
            &mut ctx,
        ));
        assert_eq!(
            outcome.reason,
            ExecutionReason::Ok,
            "approval must not be asked in a clean session"
        );
    }

    #[test]
    fn an_mcp_call_in_a_tainted_session_hits_the_approval_gate() {
        // This was the gap of the first round: the gate existed, `EXTERNAL_TOOLS`
        // was empty, i.e. it could not be triggered. This test measures exactly that
        // wiring.
        let (catalog, names) = mcp_catalog();
        let executor = bind_executor(ToolExecutor::new(catalog), &names).with_gate(SilentDeny);
        let mut ctx = context();
        let ticket = executor.active_turn();

        // First personal data is read -> the session gets tainted.
        let first = wait(executor.execute(
            &ToolCall::new("calendar_read", json!({})),
            ticket,
            &mut ctx,
        ));
        assert_eq!(first.reason, ExecutionReason::Ok);
        assert!(executor.session_tainted(), "the personal tool should have tainted the session");

        // Now the MCP call must hit the gate.
        let outcome = wait(executor.execute(
            &ToolCall::new(&names[0], json!({"command": "send the meeting"})),
            ticket,
            &mut ctx,
        ));
        assert_eq!(outcome.reason, ExecutionReason::ApprovalDenied);
        assert_eq!(outcome.state, ToolState::NeedsPermission);
        // A denial returns to the model as a NORMAL result, not as a malfunction
        // (§3.3).
        assert_eq!(outcome.to_model, DENIAL_MODEL_TEXT);
        assert!(!outcome.is_error(), "a denial is not a malfunction but the user's decision");
    }

    #[test]
    fn the_call_passes_in_an_approved_tainted_session() {
        let (catalog, names) = mcp_catalog();
        let executor = bind_executor(ToolExecutor::new(catalog), &names).with_gate(AlwaysApprove);
        let mut ctx = context();
        let ticket = executor.active_turn();

        wait(executor.execute(&ToolCall::new("calendar_read", json!({})), ticket, &mut ctx));
        let outcome = wait(executor.execute(
            &ToolCall::new(&names[0], json!({"command": "send"})),
            ticket,
            &mut ctx,
        ));
        assert_eq!(outcome.reason, ExecutionReason::Ok);
    }

    #[test]
    fn an_unbound_mcp_tool_never_triggers_the_gate() {
        // The reason this test exists: "forgetting to wire it up" is a silent
        // security loss and the compiler does not catch it. Here it becomes visible.
        let (catalog, names) = mcp_catalog();
        let executor = ToolExecutor::new(catalog).with_gate(SilentDeny); // NOT BOUND
        let mut ctx = context();
        let ticket = executor.active_turn();

        wait(executor.execute(&ToolCall::new("calendar_read", json!({})), ticket, &mut ctx));
        let outcome = wait(executor.execute(
            &ToolCall::new(&names[0], json!({"command": "send"})),
            ticket,
            &mut ctx,
        ));
        assert_eq!(
            outcome.reason,
            ExecutionReason::Ok,
            "unbound, the data leaves without being asked — `bind_executor` is mandatory"
        );
    }

    /// A gate that records every approval asked: it measures both HOW MANY TIMES it
    /// was asked and WHAT was shown. It always denies.
    struct RegistryGate(std::sync::Mutex<Vec<String>>);

    impl crate::executor::ApprovalGate for Arc<RegistryGate> {
        fn request(&self, request: &crate::executor::ApprovalRequest) -> bool {
            self.0.lock().expect("registry lock").push(request.content.clone());
            false
        }
    }

    fn registry_gate() -> Arc<RegistryGate> {
        Arc::new(RegistryGate(std::sync::Mutex::new(Vec::new())))
    }

    #[test]
    fn a_second_approval_is_not_asked_in_the_same_session() {
        let (catalog, names) = mcp_catalog();
        let gate = registry_gate();
        let executor =
            bind_executor(ToolExecutor::new(catalog), &names).with_gate(Arc::clone(&gate));
        let mut ctx = context();
        let ticket = executor.active_turn();
        wait(executor.execute(&ToolCall::new("calendar_read", json!({})), ticket, &mut ctx));

        for _ in 0..3 {
            let o = wait(executor.execute(
                &ToolCall::new(&names[0], json!({"command": "send"})),
                ticket,
                &mut ctx,
            ));
            assert_eq!(o.reason, ExecutionReason::ApprovalDenied);
        }
        // §3.3: "ToolExecutor DOES NOT PRODUCE a second approval chip in the same
        // session" — the model cannot get into an insistence loop.
        assert_eq!(gate.0.lock().expect("registry lock").len(), 1);
    }

    #[test]
    fn the_content_shown_at_approval_is_the_real_payload() {
        let (catalog, names) = mcp_catalog();
        let gate = registry_gate();
        let executor =
            bind_executor(ToolExecutor::new(catalog), &names).with_gate(Arc::clone(&gate));
        let mut ctx = context();
        let ticket = executor.active_turn();
        wait(executor.execute(&ToolCall::new("calendar_read", json!({})), ticket, &mut ctx));
        wait(executor.execute(
            &ToolCall::new(&names[0], json!({"command": "tomorrow 14:00 meeting"})),
            ticket,
            &mut ctx,
        ));

        let seen = gate.0.lock().expect("registry lock").clone();
        assert_eq!(seen.len(), 1);
        // §3.3: the body is "the real content, not a category summary".
        assert!(seen[0].contains("tomorrow 14:00 meeting"), "{}", seen[0]);
    }

    // --- Catalog wiring ---

    #[test]
    fn feed_catalog_really_makes_the_tools_visible() {
        let mut outcome = LoadOutcome::default();
        outcome.tools.push(Arc::new(FakeMCPTool {
            name: build_tool_name("ev", "run"),
        }));

        let mut catalog = ToolCatalog::new();
        let names = feed_catalog(&mut catalog, &outcome);

        assert_eq!(names, vec!["ev_run".to_string()]);
        assert!(catalog.find("ev_run").is_some(), "it MUST BE VISIBLE in the catalog");
        // An MCP tool is not a SOURCE of personal data: it must not enter the
        // tainting list.
        assert!(catalog.tainting_tools().is_empty());
    }

    #[test]
    fn with_no_connection_the_load_is_empty_and_error_free() {
        // §2.1 off by default: with no connection the network is never touched, and
        // there is no error either.
        let outcome = load_from_config(&Config::default());
        assert!(outcome.tools.is_empty());
        assert!(outcome.connection_errors.is_empty());
        assert!(outcome.skipped.is_empty());
    }

    #[test]
    fn an_invalid_address_is_recorded_as_an_error_without_going_on_the_network() {
        let setting = ConnectionSetting {
            name: "bad".into(),
            url: "http://remote-server.example.com/mcp".into(),
            key: None,
            key_env: None,
            enabled: true,
        };
        let outcome = load_connection(&setting);
        assert!(outcome.tools.is_empty());
        assert_eq!(outcome.connection_errors.len(), 1);
        assert_eq!(outcome.connection_errors[0].0, "bad");
    }

    // --- Output processing (the 4096 bypass) ---

    fn sample_tool() -> MCPTool {
        MCPTool::new(
            "ev_run",
            "run",
            "home server",
            "Runs a command.",
            ArgSchema::empty(),
            Arc::new(MCPClient::new("https://example.com/mcp", None).expect("client")),
        )
    }

    #[test]
    fn short_output_passes_as_is() {
        let ctx = context();
        let outcome = sample_tool().process_output(&ctx, "two files updated".into(), false);
        assert_eq!(outcome.to_model, "two files updated");
        assert!(outcome.chip_text.starts_with("home server · "));
    }

    #[test]
    fn long_output_is_not_dumped_on_the_model_and_goes_to_the_store() {
        let ctx = context();
        // Put a marker that occurs only at the end: let the tail rule be proven.
        let mut lines: Vec<String> =
            (0..500).map(|i| format!("line {i} filler filler")).collect();
        lines.push("ERROR: the build failed".into());
        let raw = lines.join("\n");

        let outcome = sample_tool().process_output(&ctx, raw.clone(), false);

        // Bulk data MUST NOT PASS to the model.
        assert!(!outcome.to_model.contains("line 10 filler"), "{}", outcome.to_model);
        assert!(outcome.to_model.len() < raw.len() / 4);
        // But the error lives in the tail (§5.5).
        assert!(outcome.to_model.contains("ERROR: the build failed"));
        // A single wire format: `source_ref_suffix`.
        assert!(outcome.to_model.contains("source_ref=mcp#1"), "{}", outcome.to_model);
        // All of it is in the store; the raw form also stands in the chip detail
        // (transparency).
        let record = ctx
            .from_store(&tacet_core::SourceRef("mcp#1".into()))
            .expect("must be in the store");
        assert_eq!(record.body, raw);
        assert_eq!(outcome.raw_output.as_deref(), Some(raw.as_str()));
    }

    #[test]
    fn the_servers_tool_error_is_explained_to_the_model_not_hidden_as_a_malfunction() {
        let ctx = context();
        let outcome = sample_tool().process_output(&ctx, "exit 1: no such file".into(), true);
        // `isError` is NOT a transport malfunction: the model must be able to read
        // it and explain it to the user.
        assert!(outcome.to_model.contains("no such file"));
        assert_ne!(outcome.to_model, ERROR_MODEL_TEXT);
        assert!(outcome.chip_text.contains("failed"));
    }

    #[test]
    fn empty_output_does_not_stay_silent() {
        let ctx = context();
        let outcome = sample_tool().process_output(&ctx, "   ".into(), false);
        assert_eq!(outcome.to_model, "tool returned no output");
    }

    #[test]
    fn a_transport_error_goes_through_the_two_channels() {
        // A Turkish sentence to the user, FIXED English text to the model. Raw
        // `ureq` or server text MUST NOT LEAK to the model.
        let error = convert_error(&MCPError::Server("secret server trace".into()));
        let outcome = ToolOutcome::failed(&error);
        assert_eq!(outcome.to_model, ERROR_MODEL_TEXT);
        assert!(!outcome.to_model.contains("secret"));

        assert!(matches!(convert_error(&MCPError::Timeout), ToolError::Timeout));
    }
}
