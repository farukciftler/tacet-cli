//! The client that talks to a single MCP server.
//!
//! Transport: **plain request/response HTTP** (spec 2026-07-28, SEP-2575 and
//! SEP-2567). Every request is self-describing: there is no `initialize`
//! handshake, no `Mcp-Session-Id`, and no server push. What the server needs to
//! route and to know who is calling travels in headers and in `_meta` on every
//! single request.
//!
//! THE OLD REVISION IS STILL HERE, frozen, in `legacy.rs` — most public servers
//! will speak it for months. Which one is used per server is the `spec` field
//! in `mcp.json` (`auto` by default); see `legacy.rs` for the sunset date.
//!
//! ## Why blocking
//!
//! There is NO runtime in this dependency tree: `tacet_engine::wait` drives the
//! futures on a single thread by busy-waiting. Because `MCPClient` is blocking,
//! the call site (`tacet-tools::mcp`) can call it directly from an `async` body
//! and no tokio is imposed on the architecture. The cost must be said openly:
//! while an MCP call is in flight, that thread does no other work.
//!
//! ## Why `&self` + `Mutex`
//!
//! `Tool::run` takes `&self` and the catalog holds `Arc<dyn Tool>`; changing
//! state such as the catalog cache has to live behind an internal lock.
//!
//! ## What is deliberately NOT here
//!
//! - `sampling` — refused permanently, in both revisions. A remote party
//!   pulling output out of the local model is the exact thing this product
//!   exists to prevent, so the client never offers the capability and answers
//!   the method with an error (see `handle_sampling`).
//! - Held-open streams (`subscriptions/listen`). The client is blocking
//!   request/response, full stop.
//! - Auto-retry of `tools/call`. Statelessness invites retries and a retried
//!   call WITH EFFECTS is a double-send. Idempotent reads may retry once.

use crate::elicit::{self, InputAsk, Question};
use crate::error::{MCPError, MCPResult};
use crate::jsonrpc;
use crate::sse;
use crate::transport::{HttpTransport, Reply, Request, Transport};
use serde_json::{Value, json};
use std::io::{BufReader, Read};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The MCP revision this client speaks.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

pub use crate::transport::TIMEOUT_S;

/// The cap on the `tools/list` pagination loop: a misbehaving server must not
/// lock us in forever by returning the same cursor over and over.
const MAX_PAGES: usize = 20;

/// The cap on the number of tools. Only a handful fit in the context window
/// anyway; loading thousands of tools into memory helps nobody.
const MAX_TOOLS: usize = 200;

/// The cap on a response body.
///
/// WHY THERE IS A CAP: `as_reader()` is unlimited BY UREQ'S OWN ADMISSION —
/// its documentation says a malicious server could exhaust all available
/// memory — and both consumers of that reader accumulate without a bound of
/// their own. The 120 second timeout only caps the DURATION; at local network
/// speed that is gigabytes. Measured before this cap: a single 64 MB
/// `tools/call` answer was swallowed whole and then written to the DataStore.
const MAX_BODY: u64 = 8 * 1024 * 1024;

/// The ceiling on how long a server may pin its own catalog (SEP-2549).
///
/// The clamp is OURS, not the spec's: `ttlMs` is a number the far side chooses,
/// and a hostile or simply broken server must not be able to pin a stale
/// catalog for a month. A day is long enough to be worth caching and short
/// enough that a wrong catalog fixes itself without anyone filing a bug.
pub const CATALOG_TTL_CAP: Duration = Duration::from_secs(24 * 60 * 60);

/// A `tools/list` that says nothing about caching is cached for this long —
/// long enough to spare a second process the round trip, short enough that
/// "the tool list is stale" is never a thing anyone has to think about.
const DEFAULT_TTL: Duration = Duration::from_secs(5 * 60);

/// Which revision a connection speaks. `mcp.json`'s `spec` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpecChoice {
    /// Try 2026-07-28; on a version rejection fall back ONCE to the frozen
    /// path for this session. NOTHING IS WRITTEN BACK to the config — silent
    /// config mutation is not this product's style. Pin `legacy` yourself if
    /// you want the probe gone.
    #[default]
    Auto,
    Current,
    Legacy,
}

impl SpecChoice {
    /// Reads the `spec` field. An unknown value is `None` so the config layer
    /// can say so out loud instead of silently choosing for the user.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Some(Self::Auto),
            "2026-07-28" | "current" | "new" => Some(Self::Current),
            "legacy" | "2025-06-18" => Some(Self::Legacy),
            _ => None,
        }
    }
}

/// Which revision a request is actually being written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Revision {
    Current,
    Legacy,
}

