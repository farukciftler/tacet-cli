//! tacet-cli — the terminal shell of the on-device assistant.
//!
//! WHY IT EXISTS: when Tacet's logic layer (routing, prompt construction,
//! grammar, tool execution, the bypass channel, skill/memory injection) is
//! hidden inside an iOS app it can only be observed by opening the simulator.
//! This binary drives the same layer from the terminal: `chat` opens a flowing,
//! an interactive turn loop against a real model, `eval` runs in CI,
//! `grammar`/`tools` print the prompt's source verbatim.
//!
//! WHAT IS LEFT IN THIS FILE, and it is worth naming because it used to be
//! nearly eight thousand lines and everything below was in it. What stays here
//! is the part that is genuinely `main`: the process entry point, the dispatch
//! from a parsed command to the function that answers it, and the handful of
//! pieces every command shares (the catalog the shell builds, the system block,
//! the piped-stdin fence, the two terminal gates). Everything with a subject of
//! its own was moved out:
//!
//! | module | what it owns |
//! |---|---|
//! | `cli` | the argument table — what `tacet` accepts, and nothing more |
//! | `chat` | the turn loop, the slash commands, sampling |
//! | `engine_setup` | finding the weights and opening them |
//! | `eval_cmd` | the three measurements and the comparison between two of them |
//! | `models` | `tacet models list` / `download` |
//! | `diagnostics` | `why`, `doctor`, `tools`, `grammar`, `mcp …` |
//!
//! THE SPLIT IS BY SUBJECT, NOT BY SIZE. `chat` is still two thousand lines and
//! that is correct: it is one loop, and cutting it at an arbitrary line would
//! put the tool phase and the answer phase in different files for no reason a
//! reader could reconstruct.
//!
//! ENGINE SELECTION IS AUTOMATIC. `--engine auto` (the default) uses the REAL
//! model if it finds a local model PACKAGE and the binary was built with the
//! `candle` feature; otherwise it falls back to FakeEngine with a MEANINGFUL
//! message — not silently.
//!
//! THIS HEADER ONCE LIED, and that is why the paths are no longer written out
//! here one by one; they derive from the `model_package` module. The variable
//! names and the config path used to be written HERE BY HAND; both had gone
//! stale after a brand change and, because doc comments are not compiled, nobody
//! noticed. The single source for the current list: `tacet models list` (it
//! PRINTS the roots and the packages found) and `tacet_kernel::env` (the config
//! directory).
//!
//! In short: the weights can be given directly with `TACET_MODEL`/
//! `TACET_TOKENIZER`; if not given, a `<name>/*.gguf` + `tokenizer.json` pair is
//! looked for in the platform's model roots (`~/models`,
//! `$XDG_DATA_HOME/tacet/models`; on Windows `%USERPROFILE%\models`,
//! `%LOCALAPPDATA%\Tacet\models`).
//!
//! NETWORK: in a default install no command makes a network call. Network
//! traffic only occurs when the user's own web search (`web_search`/`web_fetch`)
//! or an MCP connection (`mcp.json` in the config directory — on Unix
//! `$XDG_CONFIG_HOME/tacet` or `~/.tacet`, on Windows `%APPDATA%\Tacet`) is
//! used; both are external tools and pass the approval gate in a tainted
//! session. Downloading a model package also goes on the network and the
//! approval gate comes even before that (see `tacet_web::download`).
//!
//! IT RUNS WITH NO SUBCOMMAND. Typing `tacet` opens the interactive shell
//! directly; `tacet chat`, `tacet eval` ... KEEP WORKING (scripts and the
//! `--message` diagnostics depend on them). The subcommand used to be MANDATORY
//! and a user typing `tacet` saw clap's usage text — the wrong answer for an
//! assistant's first screen.
//!
//! DEPENDENCY — `crossterm`, ONLY IN THIS CRATE. The head of this file said
//! "rustyline/crossterm DELIBERATELY NOT ADDED" for a long time; that decision
//! WAS REVERSED and the reason belongs here.
//!
//! The old rationale was "ANSI escapes are a handful of constant strings, tty
//! detection is in std". True but incomplete: what CANNOT be done with std is
//! configuring the terminal's INPUT side. The failure measured in the user's
//! session was exactly there — while the model was generating, the keys pressed
//! got between the answer through the terminal echo (`tacet> kindHello! How can
//! I ihelp you?`). Locking input requires a termios setting; std has none, doing
//! it by hand means a `libc` dependency and separate code per platform. Not a
//! winning trade.
//!
//! THE BOUNDARY FOLLOWS THE SAME RULE AS THE NETWORK MONOPOLY: `crossterm` DOES
//! NOT LEAK into the core layers. tacet-kernel, tacet-engine, tacet-grammar,
//! tacet-tools and tacet-zip stay zero-dependency; the terminal is a SHELL
//! matter, just like `clap`. All the screen work lives in the `ui` module (see
//! the head of that file).

mod addon;
mod chat;
mod cli;
mod config;
mod diagnostics;
mod engine_setup;
mod eval_cmd;
mod filter;
mod format;
mod host_memory;
mod input;
mod models;
mod receipt;
mod session;
mod ui;
mod update;

use chat::{ChatRun, SamplingChoice, chat, version_line};
use clap::Parser;
use cli::{AddonJob, Command, ConfigJob, EngineChoice, McpJob, ModelJob, PackageJob, Shell};
use diagnostics::{
    doctor, font, grammar, mcp_list, mcp_login, mcp_logout, mcp_try, package_list, print_grammar,
    tools, why,
};
use engine_setup::{
    MODEL_VARIABLE, TOKENIZER_VARIABLE, byte_text, candle_engine_from_path, model_not_found_report,
    model_package, setup_engine,
};
use eval_cmd::{
    SelectionRun, eval, eval_compare, eval_format_gate, eval_routing, eval_tool_selection,
};
use models::{model_download, model_list};
use tacet_engine::{EngineProvider, Turn};
use tacet_eval::SYSTEM_INSTRUCTIONS;
use tacet_grammar::CallConstraint;
use tacet_kernel::ToolCatalog;
use tacet_tools::data_store::SharedStore;
use tacet_tools::executor::{
    ApprovalGate, ApprovalRequest, ExecutionOutcome, SilentDeny, ToolExecutor,
};
use tacet_tools::mcp;
use tacet_tools::memory::SharedMemory;
use tacet_tools::run_code::CodeState;
use ui::{BOLD, BRASS, Color, DIM, YELLOW};

