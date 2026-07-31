//! The TURN LOOP — the part of the shell a user actually spends time inside.
//!
//! WHY IT IS ITS OWN FILE: it was two thousand lines in the middle of
//! `main.rs`, wedged between argument parsing and the download progress bar,
//! and it is the only place in the program where the four layers meet at once —
//! the router picks the tools, the engine generates, the grammar constrains,
//! the executor runs and the gates refuse. Everything else in the shell is a
//! command that starts, prints and exits.
//!
//! THE SHAPE OF ONE USER TURN, because it is not obvious from the code and it
//! is the thing to understand before changing anything here:
//!
//!   1. The message is read; piped stdin is fenced into it if there is any.
//!   2. `Router::select` cuts the catalog down to what this message needs.
//!   3. The ONE matching skill is attached behind a `<guidance>` fence.
//!   4. Up to `MAX_TURNS` passes: generate -> is there a call -> execute ->
//!      write the result into the history -> generate again. The LAST pass is
//!      offered no tools at all, so it cannot spend itself on a call that
//!      reaches nobody.
//!   5. Whatever the model said last IS the answer.
//!
//! THE QUESTION MOVES between passes, and that detail is the crux of the loop:
//! on the first pass it sits at the END of the prompt, on later ones it moves
//! into the history IN FRONT of the tool result and the question field is left
//! empty. Repeating it after the tool result is what made the model call the
//! same tool again.

use crate::ui::{
    BOLD, BRASS, Color, DIM, LiveReporter, RESET, Screen, TurnIndicator, YELLOW, paper_code,
};
use crate::{
    CANCEL, EXTERNAL_TOOLS, EngineChoice, STDIN_CONTEXT_LIMIT, TerminalApproval, TerminalAsk,
    VERIFYING_TOOLS, announce_transcript, byte_text, dir_context, print_grammar, read_piped_stdin,
    refresh_session, session_catalog, sessions, setup_engine, stdin_fence, system_text,
    thinking_switch, to_engine_turns, tool_record,
};
use crate::{addon, config, filter, format, receipt, session, update};
use crate::{input, ui};
use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tacet_engine::{EngineProvider, Prompt, SamplingSetting, TokenCounter, Turn, wait};
use tacet_eval::FakeSelector;
use tacet_grammar::CallConstraint;
use tacet_kernel::{
    DataStore as CoreDataStore, Reporter, ToolCatalog, ToolContext, TraceCollector,
};
use tacet_memory::MemoryStore;
use tacet_skills::{InjectionState, SkillStore, injection_text};
use tacet_tools::data_store::SharedStore;
use tacet_tools::executor::{SilentDeny, ToolExecutor};
use tacet_tools::mcp;
use tacet_tools::memory::SharedMemory;
use tacet_tools::router::Router;

// ---------------------------------------------------------------------------
// chat
// ---------------------------------------------------------------------------

/// Everything `chat` needs. A STRUCT, not nine positional arguments: the list
/// had already reached six `&str`/`bool`/`Option<String>` values where a swapped
/// pair would still compile.
pub struct ChatRun {
    pub choice: EngineChoice,
    pub script: Vec<String>,
    pub show_prompt: bool,
    pub dir: String,
    pub single_message: Option<String>,
    pub model_name: String,
    pub json: bool,
    pub continue_session: bool,
    pub session_id: Option<String>,
    /// How generation samples. Already RESOLVED here (flag > config > default) —
    /// `chat` does not read the config file itself, the same way it does not
    /// resolve the model name itself.
    pub sampling: SamplingChoice,
}

/// The two sampling knobs the user is allowed to turn.
///
/// WHY THEY ARE EXPOSED AT ALL, given that the defaults are deliberate: they
/// were not exposed, and the cost was two things that could not be done at all.
/// (1) The variance of this shell could not be MEASURED — every run used
/// temperature 0 and seed 0, so "did that prompt change help, or is this
/// run-to-run noise?" had no experiment behind it. (2) A user handed a bad
/// greedy answer had no "try that again differently"; greedy is one path
/// through the model and there is no second one without a knob.
///
/// THE DEFAULTS DO NOT MOVE. `Default` here is exactly `SamplingSetting`'s —
/// temperature 0.0, seed 0 — so a user who touches nothing gets bit-identical
/// behaviour to before this existed. That is the point: the knob is new, the
/// position is not.
#[derive(Debug, Clone, Copy, Default)]
pub struct SamplingChoice {
    /// `None` = keep the engine default (greedy).
    temperature: Option<f32>,
    seed: Option<u64>,
}

/// The most a temperature may be. Above ~2 the distribution is close enough to
/// uniform that the output is noise, and accepting a number that produces
/// garbage is not a favour to anyone.
pub const MAX_TEMPERATURE: f32 = 2.0;

impl SamplingChoice {
    /// flag > config file > built-in default — the same precedence, in the same
    /// order, as `model` and `engine`.
    ///
    /// A NONSENSE VALUE IN THE CONFIG FILE IS IGNORED, LOUDLY. The file is the
    /// quietest voice in the precedence chain and a typo in it must not be able
    /// to change how the model samples; but staying silent would leave a user
    /// who wrote `temperature: "0,7"` believing it took effect.
    pub fn resolve(temperature: Option<f32>, seed: Option<u64>, color: &Color) -> Self {
        Self {
            temperature: temperature.or_else(|| Self::config_number("temperature", color)),
            seed: seed.or_else(|| Self::config_number("seed", color)),
        }
    }

    fn config_number<T: std::str::FromStr>(key: &str, color: &Color) -> Option<T> {
        let raw = config::get_str(key)?;
        match raw.trim().parse::<T>() {
            Ok(v) => Some(v),
            Err(_) => {
                eprintln!(
                    "{}",
                    color.paint(
                        YELLOW,
                        &format!(
                            "(config: `{key}` is not a number ('{raw}') — ignored; \
                             `tacet config unset {key}` clears it)"
                        )
                    )
                );
                None
            }
        }
    }

    /// The setting the engine is handed. `max_tokens` and `cancel` are NOT set
    /// here: those are properties of the turn (the prompt's length, the user's
    /// Ctrl-C), not of the user's sampling preference, and the call site owns
    /// them.
    pub fn apply(self, base: SamplingSetting) -> SamplingSetting {
        SamplingSetting {
            temperature: self
                .temperature
                .unwrap_or(base.temperature)
                .clamp(0.0, MAX_TEMPERATURE),
            seed: self.seed.unwrap_or(base.seed),
            ..base
        }
    }