impl Revision {
    pub fn label(self) -> &'static str {
        match self {
            Self::Current => PROTOCOL_VERSION,
            Self::Legacy => crate::legacy::LEGACY_PROTOCOL_VERSION,
        }
    }
}

/// A single tool definition coming from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    /// The RAW description — it may run 100-500 tokens. It DOES NOT ENTER the
    /// context IN THIS FORM; it is shortened by `bridge::truncate_description`.
    pub description: String,
    /// `inputSchema` (JSON Schema). The bridge converts this into an `ArgSchema`.
    pub schema: Value,
}

/// Server state carried across calls.
#[derive(Default)]
pub(crate) struct State {
    /// LEGACY ONLY — the current revision has no sessions.
    pub(crate) session_id: Option<String>,
    /// LEGACY ONLY.
    pub(crate) agreed_version: Option<String>,
    /// LEGACY ONLY.
    pub(crate) handshaken: bool,
    pub(crate) counter: u64,
    /// `None` until the first request decides.
    pub(crate) revision: Option<Revision>,
    pub(crate) fell_back: bool,
    pub(crate) discovered: bool,
    /// The catalog and when it goes stale (SEP-2549).
    pub(crate) cache: Option<(Instant, Vec<ToolSpec>)>,
}

pub struct MCPClient {
    pub(crate) url: String,
    /// The bearer key. In memory only; NEVER written to a log or a chip text.
    pub(crate) key: Option<String>,
    pub(crate) choice: SpecChoice,
    pub(crate) transport: Arc<dyn Transport>,
    pub(crate) asker: Arc<dyn InputAsk>,
    /// Who is told that a remote task is still running (spec §6).
    pub(crate) watch: Arc<dyn crate::tasks::TaskWatch>,
    /// The floor on the poll interval. Production never moves it; the tests
    /// lower it so measuring a three-poll task does not cost three real
    /// seconds. Named rather than hidden: a test-facing knob that pretends not
    /// to exist is how a production default gets changed by accident.
    pub(crate) poll_floor: Duration,
    pub(crate) state: Mutex<State>,
}

/// A failed send, plus the ONE thing the caller may need to know beyond the
/// message: was this the server refusing the revision itself.
pub(crate) struct Failure {
    pub(crate) error: MCPError,
    pub(crate) version_rejected: bool,
}

impl From<MCPError> for Failure {
    fn from(error: MCPError) -> Self {
        Self {
            error,
            version_rejected: false,
        }
    }
}

impl MCPClient {
    /// If `url` is not accepted it stops here — nothing goes online at all.
    pub fn new(url: impl Into<String>, key: Option<String>) -> MCPResult<Self> {
        let url = url.into();
        validate_url(&url)?;
        Ok(Self {
            url,
            key,
            choice: SpecChoice::default(),
            transport: Arc::new(HttpTransport::new()),
            asker: Arc::new(elicit::DeclineInput),
            watch: Arc::new(crate::tasks::SilentWatch),
            poll_floor: crate::tasks::POLL_MIN,
            state: Mutex::new(State::default()),
        })
    }

    pub fn with_spec(mut self, choice: SpecChoice) -> Self {
        self.choice = choice;
        self
    }