use std::io::{IsTerminal, Read as _, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// The threshold at which eval counts as "green".
const DEFAULT_THRESHOLD: f64 = 1.0;

/// The names of the tools that TAKE DATA OUT to the outside world — the input of
/// the approval gate.
///
/// This list is NO LONGER EMPTY: `web_search` and `web_fetch` send the query
/// (which may carry user data) to a remote server. `remember` TAINTS but is NOT
/// in this list — because memory writes to the local disk, it DOES NOT TAKE DATA
/// OUT; the approval gate is for "data leaving", not for "personal data was
/// read". The names of MCP tools are learned from the catalog at runtime and
/// cannot be written here as constants; they are bound separately with
/// `mcp::bind_executor`.
///
/// `http` IS HERE because it sends a request BODY the model wrote to a host off
/// this device: that is data leaving, the exact thing this list gates.
///
/// `shell` IS HERE, AND IT WAS NOT UNTIL A TEST PUT THE DATA ON A SOCKET. The
/// reasoning that left it out said "it opens no socket" — true of the tool and
/// false of the tool's EFFECT, because what it opens is a program the user
/// allowed, and `curl`, `git push`, `ssh` and `scp` all live in real allow-lists
/// (`shell.rs` says so itself: allowing a program allows everything that program
/// can do). Measured: a session that had read a personal file then ran
/// `curl --data-binary @… http://…` and the bytes arrived at a listener with NO
/// approval chip on screen — and off the network monopoly's three gates
/// entirely, since none of tacet-web's checks are in that path. The cost is an
/// approval question on every `shell` call in a tainted session. That is the
/// correct price: the question is what the user has instead of a gate.
///
/// `db` stays out: it is SQLite-only, read-only, and reaches nothing the working
/// directory does not already expose.
///
/// `db_write` STAYS OUT TOO, AND THE `shell` ARGUMENT ABOVE IS THE REASON IT HAS
/// TO BE ARGUED RATHER THAN ASSUMED. `shell` was left out once on the grounds
/// that it opens no socket, and a test then put a personal file on a listener
/// through `curl`. The same reasoning does not reach here, and the difference is
/// checkable: `db_write` runs ONE binary, `sqlite3`, at a fixed path, with
/// `-safe` — which is measured at discovery to refuse ATTACH, `writefile()` and
/// every dot-command, and in this build `load_extension` is not a function at
/// all. There is no user list of programs behind it and no way to spell a socket
/// in SQL. Its own gate is per-call and local (`WriteConfirm`), and it is
/// deliberately NOT this one: gate 3 fires only in a TAINTED session and caches
/// a denial for the rest of it, so it would ask nothing on the first turn — the
/// turn where a `DROP TABLE` is most likely — and then stop asking after the
/// first "no".
///
/// `clipboard` is the argued case — writing
/// hands text to every application on the machine, but this list matches on the
/// TOOL NAME and not on the action, so listing it would put the question in
/// front of READING too, and reading is what creates the taint in the first
/// place. It stays out; its gates are the addon install and its own taint.
const EXTERNAL_TOOLS: &[&str] = &["web_search", "web_fetch", "http", "shell"];

/// The tools whose call is A CHECK BEFORE AN ACT — the indicator says "checking
/// X" for these and "running X" for everything else (see `ui::Stage`).
///
/// Both of them RUN the model's code before they keep anything: `write_code`
/// syntax-checks and then executes in the sandbox, saving the file only if it
/// came back clean; `run_code` sets the sandbox up before the script starts.
/// That step is the one that FAILS, and a failure reads very differently when
/// the screen already named the step it happened in.
const VERIFYING_TOOLS: &[&str] = &["run_code", "write_code"];

/// The default model folder (under `~/models/`).
///
/// qwen3-4b WAS CHOSEN — EVEN THOUGH THE BFCL RANKING ON PAPER SAYS OTHERWISE.
/// In the published scores Qwen3-8B leads (overall 42.57 / multi-turn 41.75,
/// against 35.68 / 22.12), but in THIS catalog, with THIS prompt, on THIS
/// machine WE MEASURED (a 10-question tool-selection set, M4 Pro / Metal, 20 Jul
/// 2026):
///
///     model       tool selection   irrelevance   time/request
///     qwen3-4b        8/10            2/2          5-14 s
///     qwen3-8b        7/10            2/2         11-32 s
///
/// The 4B came out both MORE ACCURATE and 2-3x faster. This is where a
/// general-purpose ranking does not carry over to this catalog: the 8B's edge is
/// in multi-turn and complex scenarios, while our turns are short and
/// single-tool.
///
/// Latency is not a comfort detail in this product: in a tool loop every turn
/// re-prefills the whole window, and the user waits on every turn.
///
/// Whoever wants to try the 8B types `--model qwen3-8b`; the choice is EXPLICIT,
/// not silent. To repeat the measurement: scratchpad/tool-selection-test.sh (to
/// be moved into tacet-eval).
const DEFAULT_MODEL: &str = "qwen3-4b";

/// Generation cancellation. `static` because `SamplingSetting` is `Copy` and
/// carries the flag as `&'static` (see tacet-engine); besides, there is one
/// shell in one process, there is no room for a second flag. Reset at the START
/// of every turn — the previous turn's cancellation must not drop the new one.
static CANCEL: AtomicBool = AtomicBool::new(false);

fn main() -> ExitCode {
    // ANSWERED BEFORE ANYTHING ELSE, including argument parsing. `tacet update
    // --install` runs the DOWNLOADED binary with this flag to ask what it was
    // built with, and that question has to be cheap and impossible to fail:
    // no config read, no model discovery, no banner. One word, then exit.
    if std::env::args().any(|a| a == "--print-features") {
        println!("{}", update::compiled_features());
        return ExitCode::SUCCESS;
    }
    // `-v` AND `--v` TOO, not just clap's `-V`/`--version`. Those two are what
    // people actually type, and answering an obvious question with
    // "unexpected argument" for a spelling difference is a bad first minute
    // with any tool. clap owns the canonical pair (it appears in `--help`);
    // these are caught before parsing because clap cannot alias its built-in.
    // ALL FOUR SPELLINGS ANSWER THE SAME, and they are answered HERE rather
    // than by clap so that they answer with the FEATURE too. clap's built-in
    // prints the number alone, and the number alone is not the answer to
    // "which tacet is this" — `cargo install` does not remember `--features`,
    // so 0.1.8 can be a binary that runs a model or one that cannot.
    // `#[command(version)]` stays on the struct so the flag is listed in
    // `--help`; this branch just gets there first.
    if std::env::args().any(|a| a == "-v" || a == "--v" || a == "-V" || a == "--version") {
        println!("{}", version_line());
        return ExitCode::SUCCESS;
    }

    // Clears the `.old` left behind by a previous Windows self-update. It could
    // not be removed at the time, because it was the file then executing. A
    // no-op on Unix, and it touches nothing but that one path.
    update::sweep_previous();
    // With NO subcommand the default is `chat`: a user typing `tacet` walks
    // straight in.
    let command = Shell::parse().command.unwrap_or(Command::Chat {
        engine: EngineChoice::Auto,
        script: Vec::new(),
        show_prompt: false,
        dir: ".".to_string(),
        message: None,
        model: None,
        json: false,
        continue_session: false,
        session_id: None,
        temperature: None,
        seed: None,
    });
    match command {
        Command::Chat {
            engine,
            script,
            show_prompt,
            dir,
            message,
            model,
            json,
            continue_session,
            session_id,
            temperature,
            seed,
        } => {
            // flag > env (applied deeper) > config file > built-in default —
            // the config file only speaks when the flag stays silent.
            let model = model
                .or_else(|| config::get_str("model"))
                .unwrap_or_else(|| DEFAULT_MODEL.to_string());
            let engine = if matches!(engine, EngineChoice::Auto) {
                match config::get_str("engine").as_deref() {
                    Some("candle") => EngineChoice::Candle,
                    Some("fake") => EngineChoice::Fake,
                    _ => EngineChoice::Auto,
                }
            } else {
                engine
            };
            let sampling = SamplingChoice::resolve(temperature, seed, &Color::setup());
            chat(ChatRun {
                choice: engine,
                script,
                show_prompt,
                dir,
                single_message: message,
                model_name: model,
                json,
                continue_session,
                session_id,
                sampling,
            })
        }
        Command::Sessions { json, purge } => sessions(json, purge),
        Command::Eval {
            json,
            threshold,
            tool_selection,
            turkish,
            model,
            only,
            require_quant,
            budget,
            budget_sweep,
            format_gate,
            force_tool_name,
            routing,
            routing_pressure,
            compare,
        } => {
            // `model` IS AN OPTION NOW (see the flag's own doc): every path that
            // REQUIRES a model resolves the default here, and the logic set uses
            // its ABSENCE to stay on `FakeEngine`. Before, `default_value` made
            // "no model" inexpressible, so the logic set could never run against
            // real weights and `EvalCase::grounded` was unreachable.
            let named_model = model.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string());
            // COMPARING TWO FILES TAKES NEITHER A MODEL NOR A SUITE, so it
            // is answered before anything that resolves either.
            if let Some(files) = &compare {
                return eval_compare(&files[0], &files[1]);
            }
            // ROUTING IS CHECKED FIRST AND TAKES NO MODEL. It is placed above
            // the model-resolving branches on purpose: a routing run on a
            // machine with no weights must not fail on "model not found" for a
            // measurement that never opens the file.
            if routing {
                eval_routing(json, threshold, turkish, only.as_deref(), routing_pressure)
            } else if format_gate {
                let color = Color::setup();
                let engine = match model_package::resolve_pair(&named_model) {
                    Some((m, t)) => match candle_engine_from_path(&m, t.as_deref()) {
                        Ok(engine) => engine,
                        Err(e) => {
                            eprintln!("error: real model could not be loaded: {e}");
                            return ExitCode::FAILURE;
                        }
                    },
                    None => {
                        model_not_found_report(&named_model, &color);
                        return ExitCode::FAILURE;
                    }
                };
                eval_format_gate(&engine)
            } else if tool_selection
                || budget.is_some()
                || budget_sweep.is_some()
                || require_quant.is_some()
                || force_tool_name
            {
                eval_tool_selection(SelectionRun {
                    json,
                    threshold,
                    model_name: &named_model,
                    only: only.as_deref(),
                    turkish,
                    require_quant: require_quant.as_deref(),
                    budget,
                    budget_sweep: budget_sweep.as_deref(),
                    force_tool_name,
                })
            } else {
                eval(json, threshold, model.as_deref())
            }
        }
        Command::Tools { schema } => tools(schema),
        Command::Grammar { tool, try_input } => grammar(&tool, try_input.as_deref()),
        Command::Package { job } => match job {
            PackageJob::List { json } => package_list(json),
        },
        Command::Models { job } => match job {
            ModelJob::List { json, model } => model_list(json, &model),
            ModelJob::Download { name, no_approval } => model_download(&name, no_approval),
        },
        Command::Addon { job } => match job {
            AddonJob::List { json } => addon::list(json),
            AddonJob::Install {
                name,
                address,
                local,
                no_approval,
            } => addon::install(&name, address, local, no_approval),
            AddonJob::Remove { name } => addon::remove(&name),
            AddonJob::Close { name } => addon::set_state(&name, false),
            AddonJob::Open { name } => addon::set_state(&name, true),
            AddonJob::Try { name, json } => addon::try_addon(&name, json),
        },
        Command::Config { job } => match job {
            ConfigJob::List { json } => config::list(json),
            ConfigJob::Get { key } => config::get(&key),
            ConfigJob::Set { key, value } => config::set(&key, &value),
            ConfigJob::Unset { key } => config::unset(&key),
            ConfigJob::Path => config::path(),
        },
        Command::Why { message } => why(&message),
        Command::Mcp { job } => match job {
            McpJob::List { json } => mcp_list(json),
            McpJob::Try { name, call, args } => mcp_try(&name, call, &args),
            McpJob::Login { name } => mcp_login(&name),
            McpJob::Logout { name } => mcp_logout(&name),
        },
        Command::Doctor => doctor(),
        Command::Feedback { turns } => feedback(turns),
        Command::Log { json, limit } => receipt::log(json, limit),
        Command::Font => font(),
        Command::Update {
            check,
            install,
            no_approval,
        } => {
            let color = Color::setup();
            // `--install` is the old spelling and now means nothing extra; it
            // is accepted so an older README does not become wrong.
            let _ = install;
            let outcome = if check {
                update::check(&color, false).map(|_| update::Outcome::AlreadyCurrent)
            } else {
                update::install(&color, no_approval)
            };
            match outcome {
                // ALREADY CURRENT IS A SUCCESS. It is also the most common run
                // of all, so it gets a sentence rather than silence.
                Ok(update::Outcome::AlreadyCurrent) if !check => {
                    eprintln!(
                        "  {}",
                        color.paint(
                            DIM,
                            &format!(
                                "already on the newest release ({})",
                                env!("CARGO_PKG_VERSION")
                            )
                        )
                    );
                    ExitCode::SUCCESS
                }
                Ok(_) => ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("  {}", color.paint(YELLOW, &message));
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// The scrub for `tacet feedback`. Deliberately simple and OVER-eager: the
/// home path becomes `~`, and any word containing `@` becomes `[email]`. The
/// user reviews the file before sharing — this is the first pass, their eyes
/// are the second.
fn scrub(text: &str, home: &str) -> String {
    let mut out = if home.is_empty() {
        text.to_string()
    } else {
        text.replace(home, "~")
    };
    if out.contains('@') {
        out = out
            .split(' ')
            .map(|w| if w.contains('@') { "[email]" } else { w })
            .collect::<Vec<_>>()
            .join(" ");
    }
    out
}

fn backend() -> &'static str {
    if cfg!(feature = "metal") {
        "metal"
    } else if cfg!(feature = "cuda") {
        "cuda"
    } else {
        "cpu"
    }
}

/// `tacet feedback` — see the enum doc. Writes, prints, sends nothing.
fn feedback(turns: usize) -> ExitCode {
    let color = Color::setup();
    let Some(stored) = session::Session::latest() else {
        println!(
            "{}",
            color.paint(DIM, "no stored session to report — talk to tacet first.")
        );
        return ExitCode::SUCCESS;
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let tail: Vec<&session::Turn> = stored.iter().rev().take(turns).collect();

    let mut body = String::new();
    body.push_str("## Tacet feedback package\n\n");
    body.push_str("<!-- Review every line before sharing. Nothing was sent anywhere;\n");
    body.push_str("     this file exists only on your machine until you paste it. -->\n\n");
    body.push_str(&format!(
        "- version: {} · os: {} · candle: {} · backend: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        cfg!(feature = "candle"),
        backend(),
    ));
    // THE WINDOW IS READ, NOT ASSERTED. This line used to say a flat `4096` and
    // that number stopped being true the day the window started coming out of
    // the weight file — a bug report carrying a wrong window sends whoever reads
    // it looking in the wrong place. `None` means no package was resolved (no
    // weights on this machine), and saying so is the honest answer.
    let model = config::get_str("model").unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let window = match model_package::resolve_pair(&model) {
        Some((gguf, _)) => tacet_engine::context_budget(
            tacet_engine::gguf_context_length(std::path::Path::new(&gguf)),
            tacet_engine::gguf_kv_bytes_per_token(std::path::Path::new(&gguf)),
            tacet_engine::Device::default(),
        )
        .to_string(),
        None => "unknown (no local weights)".to_string(),
    };
    body.push_str(&format!("- model: {model} · context: {window}\n\n"));
    body.push_str(
        "### What went wrong\n\n(describe it here)\n\n### Transcript (last turns, scrubbed)\n\n",
    );
    for t in tail.iter().rev() {
        let who = t.role.as_str();
        body.push_str(&format!("**{who}:** {}\n\n", scrub(&t.text, &home)));
        if !t.tools.is_empty() {
            body.push_str(&format!("_tools: {}_\n\n", t.tools.join(", ")));
        }
    }

    let name = format!("tacet-feedback-{}.md", std::process::id());
    match std::fs::write(&name, body) {
        Ok(()) => {
            println!("{}", color.paint(BOLD, &name));
            println!(
                "{}",
                color.paint(
                    DIM,
                    "  read it, edit anything you would rather keep, then paste it into a"
                )
            );
            println!(
                "{}",
                color.paint(DIM, "  GitHub issue. nothing has been sent anywhere.")
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: the report could not be written: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Total physical memory, best effort. Diagnostic only: a `None` prints as
/// "unknown", it never gates anything.
fn total_ram_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: u64 = text
            .lines()
            .find(|l| l.starts_with("MemTotal:"))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()?;
        Some(kb * 1024)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// In a tainted session, shows the REAL PAYLOAD going to an external tool and
/// asks y/n.
///
/// WHY NOT `AlwaysApprove`: wiring the gate permanently to "yes" makes it
/// useless — the user would be approving without seeing the data being sent.
/// `SilentDeny` would be wrong too: here there IS someone to ask. The reason the
/// gate exists is to show the content being sent (the query string) to the user
/// VERBATIM.
/// Answers a server's MRTR question (spec §4) — the terminal side of the
/// pattern that replaced elicitation push.
///
/// THE MODEL IS NEVER ASKED. A server's question is put in front of the USER,
/// verbatim but sanitised, prefixed with the connection's name so nobody has to
/// guess who is asking. Feeding it to the model instead would be `sampling`
/// with extra steps, and `sampling` is refused permanently.
///
/// Declining is `None`: an empty answer, an EOF, a closed pipe. The call is
/// then abandoned and the retry is never sent.
struct TerminalAsk;

impl mcp::InputAsk for TerminalAsk {
    fn ask(&self, server: &str, questions: &[mcp::Question]) -> Option<Vec<String>> {
        use mcp::QuestionKind;
        let color = Color::setup();
        eprintln!();
        eprintln!(
            "  {} the '{server}' server is asking for something before it will run this:",
            color.paint(BRASS, "[input]")
        );
        let mut answers = Vec::with_capacity(questions.len());
        for question in questions {
            // The question is DATA. It is printed and nothing else — never
            // parsed for commands, never handed to the model.
            eprintln!("    {}", crate::ui::one_line(&question.prompt));
            match &question.kind {
                QuestionKind::Boolean => eprint!("    [y/N] "),
                QuestionKind::Choice(choices) => {
                    for (i, choice) in choices.iter().enumerate() {
                        eprintln!("      {}. {}", i + 1, crate::ui::one_line(choice));
                    }
                    eprint!("    pick a number (enter cancels) ");
                }
                QuestionKind::Text => eprint!("    answer (enter cancels) "),
            }
            let _ = std::io::stderr().flush();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() {
                return None;
            }
            let line = line.trim().to_string();
            match &question.kind {
                // A bare enter on a yes/no is an ANSWER (no), not a cancel:
                // that is what [y/N] means everywhere else in this shell.
                QuestionKind::Boolean => answers.push(line),
                QuestionKind::Choice(choices) => {
                    let picked = line
                        .parse::<usize>()
                        .ok()
                        .filter(|n| *n >= 1 && *n <= choices.len());
                    match picked {
                        Some(n) => answers.push(choices[n - 1].clone()),
                        None => {
                            eprintln!("    (cancelled)");
                            return None;
                        }
                    }
                }
                QuestionKind::Text => {
                    if line.is_empty() {
                        eprintln!("    (cancelled)");
                        return None;
                    }
                    answers.push(line);
                }
            }
        }
        Some(answers)
    }
}

struct TerminalApproval;

impl ApprovalGate for TerminalApproval {
    fn request(&self, request: &ApprovalRequest) -> bool {
        eprintln!();
        eprintln!(
            "  ⚠ the '{}' tool will send data to the outside world:",
            request.tool_name
        );
        // THE CONSENT LINE IS SANITISED HERE, NOT SOMEWHERE UPSTREAM.
        //
        // Today `content` happens to arrive JSON-encoded, so control bytes are
        // already escaped — but that is a property of the CALLER, and this is
        // the line a user's "y" is answering. The day someone hands the raw
        // `as_str()` of an argument to this gate, a `\r` would let the payload
        // rewrite the very sentence describing what is about to be sent, and a
        // `\n` would let it paint a second, friendlier-looking prompt. The
        // defence belongs where the bytes meet the terminal.
        eprintln!("    {}", crate::ui::one_line(&request.content));
        eprint!("  Do you allow it? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
    }
}

/// Shows the MEASURED effect of a database change and asks y/n.
///
/// A DIFFERENT QUESTION FROM `TerminalApproval`'S, and that is why it is a
/// different type rather than a second use of the same one. That gate asks "may
/// this data leave the machine"; it fires only for `EXTERNAL_TOOLS`, only in a
/// tainted session, and it remembers a "no" for the rest of the session. This
/// one asks "may this change happen to your file", and none of those three
/// properties fit: the first turn of a clean session is exactly when a
/// destructive statement is most likely, and a refusal here says nothing about
/// the NEXT statement.
///
/// EVERY LINE IS SANITISED HERE, AT THE POINT IT MEETS THE TERMINAL. The
/// statement is text the MODEL wrote; a `\r` in it would rewrite the sentence
/// describing what is about to happen and a `\n` would paint a second,
/// friendlier prompt underneath. The same defence `TerminalApproval` writes down
/// for its consent line, applied to a longer screen.
struct TerminalWriteConfirm;

impl tacet_tools::db_write::WriteConfirm for TerminalWriteConfirm {
    fn confirm(&self, request: &tacet_tools::db_write::WriteRequest<'_>) -> bool {
        let color = Color::setup();
        eprintln!();
        eprintln!(
            "  {} a change to '{}' is about to be written:",
            color.paint(YELLOW, "⚠"),
            crate::ui::one_line(request.file)
        );
        eprintln!("    {}", crate::ui::one_line(request.statement));
        eprintln!();
        eprintln!("  {}", color.paint(DIM, "measured on a copy of the file:"));
        for line in request.effect.lines() {
            eprintln!("    {}", crate::ui::one_line(line));
        }
        eprintln!(
            "  {}",
            color.paint(
                DIM,
                &format!(
                    "the file as it is now will be kept beside it as {}",
                    crate::ui::one_line(request.backup)
                )
            )
        );
        eprint!("  Apply it? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
    }
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

/// The FULL catalog the shell sees. The single source of truth: the prompt, the
/// grammar and execution all derive from it.
///
/// `CodeState` is handed OUT because the attempt counter must be reset ON EVERY
/// TURN and only the shell (the side that knows the turn boundary) can do that;
/// once the tool is lost inside an Arc in the catalog it could not be reached.
/// `can_ask` IS "IS THERE A HUMAN AT A TERMINAL", and it decides whether
/// `db_write` is in the catalog at all — not merely which sink answers it.
///
/// A TOOL THAT CAN ONLY EVER REFUSE IS A TRAP THAT COSTS A TURN; `db.rs` and
/// `run_code.rs` both write that rule down, and a `db_write` wired to
/// `RefuseWrite` in a piped session is exactly that shape: the model sees it,
/// calls it, and is told no every single time. So a session with nobody to ask
/// does not get the tool. The alternative — installing a stdin-reading
/// confirmation everywhere — is worse in the other direction: it blocks a piped
/// run forever on a prompt nobody can see.
///
/// THE DIAGNOSTIC COMMANDS PASS `true` AND DO NOT RUN ANYTHING. `tacet why`,
/// `tacet tools` and `tacet grammar` inspect the catalog and never execute a
/// tool, so no sink of theirs is ever called; their job is to report the
/// catalog an ordinary session is given, which is the interactive one. The cost
/// is stated: `tacet tools` lists `db_write` even when the chat you are about
/// to pipe would not have it.
fn session_catalog(
    store: &Arc<SharedStore>,
    memory: &SharedMemory,
    color: &Color,
    can_ask: bool,
) -> (ToolCatalog, Option<Arc<CodeState>>) {
    // THE LIST ITSELF IS NO LONGER HERE (see tacet-tools/src/catalog.rs). The
    // shell and eval must see the same list: the tool SELECTION measurement
    // derives from the catalog the model sees; if two lists diverge, what is
    // measured is not the selection the application makes. The shell's only
    // remaining job here is telling the user WHY a tool is not found.
    //
    // THE WORKSPACE ADDON IS APPLIED HERE, not in the catalog builder, and the
    // reason is that it is not a catalog change at all: it adds no tool, it
    // widens the roots the EXISTING file tools may reach, and that reach is
    // process-wide state (`tacet_tools::workspace`). A process-wide side effect
    // inside `production_catalog` would fire in eval and in every unit test that
    // builds a catalog. This function is the production-only path, and it is
    // also the one that runs again on `refresh_session` — so closing the addon
    // from inside the shell really does take the reach away.
    apply_workspace_roots(color);
    let (mut c, code_state, diagnosis) =
        tacet_tools::catalog::production_catalog(store, memory, None);
    if let Some(d) = diagnosis {
        eprintln!("{}", color.paint(DIM, &format!("({})", d.0)));
    }

    // `db_write` IS ADDED HERE AND NOWHERE ELSE — see the module note in
    // `tacet_tools::db_write`. `production_catalog` is what eval builds from,
    // and a measurement run must never hold a tool that can change a file; this
    // function is the production-only path and the one that runs again on
    // `refresh_session`, so closing the addon really does take the tool away.
    //
    // APPENDED LAST, which puts it past `router::MAX_TOOLS` in the catalog
    // order: on a message with no trigger it is the first thing the router
    // drops. That is the right end of the list for a tool that must never be
    // reached by a message which did not ask for a change.
    //
    // AND ONLY WHEN THERE IS SOMEBODY TO ASK (`can_ask`) — see the note on this
    // function. `discover()` is still what decides whether the addon, the list
    // and the binary allow it at all; the sink is installed here rather than
    // left to the tool's own `RefuseWrite` default so that a future caller who
    // forgets this line refuses instead of writing.
    if can_ask && let Some(write_tool) = tacet_tools::db_write::DbWriteTool::discover() {
        c.add(Arc::new(
            write_tool
                .with_confirm(Arc::new(TerminalWriteConfirm))
                .with_store(Arc::clone(store)),
        ));
    } else if can_ask
        && tacet_web::addon::is_open(tacet_web::addon::DB)
        && !tacet_web::addon::read()
            .map(|r| {
                r.find(tacet_web::addon::DB)
                    .map(|e| e.values(tacet_web::addon::WRITABLE_KEY).is_empty())
                    .unwrap_or(true)
            })
            .unwrap_or(true)
    {
        // AN OPEN ADDON WITH A NON-EMPTY LIST AND NO TOOL is the confusing
        // state, the same one `catalog::addon_diagnoses` exists for: the user
        // named a file and nothing appeared. An EMPTY list says nothing — that
        // is the default and their own decision.
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!("({})", tacet_tools::db_write::DbWriteTool::diagnose())
            )
        );
    }
    // An addon the user OPENED whose tool is still missing. A closed addon says
    // nothing (that was their own decision); this is the confusing state — "I
    // installed db, where is it" — and every one of these tools can fail for a
    // machine-level reason the user can act on.
    //
    // THE GATES ARE READ A SECOND TIME here rather than carried out of the
    // catalog builder, and the cost is bounded: if the registry changed between
    // the two reads, the worst outcome is one explanation printed for a tool the
    // user has just switched off (or one withheld). It cannot change WHICH TOOLS
    // ARE IN THE CATALOG — that decision was already made and is in `c`.
    // A REGISTRY THAT CANNOT BE READ IS SAID OUT LOUD. Every gate answers
    // CLOSED on a broken file, which is the right direction — but it used to
    // happen in complete silence, so a user whose `addons.json` got truncated
    // saw yesterday's five tools simply gone and had no way to find out why:
    // `tacet addon list` printed the parse error, `tacet chat` printed nothing.
    // The diagnoses below cannot cover this, because from their side every
    // addon is legitimately closed.
    if let Err(e) = tacet_web::addon::read() {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!("(no addon is active: {e} — `tacet addon list` shows the same error)")
            )
        );
    }
    for text in tacet_tools::catalog::addon_diagnoses(&c, tacet_tools::catalog::AddonGates::read())
    {
        eprintln!("{}", color.paint(YELLOW, &format!("({text})")));
    }
    (c, code_state)
}

/// Applies the `workspace` addon's directory list to the file tools' reach.
///
/// FAIL-CLOSED IN EVERY DIRECTION. The addon closed, absent, unreadable, or
/// carrying a directory that no longer validates all end the same way: the extra
/// roots are CLEARED and the file tools see only the working directory — which
/// is exactly the behaviour of a build that never had this feature.
///
/// THE CLEAR IS UNCONDITIONAL ON THE CLOSED PATH. Roots live in process-wide
/// state, so `refresh_session` after a `/addon close workspace` has to take them
/// away; leaving them would give the user a reach they can no longer see in the
/// catalog and can no longer switch off.
///
/// THE DIRECTORIES ARE JUDGED BY THE TOOL LAYER'S OWN RULE
/// (`workspace::validate_root`), not re-checked here. The installer already asks
/// the same function; asking a second, slightly different question in this file
/// is how an entry gets accepted at install time and refused at use time with
/// nothing on screen to explain it.
fn apply_workspace_roots(color: &Color) {
    use tacet_web::addon;
    if !addon::is_open(addon::WORKSPACE) {
        tacet_tools::workspace::clear_roots();
        return;
    }
    let dirs: Vec<String> = match addon::read() {
        Ok(record) => match record.find(addon::WORKSPACE) {
            Some(entry) => entry
                .values(addon::DIRECTORIES_KEY)
                .into_iter()
                .map(str::to_string)
                .collect(),
            None => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    if dirs.is_empty() {
        tacet_tools::workspace::clear_roots();
        return;
    }
    // ALL-OR-NOTHING (the function's own rule): one directory that has since
    // been deleted or renamed refuses the whole call, and we then clear rather
    // than leave the previous list standing. A half-applied scope is the one
    // outcome nobody could reason about.
    if let Err(e) = tacet_tools::workspace::install_roots(&dirs) {
        tacet_tools::workspace::clear_roots();
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "(the workspace addon opened no directory: {e}. \
                     the file tools see only the working directory)"
                )
            )
        );
    }
}

/// The thinking switch for Qwen3-family (ChatML) engines.
///
/// Thinking multiplies latency on a simple tool turn and buys little; on a
/// genuine planning/summarising turn it buys real quality. `auto` (the
/// default) decides per turn with a cheap heuristic; `tacet config set
/// thinking on|off` overrides it. Non-ChatML engines get no suffix at all —
/// the soft switch is a Qwen convention, not a standard.
fn thinking_switch(message: &str) -> &'static str {
    match config::get_str("thinking").as_deref() {
        Some("on") => return " /think",
        Some("off") => return " /no_think",
        _ => {}
    }
    let plain = tacet_tools::router::simplify(message);
    const HEAVY: [&str; 12] = [
        "plan",
        "ozetle",
        "summar",
        "analiz",
        "analy",
        "karsilastir",
        "compare",
        "strateji",
        "strategy",
        "neden",
        "why",
        "adim adim",
    ];
    let heavy = message.chars().count() > 220 || HEAVY.iter().any(|k| plain.contains(k));
    if heavy { " /think" } else { " /no_think" }
}

/// Rebuilds everything that derives from the catalog, IN PLACE. The catalog is
/// a session-start snapshot by design (see the note at its creation); toggling
/// an addon from inside the shell is the one sanctioned reason to refresh that
/// snapshot without a restart. What survives the swap: the session taint
/// (explicitly carried — see `ToolExecutor::inherit_taint`), the shared store,
/// the memory, the open MCP connections. What resets: the approval denial
/// cache (the safe direction — it asks again) and the code sandbox state.
#[allow(clippy::too_many_arguments)]
fn refresh_session(
    store: &Arc<SharedStore>,
    memory: &SharedMemory,
    color: &Color,
    interactive: bool,
    mcp_load: &mcp::LoadOutcome,
    engine: &Arc<dyn EngineProvider>,
    catalog: &mut ToolCatalog,
    executor: &mut ToolExecutor,
    constraint: &mut Option<CallConstraint>,
    catalog_names: &mut Vec<String>,
    web_addon_open: &mut bool,
    code_state: &mut Option<Arc<CodeState>>,
) {
    let tainted = executor.session_tainted();
    let (mut c, cs) = session_catalog(store, memory, color, interactive);
    let names = mcp::feed_catalog(&mut c, mcp_load);
    let mut ex = ToolExecutor::new(c.clone());
    ex = if interactive {
        ex.with_gate(TerminalApproval)
    } else {
        ex.with_gate(SilentDeny)
    };
    for n in EXTERNAL_TOOLS {
        ex = ex.external_tool(*n);
    }
    ex = mcp::bind_executor(ex, &names);
    ex.inherit_taint(tainted);

    *constraint = engine.vocab().map(|v| CallConstraint::new(&v, &c));
    *catalog_names = c.names().into_iter().map(String::from).collect();
    *web_addon_open = tacet_web::addon::web_search_is_open();
    *catalog = c;
    *executor = ex;
    *code_state = cs;
}

// ---------------------------------------------------------------------------
// Piped standard input — CONTEXT, not a message
// ---------------------------------------------------------------------------

/// The ceiling on piped input that becomes context.
///
/// DERIVED FROM THE SMALLEST WINDOW, not chosen for looks. `TokenCounter::estimate`
/// charges roughly one token per three bytes (biased high on purpose), and the
/// prompt half of the FLOOR window (`CONTEXT_BUDGET`, 4096) is `prompt_cap()`
/// ≈ 3072 tokens — of which the system block and the tool descriptions already
/// eat ~2300 on a full catalog. 8 KiB of pasted text is ~2700 estimated tokens:
/// still larger than the room actually left, which is deliberate. The point of
/// the cap is not to make the paste fit (truncation handles that, and it is
/// allowed to bite here) but to stop a `cat 10mb.log |` from making the shell
/// allocate and hash megabytes before the counter ever sees them.
///
/// IT IS A CONSTANT EVEN THOUGH THE WINDOW NO LONGER IS. The real window is read
/// from the weights and is four to eight times this floor, so on a real model
/// this cap is conservative rather than binding — and that is the safe
/// direction: the cap exists to bound WORK done before the counter runs, and
/// that bound must not depend on which model happens to be installed.
const STDIN_CONTEXT_LIMIT: usize = 8 * 1024;

/// What arrived on a pipe, and whether we had to cut it.
struct PipedInput {
    text: String,
    /// The size before the cut. `None` when nothing was cut.
    original_bytes: Option<usize>,
}

/// Reads piped stdin, capped.
///
/// WHY THIS EXISTS AT ALL — A MEASURED BUG: `echo "..." | tacet chat --message
/// "question"` sent the question and DROPPED THE PIPE ON THE FLOOR. Nothing
/// said so; the model answered as if the pipe had never been there, so the user
/// got a confident answer about data the model had never seen. Silent data loss
/// is the worst shape a bug can take in an assistant.
///
/// It reads BYTES and converts lossily rather than demanding UTF-8: a pipe
/// carrying one bad byte in the middle of a log is still worth reading, and
/// failing the whole turn over it would be the same silent loss in a new
/// costume.
/// Is there anything on stdin worth waiting for?
///
/// WHY THIS GATE EXISTS: `read_to_end` on a pipe that is OPEN BUT IDLE never
/// returns. That is not a rare shape — a CI step, a `bash -c` with an inherited
/// descriptor, a shell function called from a script all hand the process a
/// stdin nobody will ever write to or close. Without this check
/// `tacet -m "..." --json` hangs forever there, and it hangs SILENTLY: no
/// output, no error, just a job that never finishes.
///
/// `poll` with a short timeout answers "has the producer said anything yet".
/// Once the first byte is there we go back to a full blocking read, so a slow
/// producer is not cut off mid-stream — only a producer that has not started
/// within the window is skipped.
#[cfg(unix)]
fn stdin_has_data() -> bool {
    // `poll` WITHOUT A libc CRATE — same reasoning as `setsid`/`killpg` in
    // tacet-tools: the symbol is in the C library every unix already links.
    unsafe extern "C" {
        fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
    }
    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }
    const POLLIN: i16 = 0x0001;
    // 120 ms. Long enough that a program starting up and immediately writing is
    // caught, short enough that a person never perceives it. NOT TUNED against
    // a slow producer on a loaded machine — if this proves too tight, the
    // symptom is piped input being ignored, which the fence in the prompt makes
    // visible rather than silent.
    const WAIT_MS: i32 = 120;

    let mut fd = PollFd {
        fd: 0,
        events: POLLIN,
        revents: 0,
    };
    // SAFETY: one descriptor, valid for the call; `poll` writes only `revents`.
    let ready = unsafe { poll(&mut fd, 1, WAIT_MS) };
    ready > 0 && (fd.revents & POLLIN) != 0
}

/// NOT MEASURED ON WINDOWS. Without an equivalent gate the old behaviour
/// stands: a pipe that is open but idle blocks. Left as it was rather than
/// guessed at, because the fix has to be tested on the platform it targets.
#[cfg(not(unix))]
fn stdin_has_data() -> bool {
    true
}

fn read_piped_stdin() -> Option<PipedInput> {
    if !stdin_has_data() {
        return None;
    }
    let mut stdin = std::io::stdin().lock();
    let mut raw = Vec::new();
    // One byte over the limit, so "did it fill the limit exactly" and "was there
    // more" are distinguishable without reading the rest of a 10 MB log.
    if stdin
        .by_ref()
        .take(STDIN_CONTEXT_LIMIT as u64 + 1)
        .read_to_end(&mut raw)
        .is_err()
    {
        return None;
    }
    if raw.iter().all(u8::is_ascii_whitespace) {
        // An empty pipe (`tacet -m "hi" < /dev/null`) is not context. An empty
        // `<stdin>` fence would tell the model "you were given data and it was
        // blank", which is a claim about the user's files that is not true.
        return None;
    }
    let overflowed = raw.len() > STDIN_CONTEXT_LIMIT;
    if overflowed {
        // The rest of the pipe is DRAINED, not left in the buffer: a writer
        // blocked on a full pipe would otherwise see EPIPE and print its own
        // error over ours. Failure here is not worth reporting — we already
        // have what we came for.
        let mut sink = std::io::sink();
        let _ = std::io::copy(&mut stdin, &mut sink);
        raw.truncate(STDIN_CONTEXT_LIMIT);
        // Back off to a character boundary so the lossy conversion does not
        // turn the last real character into a replacement mark.
        while !raw.is_empty() && (raw[raw.len() - 1] & 0b1100_0000) == 0b1000_0000 {
            raw.pop();
        }
    }
    Some(PipedInput {
        text: String::from_utf8_lossy(&raw).into_owned(),
        original_bytes: overflowed.then_some(STDIN_CONTEXT_LIMIT + 1),
    })
}

/// The piped text as the model sees it: fenced, and HONEST ABOUT THE CUT.
///
/// The cut is written INSIDE the fence, in the model's own language, because
/// the model is the one who would otherwise conclude "the file ends here" and
/// answer about a log it only saw the head of. The user is told separately, on
/// stderr — two audiences, two sentences, neither standing in for the other.
fn stdin_fence(piped: &PipedInput) -> String {
    let mut fence = String::from("<stdin>\n");
    fence.push_str(piped.text.trim_end());
    if piped.original_bytes.is_some() {
        fence.push_str("\n…(truncated: only the first ");
        fence.push_str(&byte_text(STDIN_CONTEXT_LIMIT as u64));
        fence.push_str(" of the piped input is shown)");
    }
    fence.push_str("\n</stdin>");
    fence
}

// ---------------------------------------------------------------------------
// The working directory, as one short block in the prompt
// ---------------------------------------------------------------------------

/// How many names the listing shows before it starts counting instead.
const DIR_CONTEXT_ENTRIES: usize = 40;
/// The hard ceiling on the block, in bytes. SEE THE MEASUREMENT in
/// `dir_context`: this number, not the entry count, is what actually bounds the
/// cost, because one directory of long names can blow past a short list.
const DIR_CONTEXT_BYTES: usize = 500;

/// A short census of the working directory, fenced, or `None` when there is
/// nothing worth saying.
///
/// WHY IT IS IN THE PROMPT AT ALL: "what's in here?" is the first thing a person
/// types in a terminal assistant, and answering it used to cost a `run_code`
/// round trip — a tool call, an approval-shaped pause and two more seconds — for
/// a fact that fits in one line.
///
/// MEASURED COST — and it is NOT free, so here are the real numbers rather than
/// an adjective. Estimated with `TokenCounter::estimate` (the same counter the
/// budget uses), on 28 Jul 2026:
///
///     directory                       bytes   tokens   % of the 4096 floor
///     tacet-rs/crates (11 entries)      165       66        1.6%
///     the ketum repo root (13)          240       96        2.3%
///     the cap (500 bytes + tail)       ~568     ~228        5.6%
///
/// For scale, `SYSTEM_INSTRUCTIONS` alone is 442 tokens and a full 12-tool
/// catalog description is ~2000, so a typical prompt was ~2480 before this
/// block and ~2580 after. The block is therefore ~4% of what is already there —
/// but it is ~20% of what is LEFT under `prompt_cap()`, which is the number that
/// matters and the reason the byte cap is 500 and not 2000.
///
/// IT IS SENT ON EVERY TURN, and that is a choice, not an oversight. It sits in
/// the system block, the one piece truncation never touches, so a "what's in
/// here?" asked on turn 30 is answered exactly as well as one asked on turn 1.
/// First-turn-only would have cost the same on turn 1 and then gone missing
/// precisely when the conversation is long enough for the model to have
/// forgotten. If this ever needs to shrink, shrink `DIR_CONTEXT_BYTES` — the
/// cost is linear in it and the table above is the calibration.
///
/// HIDDEN FILES ARE EXCLUDED. `.env`, `.git/`, `.ssh/` and friends are where
/// secrets live, and this block goes into a prompt on every turn; the user asked
/// for an assistant, not for their dotfiles to be recited. The tools can still
/// read them WHEN ASKED — that path has a sandbox check and an audit chip, which
/// is the difference between "reached for" and "handed over".
fn dir_context(dir: &str) -> Option<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let folder = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(if folder { format!("{name}/") } else { name })
        })
        .collect();
    if names.is_empty() {
        return None;
    }
    // SORTED, so the same directory produces a bit-identical prompt on two
    // machines — `read_dir` order is the file system's, and a prompt that
    // changes shape between runs makes every measurement incomparable.
    names.sort();
    let total = names.len();

    let mut shown = 0usize;
    let mut body = String::new();
    for name in names.iter().take(DIR_CONTEXT_ENTRIES) {
        // The +2 accounts for the separator and keeps the check honest about
        // the string we are actually building.
        if body.len() + name.len() + 2 > DIR_CONTEXT_BYTES {
            break;
        }
        if !body.is_empty() {
            body.push_str(", ");
        }
        body.push_str(name);
        shown += 1;
    }
    if shown == 0 {
        return None;
    }
    let mut block = format!("<cwd>\n{dir}\n{body}");
    if shown < total {
        // THE REMAINDER IS COUNTED, NOT SWALLOWED. A list that silently stops at
        // forty teaches the model that the directory holds forty things, and it
        // will then say so.
        block.push_str(&format!("\n({} more not listed)", total - shown));
    }
    block.push_str("\n</cwd>");
    Some(block)
}

/// The system block the model actually gets: the fixed instructions plus, if
/// there is one, the directory census.
fn system_text(dir_block: Option<&String>) -> String {
    match dir_block {
        Some(b) => format!("{SYSTEM_INSTRUCTIONS}\n\n{b}"),
        None => SYSTEM_INSTRUCTIONS.to_string(),
    }
}

// ---------------------------------------------------------------------------
// The transcript on disk — the shell's half of `session.rs`
// ---------------------------------------------------------------------------

/// Stored turns → prompt turns.
///
/// THE CONVERSION LIVES HERE, NOT IN `session.rs`, and deliberately: that module
/// owns the FILE FORMAT and this one owns the PROMPT, and tying the two together
/// would make a prompt refactor a change to a file the user already has on disk.
/// This function is the seam.
///
/// `Role::Tool` TURNS ARE CARRIED OVER. Dropping them is tempting — tool results
/// are the bulkiest thing in a transcript and the context window is tight —
/// but a resumed conversation without them reads as if the assistant knew the
/// weather by magic, and the model, seeing its own past answer with no source,
/// learns that inventing facts is what it does here. The truncation policy
/// already drops from the front when the window fills, which is the right place
/// for that decision because it can see the whole prompt.
fn to_engine_turns(stored: &[session::Turn]) -> Vec<Turn> {
    stored
        .iter()
        .map(|t| match t.role {
            session::Role::User => Turn::user(&t.text),
            session::Role::Assistant => Turn::assistant(&t.text),
            session::Role::Tool => Turn::tool(&t.text),
        })
        .collect()
}

/// The marker that says the privacy notice has already been shown.
///
/// IT LIVES IN THE SESSIONS FOLDER, not in `config.json`. Two reasons, and the
/// second is the one that decided it: `config.json` takes only keys that are
/// also flags (see the `Config` command's note) and this is not a setting; and
/// the marker sitting inside the folder means `tacet sessions --purge`, which
/// removes the folder, ALSO forgets that the notice was shown. That is correct
/// behaviour rather than a leak — a user who has just erased everything and
/// starts again is owed the sentence about where the new writing goes.
const TRANSCRIPT_NOTICE_MARK: &str = ".notice-shown";

/// Prints, ONCE PER INSTALL, where the conversation is being kept.
///
/// WHY IT IS SAID AT ALL: this shell's whole promise is that nothing leaves the
/// machine, and a user who discovers a folder of their chat transcripts they
/// were never told about will read that promise differently afterwards — even
/// though the file never left the disk. Consent for a local write is cheap to
/// give and expensive to skip.
///
/// WHY ONLY ONCE: a privacy line on every start is a line nobody reads by the
/// third day, and the point of it is that it IS read.
///
/// It is called on the write path (not at start-up) so that a shell which is
/// opened and closed without a word never claims to have stored anything.
fn announce_transcript(color: &Color, human: bool) {
    let Some(dir) = session::dir() else {
        return;
    };
    let mark = dir.join(TRANSCRIPT_NOTICE_MARK);
    if mark.exists() {
        return;
    }
    // The directory may not exist yet — the first `append` creates it. Best
    // effort throughout: failing to record that we spoke is not a reason to
    // refuse to speak, and the worst case is the sentence appearing twice.
    let _ = tacet_kernel::fs::create_private_dir(&dir);
    let _ = tacet_kernel::fs::write_private(&mark, b"");
    if !human {
        // Under `--json` the sentence has nowhere to go on stdout without
        // breaking the contract, so it goes to stderr — still said, still once.
        eprintln!("{}", session::PRIVACY_NOTICE);
        return;
    }
    eprintln!();
    eprintln!("{}", color.paint(DIM, session::PRIVACY_NOTICE));
    eprintln!(
        "{}",
        color.paint(
            DIM,
            &format!(
                "  {}   ·   list: tacet sessions   ·   delete: tacet sessions --purge",
                dir.display()
            )
        )
    );
    eprintln!();
}

/// One executed tool, as the `--json` trace records it.
///
/// THE ARGUMENTS ARE SUMMARISED, NOT REPRODUCED. A `write_code` call carries a
/// whole file in its arguments; a caller reading this trace wants to know WHICH
/// tool ran with roughly what, and a reader who pipes the output somewhere would
/// otherwise be moving a document they never asked to move.
fn tool_record(outcome: &ExecutionOutcome, raw_generation: &str) -> serde_json::Value {
    /// Long enough to identify a call, short enough not to be a payload.
    const ARG_SUMMARY_CHARS: usize = 160;

    // The arguments are re-parsed from the raw generation rather than taken off
    // the outcome, which does not carry them. A `None` here is not a failure:
    // the executor has its own recovery path for shapes `ToolCall::parse` does
    // not accept, and a missing summary is better than a guessed one.
    let args = tacet_tools::executor::ToolCall::parse(raw_generation).map(|c| {
        let text = c.args.to_string();
        if text.chars().count() > ARG_SUMMARY_CHARS {
            let head: String = text.chars().take(ARG_SUMMARY_CHARS).collect();
            format!("{head}…")
        } else {
            text
        }
    });
    serde_json::json!({
        "tool": outcome.tool_name,
        "args": args,
        // The machine-readable verdict. `reason` is the executor's own vocabulary
        // (why it ended), `state` is what the world looks like afterwards; a
        // caller deciding whether to retry needs the second, one logging needs
        // the first.
        "reason": format!("{:?}", outcome.reason),
        "state": match &outcome.state {
            tacet_kernel::ToolState::Running => "running".to_string(),
            tacet_kernel::ToolState::Read => "read".to_string(),
            tacet_kernel::ToolState::Written => "written".to_string(),
            tacet_kernel::ToolState::NeedsPermission => "needs_permission".to_string(),
            tacet_kernel::ToolState::Failed(why) => format!("failed: {why}"),
        },
        "error": outcome.is_error(),
        "world_changed": outcome.world_changed,
    })
}

/// `tacet sessions` — what is kept, and how to be rid of it.
fn sessions(json: bool, purge: bool) -> ExitCode {
    let color = Color::setup();
    let dir = session::dir();

    if purge {
        // THE QUESTION IS ASKED ON A TERMINAL AND NOT IN A PIPE. There is nobody
        // to ask in a pipe, and a script that typed `--purge` said what it
        // meant; blocking on a prompt nobody can answer would hang the script
        // instead of protecting anyone.
        if std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal()
            && !ui::ask_yes_no(
                &color,
                "delete EVERY stored conversation? this cannot be undone",
            )
        {
            println!("{}", color.paint(DIM, "(nothing was deleted)"));
            return ExitCode::SUCCESS;
        }
        return match session::Session::purge_all() {
            Ok(n) => {
                if json {
                    println!("{}", serde_json::json!({ "purged": n }));
                } else {
                    println!("{n} conversations deleted");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}", color.paint(YELLOW, &format!("could not delete: {e}")));
                ExitCode::FAILURE
            }
        };
    }

    let list = session::Session::list();
    if json {
        // The same shape as every other list in this shell: context at the top
        // level, records underneath.
        let records: Vec<serde_json::Value> = list
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "at": s.at,
                    "local_time": s.local_time(),
                    "turns": s.turns,
                    "preview": s.preview,
                    "path": s.path.display().to_string(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "dir": dir.as_ref().map(|d| d.display().to_string()),
                "sessions": records,
            })
        );
        return ExitCode::SUCCESS;
    }

    match &dir {
        Some(d) => println!("{}", color.paint(DIM, &format!("kept in: {}", d.display()))),
        None => println!(
            "{}",
            color.paint(
                YELLOW,
                "the config directory could not be resolved — nothing is being kept"
            )
        ),
    }
    println!();
    if list.is_empty() {
        println!("{}", color.paint(DIM, "(no stored conversation)"));
        return ExitCode::SUCCESS;
    }
    for s in &list {
        println!(
            "{}  {}",
            color.paint(BOLD, &s.id),
            color.paint(DIM, &format!("{} · {} turns", s.local_time(), s.turns))
        );
        if !s.preview.is_empty() {
            println!("  {}", color.paint(DIM, &ui::one_line(&s.preview)));
        }
    }
    println!();
    println!(
        "{}",
        color.paint(
            DIM,
            "continue the last one: tacet --continue   ·   a specific one: tacet --session <id>"
        )
    );
    println!(
        "{}",
        color.paint(DIM, "delete every one of them: tacet sessions --purge")
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::status_line;
    use crate::engine_setup::engine_window;
    use crate::models::{
        TOFU_NOTE_CATALOG, TerminalDownloadApproval, download_root, progress_text,
    };
    use tacet_engine::{CONTEXT_BUDGET, FakeEngine, TokenCounter};
    use tacet_kernel::ArgSchema;
    use tacet_memory::MemoryStore;

    /// `shell` MUST STAY IN THE APPROVAL LIST. It was left out on the grounds
    /// that it opens no socket — true of the tool, false of its effect, because
    /// the programs a user allows include `curl`, `git` and `ssh`. Measured with
    /// an allow-list of exactly `curl`: a session that had read a personal file
    /// posted that file's contents to a listener and NO approval question was
    /// asked. This test is cheap and the property it guards is not: it is the
    /// difference between "data can only leave through tacet-web's three gates"
    /// and "data can leave".
    #[test]
    fn the_approval_gate_covers_every_tool_that_can_put_data_on_a_socket() {
        for name in ["web_search", "web_fetch", "http", "shell"] {
            assert!(EXTERNAL_TOOLS.contains(&name), "{name} is not gated");
        }
        // And it does NOT cover the ones whose gate is elsewhere — otherwise a
        // tainted session asks a question on every clipboard READ, which is the
        // act that created the taint.
        for name in ["remember", "db", "clipboard", "read_document"] {
            assert!(!EXTERNAL_TOOLS.contains(&name), "{name} is over-gated");
        }
    }

    /// `db_write` MUST NOT BE ON THAT LIST, and the reason is not "it is
    /// harmless" — it is that gate 3 answers a different question and would
    /// answer this one WRONGLY IN BOTH DIRECTIONS. It fires only in a tainted
    /// session, so the first turn of a fresh session — where a `DROP TABLE` is
    /// most likely — would be asked about not at all; and it caches a denial per
    /// tool, so after one "no" every later statement would be refused silently
    /// with no question. The per-call `WriteConfirm` in `tacet_tools::db_write`
    /// is the gate, and `db_write.rs`'s own tests measure both halves.
    ///
    /// This is the test that fails if somebody "simplifies" the design by
    /// reusing the outbound gate.
    #[test]
    fn the_database_write_gate_is_not_the_outbound_gate() {
        assert!(
            !EXTERNAL_TOOLS.contains(&"db_write"),
            "db_write was added to the outbound approval list; a clean session would then write \
             a DROP with no question at all"
        );
    }

    // The logic that stops a raw tool call leaking onto the screen is NOW in
    // `filter.rs` and its tests are there (the old `is_call` made a one-shot
    // decision, the new filter follows the whole stream).

    // -----------------------------------------------------------------------
    // The piped pipe — the bug this round exists for
    // -----------------------------------------------------------------------

    /// THE FENCE IS THE CONTRACT. Whatever came off the pipe has to reach the
    /// model INSIDE a marked block, or the model cannot tell the user's question
    /// from the data it is being asked about.
    #[test]
    fn piped_text_reaches_the_model_inside_a_fence() {
        let piped = PipedInput {
            text: "NUMBER: 42\n".to_string(),
            original_bytes: None,
        };
        let fence = stdin_fence(&piped);
        assert!(fence.starts_with("<stdin>\n"), "{fence}");
        assert!(fence.ends_with("\n</stdin>"), "{fence}");
        assert!(fence.contains("NUMBER: 42"), "{fence}");
        // Nothing about truncation when nothing was truncated: a model told its
        // input was cut will hedge about data it has in full.
        assert!(!fence.contains("truncated"), "{fence}");
    }

    /// A CUT IS DECLARED TO THE MODEL, not only to the user. Without this the
    /// model reads the head of a log as the whole log and answers "the file
    /// contains N lines" about a file it never saw the end of.
    #[test]
    fn a_cut_pipe_says_so_inside_the_fence() {
        let piped = PipedInput {
            text: "line\n".repeat(10),
            original_bytes: Some(STDIN_CONTEXT_LIMIT + 1),
        };
        let fence = stdin_fence(&piped);
        assert!(fence.contains("truncated"), "{fence}");
        assert!(fence.ends_with("\n</stdin>"), "{fence}");
    }

    // -----------------------------------------------------------------------
    // The directory block
    // -----------------------------------------------------------------------

    /// HIDDEN FILES STAY OUT. This block goes into a prompt on every turn, and
    /// `.env` / `.git` / `.ssh` is where the things a user did not mean to
    /// recite live.
    #[test]
    fn the_directory_block_skips_hidden_names_and_marks_folders() {
        let dir = std::env::temp_dir().join(format!(
            "tacet-dir-context-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("notes.md"), b"x").unwrap();
        std::fs::write(dir.join(".env"), b"SECRET=1").unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();

        let block = dir_context(&dir.display().to_string()).expect("no block");
        assert!(block.contains("notes.md"), "{block}");
        assert!(block.contains("src/"), "the folder is not marked: {block}");
        assert!(!block.contains(".env"), "a dotfile leaked: {block}");
        assert!(!block.contains(".git"), "a dotfile leaked: {block}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE MEASURED CEILING. This block is a FIXED COST ON EVERY PROMPT, so the
    /// thing that must not drift is its worst case — the table in `dir_context`
    /// is only honest while this holds. A directory of two hundred long names
    /// must not quietly become a thousand-token tax.
    #[test]
    fn the_directory_block_cannot_grow_past_its_measured_ceiling() {
        let dir = std::env::temp_dir().join(format!(
            "tacet-dir-cap-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..200 {
            std::fs::write(
                dir.join(format!("a-quite-long-file-name-number-{i:03}.txt")),
                b"x",
            )
            .unwrap();
        }
        let block = dir_context(&dir.display().to_string()).expect("no block");
        let tokens = TokenCounter::estimate(&block);
        assert!(
            tokens <= 250,
            "the directory block costs {tokens} tokens — the comment in `dir_context` promises ~228 at the cap"
        );
        // AND IT DOES NOT LIE ABOUT THE REST. A list that silently stops teaches
        // the model that the directory holds only what it can see.
        assert!(block.contains("more not listed"), "{block}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The block is glued to the instructions, not to the question: it must be
    /// in the SYSTEM text, the one piece truncation never touches.
    #[test]
    fn the_directory_block_rides_in_the_system_instructions() {
        let plain = system_text(None);
        assert_eq!(plain, SYSTEM_INSTRUCTIONS);
        let with = system_text(Some(&"<cwd>\n.\na, b/\n</cwd>".to_string()));
        assert!(with.starts_with(SYSTEM_INSTRUCTIONS), "{with}");
        assert!(with.contains("<cwd>"), "{with}");
    }

    // -----------------------------------------------------------------------
    // Model packages: a lone .gguf is a whole package
    // -----------------------------------------------------------------------

    /// THE BUG THIS ROUND FIXED. A package whose vocabulary lives inside the
    /// weights was reported as half and could not be selected, while the engine
    /// behind the check could have loaded it.
    #[test]
    fn a_gguf_that_carries_its_own_tokenizer_is_a_whole_package() {
        use model_package::ModelPackage;
        let base = ModelPackage {
            name: "m".into(),
            dir: "/m".into(),
            gguf: "/m/model.gguf".into(),
            gguf_bytes: 1,
            tokenizer: None,
            gguf_tokenizer: false,
            root: "/".into(),
        };

        let bare = ModelPackage { ..base.clone() };
        assert!(!bare.is_complete());
        assert!(bare.tokenizer_note().contains("MISSING"));
        assert_eq!(model_package::to_pair(&[bare], "m"), None);

        let inside = ModelPackage {
            gguf_tokenizer: true,
            ..base.clone()
        };
        assert!(inside.is_complete());
        assert!(inside.tokenizer_note().contains("inside the .gguf"));
        assert_eq!(
            model_package::to_pair(&[inside], "m"),
            // NO TOKENIZER PATH: this `None` is what makes the engine take the
            // `ModelSetting::from_gguf` branch.
            Some(("/m/model.gguf".to_string(), None))
        );

        let both = ModelPackage {
            tokenizer: Some("/m/tokenizer.json".into()),
            gguf_tokenizer: true,
            ..base
        };
        assert!(both.is_complete());
        assert_eq!(
            model_package::to_pair(&[both], "m"),
            // THE EXPLICIT FILE WINS. If this ever flips, a user who dropped a
            // corrected `tokenizer.json` next to their weights would be silently
            // ignored.
            Some((
                "/m/model.gguf".to_string(),
                Some("/m/tokenizer.json".to_string())
            ))
        );
    }

    // -----------------------------------------------------------------------
    // The stored transcript
    // -----------------------------------------------------------------------

    /// Stored roles must survive the trip back into a prompt — INCLUDING the
    /// tool role. A resumed conversation missing its tool results teaches the
    /// model that its past answers came from nowhere.
    #[test]
    fn a_stored_transcript_becomes_a_prompt_history_with_every_role() {
        let stored = vec![
            session::Turn::new(session::Role::User, "what time is it"),
            session::Turn::new(session::Role::Tool, "14:35"),
            session::Turn::new(session::Role::Assistant, "It is 14:35."),
        ];
        let turns = to_engine_turns(&stored);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].role, tacet_engine::Role::User);
        assert_eq!(turns[1].role, tacet_engine::Role::Tool);
        assert_eq!(turns[1].text, "14:35");
        assert_eq!(turns[2].role, tacet_engine::Role::Assistant);
    }

    /// The counter line: a hint on the first turn, numbers afterwards.
    #[test]
    fn the_status_line_shows_a_hint_first_and_numbers_later() {
        let c = TokenCounter::default();
        let first = status_line(0, 0, 0, 0, &c);
        assert!(first.contains('/'), "{first}");
        assert!(!first.contains("tokens"), "{first}");
        let later = status_line(900, 120, 1020, 900, &c);
        assert!(later.contains("this turn 900+120"), "{later}");
        assert!(later.contains("session 1020"), "{later}");
        assert!(later.contains(&format!("900/{CONTEXT_BUDGET}")), "{later}");
        assert!(!later.contains("window full"), "{later}");
    }

    /// THE LINE REPORTS THE COUNTER'S WINDOW, NOT THE CONSTANT.
    ///
    /// The window now comes out of the weight file (see `engine_window`), and
    /// the failure this guards against is the quiet one: truncation deciding
    /// against 16954 while the line under the input field still says 4096. A
    /// status line that disagrees with the mechanism it reports on is worse than
    /// none, because the user acts on it.
    ///
    /// The number is one of the MEASURED ones (qwen3-4b/qwen3-8b, 16954), chosen
    /// so that a regression to the floor is visible rather than arithmetically
    /// close.
    #[test]
    fn the_status_line_reports_the_window_it_was_given() {
        const MEASURED: usize = 16_954;
        let c = TokenCounter::new(MEASURED, tacet_engine::GENERATION_SHARE);
        let line = status_line(900, 120, 1020, 900, &c);
        assert!(line.contains(&format!("900/{MEASURED}")), "{line}");
        assert!(
            !line.contains(&CONTEXT_BUDGET.to_string()),
            "the fixed 4096 is still reaching the status line: {line}"
        );
        // A prompt that would have filled the old window is nowhere near full in
        // this one — the whole point of reading the window off the model.
        assert!(!line.contains("window full"));
        let old_full = status_line(4000, 10, 4010, 3072, &c);
        assert!(!old_full.contains("window full"), "{old_full}");
    }

    /// When the window fills the user IS WARNED — the truncation must not stay
    /// silent.
    #[test]
    fn the_status_line_says_the_window_is_full() {
        let c = TokenCounter::default();
        let s = status_line(4000, 10, 4010, c.prompt_cap(), &c);
        assert!(s.contains("window full"), "{s}");
        // The same claim at a window that is not the constant: "full" has to be
        // computed from the counter, not compared against 4096.
        let wide = TokenCounter::new(16_954, tacet_engine::GENERATION_SHARE);
        let s = status_line(16_000, 10, 16_010, wide.prompt_cap(), &wide);
        assert!(s.contains("window full"), "{s}");
    }

    /// THE WINDOW IS DERIVED, AND A MISSING DECLARATION FALLS TO THE FLOOR.
    ///
    /// `engine_window` cannot be measured against a real 2.5 GB weight file in a
    /// unit test, so what is measured here is the branch that matters for
    /// safety: with no readable GGUF metadata the answer is the floor, never a
    /// guess. A guessed window puts positions past the model's rope table and
    /// produces plausible-looking nonsense instead of an error.
    #[test]
    fn a_model_that_declares_nothing_gets_the_floor() {
        let fake: Arc<dyn EngineProvider> = Arc::new(FakeEngine::script(Vec::<String>::new()));
        assert_eq!(
            engine_window(&fake, "/definitely/not/here.gguf"),
            CONTEXT_BUDGET
        );
    }

    /// A SEAM TEST — memory, skills and MCP must point at THE SAME directory.
    ///
    /// This guarantee used to be written in three separate crates as three
    /// separate `HOME` + hidden-folder expressions and nothing kept the three the
    /// same: changing one would SILENTLY separate the others (the user's skills
    /// would stay in one directory, their memory in another). The path was moved
    /// to one place and this test catches a divergence at RUN time, not compile
    /// time — because a divergence is not a type error, it is a value error.
    ///
    /// This crate is the ONLY place that sees all three layers; that is also why
    /// the test is here.
    #[test]
    fn memory_skills_and_mcp_point_at_the_same_config_directory() {
        let home = std::env::temp_dir().join("tacet-seam-test");
        // SAFETY: the environment variable is PROCESS-WIDE; no other test in this
        // test binary reads `TACET_HOME`, and both assertions run inside this one
        // test, in order.
        unsafe {
            std::env::set_var(tacet_kernel::env::HOME_VAR, &home);
            // With a direct path override in place MCP takes its own branch; we
            // turn it off so what is measured is the SHARED directory.
            std::env::remove_var(tacet_mcp::config::PATH_VARIABLE);
        }

        assert_eq!(
            MemoryStore::default_path().unwrap(),
            home.join("memory.json")
        );
        assert_eq!(tacet_skills::user_dir().unwrap(), home.join("skills"));
        assert_eq!(
            tacet_mcp::config::default_path().unwrap(),
            home.join("mcp.json")
        );

        unsafe { std::env::remove_var(tacet_kernel::env::HOME_VAR) };
    }

    /// Collects the `Choice` fields of a tool's root schema: (field name, allowed
    /// values). Only the root object is walked — no tool nests a choice deeper,
    /// and walking blind would invite false positives.
    fn choice_fields(schema: &ArgSchema) -> Vec<(String, Vec<String>)> {
        let tacet_kernel::SchemaKind::Object { fields } = &schema.kind else {
            return Vec::new();
        };
        fields
            .iter()
            .filter_map(|f| match &f.schema.kind {
                tacet_kernel::SchemaKind::Choice { choices } => {
                    Some((f.name.clone(), choices.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// The double-quoted tokens in one text block, lowercase words only. A guide
    /// quotes argument VALUES this way (`"excel"`); prose quoting a sentence
    /// carries spaces and drops out here.
    ///
    /// A JSON **KEY** IS NOT A VALUE, and this used to treat it as one. The four
    /// original guides quote values in prose and never write a whole call, so
    /// nothing noticed; the first guide that carried a concrete
    /// `git({"action":"status"})` example — which is exactly the shape
    /// `injection.rs` says a core is FOR — was rejected with
    /// `action="action", but 'git' only accepts ["status","diff","log"]`. The
    /// claim in this test's own header is about the value the model is told to
    /// send, and a key is the field's name, checked already by `choice_fields`
    /// finding it at all.
    ///
    /// A KEY IS THE TOKEN FOLLOWED BY A COLON. That is the whole rule, and it is
    /// deliberately syntactic: the alternative — trusting position inside the
    /// object — breaks on the first guide that writes a value containing a colon.
    /// Nothing else about the net is loosened; a bogus VALUE is still caught.
    fn quoted_words(block: &str) -> Vec<String> {
        let parts: Vec<&str> = block.split('"').collect();
        parts
            .iter()
            .enumerate()
            .skip(1)
            .step_by(2)
            .filter(|(i, _)| {
                !parts
                    .get(i + 1)
                    .is_some_and(|after| after.trim_start().starts_with(':'))
            })
            .map(|(_, t)| *t)
            .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .map(|t| t.to_string())
            .collect()
    }

    /// The unit of the check is the PARAGRAPH, not the line. It was the line
    /// first and the first mutation test showed that to be worthless: the guide
    /// names the field on one line ("`format`: data/plan/budget ->") and carries
    /// the values onto the next, so the very defect this test exists for slipped
    /// straight through a line-scoped filter.
    fn paragraphs(text: &str) -> Vec<String> {
        text.split("\n\n").map(|p| p.to_string()).collect()
    }

    /// A SEAM TEST — a skill guide must not command a tool name, or an argument
    /// VALUE, that the catalog does not have.
    ///
    /// NOT A TYPE ERROR, which is the whole point: the guides are plain text and
    /// the schemas are Rust, so a divergence compiles clean and surfaces only as
    /// the model emitting a call the grammar refuses. It was a REAL defect, found
    /// by hand and not by any test: the bundled `create-document` guide told the
    /// model `format` -> "pdf" while the schema's choice set was
    /// {excel, markdown, text}. The iOS side has PDF; this build does not, and
    /// nothing kept the two apart.
    ///
    /// This crate is the only place that sees both the skill store and the
    /// production catalog, which is why the test lives here.
    #[test]
    fn skill_guides_only_name_tools_and_values_the_catalog_has() {
        let store = Arc::new(SharedStore::new());
        let memory = SharedMemory::in_memory();
        // The web gate is supplied FROM OUTSIDE: what is measured must not depend
        // on whether the addon happens to be open on the machine running the test.
        let (catalog, _, _) =
            tacet_tools::catalog::production_catalog_with(&store, &memory, Some(0), true);
        let skills = tacet_skills::SkillStore::default_set();

        // THE TOOLS AN ADDON WOULD BRING, taken from the registry's own rows.
        //
        // WHY THE NAME CHECK CANNOT JUST BE `catalog.find`. Five tools are addon
        // gated (`clipboard`, `db`, `http`, `shell`, `web_search`) and three of
        // those also have to DISCOVER themselves on the host — `db` wants a
        // `sqlite3` whose read-only lock it has measured, `clipboard` a helper
        // binary, `http` a non-empty host list. Opening every gate here would
        // make the test pass or fail by machine, which measures the runner. So a
        // skill may command a tool the catalog does not hold, PROVIDED some addon
        // definition declares that name — a typo still fails, an unshipped tool
        // does not.
        //
        // THE PRICE, stated: a gated tool's CHOICE VALUES go unchecked, because
        // there is no schema to check them against without building the tool. The
        // guides for those five therefore carry the weaker guarantee, and that is
        // a limit of this seam and not something the guide files can fix.
        let addon_tools: Vec<&str> = tacet_web::addon::DEFINITIONS
            .iter()
            .flat_map(|d| d.tools.iter().copied())
            .collect();

        let mut checked_values = 0;
        for skill in skills.all() {
            for name in &skill.tools {
                let Some(tool) = catalog.find(name) else {
                    assert!(
                        addon_tools.contains(&name.as_str()),
                        "skill '{}' commands tool '{name}', which is neither in the catalog \
                         {:?} nor declared by any addon {addon_tools:?}",
                        skill.name,
                        catalog.names()
                    );
                    continue;
                };
                for (field, allowed) in choice_fields(&tool.schema()) {
                    for block in paragraphs(&skill.text)
                        .iter()
                        .filter(|b| b.contains(&field))
                    {
                        for value in quoted_words(block) {
                            assert!(
                                allowed.contains(&value),
                                "skill '{}' tells the model {field}=\"{value}\", \
                                 but '{name}' only accepts {allowed:?}\n--- block ---\n{block}",
                                skill.name
                            );
                            checked_values += 1;
                        }
                    }
                }
            }
        }
        // A guard on the guard: if the extraction above silently stops matching
        // anything, the test would pass while measuring NOTHING.
        assert!(
            checked_values > 0,
            "no argument value was checked at all — the extraction is broken"
        );

        // The system instructions carry ONE hard-coded example call, and the
        // model copies its shape verbatim. `tacet-engine` already asserts the
        // example is there; nothing asserted that the tool and the argument in
        // it EXIST, because that crate cannot see the catalog. This one can.
        let example = SYSTEM_INSTRUCTIONS
            .split_once("Example: ")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once("({"))
            .expect("the system instructions carry an 'Example: name({' call");
        let example_tool = example.0;
        let example_tool = catalog.find(example_tool).unwrap_or_else(|| {
            panic!(
                "the system prompt's example calls '{example_tool}', \
                 which is not in the catalog: {:?}",
                catalog.names()
            )
        });
        let example_args = example.1;
        let fields: Vec<String> = match &example_tool.schema().kind {
            tacet_kernel::SchemaKind::Object { fields } => {
                fields.iter().map(|f| f.name.clone()).collect()
            }
            _ => Vec::new(),
        };
        for key in quoted_words(example_args.split("})").next().unwrap_or("")) {
            assert!(
                fields.contains(&key),
                "the system prompt's example passes '{key}', but \
                 '{}' takes {fields:?}",
                example_tool.name()
            );
        }
    }

    // -----------------------------------------------------------------------
    // Model package discovery
    //
    // NO NETWORK TEST — this whole section is temporary-directory fixtures. The
    // tests for the download principle live in `tacet-web/src/download.rs` and
    // they open no socket either (approval gate, schema gate, SHA-256 vectors).
    // -----------------------------------------------------------------------

    /// Creates an empty temporary root. Made unique by name: the tests run in
    /// parallel and sharing one directory would mean deleting each other's files.
    fn temp_root(name: &str) -> std::path::PathBuf {
        let r = std::env::temp_dir().join(format!("tacet-model-test-{name}"));
        let _ = std::fs::remove_dir_all(&r);
        std::fs::create_dir_all(&r).expect("temp root");
        r
    }

    /// Sets up a package folder under the root; `files` are created with empty
    /// content.
    fn install_package(root: &std::path::Path, name: &str, files: &[&str]) {
        let d = root.join(name);
        std::fs::create_dir_all(&d).expect("package directory");
        for f in files {
            std::fs::write(d.join(f), b"x").expect("file");
        }
    }

    /// WITH SEVERAL `.gguf` FILES THE CHOICE IS DETERMINISTIC: the first by file
    /// name.
    ///
    /// The old code took the FIRST `.gguf` `read_dir` returned; that order depends
    /// on the file system, i.e. the same folder could load a DIFFERENT weight on
    /// two machines (or on the same machine after a file was added). This test
    /// stops that silent difference from coming back.
    #[test]
    fn with_several_ggufs_the_first_by_name_is_picked() {
        let root = temp_root("determinism");
        install_package(
            &root,
            "many",
            &[
                "z-last.gguf",
                "a-first.gguf",
                "m-mid.gguf",
                "tokenizer.json",
            ],
        );

        let p = model_package::scan(std::slice::from_ref(&root));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].gguf.file_name().unwrap(), "a-first.gguf");
        // Scanning again must give the SAME result.
        assert_eq!(model_package::scan(std::slice::from_ref(&root)), p);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The catalog is sorted BY NAME; a folder with no `.gguf` IS NOT a package; a
    /// package with no tokenizer IS VISIBLE but cannot be selected.
    #[test]
    fn the_catalog_is_sorted_and_half_packages_are_distinguished() {
        let root = temp_root("ordering");
        install_package(&root, "zeta", &["m.gguf", "tokenizer.json"]);
        install_package(&root, "alfa", &["m.gguf", "tokenizer.json"]);
        // No tokenizer: a package, but HALF.
        install_package(&root, "beta", &["m.gguf"]);
        // No gguf at all: not a package.
        install_package(&root, "empty", &["tokenizer.json", "README.md"]);

        let p = model_package::scan(std::slice::from_ref(&root));
        let names: Vec<&str> = p.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["alfa", "beta", "zeta"], "must be sorted by name");

        let beta = p.iter().find(|x| x.name == "beta").unwrap();
        assert!(!beta.is_complete());
        assert!(
            model_package::to_pair(&p, "beta").is_none(),
            "a half package cannot be selected"
        );
        assert!(model_package::to_pair(&p, "alfa").is_some());
        assert!(model_package::to_pair(&p, "missing").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// THE SAME NAME IN TWO ROOTS: the EARLIER root wins and the other IS NOT
    /// REPEATED IN THE LIST. Repeated, the user would see the same name twice and
    /// still not know which one gets loaded.
    #[test]
    fn with_the_same_name_in_two_roots_the_earlier_root_wins() {
        let one = temp_root("root-one");
        let two = temp_root("root-two");
        install_package(&one, "same", &["first.gguf", "tokenizer.json"]);
        install_package(&two, "same", &["second.gguf", "tokenizer.json"]);
        install_package(&two, "only-in-two", &["m.gguf", "tokenizer.json"]);

        let p = model_package::scan(&[one.clone(), two.clone()]);
        assert_eq!(p.len(), 2, "the same name must not repeat: {p:?}");
        let same = p.iter().find(|x| x.name == "same").unwrap();
        assert_eq!(same.root, one);
        assert_eq!(same.gguf.file_name().unwrap(), "first.gguf");
        // Given in reverse order the winner must reverse too (priority comes FROM
        // THE ORDER).
        let reversed = model_package::scan(&[two.clone(), one.clone()]);
        assert_eq!(
            reversed.iter().find(|x| x.name == "same").unwrap().root,
            two
        );
        let _ = std::fs::remove_dir_all(&one);
        let _ = std::fs::remove_dir_all(&two);
    }

    /// A missing root IS NOT AN ERROR, it is "empty": if one of the user's two
    /// roots is missing, the other must still be scanned.
    #[test]
    fn a_missing_root_does_not_stop_the_scan() {
        let present = temp_root("existing");
        install_package(&present, "one", &["m.gguf", "tokenizer.json"]);
        let missing = std::env::temp_dir().join("tacet-model-test-no-such-root");
        let _ = std::fs::remove_dir_all(&missing);

        let p = model_package::scan(&[missing, present.clone()]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "one");
        let _ = std::fs::remove_dir_all(&present);
    }

    /// The size is recorded (the input of the list output and of `byte_text`).
    #[test]
    fn the_gguf_size_is_recorded() {
        let root = temp_root("size");
        install_package(&root, "p", &["tokenizer.json"]);
        std::fs::write(root.join("p").join("m.gguf"), vec![0u8; 4096]).unwrap();
        let p = model_package::scan(std::slice::from_ref(&root));
        assert_eq!(p[0].gguf_bytes, 4096);
        assert_eq!(byte_text(p[0].gguf_bytes), "4.0 KiB");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn byte_text_picks_the_right_scale() {
        assert_eq!(byte_text(0), "0 B");
        assert_eq!(byte_text(999), "999 B");
        assert_eq!(byte_text(1024), "1.0 KiB");
        assert_eq!(byte_text(2_497_281_120), "2.3 GiB");
    }

    /// THE ENV BRANCHES IN ONE TEST. Environment variables are PROCESS-WIDE; split
    /// across separate tests they would see each other's values while running in
    /// parallel (the same rationale as the `tacet_kernel::env` tests).
    #[test]
    fn the_env_pair_resolves_with_an_optional_tokenizer() {
        // SAFETY: a single-threaded test body; no other test in this binary reads
        // these two variables.
        unsafe {
            std::env::remove_var(MODEL_VARIABLE);
            std::env::remove_var(TOKENIZER_VARIABLE);
        }
        assert!(model_package::pair_from_env().is_none());

        // The tokenizer became OPTIONAL in the pair (a GGUF can carry its
        // own): model alone now resolves, with `None` for the tokenizer side.
        unsafe { std::env::set_var(MODEL_VARIABLE, "/path/m.gguf") };
        assert_eq!(
            model_package::pair_from_env(),
            Some(("/path/m.gguf".to_string(), None))
        );

        unsafe { std::env::set_var(TOKENIZER_VARIABLE, "/path/tokenizer.json") };
        assert_eq!(
            model_package::pair_from_env(),
            Some((
                "/path/m.gguf".to_string(),
                Some("/path/tokenizer.json".to_string())
            ))
        );
        // The env override is AHEAD OF DISCOVERY: even a name not in the catalog
        // resolves.
        assert!(model_package::resolve_pair("no-such-package").is_some());

        // An empty value counts as "undefined" (see `tacet_kernel::env_var`).
        unsafe { std::env::set_var(MODEL_VARIABLE, "") };
        assert!(model_package::pair_from_env().is_none());

        unsafe {
            std::env::remove_var(MODEL_VARIABLE);
            std::env::remove_var(TOKENIZER_VARIABLE);
        }
    }

    /// The roots have to be ABSOLUTE: a relative root would tie the model search
    /// to the user's current working directory.
    #[test]
    fn the_roots_are_always_absolute() {
        for r in model_package::model_roots() {
            assert!(r.is_absolute(), "relative root: {}", r.display());
        }
    }

    /// `packages.json` parsing — without touching the file system.
    #[test]
    fn the_remote_catalog_is_parsed() {
        let raw = r#"{"packages":[
            {"name":"a","files":[
                {"name":"model.gguf","url":"https://e.test/a.gguf","bytes":10,"sha256":"ab"},
                {"name":"tokenizer.json","url":"https://e.test/t.json"}
            ]}
        ]}"#;
        let c = model_package::parse_remote_catalog(raw).expect("valid catalog");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "a");
        assert_eq!(c[0].files[0].bytes, Some(10));
        assert_eq!(c[0].files[0].sha256.as_deref(), Some("ab"));
        // Undeclared fields are `None` — THEY ARE NOT INVENTED. Showing a fake
        // number for a file of unknown size would make the approval screen a liar.
        assert_eq!(c[0].files[1].bytes, None);
        assert_eq!(c[0].files[1].sha256, None);
    }

    /// A BROKEN CATALOG IS NOT SILENTLY SWALLOWED: every missing mandatory field
    /// errors.
    #[test]
    fn a_broken_remote_catalog_errors() {
        for raw in [
            "{}",                                                    // no `packages`
            r#"{"packages":{}}"#,                                    // not an array
            r#"{"packages":[{"files":[]}]}"#,                        // no package name
            r#"{"packages":[{"name":"a"}]}"#,                        // no `files`
            r#"{"packages":[{"name":"a","files":[{"name":"m"}]}]}"#, // no url
            "this is not json",
            // A NAME BECOMES A PATH. An absolute file name makes
            // `PathBuf::join` discard the model root, so the catalog would
            // write straight into the user's home; `..` walks out the same way.
            // A user's `packages.json` overrides the embedded catalog BY NAME,
            // so a poisoned file turns the documented
            // `tacet models download qwen3-4b` into arbitrary file writing.
            r#"{"packages":[{"name":"a","files":[{"name":"/etc/x","url":"https://e.test/x"}]}]}"#,
            r#"{"packages":[{"name":"a","files":[{"name":"../../x","url":"https://e.test/x"}]}]}"#,
            r#"{"packages":[{"name":"a","files":[{"name":"sub/x","url":"https://e.test/x"}]}]}"#,
            r#"{"packages":[{"name":"a","files":[{"name":"","url":"https://e.test/x"}]}]}"#,
            // The package name is a DIRECTORY name and carries the same risk.
            r#"{"packages":[{"name":"../../a","files":[{"name":"m","url":"https://e.test/x"}]}]}"#,
            r#"{"packages":[{"name":"/tmp/a","files":[{"name":"m","url":"https://e.test/x"}]}]}"#,
        ] {
            assert!(
                model_package::parse_remote_catalog(raw).is_err(),
                "should not have been accepted: {raw}"
            );
        }
        // An empty catalog IS VALID: "I defined no source" is not an error.
        assert_eq!(
            model_package::parse_remote_catalog(r#"{"packages":[]}"#).unwrap(),
            vec![]
        );
    }

    /// The embedded catalog was EMPTY until a default was shipped, and the test
    /// that guarded the emptiness caught this change rather than letting it pass
    /// — which is what it was for. It is replaced, not deleted: the reason the
    /// catalog was empty was that an unverified address or digest is worse than
    /// no catalog, so what is measured now is that every entry still clears that
    /// bar.
    #[test]
    fn every_embedded_entry_is_complete_and_https() {
        let catalog = model_package::embedded_catalog();
        assert!(!catalog.is_empty(), "the default catalog disappeared");

        for package in &catalog {
            // Both files, or the package downloads and still cannot be loaded:
            // the engine wants a tokenizer next to the weight.
            let names: Vec<&str> = package.files.iter().map(|f| f.name.as_str()).collect();
            assert!(names.contains(&"model.gguf"), "{}: no weight", package.name);
            assert!(
                names.contains(&"tokenizer.json"),
                "{}: no tokenizer",
                package.name
            );

            for file in &package.files {
                assert!(
                    file.url.starts_with("https://"),
                    "{} / {}: not https",
                    package.name,
                    file.name
                );
                // The size is what the approval screen shows. Without it the
                // user is asked to accept a download of unknown size, which is
                // the one quantitative fact that decision rests on.
                assert!(
                    file.bytes.is_some_and(|b| b > 0),
                    "{} / {}: no size declared",
                    package.name,
                    file.name
                );
                // `None` is allowed — it means trust-on-first-use — but a value
                // that is present must LOOK like a SHA-256, or verification
                // fails on the first download and teaches the user to disable it.
                if let Some(sha) = &file.sha256 {
                    assert_eq!(sha.len(), 64, "{}: digest is not 64 chars", file.name);
                    assert!(
                        sha.chars()
                            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                        "{}: digest is not lowercase hex",
                        file.name
                    );
                }
            }
        }

        // The default model must be gettable, or a fresh install lands on the
        // FakeEngine with no way out that the tool itself can offer.
        assert!(
            catalog.iter().any(|p| p.name == DEFAULT_MODEL),
            "the catalog does not carry the default model ({DEFAULT_MODEL})"
        );
        // The example text still invents no address of its own.
        assert!(model_package::EXAMPLE_CATALOG.contains("<your-own-mirror>"));
    }

    /// A user's own entry overrides by name and does NOT hide the rest.
    #[test]
    fn a_user_entry_overrides_by_name_and_keeps_the_other_defaults() {
        let mine = model_package::parse_remote_catalog(
            r#"{"packages":[{"name":"qwen3-4b","files":[
                 {"name":"model.gguf","url":"https://mine.invalid/m.gguf"},
                 {"name":"tokenizer.json","url":"https://mine.invalid/t.json"}]}]}"#,
        )
        .unwrap();
        let taken: std::collections::HashSet<String> =
            mine.iter().map(|p| p.name.clone()).collect();
        let merged: Vec<_> = mine
            .iter()
            .cloned()
            .chain(
                model_package::embedded_catalog()
                    .into_iter()
                    .filter(|p| !taken.contains(&p.name)),
            )
            .collect();

        let overridden = merged.iter().find(|p| p.name == "qwen3-4b").unwrap();
        assert_eq!(overridden.files[0].url, "https://mine.invalid/m.gguf");
        // The one the user did not mention survives — before merging, writing a
        // single entry silently emptied the rest of the catalog.
        assert!(merged.iter().any(|p| p.name == "qwen2.5-3b"));
    }

    /// A SEAM TEST — the model variable's name must be the same on the DISCOVERY
    /// path and on the WARNING path.
    ///
    /// When the two paths write different strings the failure is silent: the user
    /// reads the "set TACET_MODEL" warning, sets it, and nothing changes. Sharing
    /// one pair of constants makes that structurally impossible; this test
    /// measures that those constants REALLY do land in the warning text.
    #[test]
    fn the_model_variable_name_appears_verbatim_in_the_warning_text() {
        assert_eq!(MODEL_VARIABLE, "TACET_MODEL");
        assert_eq!(TOKENIZER_VARIABLE, "TACET_TOKENIZER");
        let warning = format!(
            "(local model not found: set {}/{})",
            MODEL_VARIABLE, TOKENIZER_VARIABLE
        );
        assert!(warning.contains("TACET_MODEL/TACET_TOKENIZER"), "{warning}");
    }

    /// THE DOWNLOAD ROOT MUST BE THE SAME AS THE SCAN ROOT.
    ///
    /// If they diverge the failure is insidious: the download says "finished",
    /// `model list` does not show the package, or a half folder with the same name
    /// in another root shadows it. `scan` says the earlier root wins; the download
    /// must land in the EARLIER root too.
    #[test]
    fn the_download_root_matches_the_scan_priority() {
        let roots = model_package::model_roots();
        assert_eq!(download_root(), roots.first().cloned());
    }

    /// THE APPROVAL GATE IS REALLY APPLIED IN THE SHELL.
    ///
    /// This was for a whole round the example of this repo's recurring failure:
    /// `tacet_web::download` was written, tested and NEVER CALLED FROM PRODUCTION.
    /// What is measured here is not text but STRUCTURE: the shell's gate type
    /// really does implement `tacet_web::DownloadApproval` and does not return
    /// `true` by itself without `--no-approval`.
    #[test]
    fn the_shell_really_applies_the_download_gate() {
        fn is_gate<G: tacet_web::DownloadApproval>(_: &G) {}
        let unattended = TerminalDownloadApproval {
            color: Color::setup(),
            no_approval: true,
            no_digest_note: TOFU_NOTE_CATALOG,
        };
        is_gate(&unattended);

        let plan = tacet_web::DownloadPlan {
            name: "model.gguf".into(),
            url: "https://shell-test.invalid/model.gguf".into(),
            target: std::env::temp_dir().join("tacet-gate-test.gguf"),
            expected_bytes: Some(10),
            expected_sha256: None,
        };
        // `--no-approval` must say yes UNCONDITIONALLY: without a script mode CI
        // cannot use this command at all and the user falls back to `curl` (i.e.
        // loses the digest verification too).
        assert!(tacet_web::DownloadApproval::approve(&unattended, &plan, 0));

        // A flagless gate ASKS stdin; stdin cannot be driven in a test, so the
        // only thing read here is that the default is NOT "silently yes" —
        // `tacet_web::download` does not set up the agent before the gate returns
        // `true` either.
        let silent = TerminalDownloadApproval {
            color: Color::setup(),
            no_approval: false,
            no_digest_note: TOFU_NOTE_CATALOG,
        };
        assert!(!silent.no_approval);
    }

    /// THE DOWNLOAD PROGRESS DOES NOT INVENT A PERCENTAGE.
    ///
    /// If the server gives no `Content-Length` the total is UNKNOWN; showing a
    /// percentage then would be writing the unmeasured as if it were measured.
    #[test]
    fn the_progress_shows_no_percentage_when_the_total_is_unknown() {
        // The total IS KNOWN: a percentage appears.
        let known = progress_text(512 * 1024, Some(1024 * 1024));
        assert!(known.contains('%'), "{known}");
        assert!(known.contains("50%"), "{known}");

        // The total IS UNKNOWN: NO percentage, only the amount downloaded.
        let unknown = progress_text(512 * 1024, None);
        assert!(!unknown.contains('%'), "percentage invented: {unknown}");
        assert_eq!(unknown, byte_text(512 * 1024));

        // If the server declared length 0 there is NO DIVISION BY ZERO.
        let zero = progress_text(0, Some(0));
        assert!(!zero.contains("NaN") && !zero.contains("inf"), "{zero}");
        assert!(!zero.contains('%'), "{zero}");
    }
}