    /// Is anything off the default — the status line and the banner say so.
    /// A shell sampling at 0.9 that looks exactly like a shell sampling at 0 is
    /// how an unreproducible measurement gets taken without anyone noticing.
    pub fn line(self) -> Option<String> {
        let t = self.temperature.filter(|v| *v > f32::EPSILON);
        match (t, self.seed) {
            (None, None) => None,
            (t, s) => {
                let mut parts = Vec::new();
                if let Some(t) = t {
                    parts.push(format!("temperature {t}"));
                }
                if let Some(s) = s {
                    parts.push(format!("seed {s}"));
                }
                Some(parts.join(" · "))
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
pub fn chat(run: ChatRun) -> ExitCode {
    let ChatRun {
        choice,
        script,
        show_prompt,
        dir,
        single_message,
        model_name,
        json,
        continue_session,
        session_id,
        sampling,
    } = run;
    let dir = dir.as_str();
    let model_name = model_name.as_str();
    let color = Color::setup();
    let screen = Screen::setup();
    let interactive = single_message.is_none();
    // `--json` OWNS THE WHOLE OF STDOUT. Every human decoration below asks this
    // question rather than `interactive`, because the two are not the same: a
    // `--json` run in a terminal is still a machine-readable run.
    let human = !json;

    // THE PIPE IS CONTEXT ONLY ALONGSIDE `--message`.
    //
    // With no `--message` the loop READS ITS MESSAGES from stdin line by line
    // (`input::read` falls back to `read_line` off a tty) and every script in
    // the wild depends on that. Slurping stdin here would take those lines away
    // and break them silently — trading one silent bug for another.
    let piped = if single_message.is_some() && !std::io::stdin().is_terminal() {
        read_piped_stdin()
    } else {
        None
    };

    // THE WINDOW COMES OUT OF THE MODEL, not out of a constant. Everything that
    // sizes itself to the window — truncation, the generation cap, the status
    // line — takes it from the `TokenCounter` built below with this number.
    let (engine, window) = match setup_engine(choice, script, model_name, &color, &screen, human) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let store = Arc::new(SharedStore::new());

    // MEMORY: in interactive mode PERSISTENT to memory.json in the config
    // directory; in single-message/diagnostic mode in-memory (do not dirty the
    // real home directory). In an environment where it cannot write to disk it
    // also works with the in-memory store.
    let memory = if interactive {
        match MemoryStore::default_path() {
            Some(p) => SharedMemory::new(MemoryStore::from_file(p)),
            None => SharedMemory::in_memory(),
        }
    } else {
        SharedMemory::in_memory()
    };

    // THE TRANSCRIPT ON DISK.
    //
    // PERSISTED IN INTERACTIVE MODE, and in a one-shot run ONLY IF the user
    // asked for a transcript by naming one (`--continue` / `--session`). The
    // rule is the same one memory follows two blocks up, for the same reason: a
    // CI script running `tacet -m` in a loop would otherwise write a file per
    // invocation and, at fifty sessions, evict the conversations the user
    // actually had. Naming a session is the opt-in.
    let keep_transcript = interactive || continue_session || session_id.is_some();
    let mut chat_session = if keep_transcript {
        Some(session::Session::start())
    } else {
        None
    };

    // WHAT IS BEING CONTINUED. `--session <id>` is more specific than
    // `--continue`, so it wins; asking for both is not an error worth refusing
    // a shell over.
    let mut resumed: Vec<Turn> = Vec::new();
    if let Some(id) = &session_id {
        match session::Session::load(id) {
            Some(turns) => resumed = to_engine_turns(&turns),
            // NOT SILENT. A typo'd id that quietly opened an empty session is
            // the shape of failure this whole module exists to avoid: the user
            // would talk to a model that has forgotten everything and blame the
            // model.
            None => eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    &format!("(no session named '{id}' — starting a fresh one; `tacet sessions` lists them)")
                )
            ),
        }
    } else if continue_session {
        match session::Session::latest() {
            Some(turns) => resumed = to_engine_turns(&turns),
            None => eprintln!(
                "{}",
                color.paint(
                    DIM,
                    "(no stored session to continue — starting a fresh one)"
                )
            ),
        }
    }
    if !resumed.is_empty() && human {
        println!(
            "{}",
            color.paint(
                DIM,
                &format!("(continuing — {} earlier turns loaded)", resumed.len())
            )
        );
    }

    // SKILLS: the embedded skills + (if present) the user's `skills` directory.
    let mut skill_store = SkillStore::default_set();
    if let Some(d) = tacet_skills::user_dir()
        && d.is_dir()
    {
        skill_store.load_from_dir(d);
    }
    let mut injection_state = InjectionState::new();

    let (mut catalog, mut code_state) = session_catalog(&store, &memory, &color);

    // THE ADDON GATE'S VALUE IS READ AT SESSION START — AT THE SAME MOMENT as
    // the catalog. The catalog is set up at session start too; reading the two at
    // different moments would produce an inconsistent state like "no tools but no
    // hint either". An addon installed mid-session from ANOTHER terminal is not
    // visible in this session; toggling one from INSIDE this shell goes through
    // `refresh_session`, which re-reads both together.
    let mut web_addon_open = tacet_web::addon::web_search_is_open();

    // MCP CONNECTIONS — `mcp.json` in the config directory. If the file is
    // missing it does nothing and NO NETWORK CALL IS MADE.
    let mcp_load = mcp::load_from_default_with(Arc::new(TerminalAsk));
    let mcp_names = mcp::feed_catalog(&mut catalog, &mcp_load);
    report_mcp(&mcp_load, &color);

    // THE APPROVAL GATE: a real question in interactive mode, SilentDeny in
    // diagnostic mode.
    let mut executor = ToolExecutor::new(catalog.clone());
    if interactive {
        executor = executor.with_gate(TerminalApproval);
    } else {
        executor = executor.with_gate(SilentDeny);
    }
    for name in EXTERNAL_TOOLS {
        executor = executor.external_tool(*name);
    }
    // MCP tools are EXTERNAL TOOLS too: in a tainted session each of them passes
    // the approval gate.
    executor = mcp::bind_executor(executor, &mcp_names);

    // THE CHIPS ARE LIVE NOW. Traces used to be printed in a batch AFTER the turn
    // ended; while a tool ran the screen was silent and the user kept looking to
    // see whether it had hung. `LiveReporter` drops the chip the moment it
    // starts and, when it finishes, writes the result over the same line. In
    // single-message (diagnostic) mode live printing is OFF: there the chips are
    // printed in a batch at the end of the turn, and both at once would duplicate
    // the output.
    let traces = Arc::new(LiveReporter::new(
        Arc::new(TraceCollector::new()),
        Arc::clone(&screen),
        interactive,
    ));
    let mut ctx = ToolContext::new(
        Arc::clone(&store) as Arc<dyn CoreDataStore>,
        dir,
        Arc::clone(&traces) as Arc<dyn Reporter>,
    );

    // The remote tools get a floor in the budget: a catalog the model is never
    // shown is a catalog that does not exist as far as the answer is concerned.
    let router = Router::new().reserving(mcp_names.clone());
    // THE COUNTER CARRIES THE MODEL'S OWN WINDOW. `TokenCounter::default()` used
    // to be built here, which hard-wired 4096 — a constant of a DIFFERENT
    // architecture (iOS FoundationModels really does hand out 4096). Running our
    // own weights through candle, the files declare four to eight times that,
    // and the cost of the old default was measurable: with the tool catalog at
    // ~1550 tokens, ~1100 were left for the conversation, so a user asking for a
    // script watched two older turns leave the window on EVERY turn.
    //
    // `GENERATION_SHARE` is NOT scaled with the window: it is the MINIMUM room
    // truncation reserves, and the real generation cap is derived from the
    // length of the prompt (`TokenCounter::generation_cap`) — so a bigger window
    // already gives generation more room without touching this number.
    let counter = TokenCounter::new(window, tacet_engine::GENERATION_SHARE);
    let mut history: Vec<Turn> = resumed;

    // THE DIRECTORY CENSUS IS TAKEN ONCE, at session start, not per turn.
    //
    // A `read_dir` per turn would put a syscall on the latency path of every
    // question to save a staleness the user can fix by restarting; and worse,
    // the prompt would change shape mid-conversation for reasons the user never
    // did, which is precisely the kind of drift that makes a measurement
    // meaningless. See `dir_context` for the measured token cost.
    let dir_block = dir_context(dir);
    let system = system_text(dir_block.as_ref());
    // TOKEN ACCOUNTING. No new counter WAS INVENTED: the prompt side comes from
    // `TokenCounter`'s truncation report (`final_estimate` — the real prompt size
    // AFTER truncation), the generation side from the engine's reported
    // `Generation::token_count`. What the user needs to see is the room left in
    // the window: when the budget filled, old turns dropped SILENTLY (see
    // `report.changed()`), i.e. the user only realised their context had been
    // truncated once the answer broke.
    let mut session_tokens = 0usize;
    let mut last_turn_prompt = 0usize;
    let mut last_turn_generation = 0usize;
    let mut last_context = 0usize;
    // The most recent file a tool produced — what ctrl-o / /preview opens.
    let mut last_artifact: Option<std::path::PathBuf> = None;
    let mut input_history: Vec<String> = Vec::new();

    // THE CONSTRAINT is set up ONCE OUTSIDE the loop (the trie cost is
    // proportional to the vocabulary size). The catalog is the FULL catalog — the
    // constraint must mirror what the executor accepts.
    // THE STREAM FILTER'S TOOL NAMES. Taken from the FULL catalog, not from the
    // subset selected in that turn: the model sometimes produces a name outside
    // its budget too and that line must not spill onto the screen (see filter.rs).
    let mut catalog_names: Vec<String> = catalog.names().into_iter().map(String::from).collect();

    let mut constraint = engine.vocab().map(|v| CallConstraint::new(&v, &catalog));
    if constraint.is_none() {
        eprintln!("{}", color.paint(YELLOW, "(warning: the engine does not declare its vocabulary — generation is UNCONSTRAINED)"));
    }

    // THE ENTRY SCREEN. Brand rule: the name is "Tacet" — capital first letter.
    // Lowercase `tacet` is only THE BINARY's name (the command typed in a shell),
    // not the brand's. NO green dot, status dot or badge: state is told in words
    // alone. The single accent is the brass full stop after the name — the
    // brand's ensō dot in its typographic form ("the sentence ends here") — and
    // the spinning ensō of the turn indicator; everything else stays in
    // ink/grey tones.
    //
    // UNDER `--json` NOTHING OF THIS IS PRINTED. The contract is one line of
    // JSON on stdout and nothing else; a banner above it makes `| jq` fail on
    // the first byte, which is not a decoration problem, it is a broken command.
    if human {
        if interactive {
            println!(
                "{}{}  {}",
                color.paint(BOLD, "Tacet"),
                color.paint(BRASS, "."),
                color.paint(DIM, &version_line().replace("tacet ", ""))
            );
            // THE SAMPLING LINE IS PART OF THE IDENTITY LINE when it is off the
            // default. A shell sampling at 0.9 that looks exactly like a shell
            // sampling greedily is how an unreproducible run gets taken for a
            // reproducible one — the same argument `EngineIdentity` makes about
            // never letting a measurement hide its own subject.
            let sampling_note = sampling
                .line()
                .map(|s| format!(" · {s}"))
                .unwrap_or_default();
            println!(
                "{}",
                color.paint(
                    DIM,
                    &format!(
                        "{} · {} tools{sampling_note} · /help",
                        engine.name(),
                        catalog.tools().len()
                    )
                )
            );
        } else {
            println!(
                "Tacet — engine: {} · tools: {}",
                engine.name(),
                catalog.tools().len()
            );
        }
        println!();
    }

    // THE PIPE IS ANNOUNCED, AND ITS CUT IS ANNOUNCED LOUDER. On stderr, so it
    // never lands in `--json` output; the model is told separately, inside the
    // fence (see `stdin_fence`) — the user and the model need the same fact in
    // two different places, and neither notice covers the other.
    if let Some(p) = &piped {
        match p.original_bytes {
            None => eprintln!(
                "{}",
                color.paint(
                    DIM,
                    &format!(
                        "(read {} from the pipe as context)",
                        byte_text(p.text.len() as u64)
                    )
                )
            ),
            Some(_) => eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    &format!(
                        "(the piped input was CUT at {} — the model sees only the beginning; \
                         the context window cannot hold more)",
                        byte_text(STDIN_CONTEXT_LIMIT as u64)
                    )
                )
            ),
        }
    }