    /// Swaps the socket for something else — the record/replay transport in the
    /// protocol tests. There is no other way in: the transport is not settable
    /// after construction.
    pub fn with_transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = transport;
        self
    }

    /// Who answers a server's questions (spec §4). Left alone, nobody does.
    pub fn with_asker(mut self, asker: Arc<dyn InputAsk>) -> Self {
        self.asker = asker;
        self
    }

    /// Who is told a remote task is still running. Left alone, nobody is.
    pub fn with_watch(mut self, watch: Arc<dyn crate::tasks::TaskWatch>) -> Self {
        self.watch = watch;
        self
    }

    /// Lowers the poll floor. FOR TESTS: a task measured over three polls must
    /// not cost three real seconds of CI time.
    pub fn with_poll_floor(mut self, floor: Duration) -> Self {
        self.poll_floor = floor;
        self
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn spec_choice(&self) -> SpecChoice {
        self.choice
    }

    /// Which revision this connection settled on, once it has spoken.
    pub fn revision(&self) -> Option<Revision> {
        self.state.lock().expect("mcp state lock").revision
    }

    /// Did `auto` fall back to the frozen path — the chip says so when it did.
    pub fn fell_back(&self) -> bool {
        self.state.lock().expect("mcp state lock").fell_back
    }

    /// Drops the cached catalog. Called by an explicit refresh; there is no
    /// background refresh, because there is no background.
    pub fn refresh(&self) {
        self.state.lock().expect("mcp state lock").cache = None;
    }

    /// LEGACY COMPATIBILITY. On the current revision there is no handshake and
    /// this does nothing at all — kept so the one online smoke test and any
    /// caller written against the old client still compile and behave.
    pub fn handshake(&self) -> MCPResult<()> {
        match self.planned_revision() {
            Revision::Current => Ok(()),
            Revision::Legacy => crate::legacy::handshake(self).map_err(|f| f.error),
        }
    }

    /// The server's own description of itself (`server/discover`).
    ///
    /// LAZY AND ONCE: capabilities are also derivable from responses, so this
    /// is a nicety, not a precondition. Its failure is not the connection's
    /// failure — the caller may ignore it.
    pub fn discover(&self) -> MCPResult<Value> {
        {
            let mut state = self.state.lock().expect("mcp state lock");
            if state.discovered {
                return Ok(Value::Null);
            }
            state.discovered = true;
        }
        self.send_read(Revision::Current, "server/discover", None, json!({}))
            .map_err(|f| f.error)
    }

    /// The server's tools, cached per SEP-2549.
    pub fn tools(&self) -> MCPResult<Vec<ToolSpec>> {
        if let Some(cached) = self.cached_tools() {
            return Ok(cached);
        }
        let revision = self.planned_revision();
        let outcome = match revision {
            Revision::Current => {
                // Discovery rides along with the first catalog build and is
                // never allowed to break it.
                let _ = self.discover();
                self.fetch_tools(Revision::Current)
            }
            Revision::Legacy => self.fetch_tools(Revision::Legacy),
        };

        let (specs, ttl) = match outcome {
            Ok(pair) => {
                self.settle(revision);
                pair
            }
            // THE ONE FALLBACK. It happens HERE, while building the catalog,
            // and nowhere else: a tool cannot be called before it is in the
            // catalog, so by call time the revision is already settled — which
            // is how `tools/call` keeps its "never retried" promise while
            // `auto` still works.
            Err(failure)
                if failure.version_rejected
                    && self.choice == SpecChoice::Auto
                    && revision == Revision::Current =>
            {
                {
                    let mut state = self.state.lock().expect("mcp state lock");
                    state.fell_back = true;
                    state.revision = Some(Revision::Legacy);
                }
                self.fetch_tools(Revision::Legacy).map_err(|f| f.error)?
            }
            Err(failure) => return Err(failure.error),
        };

        let ttl = clamp_ttl(ttl);
        let mut state = self.state.lock().expect("mcp state lock");
        state.cache = Some((Instant::now() + ttl, specs.clone()));
        Ok(specs)
    }

    /// Calls the remote tool and reduces the content to plain text.
    ///
    /// IMPORTANT: the approval gate is passed BEFORE this, in `ToolExecutor`.
    /// Everything that gets here is what the user saw and approved.
    ///
    /// The returned `is_error` is the SERVER's own tool error (`isError`): not a
    /// transport failure but the tool's normal outcome, and it is told to the
    /// model.
    ///
    /// NEVER RETRIED on transport error — a retried call with effects is a
    /// double-send. `server` is the connection's user-visible name; it prefixes
    /// any question the server asks (spec §4).
    pub fn call_tool(&self, name: &str, arguments: &Value) -> MCPResult<(String, bool)> {
        self.call_tool_as(name, arguments, name)
    }

    /// `call_tool`, with the label questions are prefixed with.
    pub fn call_tool_as(
        &self,
        name: &str,
        arguments: &Value,
        server: &str,
    ) -> MCPResult<(String, bool)> {
        self.call_tool_watching(name, arguments, server, Arc::clone(&self.watch))
    }

    /// `call_tool_as`, with the watcher for THIS call. The chip belongs to one
    /// call, so the thing that paints it cannot live on a client shared by all
    /// of them.
    pub fn call_tool_watching(
        &self,
        name: &str,
        arguments: &Value,
        server: &str,
        watch: Arc<dyn crate::tasks::TaskWatch>,
    ) -> MCPResult<(String, bool)> {
        let revision = self.planned_revision();
        if revision == Revision::Legacy {
            crate::legacy::handshake(self).map_err(|f| f.error)?;
            let result = self
                .send_once(
                    Revision::Legacy,
                    "tools/call",
                    Some(name),
                    json!({ "name": name, "arguments": arguments }),
                )
                .map_err(|f| f.error)?;
            return Ok(flatten_content(&result));
        }

        // MRTR (spec §4): the server may answer with questions instead of a
        // result; we answer and re-send THE SAME call. The rounds are counted
        // so a server cannot hold the turn hostage.
        let mut responses: Option<Value> = None;
        let mut rounds = 0usize;
        loop {
            let mut params = json!({ "name": name, "arguments": arguments });
            if let Some(answers) = &responses {
                params["inputResponses"] = answers.clone();
            }
            let result = self
                .send_once(Revision::Current, "tools/call", Some(name), params)
                .map_err(|f| f.error)?;

            if !elicit::is_input_required(&result) {
                self.settle(Revision::Current);
                // The call may have STARTED something rather than finished it
                // (spec §6). Waiting for it is polling, never a held stream.
                if let Some(task) = crate::tasks::task_id(&result) {
                    return self.await_task(&task, watch.as_ref());
                }
                return Ok(flatten_content(&result));
            }

            rounds += 1;
            if rounds > elicit::MAX_INPUT_ROUNDS {
                return Err(MCPError::InputRounds(elicit::MAX_INPUT_ROUNDS));
            }
            let questions: Vec<Question> = elicit::parse_questions(&result);
            let Some(answers) = self.asker.ask(server, &questions) else {
                // Declined or abandoned: the retry is NEVER sent.
                return Err(MCPError::InputDeclined);
            };
            responses = Some(elicit::build_responses(&questions, &answers));
        }
    }

    /// Waits for a remote task, politely and with an end (spec §6).
    ///
    /// `tasks/get` is a READ, so it may retry once on a transport hiccup — a
    /// poll that repeats changes nothing on the far side. The deadline ends OUR
    /// WAITING, not the task: the sentence says so, because a user told "it
    /// failed" would go looking for a failure that never happened.
    fn await_task(
        &self,
        task: &str,
        watch: &dyn crate::tasks::TaskWatch,
    ) -> MCPResult<(String, bool)> {
        let started = Instant::now();
        let mut wait = crate::tasks::interval(None).max(self.poll_floor);
        loop {
            let result = self
                .send_read(
                    Revision::Current,
                    "tasks/get",
                    Some(task),
                    json!({ "taskId": task }),
                )
                .map_err(|f| f.error)?;
            let state = crate::tasks::state(&result);
            watch.tick(&crate::tasks::Progress {
                id: task.to_string(),
                status: state.label().to_string(),
                elapsed: started.elapsed(),
            });
            if state.is_done() {
                return Ok(flatten_content(&result));
            }
            if started.elapsed() >= crate::tasks::DEADLINE {
                return Err(MCPError::TaskDeadline(
                    crate::tasks::DEADLINE.as_secs(),
                ));
            }
            wait = crate::tasks::interval(crate::tasks::suggested_ms(&result))
                .max(self.poll_floor);
            std::thread::sleep(wait);
        }
    }

    /// A server asking the CLIENT to run its model. Refused, permanently, in
    /// both revisions — the whole product exists to keep the local model's
    /// output local. Kept as a named function so the refusal is greppable and
    /// so nobody has to wonder whether we "just don't support it yet".
    pub fn handle_sampling(&self, id: u64) -> Vec<u8> {
        jsonrpc::error_body(
            id,
            -32601,
            "sampling is not offered by this client and never will be",
        )
    }

    // --- Internals ---

    fn cached_tools(&self) -> Option<Vec<ToolSpec>> {
        let state = self.state.lock().expect("mcp state lock");
        match &state.cache {
            Some((expiry, specs)) if Instant::now() < *expiry => Some(specs.clone()),
            _ => None,
        }
    }

    /// Which revision the next request will be written in.
    fn planned_revision(&self) -> Revision {
        let state = self.state.lock().expect("mcp state lock");
        if let Some(settled) = state.revision {
            return settled;
        }
        match self.choice {
            SpecChoice::Legacy => Revision::Legacy,
            _ => Revision::Current,
        }
    }

    fn settle(&self, revision: Revision) {
        let mut state = self.state.lock().expect("mcp state lock");
        if state.revision.is_none() {
            state.revision = Some(revision);
        }
    }

    pub(crate) fn next_id(&self) -> u64 {
        let mut state = self.state.lock().expect("mcp state lock");
        state.counter += 1;
        state.counter
    }

    /// The catalog, one page at a time. Returns the specs and the server's own
    /// cache lifetime if it stated one.
    fn fetch_tools(
        &self,
        revision: Revision,
    ) -> Result<(Vec<ToolSpec>, Option<Duration>), Failure> {
        if revision == Revision::Legacy {
            crate::legacy::handshake(self)?;
        }
        let mut total: Vec<ToolSpec> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut ttl: Option<Duration> = None;
        let mut page = 0usize;

        loop {
            page += 1;
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let result = self.send_read(revision, "tools/list", None, params)?;
            total.extend(extract_specs(&result));
            if ttl.is_none() {
                ttl = result
                    .get("ttlMs")
                    .and_then(Value::as_u64)
                    .map(Duration::from_millis);
            }

            let next = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            // A server handing back the same cursor must not keep us in a loop.
            cursor = if next == cursor { None } else { next };
            if cursor.is_none() || page >= MAX_PAGES || total.len() >= MAX_TOOLS {
                break;
            }
        }

        total.truncate(MAX_TOOLS);
        Ok((total, ttl))
    }

    /// An IDEMPOTENT call — a read. Retried EXACTLY ONCE on a transport error,
    /// because a read that changes nothing costs nothing to repeat. Anything
    /// with effects goes through `send_once` instead.
    pub(crate) fn send_read(
        &self,
        revision: Revision,
        method: &str,
        name: Option<&str>,
        params: Value,
    ) -> Result<Value, Failure> {
        match self.send_once(revision, method, name, params.clone()) {
            Err(failure) if is_transport_error(&failure.error) && !failure.version_rejected => {
                self.send_once(revision, method, name, params)
            }
            other => other,
        }
    }

    /// One request, one response. No retry, whatever happens.
    pub(crate) fn send_once(
        &self,
        revision: Revision,
        method: &str,
        name: Option<&str>,
        mut params: Value,
    ) -> Result<Value, Failure> {
        let id = self.next_id();
        if revision == Revision::Current {
            attach_meta(&mut params);
        }
        let request = self.build_request(revision, method, name, id, params);
        let reply = self.transport.post(&request)?;
        self.read_reply(revision, reply, id)
    }

    fn build_request(
        &self,
        revision: Revision,
        method: &str,
        name: Option<&str>,
        id: u64,
        params: Value,
    ) -> Request {
        let mut headers = vec![
            ("Content-Type".into(), "application/json".into()),
            (
                "Accept".into(),
                "application/json, text/event-stream".into(),
            ),
        ];
        match revision {
            Revision::Current => {
                // SEP-2243: gateways route on these headers, and a client that
                // omits them starts meeting rejections in the wild. They mirror
                // the body exactly — the body stays the source of truth.
                headers.push(("MCP-Protocol-Version".into(), PROTOCOL_VERSION.into()));
                headers.push(("Mcp-Method".into(), method.to_string()));
                if let Some(name) = name {
                    headers.push(("Mcp-Name".into(), name.to_string()));
                }
            }
            Revision::Legacy => crate::legacy::add_headers(self, &mut headers),
        }
        if let Some(key) = self.key.as_deref().filter(|k| !k.is_empty()) {
            headers.push(("Authorization".into(), format!("Bearer {key}")));
        }
        Request {
            url: self.url.clone(),
            headers,
            body: jsonrpc::request_body(id, method, params),
        }
    }

    fn read_reply(&self, revision: Revision, reply: Reply, id: u64) -> Result<Value, Failure> {
        let Reply {
            status,
            is_sse,
            session_id,
            body,
        } = reply;
        if revision == Revision::Legacy {
            crate::legacy::remember_session(self, session_id);
        }

        match status {
            200..=299 => {}
            401 | 403 => {
                return Err(MCPError::Authorization.into());
            }
            code => {
                // THE BODY IS READ BEFORE JUDGING. Measured against a real
                // server: a `400` whose body said "params._meta must carry the
                // required envelope keys" was read as "this server does not
                // speak the revision", and the connection quietly downgraded
                // itself to the frozen path — on a server that spoke the
                // current revision perfectly well. A malformed request of ours
                // and a revision the server refuses are different failures and
                // must not share a branch.
                let detail = read_error_message(body);
                let says_version = detail.as_deref().is_some_and(mentions_version);
                let version_rejected = match code {
                    // These codes are about the version by definition.
                    406 | 412 | 426 => true,
                    // A bare 400 with no readable reason gets the benefit of the
                    // doubt (a server too old to know the revision may not
                    // manage a JSON-RPC error either); a 400 that DID explain
                    // itself is taken at its word.
                    400 => detail.is_none() || says_version,
                    _ => false,
                };
                return Err(Failure {
                    error: match detail {
                        Some(message) => MCPError::Server(format!("HTTP {code}: {message}")),
                        None => MCPError::Server(format!("HTTP {code}")),
                    },
                    version_rejected,
                });
            }
        }

        // THE CAP IS PUT ON THE READER, NOT ON A BUFFERED STRING. Reading the
        // whole body first would be simpler, but the SSE branch must be able to
        // RETURN THE MOMENT its own event arrives.
        let tripped = Arc::new(AtomicBool::new(false));
        let reader = BufReader::new(Limited {
            inner: body,
            left: MAX_BODY,
            tripped: Arc::clone(&tripped),
        });
        let parsed = if is_sse {
            sse::find_event(reader, id)
        } else {
            serde_json::from_reader(reader)
                .map_err(|_| MCPError::Malformed)
                .and_then(|parsed: Value| {
                    jsonrpc::select_response(&parsed, id)
                        .cloned()
                        .ok_or(MCPError::Malformed)
                })
        }
        // A body cut off at the cap produces broken JSON, so WITHOUT THIS the
        // failure would be reported as "the server response was not understood"
        // and the user would go hunting for a bug in a healthy server.
        .map_err(|e| {
            if tripped.load(Ordering::Relaxed) {
                MCPError::TooLarge(MAX_BODY)
            } else {
                e
            }
        })?;

        jsonrpc::extract_result(&parsed).map_err(|error| Failure {
            version_rejected: match &error {
                MCPError::Server(message) => mentions_version(message),
                _ => false,
            },
            error,
        })
    }
}

/// How long a catalog may be kept, given what the server asked for.
///
/// PURE so the ceiling is measurable: a 30 day `ttlMs` and a 24 hour one are
/// indistinguishable from outside within a test's lifetime, and "we clamp it"
/// would otherwise be a claim rather than a guarantee.
pub fn clamp_ttl(asked: Option<Duration>) -> Duration {
    asked.unwrap_or(DEFAULT_TTL).min(CATALOG_TTL_CAP)
}

/// The client's ENVELOPE, on EVERY request — there is no handshake to carry it,
/// so identity, revision and capabilities travel with each one (spec §3.1).
///
/// MEASURED, not guessed: a server answering the current revision rejected a
/// request carrying only `clientInfo` with "params._meta must be an object
/// carrying the required protocolVersion and clientCapabilities envelope keys".
/// The header says the same thing as `protocolVersion` here; a gateway reads
/// the header, the server reads the body, and they must not be able to
/// disagree.
///
/// WHAT THE CAPABILITIES SAY IS ALSO WHAT THEY DO NOT SAY: `sampling` is absent
/// and always will be. A server cannot ask for something the client never
/// offered, so the refusal starts here rather than at the point of refusing.
fn attach_meta(params: &mut Value) {
    let meta = json!({
        "io.modelcontextprotocol/clientInfo": {
            "name": "tacet",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION,
        "io.modelcontextprotocol/clientCapabilities": { "tools": {} },
    });
    match params {
        Value::Object(map) => {
            map.insert("_meta".into(), meta);
        }
        // Params that are not an object cannot carry `_meta`. Nothing in this
        // client builds such params; if something ever does, the request still
        // goes out rather than failing over a nicety.
        _ => {}
    }
}

/// Reads the reason out of an error response, capped and made safe to print.
///
/// Failing to read it is not itself an error: some servers answer a rejected
/// request with an empty body, and "HTTP 400" is still a truthful sentence.
fn read_error_message(body: Box<dyn Read>) -> Option<String> {
    let mut text = String::new();
    body.take(64 * 1024).read_to_string(&mut text).ok()?;
    let parsed: Value = serde_json::from_str(&text).ok()?;
    // A batch answer is still just a carrier for the one error we want.
    let object = match &parsed {
        Value::Array(items) => items.iter().find(|i| i.get("error").is_some())?,
        other => other,
    };
    let message = object.get("error")?.get("message")?.as_str()?;
    Some(crate::error::safe_for_screen(message))
}

/// Does this sentence say "I do not speak that revision"?
///
/// PHRASES, NOT KEYWORDS. The first version of this asked whether the message
/// mentioned "protocol" and "version" — and a real server's complaint that our
/// `_meta` was missing `io.modelcontextprotocol/protocolVersion` matched it
/// word for word, downgrading a perfectly current server to the frozen path.
/// The words a server uses to name a field are not the words it uses to refuse
/// a revision, so the match is on the refusal itself.
///
/// The failure mode of being too narrow is the safe one: a refusal we do not
/// recognise surfaces as the server's own sentence, and the user pins
/// `"spec": "legacy"` once. Being too broad silently degrades a working
/// connection and nobody ever finds out.
fn mentions_version(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let flat = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    [
        "unsupported protocol",
        "protocol version not supported",
        "protocol version is not supported",
        "unsupported version",
        "unknown protocol version",
        "invalid protocol version",
        "protocol version mismatch",
        "incompatible protocol",
        "does not support protocol",
        "no longer supported",
    ]
    .iter()
    .any(|phrase| flat.contains(phrase))
}

/// Is this a failure of the WIRE rather than of the far side's logic — the only
/// kind a read may be repeated for.
fn is_transport_error(error: &MCPError) -> bool {
    matches!(error, MCPError::Timeout | MCPError::Unreachable)
}

/// A reader that stops at `left` bytes and REMEMBERS that it did.
///
/// `ureq` has a `limit()` of its own, but what it reports back is an ordinary
/// read error, indistinguishable from a connection that broke — and the two
/// need different sentences (see `MCPError::TooLarge`). The flag is the whole
/// reason this type exists.
struct Limited<R> {
    inner: R,
    left: u64,
    tripped: Arc<AtomicBool>,
}

impl<R: Read> Read for Limited<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            self.tripped.store(true, Ordering::Relaxed);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the MCP response body went past the cap",
            ));
        }
        let take = buffer.len().min(self.left as usize);
        let read = self.inner.read(&mut buffer[..take])?;
        self.left -= read as u64;
        Ok(read)
    }
}