    // Counts COMPLETED turns, for the one-time update offer below. Not a
    // metric and not persisted: it exists so the question lands after the
    // user has actually used the shell rather than on the first line.
    let mut completed_turns: usize = 0;
    // WHY A FLAG AND NOT JUST A PRINT: with `--json` the failure has to reach
    // the READER, and the reader is a script. An engine error printed to stderr
    // while stdout carries `"answer":""` and the process exits 0 is a silent
    // failure — `jq -r .answer` yields an empty string and the pipeline carries
    // on as though the model had answered.
    let mut turn_error: Option<String> = None;
    let mut any_turn_failed = false;
    loop {
        let message = match single_message.clone() {
            Some(m) => m,
            None => {
                // THE INPUT FIELD. On a tty a framed field + the slash command
                // list + the token counter; with no tty `input::read` falls back
                // to the old `read_line` and writes NOT ONE EXTRA BYTE to the
                // screen (piped output staying parseable is mandatory).
                let state = status_line(
                    last_turn_prompt,
                    last_turn_generation,
                    session_tokens,
                    last_context,
                    &counter,
                );
                match input::read(&screen, &state, &mut input_history) {
                    input::Input::Done => break, // EOF (ctrl-d)
                    input::Input::Line(s) => {
                        // The sent message STAYS in the transcript: the frame was
                        // erased, and a one-line trace is printed in its place so
                        // a user looking back sees what they asked.
                        if screen.tty() && !s.trim().is_empty() {
                            for line in s.lines() {
                                // Brass, like the landing demo's prompt symbol:
                                // the mark of "the user said this".
                                println!("{} {}", color.paint(BRASS, "›"), line);
                            }
                        }
                        s
                    }
                }
            }
        };

        // AN EMPTY LINE NO LONGER EXITS. It used to, and an Enter pressed by
        // accident closed the session (along with its history); exiting must be
        // an explicit intent. EOF (Ctrl-D) still exits — that already means
        // "input is finished".
        if message.is_empty() {
            if single_message.is_some() {
                break;
            }
            continue;
        }

        // Slash commands: they DO NOT GO to the model as a message.
        // `is_command` AND NOT `starts_with('/')`: an absolute path is the
        // natural way to name a directory in a message, and it also starts with
        // a slash. See the rule and the transcript that forced it in `input`.
        if input::is_command(&message) {
            // A SLASH COMMAND HAS NO JSON SHAPE. Every one of them prints a
            // human table to stdout, which is exactly the byte stream `--json`
            // promises not to produce. Saying so in JSON keeps the promise:
            // the caller gets a parseable line and a non-zero exit instead of a
            // table that breaks their parser three fields in.
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "error": "slash commands have no --json form",
                        "command": message.trim(),
                    })
                );
                return ExitCode::FAILURE;
            }
            // Menus REPLAY commands (see SlashResult::Replay): the loop feeds
            // the produced command back through the same gate, so a menu pick
            // and a typed command are literally the same code path. The depth
            // guard is a backstop against a menu replaying a menu forever.
            let mut command_text = message.clone();
            let mut quit = false;
            for _depth in 0..4 {
                let cleared = command_text.trim() == "/clear";
                // An /addon verb with arguments changes the registry; the running
                // session must see the change (the transcript that forced this:
                // `/addon on web-search` said "opened" while the session kept
                // answering "the addon is CLOSED" until a restart).
                let addon_touched = command_text.trim_start().starts_with("/addon ");
                match slash(
                    &command_text,
                    &catalog,
                    &memory,
                    &mut history,
                    &engine,
                    &color,
                    &last_artifact,
                    &screen,
                ) {
                    SlashResult::Quit => {
                        quit = true;
                        break;
                    }
                    SlashResult::Replay(next) => {
                        // The transcript keeps the outcome: the produced command
                        // is echoed exactly like a typed one.
                        println!("{} {}", color.paint(BRASS, "›"), next);
                        command_text = next;
                        continue;
                    }
                    SlashResult::Handled => {
                        // /clear must also clear the COUNTER LINE: leaving the old
                        // "context 2842/4096" standing read as "clear did nothing".
                        // The session total stays — it is a lifetime figure, not a
                        // window figure.
                        if cleared {
                            last_turn_prompt = 0;
                            last_turn_generation = 0;
                            last_context = 0;
                            // AND IT CLOSES THE TRANSCRIPT. `/clear` says "forget
                            // this conversation"; a shell that kept appending to the
                            // same file would leave the cleared turns sitting in the
                            // record `--continue` reloads — the user would have said
                            // forget and been remembered anyway. The file already
                            // written is NOT deleted (that is `tacet sessions
                            // --purge`, and deleting on a keystroke that reads as
                            // "start fresh" would be a surprise); from here on the
                            // new turns go to a NEW file.
                            if let Some(s) = &mut chat_session {
                                *s = session::Session::start();
                            }
                        }
                        if addon_touched {
                            refresh_session(
                                &store,
                                &memory,
                                &color,
                                interactive,
                                &mcp_load,
                                &engine,
                                &mut catalog,
                                &mut executor,
                                &mut constraint,
                                &mut catalog_names,
                                &mut web_addon_open,
                                &mut code_state,
                            );
                            println!(
                                "{}",
                                color.paint(
                                    DIM,
                                    &format!(
                                        "(catalog refreshed — {} tools)",
                                        catalog.tools().len()
                                    )
                                )
                            );
                        }
                        break;
                    }
                    SlashResult::Unknown => {
                        println!("{}", color.paint(DIM, "(unknown command; /help)"));
                        break;
                    }
                }
            }
            if quit || single_message.is_some() {
                break;
            }
            continue;
        }

        // A WEB REQUEST WITH NO ADDON DOES NOT STAY SILENT.
        //
        // While the gate is closed `web_search` is not in the catalog at all; the
        // model therefore DOES NOT EVEN ATTEMPT to search — it answers from
        // memory or says "I can't". Neither tells the user WHAT IS MISSING. This
        // line does: a sentence takes the place of the tool's absence, not
        // silence.
        //
        // The condition is TWO-SIDED: (1) the gate is closed, (2) the message's
        // dominant intent is Web. Without the second it would print an addon
        // advert on every turn.
        //
        // In an interactive session with the addon INSTALLED the sentence is a
        // QUESTION, not a hint: the user's message already says they want the
        // web, so the shell offers the switch instead of telling them to type a
        // command. On yes, the catalog refreshes and THIS SAME message goes to
        // the model with the web tools available — no retyping.
        if !web_addon_open && addon::is_web_request(&message) {
            if interactive
                && screen.tty()
                && addon::web_installed()
                && ui::ask_yes_no(
                    &color,
                    "this looks like a web question and web search is off — turn it on?",
                )
            {
                let _ = addon::set_state(tacet_web::addon::WEB_SEARCH, true);
                refresh_session(
                    &store,
                    &memory,
                    &color,
                    interactive,
                    &mcp_load,
                    &engine,
                    &mut catalog,
                    &mut executor,
                    &mut constraint,
                    &mut catalog_names,
                    &mut web_addon_open,
                    &mut code_state,
                );
                println!(
                    "{}",
                    color.paint(
                        DIM,
                        &format!("(catalog refreshed — {} tools)", catalog.tools().len())
                    )
                );
            } else {
                println!(
                    "{}",
                    color.paint(YELLOW, &format!("({})", addon::closed_gate_message(true)))
                );
            }
        }

        // A new turn: the side-effect flag and the attempt counter are reset;
        // taint and the store are not (they are session-lived).
        //
        // THE CANCEL FLAG IS RESET HERE TOO. A Ctrl-C pressed in the previous
        // turn must not drop the new one — the same logic as the turn ticket in
        // `ToolExecutor` (see the "the old turn's word does not bind the new one"
        // note there).
        CANCEL.store(false, Ordering::Relaxed);
        let ticket = executor.new_turn();
        traces.reset();
        if let Some(s) = &code_state {
            s.new_turn();
        }
        injection_state.begin_turn();

        // THIS TURN'S TOOL RESULTS. They are appended to the persistent `history`
        // when the turn ENDS.
        //
        // WHY SEPARATE: the user message used to be pushed into `history` at the
        // start of the turn, but `Prompt::new(..., &message)` ALREADY takes it as
        // the `question` — the result was the message entering the prompt TWICE
        // (once inside `<history>`, once at the end). Seen and confirmed in the
        // `--show-prompt` output. The double write eats room in the window
        // and, by showing the small model the same question in two different
        // contexts, encouraged it to repeat the question.
        let mut turn_tools: Vec<Turn> = Vec::new();

        // WHAT THE MODEL IS ASKED, as opposed to what the user typed. They are
        // the same string unless something was piped in, in which case the pipe
        // is fenced ABOVE the question — data first, instruction last, because
        // in a small model the final block carries the most weight and the
        // question is the instruction.
        //
        // `message` STAYS THE USER'S OWN WORDS everywhere else in this turn:
        // the router selects tools from it, the skill store matches on it, the
        // memory store queries with it. Handing an 8 KiB log to a keyword router
        // would let the pasted text, not the question, decide which tools the
        // model is even shown.
        let mut asked = match &piped {
            Some(p) => format!("{}\n\n{message}", stdin_fence(p)),
            None => message.clone(),
        };
        // Selective thinking (Qwen soft switch) — see `thinking_switch`.
        if engine.template() == tacet_engine::Template::ChatML {
            asked.push_str(thinking_switch(&message));
        }

        // The tool budget derives ONLY from the user message.
        let selected: ToolCatalog = router.select(&message, &catalog).into_iter().collect();
        let selected_names: Vec<String> = selected.names().into_iter().map(String::from).collect();

        // SKILL INJECTION (700 limit, NOT EMBEDDED into the system instruction):
        // the SINGLE skill matching the message, into that turn's prompt behind a
        // `<guidance>` fence. Turn-distance repeat suppression via
        // `injection_state`: the same skill is not added again on every turn.
        let mut guide = skill_store
            .matching(&message, Some(&selected_names))
            .and_then(|s| {
                if injection_state.is_needed(&s.name) {
                    injection_state.mark(&s.name);
                    Some(injection_text(s))
                } else {
                    None
                }
            });

        // THE WEB NUDGE. The intent detector already fires when the gate is
        // CLOSED (it offers the switch); with the gate OPEN the same signal now
        // reaches the MODEL. Measured without it: ferry times were answered
        // from memory (wrong) and the user had to say "search the internet" as
        // a second turn — the small model simply does not reach for web_search
        // on its own. One guide sentence, only on turns whose dominant intent
        // is the web, fixes the reach without touching any other question.
        if web_addon_open && addon::is_web_request(&message) {
            const WEB_NUDGE: &str = "this question needs live information from the internet. \
                 Call the web_search tool first; do not answer it from memory.";
            guide = Some(match guide {
                Some(g) => format!("{g}\n{WEB_NUDGE}"),
                None => WEB_NUDGE.to_string(),
            });
        }

        // MEMORY INJECTION (600 limit): the notes matching the message, in the
        // system block.
        let memory_text = memory.with(|s| s.injection_text(&message)).flatten();

        let mut answer = String::new();
        // The tokens of this USER turn. The inner loop (the tool turns) goes to
        // the model more than once; the "this turn" number shown to the user is
        // the sum of all of them — that is the real cost spent on a single
        // question.
        let mut turn_prompt = 0usize;
        // See the truncation notice below: printed at most once per turn.
        let mut truncation_reported = false;
        let mut turn_generation = 0usize;
        // Every tool that really ran this turn, in order — the `--json` trace
        // and the tool-name list the transcript stores.
        let mut turn_calls: Vec<serde_json::Value> = Vec::new();
        // DID THE INNER LOOP REACH AN ENDING, or did it just run out of turns?
        //
        // Every `break` below is an ENDING — an answer, a cancel, an engine
        // error, a side effect we will not retry. Falling out of the `for`
        // instead means the model called a tool on all `MAX_TURNS` passes and
        // never wrote a sentence, and until this flag existed that outcome was
        // INVISIBLE: `answer` stayed empty, nothing was printed, no assistant
        // turn was stored, `--json` reported `"answer": ""` with no error, and
        // the process exited SUCCESS. That contradicts this file's own rule at
        // the bottom ("a turn that never produced an answer is a FAILED RUN")
        // and it reads to the user as the shell swallowing their question.
        let mut settled = false;
        // Set by a duplicate call; read by `final_turn` on the next pass.
        let mut must_answer = false;
        for turn in 0..tacet_eval::MAX_TURNS {
            // THE LAST PASS IS OFFERED NO TOOLS, so it cannot spend itself on
            // another call.
            //
            // MEASURED, 30 Jul 2026 (qwen3-4b, logic set with a real engine):
            // turns ended with the model still calling when the budget ran out —
            // `calculate ×4`, `time ×4`, `send_out ×4`. The executor's duplicate
            // guard was working the whole time: the repeat never ran, it came
            // back as `duplicate_call: … Either answer the user with what you
            // have, or call a different tool`. The model ignored the sentence and
            // called again. That is the lesson this codebase already wrote down
            // for the loop itself — "loop prevention must rest on code, not
            // text" — applied to the WAY OUT of the loop, which had stayed a
            // text nudge.
            //
            // WITH NO `<tools>` BLOCK AND NO CONSTRAINT there is nothing to name
            // and nothing to imitate, so the pass produces prose. The cost is one
            // tool round: three remain per user turn, and the deepest chain in
            // the suite uses two.
            //
            // THIS DOES NOT REPLACE `settled`. A model that writes a call from
            // memory even with the list gone still ends the turn empty, and that
            // outcome must keep being reported rather than hidden by the fix that
            // was supposed to prevent it.
            let final_turn = turn + 1 == tacet_eval::MAX_TURNS || must_answer;
            // History = the previous turns + the results of the tools that ran in
            // THIS turn. The question also sits at the end separately (see the
            // `turn_tools` comment).
            // THE QUESTION'S PLACE CHANGES WITH THE TURN, and this is the crux of
            // the tool loop:
            //
            // * First turn — no tool has run yet: the question sits in the
            //   `question` field, i.e. at the END of the prompt (in a small model
            //   the last block carries the most weight).
            // * Later turns — a tool result arrived: the question moves into the
            //   history, IN FRONT OF the tool call, and `question` is left EMPTY.
            //   The question used to be repeated at the end on every turn; right
            //   after the tool result the model saw the same request again, took
            //   it for unanswered and called the tool again. That is where the
            //   loop came from.
            let first_turn = turn_tools.is_empty();
            let question = if first_turn { asked.as_str() } else { "" };
            let previous: Vec<Turn> = if first_turn {
                history.clone()
            } else {
                history
                    .iter()
                    .cloned()
                    // `asked`, not `message`: once the question moves into the
                    // history the piped data has to move with it, or the model
                    // loses the very thing it was asked about the moment it
                    // calls its first tool.
                    .chain(std::iter::once(Turn::user(&asked)))
                    .chain(turn_tools.iter().cloned())
                    .collect()
            };
            // ON THE LAST PASS THE SYSTEM TEXT SWAPS ITS TAIL: the call
            // instructions are replaced by the statement that a call is now
            // inert (see `FINAL_PASS_INSTRUCTION`). Appending rather than
            // replacing keeps the identity, the working directory block and
            // everything else the shell put in `system`.
            let system_now = if final_turn {
                format!("{system}\n\n{}", tacet_engine::FINAL_PASS_INSTRUCTION)
            } else {
                system.clone()
            };
            let mut prompt = Prompt::new(&system_now, question).with_history(previous);
            if !final_turn {
                prompt = prompt.with_tools(&selected);
            }
            if let Some(g) = &guide {
                prompt = prompt.with_guide(g);
            }
            if let Some(m) = &memory_text {
                prompt = prompt.with_memory(m);
            }
            let report = counter.truncate(&mut prompt);
            turn_prompt += report.final_estimate;
            // The context-fullness measure is the size of the LAST prompt: that
            // is the room left in the window, not a cumulative total.
            last_context = report.final_estimate;
            // ONCE PER USER TURN. The prompt is rebuilt for every tool round,
            // so this used to print two or three times inside a single answer
            // (measured: "(2 turns dropped)" then "(4 turns dropped)" twice) —
            // noise that reads like the shell is malfunctioning.
            if report.changed() && !truncation_reported {
                truncation_reported = true;
                // ALL THREE SACRIFICES ARE NAMED, not just the cheapest one.
                // Only `dropped_turns` was reported here, so the two that
                // actually lose the CURRENT request went by in silence: the
                // guide being dropped, and the question itself being cut. The
                // question is cut FROM THE FRONT, which is exactly where piped
                // content sits — `cat big.log | tacet -m "summarise"` could lose
                // the head of the file and answer confidently about the rest.
                // Silent loss of the user's own input is the worst outcome in
                // this file; it looks like the model ignored them.
                let mut parts = Vec::new();
                if report.dropped_turns > 0 {
                    parts.push(format!(
                        "{} older turns left the window",
                        report.dropped_turns
                    ));
                }
                if report.guide_dropped {
                    parts.push("the skill guide was dropped".to_string());
                }
                if report.question_truncated {
                    parts.push("THE START OF YOUR INPUT WAS CUT".to_string());
                }
                eprintln!(
                    "{}",
                    color.paint(
                        if report.question_truncated {
                            YELLOW
                        } else {
                            DIM
                        },
                        &format!("(making room: {})", parts.join(" · "))
                    )
                );
            }
            if show_prompt {
                // THE TEMPLATE IS TAKEN FROM THE ENGINE. It used to print
                // `prompt.text()` here (i.e. always `Template::Plain`): the header
                // said "template: ChatML" while the diagnostic output showed plain
                // text, which made prompt bugs impossible to see. Diagnostic
                // output has to be the same wire that REALLY goes to the model.
                let wire = prompt.text_with_template(engine.template());
                let dump = format!(
                    "--- PROMPT ({:?}) ---\n{wire}\n--- ~{} tokens (estimate) ---",
                    engine.template(),
                    TokenCounter::estimate(&wire)
                );
                // UNDER `--json` IT MOVES TO STDERR RATHER THAN DISAPPEARING.
                // Asking for both flags is asking for two different things at
                // once, and the useful reading of that is "the machine answer on
                // stdout, the diagnostic beside it" — dropping a diagnostic the
                // user explicitly requested would be the worse answer.
                if human {
                    println!("{dump}");
                } else {
                    eprintln!("{dump}");
                }
            }

            // THE INDICATOR + THE INPUT LOCK OPEN BEFORE GENERATION AND CLOSE WHEN
            // THE TOOL FINISHES. The lock lasting past generation is MANDATORY:
            // keys pressed while a tool runs (a web search takes seconds) must not
            // spill onto the screen either. With no tty `TurnIndicator` does
            // nothing; under `--json` it is asked for the same nothing on purpose
            // (see `TurnIndicator::disabled` — a machine-readable run should not
            // have stage words scribbled across its stderr).
            let mut indicator = if human {
                TurnIndicator::start(Arc::clone(&screen), &CANCEL, "thinking")
            } else {
                TurnIndicator::disabled(Arc::clone(&screen))
            };
            // PREFILL. The number is the prompt size AFTER truncation — the same
            // figure `--show-prompt` and the status line report, and it is an
            // ESTIMATE, which is why the line prints it with a `~`. Naming this
            // wait separates "the machine is chewing on 3000 tokens of prompt"
            // from "a tool went to the network"; they take similar amounts of
            // time and, unnamed, both read as frozen.
            indicator.stage(ui::Stage::Prefill {
                tokens: report.final_estimate,
            });

            // STREAMING GENERATION: as the model produces tokens they pour onto
            // the screen. The spinner fills the 5-15 seconds until the first
            // token; when the first fragment arrives the indicator GOES QUIET and
            // gives way to the text. The streaming text is the generated text
            // itself (not a hallucination); the chip text will still be produced
            // by the TOOL.
            let streaming = AtomicBool::new(false);
            // THE STREAM PASSES THROUGH TWO FILTERS — the order matters:
            //
            //   raw tokens → CallFilter → Formatter → screen
            //
            // 1. `CallFilter` strips raw tool calls. There used to be a ONE-SHOT
            //    decision here (is the first word `name(`?) and when the model
            //    wrote a sentence before the call the decision had already been
            //    made as "plain text", so the call poured onto the screen — that
            //    was exactly the failure the user reported. The new filter follows
            //    the stream FROM START TO FINISH (see filter.rs).
            // 2. `Formatter` turns markdown into ANSI; it buffers a half-finished
            //    marker and prints it when it closes (see format.rs).
            //
            // Both are disabled with no tty / when not interactive.
            let filter = std::sync::Mutex::new(filter::CallFilter::new(catalog_names.clone()));
            let formatter = std::sync::Mutex::new(format::Formatter::new(screen.tty()));
            // The stream pours onto the screen ONLY in interactive mode. In
            // single-message/diagnostic mode the answer is printed once at the
            // end; streaming would duplicate the output (streaming text + the
            // "Tacet:" line).
            // THE REPETITION GUARD. Small models sometimes spiral: the same
            // passage re-announced and re-printed three, four times until the
            // token cap fills (measured on the 4B: a calculator repeated with
            // "let me provide a clean version now" between copies). Tool calls
            // have a structural repeat gate; this is the same idea for TEXT.
            // When a long stretch of the answer reappears verbatim, generation
            // is stopped through the same flag Ctrl-C uses; `repetition_stop`
            // picks the honest message below. The window is long (240 chars,
            // whitespace collapsed) so legitimate structure — lists, similar
            // rows — does not trip it; only a whole repeated passage can.
            let repetition_stop = AtomicBool::new(false);
            let answer_seen = std::sync::Mutex::new((String::new(), 0usize));

            // `interactive` USED TO STAND IN FOR "paint the screen" and it no
            // longer can: a `--json` run in a terminal is interactive by every
            // other measure and must still not stream a single byte onto stdout.
            let stream_to_screen = interactive && human;
            // GENERATION PROGRESS. Counted from the callbacks, which is a TRUE
            // LOWER BOUND rather than a guess: the engine fires this at most once
            // per accepted token, so the count can lag reality but can never
            // exceed it (it misses only steps whose decode added no new text —
            // see `candle_engine::run_loop`). An invented "tokens/sec" or a
            // percentage would be the kind of unmeasured figure this shell
            // refuses to put on screen.
            let streamed_tokens = std::sync::atomic::AtomicUsize::new(0);
            let listener = |chunk: &str| {
                // COUNTED BEFORE THE EARLY RETURN. In single-message mode nothing
                // streams to the screen, but the indicator is the only thing the
                // user has to look at there — that is exactly where the count is
                // worth the most.
                let seen = streamed_tokens.fetch_add(1, Ordering::Relaxed) + 1;
                // Once the answer is flowing this does NOT take the line back
                // from the text (see `stage_wakes`); it only updates the words
                // the drawing thread will use on its next 100 ms beat, so a
                // per-token call costs a lock and nothing else.
                indicator.stage(ui::Stage::Generating { tokens: seen });
                if !stream_to_screen {
                    return;
                }
                let mut visible = filter.lock().expect("filter lock").feed(chunk);
                {
                    let mut seen = answer_seen.lock().expect("repeat lock");
                    seen.0.push_str(&visible);
                    // Throttled: the check runs every ~160 new chars, not per token.
                    if seen.0.len() >= seen.1 + 160 {
                        seen.1 = seen.0.len();
                        let flat = seen.0.split_whitespace().collect::<Vec<_>>().join(" ");
                        let chars: Vec<char> = flat.chars().collect();
                        const WINDOW: usize = 240;
                        if chars.len() > WINDOW * 2 {
                            let tail: String = chars[chars.len() - WINDOW..].iter().collect();
                            let head: String = chars[..chars.len() - WINDOW].iter().collect();
                            if head.contains(&tail) {
                                repetition_stop.store(true, Ordering::Relaxed);
                                CANCEL.store(true, Ordering::Relaxed);
                            }
                        }
                    }
                }
                if !streaming.load(Ordering::Relaxed) {
                    // Empty lines AT THE START of the answer are dropped: the
                    // model often begins by skipping a line, leaving the "Tacet"
                    // prefix hanging above an empty line.
                    let trimmed = visible.trim_start();
                    if trimmed.is_empty() {
                        return;
                    }
                    visible = trimmed.to_string();
                    indicator.quiet();
                    streaming.store(true, Ordering::Relaxed);
                    screen.write(&color.paint(DIM, "Tacet "));
                    // Night theme: the answer flows in paper ink from here on;
                    // the closing RESET is written where the answer ends.
                    screen.write(paper_code());
                }
                let formatted = formatter.lock().expect("format lock").feed(&visible);
                if !formatted.is_empty() {
                    screen.write(&formatted);
                }
            };
            let generation = match wait(
                engine.generate_streaming(
                    &prompt,
                    // NO CONSTRAINT ON THE LAST PASS — see `final_turn`. Leaving
                    // it on would keep a call reachable from a prompt that no
                    // longer lists one, which is the worst of both.
                    constraint
                        .as_ref()
                        .filter(|_| !final_turn)
                        .map(|c| c as &dyn tacet_engine::Constrainer),
                    // THE CANCEL FLAG PASSES TO THE ENGINE: Ctrl-C stops generation at
                    // the next token. Without the flag a cancel would only be noticed
                    // after the cap filled.
                    //
                    // THE CAP DERIVES FROM THE PROMPT, it is not fixed (see
                    // `TokenCounter::generation_cap`): with a short prompt the unused
                    // part of the window is given to generation. On thinking models
                    // the difference is decisive — with a fixed cap the 8B was cut off
                    // before finishing its thinking block and never called a tool.
                    // THE USER'S SAMPLING CHOICE IS LAYERED ON LAST (see
                    // `SamplingChoice`), and it touches only `temperature` and
                    // `seed`: the cancel flag and the cap are properties of THIS
                    // TURN, not of a preference, so they stay owned here.
                    sampling.apply(SamplingSetting {
                        cancel: Some(&CANCEL),
                        max_tokens: counter.generation_cap(&prompt),
                        ..Default::default()
                    }),
                    &listener,
                ),
            ) {
                Ok(g) => g,
                Err(e) => {
                    indicator.finish();
                    eprintln!("\nengine error: {e}");
                    turn_error = Some(e.to_string());
                    any_turn_failed = true;
                    settled = true;
                    break;
                }
            };
            turn_generation += generation.token_count;
            // Diagnostic dump (env-gated, the same key as in `LiveReporter`): the
            // model's RAW generation — the call shape (a grammared `name({...})`
            // or the bare JSON that falls into the recovery path) and, if present,
            // the thinking block can only be seen from here.
            if tacet_kernel::env_var("TACET_TRACE_DUMP").is_some() {
                if let Some(t) = &generation.thinking {
                    eprintln!(
                        "\n--- thinking ({} tokens total) ---\n{t}",
                        generation.token_count
                    );
                }
                eprintln!(
                    "\n--- raw generation (stop: {:?}) ---\n{}\n---",
                    generation.stop, generation.text
                );
            }
            // DRAIN THE STREAM BUFFERS. Both filters may have a queue awaiting a
            // decision (a last word that is the prefix of a tool name, or an
            // unclosed `**`). Without draining, the last fragment of the answer
            // WOULD BE LOST — the price of the requirement that one-word answers
            // like "Yes" are not swallowed is exactly these two lines.
            let remaining = {
                let last = filter.lock().expect("filter lock").finish();
                let mut f = formatter.lock().expect("format lock");
                let mut s = f.feed(&last);
                s.push_str(&f.finish());
                s
            };
            // If nothing has streamed yet, leading whitespace is dropped (in
            // streaming text `listener` does this).
            let remaining = if streaming.load(Ordering::Relaxed) {
                remaining
            } else {
                remaining.trim_start().to_string()
            };
            if stream_to_screen
                && !streaming.load(Ordering::Relaxed)
                && !remaining.trim().is_empty()
            {
                indicator.quiet();
                streaming.store(true, Ordering::Relaxed);
                screen.write(&color.paint(DIM, "Tacet "));
                screen.write(paper_code());
            }
            if streaming.load(Ordering::Relaxed) {
                if !remaining.is_empty() {
                    screen.write(&remaining);
                }
                // Close the paper ink with the PLAIN reset: whatever follows
                // the answer (chips, the input frame) styles itself.
                screen.write(RESET);
                screen.write("\n");
            }
            indicator.quiet();

            // CANCEL: let no tool run and let the turn end. We hook into
            // `ToolExecutor`'s own cancel mechanism — half-generated text may well
            // be a tool call, and running that call in a cancelled turn would mean
            // ignoring the user saying "stop".
            if CANCEL.load(Ordering::Relaxed) {
                indicator.finish();
                if repetition_stop.load(Ordering::Relaxed) {
                    // Not the user's stop: the guard's. What streamed stays on
                    // screen AND in the history — the first copy of the passage
                    // is usually a perfectly good answer.
                    if human {
                        screen.line(
                            &color.paint(DIM, "  (stopped: the answer began repeating itself)"),
                        );
                    }
                    answer = generation.text;
                } else {
                    executor.cancel();
                    if human {
                        screen.line(&color.paint(DIM, "  (stopped)"));
                    }
                    answer = String::new();
                }
                settled = true;
                break;
            }

            if !generation.stop.is_complete() {
                indicator.finish();
                // WHY IT IS SAID: without this warning, the 8B hitting the token
                // cap was taken for "the model is broken" — with Length and
                // Cancelled named by the same sentence, the diagnosis was left to
                // the user. (Cancel is normally caught above; in practice this
                // branch is the cap.)
                let reason = match generation.stop {
                    tacet_engine::StopReason::Length => "the token cap filled",
                    _ => "cancelled",
                };
                eprintln!(
                    "{}",
                    color.paint(YELLOW, &format!("(generation was cut short: {reason})"))
                );
                settled = true;
                break;
            }

            // WHICH TOOL IS ABOUT TO RUN — for the indicator only.
            //
            // THE CALL IS PARSED A SECOND TIME and that is deliberate:
            // `ToolCall::parse` is pure, it takes a string and returns a name,
            // and the alternative was reading the name back out of the CHIP,
            // which is a screen object carrying an icon and a human sentence.
            // Deriving a fact from its own presentation is how the two drift.
            // `None` here is not a failure — the executor has a recovery layer
            // for nameless JSON that `parse` deliberately does not know about —
            // and in that case the indicator simply keeps the words it had.
            //
            // "checking" vs "running": the two code tools RUN the model's code
            // before they keep anything (write_code's syntax pass, run_code's
            // sandbox setup), and that check is the step that FAILS. A failure is
            // much easier to read when the screen already said which step was in
            // progress. THE LIMIT IS HONEST: from here we cannot see the moment
            // the check ends and the acting begins, so the word stands for the
            // whole call — and for these two the call IS mostly the check. The
            // tool's own chip is what announces the acting part.
            if let Some(call) = tacet_tools::executor::ToolCall::parse(&generation.text) {
                indicator.stage(if VERIFYING_TOOLS.contains(&call.name.as_str()) {
                    ui::Stage::Verifying { name: call.name }
                } else {
                    ui::Stage::Tool { name: call.name }
                });
            }
            let Some(outcome) = wait(executor.execute_raw(&generation.text, ticket, &mut ctx))
            else {
                indicator.finish();
                answer = generation.text;
                // THE SAFETY VALVE: if nothing was printed in the stream, the
                // answer is printed HERE. This can happen two ways — either the
                // answer is a single word and generation ends before the buffer
                // can decide, or the text taken for a call turns out not to be a
                // valid tool call. In both cases leaving the screen blank would
                // mean swallowing the answer.
                if stream_to_screen && !streaming.load(Ordering::Relaxed) {
                    screen.write(&color.paint(DIM, "Tacet "));
                    screen.write(paper_code());
                    screen.line(&format::Formatter::all(screen.tty(), answer.trim()));
                    screen.write(RESET);
                }
                settled = true;
                break;
            };
            indicator.finish();
            // A REPEATED CALL ENDS THE TOOL PHASE OF THIS TURN.
            //
            // THE EXECUTOR ALREADY DID ITS HALF and it worked: the same (tool,
            // arguments) pair does not RUN a second time in a turn — that gate is
            // code, not text, and it held. What was missing was a CONSEQUENCE.
            // The refusal came back as a sentence and the model, being a 4B,
            // ignored it and called again; the user waited through every extra
            // generation. From here the next pass runs the way the LAST pass
            // runs: no tool list, no grammar, `FINAL_PASS_INSTRUCTION` instead of
            // the call instructions. The model cannot repeat what it is no longer
            // offered.
            if outcome.reason == tacet_tools::executor::ExecutionReason::RepeatedCall {
                must_answer = true;
            }
            let is_error = outcome.is_error();
            let retryable = outcome.retryable;

            // THE MACHINE-READABLE TRACE, recorded HERE rather than reconstructed
            // from the chips afterwards. A chip carries an icon and a human
            // sentence — deliberately, it is a screen object — and reverse
            // engineering a tool name out of it would be a second, weaker source
            // for a fact this line already has exactly.
            turn_calls.push(tool_record(&outcome, &generation.text));

            // THE MODEL MUST SEE ITS OWN CALL IN THE HISTORY. Only the tool RESULT
            // used to be fed back; on the next turn the model saw a context-free
            // result line with the REPEATED user question right below it, took the
            // question for unanswered and called the same tool again — up to the
            // turn limit, without ever answering the user. Writing the call as an
            // `assistant` turn is what establishes the "I asked for this, and the
            // result came" context.
            turn_tools.push(Turn::assistant(generation.text.trim()));
            turn_tools.push(Turn::tool(outcome.to_model.clone()));

            // NO RECOVERY TURN AFTER A SIDE EFFECT (only at the intersection of an
            // error and an irreversible side effect).
            if is_error && !retryable {
                eprintln!(
                    "{}",
                    color.paint(
                        YELLOW,
                        "(a side effect happened — no recovery turn was opened)"
                    )
                );
                answer = "The operation partly went through; I did not retry.".to_string();
                settled = true;
                break;
            }
        }

        // THE TURN BUDGET RAN OUT WITH NOTHING TO SHOW. The model called a tool
        // on every one of `MAX_TURNS` passes and never wrote a sentence. This is
        // NOT dressed up as an answer: an invented closing line would put words
        // in the model's mouth, and the tools that DID run are already on screen
        // as chips, so the user can see what happened. What they could not see
        // before was that it had STOPPED.
        //
        // It counts as a failed turn for the same reason an engine error does:
        // the caller of `tacet -m ... --json` checks the exit code, and a run
        // that produced no answer must not report success.
        if !settled {
            any_turn_failed = true;
            turn_error = Some(format!(
                "the turn budget ({}) ran out — a tool was called on every pass and no answer was written",
                tacet_eval::MAX_TURNS
            ));
            if human {
                eprintln!(
                    "{}",
                    color.paint(
                        YELLOW,
                        &format!(
                            "(stopped after {} tool turns without an answer — try asking for one \
                             step at a time)",
                            tacet_eval::MAX_TURNS
                        )
                    )
                );
            }
        }

        last_turn_prompt = turn_prompt;
        last_turn_generation = turn_generation;
        session_tokens += turn_prompt + turn_generation;

        // THE LAST ARTIFACT — what ctrl-o / /preview shows. Code is hidden by
        // default (a tool call never pours onto the screen); this remembers
        // which file the turn produced so the user can peek at it on demand.
        if let Some(p) = traces
            .traces()
            .iter()
            .rev()
            .find_map(|t| t.file_path.clone())
        {
            last_artifact = Some(p);
        }

        // THE RECEIPT CHAIN — every trace of this turn, witnessed by pure
        // code (see receipt.rs; `tacet log` shows and verifies them).
        {
            let at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            for t in traces.traces() {
                receipt::append(at, &t.icon, &t.text, &format!("{:?}", t.state));
            }
        }

        // Chips: Tacet does not hide what it did. In interactive mode they were
        // already printed LIVE (see `LiveReporter`); printing them again here
        // would duplicate the screen. In single-message/diagnostic mode this is
        // the only place they are printed — and under `--json` they are not
        // printed at all, because the same facts leave in the `tools` field.
        if !interactive && human {
            for trace in traces.traces() {
                if !trace.text.trim().is_empty() {
                    println!(
                        "  {}",
                        color.paint(
                            DIM,
                            &format!("[{}] {} · {:?}", trace.icon, trace.text, trace.state)
                        )
                    );
                }
            }
        }
        // THE PERSISTENT HISTORY is written when the turn ends and its ORDER is
        // meaningful: user -> tool results -> assistant.
        //
        // `asked`, NOT `message`: what is replayed on the next turn has to be
        // what the model was actually given, or a `--continue` would resume a
        // conversation whose first question referred to data that is no longer
        // anywhere in the context.
        history.push(Turn::user(&asked));
        history.append(&mut turn_tools);
        if !answer.is_empty() {
            history.push(Turn::assistant(&answer));
            // In interactive mode the answer was already printed while streaming;
            // do not print it again.
            if !interactive && human {
                println!("Tacet: {answer}");
            }
        }

        let tool_names: Vec<String> = turn_calls
            .iter()
            .filter_map(|c| c.get("tool").and_then(|t| t.as_str()).map(str::to_string))
            .collect();

        // THE TRANSCRIPT IS WRITTEN AT THE END OF THE TURN, not at its start.
        //
        // A turn only becomes a conversation once there is an answer; appending
        // the question first and then crashing would leave a record of a
        // question nobody answered, and `--continue` would replay it as though
        // the model had simply ignored the user.
        if let Some(s) = &mut chat_session {
            // FIRST WRITE, ONE NOTICE. The user is told where their words are
            // going BEFORE the file grows, once per install — see
            // `announce_transcript`.
            announce_transcript(&color, human);
            let stored_user = session::Turn::new(session::Role::User, &asked);
            let mut failure = s.append(&stored_user).err();
            if !answer.is_empty() {
                let stored_answer = session::Turn::new(session::Role::Assistant, &answer)
                    .with_tools(tool_names.clone());
                failure = failure.or(s.append(&stored_answer).err());
            }
            // SAID ONCE, THEN THE SHELL SHUTS UP ABOUT IT. A full disk fails on
            // every turn, and a warning per turn would bury the conversation;
            // but staying silent from the start would let the user believe a
            // transcript exists that does not. So: report, then stop writing.
            if let Some(reason) = failure {
                // THE PATH IS NAMED. "Could not write" without a target sends
                // the user looking through a config directory they have never
                // opened; a permissions or full-disk problem is fixable only by
                // someone who knows which file to look at.
                let where_to = s
                    .path()
                    .map(|p| format!(" ({})", p.display()))
                    .unwrap_or_default();
                eprintln!(
                    "{}",
                    color.paint(
                        YELLOW,
                        &format!(
                            "(this conversation is NOT being saved{where_to} — {reason}; nothing \
                             else will be written this session)"
                        )
                    )
                );
                chat_session = None;
            }
        }

        if json {
            // ONE LINE, ON STDOUT, AND IT IS THE ONLY THING ON STDOUT. In an
            // interactive `--json` session this makes the stream JSONL: one
            // object per turn, which is what a reader consuming a live pipe can
            // actually parse.
            let mut record = serde_json::json!({
                "answer": answer,
                "tools": tool_names,
                "traces": turn_calls,
                "tokens": {
                    "prompt": turn_prompt,
                    "generation": turn_generation,
                    "context": last_context,
                    "session": session_tokens,
                },
                // `null` when nothing is being kept (see `keep_transcript`):
                // an invented id would point at a file that does not exist.
                "session": chat_session.as_ref().and_then(session::Session::id),
            });
            // ABSENT ON SUCCESS, so `has("error")` is the check a script makes.
            // Present means `answer` is not an answer — do not treat an empty
            // string as one.
            //
            // IT IS INSERTED RATHER THAN LISTED ABOVE, and that is the whole
            // point: written as `"error": turn_error` the field serialised to
            // `null` on a clean turn, so it was ALWAYS present and the documented
            // `has("error")` check was true for every line the shell ever
            // printed. The comment described a contract the code did not keep.
            if let Some(reason) = &turn_error
                && let Some(map) = record.as_object_mut()
            {
                map.insert("error".into(), serde_json::Value::String(reason.clone()));
            }
            println!("{record}");
        } else {
            println!();
        }
        // Cleared per turn: an interactive session must not carry one failure
        // into every later JSON line.
        turn_error = None;
        let _ = std::io::stdout().flush();

        if single_message.is_some() {
            break;
        }

        // ONE-SHOT RUNS NEVER REACH HERE — the branch above leaves first. A
        // script piping `--message` must not be stopped by a question.
        completed_turns += 1;
        update::maybe_offer(&color, completed_turns);
    }

    // The session is over: this is the one place a notice costs the user
    // nothing. At start-up it would delay the first paint and block on the
    // network; mid-session it would interrupt. It prints only if the user
    // turned the check on, and stays silent when the check fails.
    if let Some(line) = update::daily_notice(&color) {
        eprintln!();
        eprintln!("{line}");
    }

    // A turn that never produced an answer is a FAILED RUN. This matters most
    // for `tacet -m "..." --json`, where the caller is a script and the exit
    // code is the only signal it checks before piping the output onward.
    if any_turn_failed {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// `tacet 0.1.9 (metal)` — the version AND what it can do.
///
/// The feature belongs on this line because it is the difference between a
/// binary that runs a model and one that cannot, and that difference has
/// already cost a user an afternoon: `cargo install` does not remember
/// `--features`, so the same version number can mean either program.
pub fn version_line() -> String {
    format!(
        "tacet {} ({})",
        env!("CARGO_PKG_VERSION"),
        update::compiled_features()
    )
}

/// The interactive settings menu behind a bare `/config`.
///
/// Enter CYCLES the value and writes it immediately, and the menu STAYS OPEN —
/// the row redrawing with its new value is the confirmation, so there is no save
/// step and no "did that take?" line to read. A settings screen with a confirm
/// button invites that question; the file is one line of JSON either way.
fn config_menu(color: &Color) {
    // Piped: print the list and leave. `/config` in a script must produce text,
    // not a control surface.
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        let _ = config::list(false);
        return;
    }
    let mut selected = 0usize;
    let mut restart_needed = false;

    loop {
        let rows: Vec<ui::MenuRow> = config::keys()
            .into_iter()
            .map(|(key, help)| ui::MenuRow {
                label: format!("{key:<14}"),
                value: config::shown_value(key),
                hint: help.to_string(),
            })
            .collect();

        match ui::menu(color, "settings · enter cycles the value", &rows, selected) {
            ui::MenuOutcome::Cancelled => break,
            ui::MenuOutcome::Chose(index) => {
                selected = index;
                let (key, _) = config::keys()[index];
                let Some(next) = config::next_value(key) else {
                    // `model` with nothing on disk is the only key with no list.
                    // Saying so beats a keypress that appears to do nothing.
                    println!(
                        "{}",
                        color.paint(
                            DIM,
                            &format!(
                                "  ({key}: nothing to choose from — `/config set {key} <value>`)"
                            )
                        )
                    );
                    break;
                };
                match config::set_value(key, &next) {
                    Ok(()) => {
                        // The theme is the one setting that can land NOW; every
                        // other one is read at start-up.
                        if key == "theme" {
                            ui::set_theme(&next);
                        } else {
                            restart_needed = true;
                        }
                    }
                    Err(e) => {
                        println!("{}", color.paint(YELLOW, &format!("  {e}")));
                        break;
                    }
                }
            }
        }
    }

    // Said ONCE, on the way out, rather than after every keypress: a line that
    // repeats on every cycle turns into noise and stops being read.
    if restart_needed {
        println!(
            "{}",
            color.paint(
                DIM,
                "  (changes other than the theme apply the next time tacet starts)"
            )
        );
    }
}