/// Extracts the specs out of a `tools/list` result. PURE — its test needs no
/// network.
///
/// An item without a name is SILENTLY skipped: we cannot show a nameless tool
/// in the catalog, it has no signature the model could call.
pub fn extract_specs(result: &Value) -> Vec<ToolSpec> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| {
                    let name = i.get("name").and_then(Value::as_str)?;
                    if name.is_empty() {
                        return None;
                    }
                    Some(ToolSpec {
                        name: name.to_string(),
                        description: i
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        schema: i.get("inputSchema").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reduces a `tools/call` result's `content` array to plain text.
///
/// Non-text content (image/audio) is not carried in v1 but IT IS NOT SWALLOWED
/// SILENTLY: a marker is put in its place, otherwise the model thinks "the tool
/// returned nothing" and retries the job.
pub fn flatten_content(result: &Value) -> (String, bool) {
    let parts: Vec<String> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| {
                    if let Some(t) = i.get("text").and_then(Value::as_str) {
                        return Some(t.to_string());
                    }
                    match i.get("type").and_then(Value::as_str) {
                        Some(kind) if kind != "text" => Some(format!("[{kind} content omitted]")),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    (parts.join("\n"), is_error)
}

/// The address acceptance rule (§3.1): `https://` everywhere, plain `http://`
/// ONLY on local network addresses.
///
/// WHY HERE: the rule must live in the network layer itself. If it lives in the
/// settings screen (or in the head of the user hand-writing the config file), a
/// second call path skips it and the token goes out in plain text.
pub fn validate_url(url: &str) -> MCPResult<()> {
    let error = || MCPError::InvalidAddress(url.to_string());
    if let Some(rest) = url.strip_prefix("https://") {
        return if rest.is_empty() {
            Err(error())
        } else {
            Ok(())
        };
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return Err(error());
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or_else(|| rest.split(['/', '?', '#']).next().unwrap_or_default())
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();

    if is_local(&host) {
        Ok(())
    } else {
        Err(error())
    }
}

/// Is it a local network address. The list is kept narrow: every address we
/// cannot be sure about is forced to ask for `https`.
fn is_local(host: &str) -> bool {
    if matches!(host, "localhost" | "127.0.0.1" | "::1") || host.ends_with(".local") {
        return true;
    }
    let octets: Vec<u8> = host
        .split('.')
        .filter_map(|p| p.parse::<u8>().ok())
        .collect();
    if octets.len() != 4 || host.split('.').count() != 4 {
        return false;
    }
    match octets.as_slice() {
        [10, ..] => true,
        [192, 168, ..] => true,
        [172, second, ..] if (16..=31).contains(second) => true,
        [127, ..] => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn https_is_always_accepted() {
        assert!(validate_url("https://example.com/mcp").is_ok());
    }

    #[test]
    fn plain_http_only_on_the_local_network() {
        assert!(validate_url("http://localhost:8080/mcp").is_ok());
        assert!(validate_url("http://127.0.0.1:3000").is_ok());
        assert!(validate_url("http://192.168.1.20/mcp").is_ok());
        assert!(validate_url("http://10.0.0.5/mcp").is_ok());
        assert!(validate_url("http://172.20.0.3/mcp").is_ok());
        assert!(validate_url("http://server.local/mcp").is_ok());

        assert!(validate_url("http://example.com/mcp").is_err());
        assert!(
            validate_url("http://172.32.0.1/mcp").is_err(),
            "172.32 is outside the private range"
        );
        assert!(validate_url("http://8.8.8.8/mcp").is_err());
    }

    #[test]
    fn non_http_schemes_are_rejected() {
        for u in [
            "ws://localhost/mcp",
            "file:///etc/passwd",
            "example.com",
            "",
        ] {
            assert!(
                validate_url(u).is_err(),
                "{u} should not have been accepted"
            );
        }
    }

    #[test]
    fn an_invalid_address_does_not_let_the_client_be_built() {
        // The rule lives in the network layer: with a bad address not even a
        // client OBJECT comes into existence, so there is no path left to
        // accidentally send a request to that address.
        assert!(MCPClient::new("http://example.com/mcp", None).is_err());
    }

    #[test]
    fn the_spec_field_is_read_and_a_typo_is_not_guessed_at() {
        assert_eq!(SpecChoice::parse("auto"), Some(SpecChoice::Auto));
        assert_eq!(SpecChoice::parse("2026-07-28"), Some(SpecChoice::Current));
        assert_eq!(SpecChoice::parse("LEGACY"), Some(SpecChoice::Legacy));
        assert_eq!(SpecChoice::parse("2025-06-18"), Some(SpecChoice::Legacy));
        assert_eq!(SpecChoice::parse("newest"), None);
    }

    #[test]
    fn only_a_sentence_about_the_revision_means_the_wrong_revision() {
        assert!(mentions_version("Unsupported protocol version: 2026-07-28"));
        assert!(mentions_version("this protocol version is not supported here"));
        assert!(mentions_version("2026-07-28 is no longer supported"));
        // The one that taught us the difference: a real server rejecting a
        // request of OURS, not the revision.
        assert!(!mentions_version(
            "params._meta must be an object carrying the required \
             'io.modelcontextprotocol/protocolVersion' and \
             'io.modelcontextprotocol/clientCapabilities' envelope keys"
        ));
        assert!(!mentions_version("the database is on fire"));
        // The near miss the phrase list exists for: a complaint about the
        // FIELD, using the same two words.
        assert!(!mentions_version(
            "invalid params: _meta.protocolVersion must be a string"
        ));
    }

    #[test]
    fn specs_are_extracted_and_a_nameless_item_is_skipped() {
        let result = json!({"tools": [
            {"name": "run", "description": "command", "inputSchema": {"type": "object"}},
            {"description": "nameless"},
            {"name": ""},
            {"name": "plain"},
        ]});
        let specs = extract_specs(&result);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "run");
        assert_eq!(specs[1].name, "plain");
        assert_eq!(
            specs[1].schema,
            Value::Null,
            "a schema-less tool arrives with a null schema"
        );
    }

    #[test]
    fn the_content_is_flattened() {
        let result = json!({"content": [
            {"type": "text", "text": "first"},
            {"type": "text", "text": "second"},
        ]});
        assert_eq!(flatten_content(&result), ("first\nsecond".into(), false));
    }

    #[test]
    fn non_text_content_is_not_swallowed_silently() {
        let result = json!({"content": [{"type": "image", "data": "..."}]});
        let (text, is_error) = flatten_content(&result);
        assert_eq!(text, "[image content omitted]");
        assert!(!is_error);
    }

    #[test]
    fn the_servers_own_tool_error_is_marked() {
        let result = json!({"content": [{"type":"text","text":"exit 1"}], "isError": true});
        assert_eq!(flatten_content(&result), ("exit 1".into(), true));
    }

    /// GOES ONLINE — deliberately `#[ignore]`. Runs with
    /// `cargo test -p tacet-mcp -- --ignored` when TACET_MCP_TEST_URL is given.
    #[test]
    #[ignore = "goes online; needs TACET_MCP_TEST_URL"]
    fn a_real_server_answers() {
        let Some(url) = tacet_kernel::env_var("TACET_MCP_TEST_URL") else {
            panic!("TACET_MCP_TEST_URL is not defined");
        };
        let url = url.to_string_lossy().into_owned();
        let key =
            tacet_kernel::env_var("TACET_MCP_TEST_KEY").map(|k| k.to_string_lossy().into_owned());
        let client = MCPClient::new(url, key).expect("client");
        let tools = client.tools().expect("tools/list");
        println!(
            "{} tools found over {}",
            tools.len(),
            client.revision().map(Revision::label).unwrap_or("?")
        );
        for t in &tools {
            println!(
                "  {} — {}",
                t.name,
                crate::bridge::truncate_description(&t.description)
            );
        }
    }
}