/// The counter line sitting under the input field.
///
/// THREE NUMBERS, EACH ANSWERING A DIFFERENT QUESTION:
///   * `this turn` — the prompt + generation spent on the last question (tool
///     turns included),
///   * `session` — the total since the shell opened,
///   * `context` — the LAST prompt's place in the window the MODEL declared
///     (see `engine_window`; it is no longer a constant). This is the
///     critical one: when the window fills, old turns drop SILENTLY (see
///     `TokenCounter::truncate`) and the user only noticed it when the model
///     "forgot" something.
///
/// The numbers ARE ESTIMATES (see `TokenCounter::estimate` — deliberately biased
/// high); no separate counter was invented.
pub fn status_line(
    turn_prompt: usize,
    turn_generation: usize,
    session: usize,
    context: usize,
    counter: &TokenCounter,
) -> String {
    if session == 0 {
        return "/ command list · ctrl-c stops · ctrl-d exits".to_string();
    }
    let cap = counter.prompt_cap();
    let fullness = if context >= cap {
        " · window full, old turns dropping"
    } else {
        ""
    };
    // THE DENOMINATOR IS THE COUNTER'S OWN BUDGET, never the constant. It used
    // to print `CONTEXT_BUDGET` while truncation was already deciding against a
    // different number — a status line that disagrees with the mechanism it
    // reports on is worse than no status line, because it is believed.
    format!(
        "this turn {}+{} · session {session} · context {context}/{} tokens{fullness}",
        turn_prompt, turn_generation, counter.budget
    )
}

// ---------------------------------------------------------------------------
// Slash commands
// ---------------------------------------------------------------------------

enum SlashResult {
    Quit,
    Handled,
    Unknown,
    /// A menu choice turned into a COMMAND: run `0` as if the user had typed
    /// it. Menus do not act by themselves — every action flows through the
    /// same slash machinery (approval, catalog refresh, receipts) as a typed
    /// command, so the two paths cannot behave differently.
    Replay(String),
}

// THE ARGUMENT LIST IS THE SESSION, not a design smell. Every one of these is a
// piece of live session state a slash command may have to read or replace
// (`/clear` the history, `/addon` the catalog, `/preview` the last artifact);
// bundling them into a struct would only move the same list one file away, and
// `refresh_session` above carries the same allow for the same reason.
#[allow(clippy::too_many_arguments)]
fn slash(
    command: &str,
    catalog: &ToolCatalog,
    memory: &SharedMemory,
    history: &mut Vec<Turn>,
    engine: &Arc<dyn EngineProvider>,
    color: &Color,
    last_artifact: &Option<std::path::PathBuf>,
    screen: &Screen,
) -> SlashResult {
    let name = command.split_whitespace().next().unwrap_or("");
    match name {
        // `/exit` IS KEPT AS AN ALIAS: it is the habit carried over from other
        // shells and refusing it costs nothing.
        "/quit" | "/exit" => SlashResult::Quit,
        "/help" => {
            // THE VERSION SITS AT THE TOP OF HELP because "which build am I in"
            // is the question that gets asked when something behaves oddly, and
            // the answer includes the feature: a shell on the fake engine looks
            // identical to one on a real model until it answers.
            println!(
                "{}  {}",
                color.paint(BOLD, "commands"),
                color.paint(DIM, &version_line())
            );
            // THE LIST IS IN ONE PLACE (input::COMMANDS). There is no chance of
            // the `/` popup list and this output diverging: both read the same
            // array.
            for c in input::COMMANDS {
                println!("  {:12} {}", c.name, color.paint(DIM, c.description));
            }
            println!(
                "  {}",
                color.paint(
                    DIM,
                    "typing / opens the command list · while generating, ctrl-c stops the answer"
                )
            );
            SlashResult::Handled
        }
        // EVAL AND GRAMMAR WORK FROM INSIDE TOO. Both remain as subcommands
        // (scripts depend on them) but being able to look without leaving the
        // shell is the request itself — "open Tacet and reach everything from
        // inside it". `/eval` runs the LOGIC set: with the fake engine, in
        // seconds. We did not wire the tool SELECTION set here — it takes minutes
        // and the shell would stay locked for that whole time; its place is still
        // `tacet eval --tool-selection`.
        "/eval" => {
            let report = tacet_eval::run(&tacet_eval::all(), &FakeSelector);
            print!("{}", report.table());
            SlashResult::Handled
        }
        "/grammar" => {
            let Some(tool_name) = command.split_whitespace().nth(1) else {
                println!("{}", color.paint(DIM, "(usage: /grammar <tool name>)"));
                return SlashResult::Handled;
            };
            print_grammar(tool_name, catalog, color);
            SlashResult::Handled
        }
        "/tools" => {
            for tool in catalog.tools() {
                let tag = if tool.taints_session() {
                    color.paint(YELLOW, " [taints]")
                } else {
                    String::new()
                };
                println!("  {}{}", tool.name(), tag);
                println!("    {}", color.paint(DIM, tool.description()));
            }
            SlashResult::Handled
        }
        "/memory" => {
            let output = memory.with(|s| {
                if s.count() == 0 {
                    "(memory is empty)".to_string()
                } else {
                    s.notes()
                        .iter()
                        .map(|n| format!("  · {}", n.text))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            });
            println!(
                "{}",
                output.unwrap_or_else(|| "(memory could not be read)".into())
            );
            SlashResult::Handled
        }
        "/history" => {
            if history.is_empty() {
                println!("{}", color.paint(DIM, "(history is empty)"));
            }
            for t in history.iter() {
                println!(
                    "  {:?}: {}",
                    t.role,
                    t.text.chars().take(80).collect::<String>()
                );
            }
            SlashResult::Handled
        }
        "/model" => {
            println!("  engine: {}", engine.name());
            println!("  template: {:?}", engine.template());
            println!(
                "  constraint: {}",
                if engine.vocab().is_some() {
                    "on"
                } else {
                    "off"
                }
            );
            SlashResult::Handled
        }
        // The peek key. Code stays HIDDEN by default — a tool call never pours
        // onto the screen — and this is the other half of that bargain: one
        // keystroke (ctrl-o) shows what was just written, numbered, capped.
        "/preview" => {
            match last_artifact {
                None => println!(
                    "{}",
                    color.paint(DIM, "(nothing saved in this session yet)")
                ),
                Some(p) => match std::fs::read_to_string(p) {
                    Err(e) => println!(
                        "{}",
                        color.paint(YELLOW, &format!("({} could not be read: {e})", p.display()))
                    ),
                    Ok(text) => {
                        const CAP: usize = 60;
                        println!("{}", color.paint(BOLD, &p.display().to_string()));
                        for (i, line) in text.lines().take(CAP).enumerate() {
                            println!("  {} {line}", color.paint(DIM, &format!("{:>4}", i + 1)));
                        }
                        let total = text.lines().count();
                        if total > CAP {
                            println!(
                                "{}",
                                color.paint(
                                    DIM,
                                    &format!("  … {} more lines in the file", total - CAP)
                                )
                            );
                        }
                    }
                },
            }
            SlashResult::Handled
        }
        "/clear" => {
            history.clear();
            println!("{}", color.paint(DIM, "(history deleted — the fixed prompt and tools still occupy the window on the next turn)"));
            SlashResult::Handled
        }
        // The in-shell face of `tacet sessions`. LISTING ONLY — `--purge` is
        // deliberately not reachable from here: deleting every conversation is
        // not a thing to be one keystroke away from in the middle of one.
        "/sessions" => {
            let _ = sessions(false, false);
            SlashResult::Handled
        }
        // The in-shell view of `tacet addon list`: which addons are installed
        // and whether they are on. Managing them (install/remove) stays a CLI
        // subcommand — it can restart docker containers and asks questions,
        // neither belongs in the middle of a conversation.
        "/plugins" | "/addons" => {
            // The MENU is the primary face; the printed list stays as the
            // non-tty fallback. Every pick becomes a Replay so it runs through
            // the exact command path a typed `/addon …` takes.
            if screen.tty() {
                let record = tacet_web::addon::read().ok();
                let installed_open = |n: &str| {
                    record
                        .as_ref()
                        .map(|r| (r.find(n).is_some(), r.is_open(n)))
                        .unwrap_or((false, false))
                };
                let items: Vec<(String, String)> = tacet_web::addon::DEFINITIONS
                    .iter()
                    .map(|d| {
                        let (inst, open) = installed_open(d.name);
                        let state = if !inst {
                            "not installed"
                        } else if open {
                            "installed · on"
                        } else {
                            "installed · off"
                        };
                        (format!("{} · {}", d.name, state), d.summary.to_string())
                    })
                    .collect();
                let Some(i) = input::menu(screen, "addons — enter opens, esc closes", &items)
                else {
                    return SlashResult::Handled;
                };
                let d = &tacet_web::addon::DEFINITIONS[i];
                let (inst, open) = installed_open(d.name);
                let actions: Vec<(String, String)> = if !inst {
                    vec![
                        (
                            format!("install {}", d.name),
                            "download nothing? it asks its own questions first".into(),
                        ),
                        ("back".into(), "".into()),
                    ]
                } else {
                    vec![
                        (
                            if open {
                                format!("turn {} off", d.name)
                            } else {
                                format!("turn {} on", d.name)
                            },
                            "takes effect immediately in this session".into(),
                        ),
                        (
                            format!("remove {}", d.name),
                            "uninstall; settings are forgotten".into(),
                        ),
                        ("back".into(), "".into()),
                    ]
                };
                let title = format!("{} — {}", d.name, d.summary);
                return match input::menu(screen, &title, &actions) {
                    None => SlashResult::Handled,
                    Some(a) => {
                        let cmd = if !inst {
                            match a {
                                0 => Some(format!("/addon install {}", d.name)),
                                _ => None,
                            }
                        } else {
                            match a {
                                0 => Some(format!(
                                    "/addon {} {}",
                                    if open { "off" } else { "on" },
                                    d.name
                                )),
                                1 => Some(format!("/addon remove {}", d.name)),
                                _ => None,
                            }
                        };
                        match cmd {
                            Some(c) => SlashResult::Replay(c),
                            None => SlashResult::Handled,
                        }
                    }
                };
            }
            let _ = addon::list(false);
            println!("{}", color.paint(DIM, "  (in here: /addon install <name> · /addon remove <name> · /addon on|off <name>)"));
            SlashResult::Handled
        }
        // Managing addons WITHOUT leaving the shell. The transcript that forced
        // this: the list's hint says `tacet addon install web-search`, the user
        // typed exactly that into the chat, and it went to the MODEL as a
        // message. Slash commands never reach the model, so the same verbs live
        // here. Install's [y/N] question still works — between turns the
        // terminal is out of raw mode and stdin is a plain line read.
        "/addon" => {
            let mut parts = command.split_whitespace().skip(1);
            match (parts.next(), parts.next()) {
                (Some("install"), Some(name)) => {
                    let _ = addon::install(name, None, false, false);
                }
                (Some("remove"), Some(name)) => {
                    let _ = addon::remove(name);
                }
                (Some("on") | Some("open"), Some(name)) => {
                    let _ = addon::set_state(name, true);
                }
                (Some("off") | Some("close"), Some(name)) => {
                    let _ = addon::set_state(name, false);
                }
                (None, _) => {
                    let _ = addon::list(false);
                }
                _ => {
                    println!("{}", color.paint(DIM, "(usage: /addon install <name> · /addon remove <name> · /addon on|off <name>)"));
                }
            }
            SlashResult::Handled
        }
        // Colour themes: list them (each row previewed in its own accent) or
        // switch live — the new palette takes over from the very next line and
        // is persisted, so the next start opens the same way.
        "/themes" => {
            match command.split_whitespace().nth(1) {
                None if screen.tty() => {
                    let active = ui::active_theme().name;
                    let items: Vec<(String, String)> = ui::THEMES
                        .iter()
                        .map(|t| {
                            let mark = if t.name == active { " · active" } else { "" };
                            (format!("{}{mark}", t.name), t.description.to_string())
                        })
                        .collect();
                    return match input::menu(screen, "themes — enter applies, esc closes", &items)
                    {
                        Some(i) => SlashResult::Replay(format!("/themes {}", ui::THEMES[i].name)),
                        None => SlashResult::Handled,
                    };
                }
                None => {
                    let active = ui::active_theme().name;
                    for t in ui::THEMES {
                        let mark = if t.name == active { "›" } else { " " };
                        println!(
                            "  {mark} {}{:<9}{} {}",
                            ui::theme_accent(t),
                            t.name,
                            ui::reset_code(),
                            color.paint(DIM, t.description)
                        );
                    }
                    println!("{}", color.paint(DIM, "  (switch: /themes night)"));
                }
                Some(name) => {
                    if ui::set_theme(name) {
                        if let Err(e) = config::set_value("theme", name) {
                            println!(
                                "{}",
                                color.paint(YELLOW, &format!("  (applied, but not saved: {e})"))
                            );
                        }
                        println!("  theme: {}{name}{}", ui::brass_code(), ui::reset_code());
                    } else {
                        let names: Vec<&str> = ui::THEMES.iter().map(|t| t.name).collect();
                        println!(
                            "{}",
                            color.paint(
                                DIM,
                                &format!("(unknown theme — themes: {})", names.join(", "))
                            )
                        );
                    }
                }
            }
            SlashResult::Handled
        }
        // The in-shell face of `tacet config`. Reading and writing the file is
        // side-effect-free, so it is safe mid-conversation — but every current
        // key is read AT STARTUP (model/engine when the shell opens, theme
        // once per process), so a change is honest about when it lands.
        "/config" => {
            let mut parts = command.split_whitespace().skip(1);
            match (parts.next(), parts.next(), parts.next()) {
                // BARE `/config` IS A MENU, not a listing. Typing a key, its
                // spelling and one of its legal values is three chances to be
                // wrong before the first success; a list shows all three and
                // Enter is the only verb. `get`/`set`/`unset` still work — they
                // are what a script uses, and what the CLI subcommand shares.
                (None, _, _) => config_menu(color),
                (Some("get"), Some(key), _) => match config::get_str(key) {
                    Some(v) => println!("  {key} = {v}"),
                    None => println!("{}", color.paint(DIM, &format!("  ({key} is unset)"))),
                },
                (Some("set"), Some(key), Some(value)) => match config::set_value(key, value) {
                    Ok(()) => {
                        println!("  {key} = {value}");
                        // The theme is the one setting that can land NOW.
                        if key == "theme" {
                            let _ = ui::set_theme(value);
                        } else {
                            println!(
                                "{}",
                                color.paint(DIM, "  (takes effect the next time tacet starts)")
                            );
                        }
                    }
                    Err(e) => println!("{}", color.paint(YELLOW, &format!("  {e}"))),
                },
                (Some("unset"), Some(key), _) => {
                    let _ = config::unset(key);
                }
                _ => {
                    println!("{}", color.paint(DIM, "(usage: /config · /config get <key> · /config set <key> <value> · /config unset <key>)"));
                }
            }
            SlashResult::Handled
        }
        _ => SlashResult::Unknown,
    }
}

/// Reports the MCP load to the user — none of it is silently swallowed.
pub fn report_mcp(load: &mcp::LoadOutcome, color: &Color) {
    for (connection, reason) in &load.connection_errors {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "(mcp: the '{connection}' connection could not be established — {reason})"
                )
            )
        );
    }
    for skipped in &load.skipped {
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "(mcp: the '{}' tool was skipped [{}] — {})",
                    skipped.remote_name, skipped.connection, skipped.reason
                )
            )
        );
    }
    for note in &load.notes {
        eprintln!("{}", color.paint(DIM, &format!("(mcp: {note})")));
    }
    if !load.tools.is_empty() {
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "(mcp: {} remote tools added to the catalog)",
                    load.tools.len()
                )
            )
        );
    }
}

#[cfg(test)]
mod sampling_tests {
    use super::*;

    /// TOUCHING NOTHING MUST CHANGE NOTHING. The sampling knobs were added so
    /// variance could be MEASURED and a retry could take a different path; they
    /// were not added to move the default. If this test ever fails, every
    /// number ever recorded against this shell has silently changed meaning.
    #[test]
    fn the_sampling_default_is_still_greedy_and_seeded_at_zero() {
        let base = SamplingSetting::default();
        let applied = SamplingChoice::default().apply(base);
        assert_eq!(applied.temperature, base.temperature);
        assert_eq!(applied.seed, base.seed);
        assert_eq!(applied.temperature, 0.0, "the default must stay greedy");
        assert!(SamplingChoice::default().line().is_none());
    }

    /// The knobs move only what they own. `max_tokens` is derived from the
    /// prompt's length and `cancel` is the user's Ctrl-C — a preference must not
    /// be able to reach either, or a config file could quietly cap generation.
    #[test]
    fn sampling_touches_only_temperature_and_seed() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let base = SamplingSetting {
            max_tokens: 777,
            cancel: Some(&FLAG),
            ..Default::default()
        };
        let applied = SamplingChoice {
            temperature: Some(0.8),
            seed: Some(42),
        }
        .apply(base);
        assert_eq!(applied.temperature, 0.8);
        assert_eq!(applied.seed, 42);
        assert_eq!(applied.max_tokens, 777);
        assert!(applied.cancel.is_some());
        assert_eq!(applied.repeat_penalty, base.repeat_penalty);
    }

    /// An out-of-range temperature is CLAMPED, not obeyed and not refused.
    /// Above ~2 the distribution is near-uniform and the output is noise; a
    /// negative one is not a temperature at all. Refusing to start the shell
    /// over a number in a config file would be the worse answer.
    #[test]
    fn an_impossible_temperature_is_clamped() {
        let hot = SamplingChoice {
            temperature: Some(50.0),
            seed: None,
        }
        .apply(SamplingSetting::default());
        assert_eq!(hot.temperature, MAX_TEMPERATURE);
        let cold = SamplingChoice {
            temperature: Some(-3.0),
            seed: None,
        }
        .apply(SamplingSetting::default());
        assert_eq!(cold.temperature, 0.0);
    }

    /// A non-greedy session must SAY SO. The banner line is the only place the
    /// user can see that this shell is not the reproducible one.
    #[test]
    fn a_raised_temperature_shows_up_on_the_banner() {
        let note = SamplingChoice {
            temperature: Some(0.7),
            seed: Some(9),
        }
        .line()
        .expect("a non-default sampling must produce a line");
        assert!(note.contains("0.7"), "{note}");
        assert!(note.contains('9'), "{note}");
        // Temperature 0 is the default; naming it would put a permanent, useless
        // decoration on every banner.
        assert!(
            SamplingChoice {
                temperature: Some(0.0),
                seed: None
            }
            .line()
            .is_none()
        );
    }
}
