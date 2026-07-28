//! tacet-cli — the terminal shell of the on-device assistant.
//!
//! WHY IT EXISTS: when Tacet's logic layer (routing, prompt construction,
//! grammar, tool execution, the bypass channel, skill/memory injection) is
//! hidden inside an iOS app it can only be observed by opening the simulator.
//! This binary drives the same layer from the terminal: `chat` opens a flowing,
//! an interactive turn loop against a real model, `eval` runs in CI,
//! `grammar`/`tools` print the prompt's source verbatim.
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
mod config;
mod filter;
mod format;
mod input;
mod session;
mod ui;
mod update;

use clap::{Parser, Subcommand, ValueEnum};
use tacet_engine::{
    CONTEXT_BUDGET, EngineProvider, FakeEngine, Prompt, SamplingSetting, TokenCounter, Turn, wait,
};
use tacet_eval::{FakeSelector, SYSTEM_INSTRUCTIONS};
use tacet_grammar::{CallConstraint, Grammar};
use tacet_kernel::{
    ArgSchema, DataStore as CoreDataStore, Reporter, ToolCatalog, ToolContext, TraceCollector,
};
use tacet_memory::MemoryStore;
use tacet_skills::{InjectionState, SkillStore, injection_text};
use tacet_tools::data_store::SharedStore;
use tacet_tools::executor::{
    ApprovalGate, ApprovalRequest, ExecutionOutcome, SilentDeny, ToolExecutor,
};
use tacet_tools::mcp;
use tacet_tools::memory::SharedMemory;
use tacet_tools::router::Router;
use tacet_tools::run_code::CodeState;
use ui::{BOLD, BRASS, Color, DIM, LiveReporter, RESET, Screen, TurnIndicator, YELLOW, paper_code};

use std::io::{IsTerminal, Read as _, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
const EXTERNAL_TOOLS: &[&str] = &["web_search", "web_fetch"];

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
/// Latency is not a comfort detail in this product: in a tool loop with a 4096
/// window every turn re-prefills, and the user waits on every turn.
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

#[derive(Parser)]
#[command(
    name = "tacet",
    about = "Tacet — the terminal shell of the on-device assistant"
)]
struct Shell {
    /// THE SUBCOMMAND IS OPTIONAL. If not given, the interactive shell opens;
    /// the slash commands inside it (`/eval`, `/tools`, `/grammar`, ...) reach
    /// the same jobs from there. The subcommands were not removed: scripts and
    /// the `--message` diagnostics depend on them, and a shell staying
    /// scriptable matters more than decoration.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive chat: reads a message, drives the tool loop, prints a
    /// streaming answer.
    Chat {
        #[arg(long, value_enum, default_value_t = EngineChoice::Auto)]
        engine: EngineChoice,
        /// FakeEngine script (repeatable). If not given, a fixed answer is
        /// returned.
        #[arg(long)]
        script: Vec<String>,
        /// Print the whole prompt going to the model on every turn — diagnostic.
        #[arg(long)]
        show_prompt: bool,
        /// Working directory (the sandbox root).
        #[arg(long, default_value = ".")]
        dir: String,
        /// Run a single message and exit (for scripted diagnostics). In this
        /// mode the approval gate stays SilentDeny: there is nobody to ask, NO
        /// DATA LEAVES.
        #[arg(long)]
        message: Option<String>,
        /// The local model folder to use (`~/models/<name>`). When omitted,
        /// the `model` key of `tacet config` applies, then `qwen3-4b`.
        /// `TACET_MODEL`/`TACET_TOKENIZER` OVERRIDE all of these.
        #[arg(long)]
        model: Option<String>,
        /// One line of JSON per turn on stdout, and NOTHING ELSE — no banner,
        /// no chips, no colour, no streaming text.
        ///
        /// WHY IT EXISTS: `tacet -m "..."` was already scriptable, but the
        /// answer arrived mixed into human decoration ("Tacet: ", chip lines,
        /// a blank line) and the only way to consume it was to guess at the
        /// prefix. A caller that wants a machine answer should not have to
        /// parse a screen; `tacet -m "..." --json | jq -r .answer` is the whole
        /// contract.
        #[arg(long)]
        json: bool,
        /// Loads the most recent stored session into this one's history.
        ///
        /// `continue` IS A RUST KEYWORD, hence the field rename. The flag the
        /// user types is `--continue`, which is the name every other shell
        /// uses for this.
        #[arg(long = "continue")]
        continue_session: bool,
        /// Loads ONE named session (`tacet sessions` prints the names).
        #[arg(long = "session", value_name = "ID")]
        session_id: Option<String>,
    },
    /// Lists the conversations kept on disk — and deletes them.
    ///
    /// A SEPARATE TOP-LEVEL COMMAND, not `config sessions`: this is the one
    /// place where the answer to "what does Tacet keep about me, and how do I
    /// get rid of it" lives, and a privacy answer buried under a settings verb
    /// is an answer nobody finds. `/sessions` inside the shell reaches the same
    /// listing.
    Sessions {
        /// JSON instead of a human table — same pattern as every other list.
        #[arg(long)]
        json: bool,
        /// Deletes EVERY stored session and the folder itself. On a terminal it
        /// asks first; piped it does not (a script that typed this flag meant
        /// it, and there is nobody to ask).
        #[arg(long)]
        purge: bool,
    },
    /// Runs the eval set; the exit code depends on the success rate.
    Eval {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = DEFAULT_THRESHOLD)]
        threshold: f64,
        /// Runs the TOOL SELECTION set instead of the logic set — with a REAL
        /// model.
        ///
        /// A separate flag, because both what is measured and the threshold
        /// differ: the logic set is deterministic with FakeEngine and 100% is
        /// expected; the selection set measures the model's own choice, takes
        /// minutes and 100% is not expected.
        #[arg(long)]
        tool_selection: bool,
        /// The local model folder the selection set will use (`~/models/<name>`).
        #[arg(long, default_value = DEFAULT_MODEL)]
        model: String,
        /// Run only the cases whose name contains this string (selection set;
        /// diagnostic).
        #[arg(long)]
        only: Option<String>,
    },
    /// Lists the catalog and its schemas.
    Tools {
        #[arg(long)]
        schema: bool,
    },
    /// Shows the grammar generated from a tool's schema (diagnostic).
    Grammar {
        #[arg(long)]
        tool: String,
        #[arg(long)]
        try_input: Option<String>,
    },
    /// Shows the SKILL packages: the ones EMBEDDED in the build + the ones
    /// loaded from the user's config directory.
    ///
    /// A NAME CLASH WAS RESOLVED. This shell has two separate notions of
    /// "package" and for a while they shared one name: (1) a SKILL package —
    /// kilobytes of markdown, injected into the prompt, living in the config
    /// directory; (2) a MODEL package — gigabytes of weights, setting up the
    /// engine, living in the model root. What a user typing `tacet package list`
    /// expected was ambiguous. The fix is not an abbreviation but a SPLIT:
    /// `package` means the skill one (it came first), the model side takes its
    /// own name (`model`). Had the two been gathered under one command with a
    /// flag (`package list --model`), both the outputs and the options would
    /// have been mixed up.
    Package {
        #[command(subcommand)]
        job: PackageJob,
    },
    /// Manages MODEL packages (weight files).
    // `model` IS AN ALIAS, measured from the wild: every piece of prose
    // (website, install script, a human sentence) naturally writes the
    // singular — "tacet model download" — and the shell answered with a usage
    // error. Accepting both costs nothing; the canonical name stays plural to
    // match `packages`.
    #[command(alias = "model")]
    Models {
        #[command(subcommand)]
        job: ModelJob,
    },
    /// Manages ADDONS: install, list, try.
    ///
    /// NOT A THIRD "package" NOTION. This shell now has three things and the
    /// distinction is sharp: a SKILL package injects text into the prompt
    /// (`package`), a MODEL package is a weight file (`model`), and an ADDON
    /// changes THE CATALOG — until it is installed the `web_search` tool does
    /// not exist at all. The first two determine what the assistant KNOWS, the
    /// third what it CAN DO.
    ///
    /// CLOSED BY DEFAULT. Without an installed addon no web tool is in the
    /// catalog; the "data does not leave the device" default is applied not as a
    /// setting but as the ABSENCE of the tool.
    Addon {
        #[command(subcommand)]
        job: AddonJob,
    },
    /// Personal defaults: a small `config.json` in the config directory.
    ///
    /// ONLY keys that already exist as flags are accepted (`model`, `engine`) —
    /// the file is a way to stop retyping a flag, not a second settings
    /// system. Precedence: flag > environment variable > config file >
    /// built-in default.
    Config {
        #[command(subcommand)]
        job: ConfigJob,
    },
    /// Shows how to give Tacet its intended look (font + colours).
    ///
    /// HONESTY FIRST: a terminal program CANNOT change the terminal's font —
    /// that setting belongs to the terminal emulator, not to the process
    /// running inside it. This command exists so "how do I make it look like
    /// the website?" has a one-word answer: it names the brand font and shows
    /// where each common terminal keeps the setting. It prints and exits; it
    /// changes nothing.
    Font,

    /// Asks GitHub whether a newer release exists.
    ///
    /// NOTHING CHECKS BY ITSELF. There is no start-up check and no timer: a
    /// program whose promise is that it stays off the network cannot quietly
    /// go online to ask about itself. This runs when it is typed, and only
    /// `--install` writes anything.
    Update {
        /// Downloads the release build for this platform and replaces this
        /// binary with it. The download passes the same approval gate as a
        /// model package.
        #[arg(long)]
        install: bool,
        /// Skips the question. What is being downloaded is still printed.
        #[arg(long = "no-approval")]
        no_approval: bool,
    },
}

/// The jobs of the `config` subcommand. The shape mirrors every other list
/// command in this shell: human text by default, `--json` for scripts.
#[derive(Subcommand)]
enum ConfigJob {
    /// Lists every known key, its current value and the file's location.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Prints one value. Exits with 1 when the key is unset.
    Get { key: String },
    /// Sets a key. Unknown keys and invalid values are refused.
    Set { key: String, value: String },
    /// Removes a key; the built-in default applies again.
    Unset { key: String },
    /// Prints the path of `config.json`.
    Path,
}

/// The jobs of the `addon` subcommand. The shape and the `--json` pattern are
/// the same as `package`/`model`: context at the top level (the registry path),
/// the record array under it.
#[derive(Subcommand)]
enum AddonJob {
    /// What is installed, which one is open, what its address is.
    List {
        /// JSON instead of a human table — for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Installs the addon. The only name installable today: `web-search`.
    ///
    /// A FLAGLESS CALL ASKS (local docker, or my own address); the flags are for
    /// scripts and skip the questions.
    Install {
        /// The addon name (`web-search`).
        name: String,
        /// Your own SearXNG address. https is mandatory; plain http only on a
        /// local network.
        #[arg(long)]
        address: Option<String>,
        /// Set up a local SearXNG with docker.
        #[arg(long)]
        local: bool,
        /// Skips the docker approval question — FOR SCRIPTS (see `model download
        /// --no-approval`: same rationale, same long name).
        #[arg(long = "no-approval")]
        no_approval: bool,
    },
    /// Deletes the record; the tools drop out of the catalog.
    Remove { name: String },
    /// Closes it without deleting the record (address and settings are kept).
    Close { name: String },
    /// Reopens a closed addon.
    Open { name: String },
    /// Tries an installed addon with a REAL query. IT GOES ON THE NETWORK.
    Try {
        name: String,
        #[arg(long)]
        json: bool,
    },
}

/// The jobs of the `package` subcommand.
///
/// WHY A SUB-SUBCOMMAND: today there is one job (`list`) but the natural
/// continuations of package management (`show`, `verify`) go under the same
/// name. Had a flat flag like `package-list` been chosen, every new job would
/// have become a new command at the root level.
#[derive(Subcommand)]
enum PackageJob {
    /// Lists the installed skill packages.
    List {
        /// JSON instead of a human table — for scripts.
        #[arg(long)]
        json: bool,
    },
}

/// The jobs of the `model` subcommand.
///
/// `download` IS HERE AND WIRED INTO PRODUCTION. In a previous round a note sat
/// above this enum saying "the download subcommand DOES NOT EXIST yet, blocked":
/// the download principle had been written and tested in `tacet_web::download`
/// (approval gate, resume with Range, atomic swap, official SHA-256 vectors) but
/// this crate did not see `tacet-web`, meaning the mechanism had ZERO CALLERS IN
/// PRODUCTION. The dependency line (`tacet-web.workspace = true`) was added and
/// the branch wired below; `model_download` really does call
/// `tacet_web::download`.
///
/// THE SHELL OPENS NO SOCKET. This crate does not pull `ureq`; it only supplies
/// the terminal side — the `[y/N]` approval question and the progress line — and
/// hands it to `tacet_web`. The network monopoly thus stays auditable by eye at
/// the manifest level too.
#[derive(Subcommand)]
enum ModelJob {
    /// Lists the installed model packages: name, size, path, whether selected.
    List {
        /// JSON instead of a human table — for scripts. The field layout follows
        /// the same pattern as `package list --json`: context at the top level
        /// (the roots), the record array under it.
        #[arg(long)]
        json: bool,
        /// The criterion for the "selected" column: which package `chat` would
        /// use under this name.
        #[arg(long, default_value = DEFAULT_MODEL)]
        model: String,
    },
    /// Downloads a model package described in `packages.json`.
    ///
    /// THE CATALOG IS NOT EMBEDDED: both the address and the digest belong to
    /// the user (rationale in `model_package::embedded_catalog`). That is, this
    /// command downloads from the source the USER wrote — not from a mirror we
    /// picked.
    Download {
        /// The package name in `packages.json`. The folder on disk takes this
        /// name too.
        name: String,
        /// Skips the approval question — FOR SCRIPTS, and deliberately written
        /// long.
        ///
        /// WHY IT EXISTS: without an unattended mode a CI/install script cannot
        /// use this command at all and the user falls back to `curl` — that is,
        /// loses the digest verification too. WHY IT IS NAMED THIS WAY: a
        /// shortcut like `-y` gets typed by reflex; this flag starts a GB-sized
        /// download.
        #[arg(long = "no-approval")]
        no_approval: bool,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum EngineChoice {
    /// candle if a local model exists, fake otherwise — automatic.
    Auto,
    Fake,
    Candle,
}

fn main() -> ExitCode {
    // ANSWERED BEFORE ANYTHING ELSE, including argument parsing. `tacet update
    // --install` runs the DOWNLOADED binary with this flag to ask what it was
    // built with, and that question has to be cheap and impossible to fail:
    // no config read, no model discovery, no banner. One word, then exit.
    if std::env::args().any(|a| a == "--print-features") {
        println!("{}", update::compiled_features());
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
            })
        }
        Command::Sessions { json, purge } => sessions(json, purge),
        Command::Eval {
            json,
            threshold,
            tool_selection,
            model,
            only,
        } => {
            if tool_selection {
                eval_tool_selection(json, threshold, &model, only.as_deref())
            } else {
                eval(json, threshold)
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
        Command::Font => font(),
        Command::Update {
            install,
            no_approval,
        } => {
            let color = Color::setup();
            let outcome = if install {
                update::install(&color, no_approval).map(|()| true)
            } else {
                update::check(&color, false)
            };
            match outcome {
                Ok(_) => ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("  {}", color.paint(YELLOW, &message));
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// `tacet font` — the appearance guide. See the enum doc for why this prints
/// instructions instead of applying anything: the font is the TERMINAL's
/// setting and the terminal is the user's. The colour story is told here too,
/// because it is the same question ("why doesn't it look like the site?") and
/// the same answer (your terminal owns the canvas; Tacet adapts to it).
fn font() -> ExitCode {
    let color = Color::setup();
    println!("{}{}", color.paint(BOLD, "Tacet"), color.paint(BRASS, "."));
    println!();
    println!("Tacet inherits its font from your terminal — a program cannot change");
    println!("it by itself. It is designed for JetBrains Mono (free, OFL licence):");
    println!(
        "{}",
        color.paint(DIM, "  https://www.jetbrains.com/lp/mono/")
    );
    println!();
    println!("Where your terminal keeps the setting:");
    println!("  Terminal.app      Settings… > Profiles > Font");
    println!("  iTerm2            Settings… > Profiles > Text > Font");
    println!("  VS Code           \"terminal.integrated.fontFamily\": \"JetBrains Mono\"");
    println!(
        "  kitty             font_family JetBrains Mono   {}",
        color.paint(DIM, "(~/.config/kitty/kitty.conf)")
    );
    println!(
        "  Ghostty           font-family = JetBrains Mono {}",
        color.paint(DIM, "(~/.config/ghostty/config)")
    );
    println!("  Windows Terminal  Settings > Defaults > Appearance > Font face");
    println!();
    println!("Any monospaced font works — the setting is yours, this is only the one");
    println!("the brand is drawn in.");
    println!();
    println!("Colours: Tacet maps its palette onto YOUR terminal theme. The night");
    println!("ground and the paper ink come from the terminal itself; only the brass");
    println!(
        "accent ({}) is Tacet's own, and only for brand moments — the",
        color.paint(BRASS, "this colour")
    );
    println!("banner's full stop and the spinning ensō. A dark theme sits closest");
    println!("to the brand, but nothing breaks on a light one.");
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// The approval gate — a real question in the terminal.
// ---------------------------------------------------------------------------

/// In a tainted session, shows the REAL PAYLOAD going to an external tool and
/// asks y/n.
///
/// WHY NOT `AlwaysApprove`: wiring the gate permanently to "yes" makes it
/// useless — the user would be approving without seeing the data being sent.
/// `SilentDeny` would be wrong too: here there IS someone to ask. The reason the
/// gate exists is to show the content being sent (the query string) to the user
/// VERBATIM.
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

// ---------------------------------------------------------------------------
// Engine selection
// ---------------------------------------------------------------------------

/// The variables that point at the model weights.
///
/// The names are NOT written out HERE; they live in two constants — so that the
/// DISCOVERY path (`model_package::pair_from_env`) and the WARNING paths
/// (`model_not_found_report`, `model_list`) are forced to use the same string.
/// When they were written separately, the user could be advised to set a
/// variable name that did not exist.
const MODEL_VARIABLE: &str = "TACET_MODEL";
const TOKENIZER_VARIABLE: &str = "TACET_TOKENIZER";

// ---------------------------------------------------------------------------
// The model package catalog — discovery, ordering, the remote catalog file
// ---------------------------------------------------------------------------

/// Local model PACKAGES: where they are looked for, which one is picked, what
/// information is shown.
///
/// WHY A SEPARATE MODULE: this whole job used to be a single function called
/// `model_paths` and it had THREE SEPARATE failures. (1) It only looked at
/// `$HOME/models`; since `HOME` does not resolve on Windows it NEVER worked
/// there. (2) It took the "first" `.gguf` in the folder — `read_dir` order
/// depends on the file system, so in a folder holding two weights WHICH ONE got
/// loaded was unpredictable. (3) When it found nothing it printed a one-line
/// guess to the user; it DID NOT SAY which roots it searched, so the answer to
/// "my file is right there but it doesn't see it" existed nowhere.
///
/// NO NETWORK. This whole module is the local file system and environment
/// variables; not one line opens a socket. Downloading is a SEPARATE layer
/// (`tacet_web::download`) and passes the approval gate.
mod model_package {
    use super::{MODEL_VARIABLE, TOKENIZER_VARIABLE};
    use std::path::{Path, PathBuf};

    /// The remote package catalog's name inside the config directory.
    pub const CATALOG_FILE: &str = "packages.json";

    /// An installed model package: one `.gguf` and (if present) its tokenizer.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ModelPackage {
        /// The folder name — what the user types with `--model`.
        pub name: String,
        pub dir: PathBuf,
        pub gguf: PathBuf,
        pub gguf_bytes: u64,
        /// `tokenizer.json` SITTING NEXT TO THE WEIGHTS, if the user has one.
        /// `None` no longer means the package is half — see `gguf_tokenizer`.
        pub tokenizer: Option<PathBuf>,
        /// Does the `.gguf` carry its own vocabulary in a shape we can rebuild.
        ///
        /// MEASURED ONCE, AT DISCOVERY, and only when it can change the answer.
        /// `gguf_has_tokenizer` walks the metadata header and stops before the
        /// tensor section (4-6 ms on a 2.5 GB file), but `models list` scans
        /// every package, so a needless read per package would be a visible
        /// pause on a machine holding several weights. When a `tokenizer.json`
        /// is already there the field is left `false` WITHOUT asking: the file
        /// wins anyway, so the answer could not change the outcome.
        pub gguf_tokenizer: bool,
        /// The root this package was found in — the same name can sit in two
        /// roots and the user needs to see WHICH ONE wins.
        pub root: PathBuf,
    }

    impl ModelPackage {
        /// Is it ENOUGH to set up an engine.
        ///
        /// THIS USED TO BE `self.tokenizer.is_some()` AND THAT WAS A BUG, not a
        /// policy: a `.gguf` already carries its vocabulary, its merges and its
        /// special tokens, so a user who downloaded one file by hand was told
        /// their package was half and could not be selected — while the engine
        /// sitting behind this check could have loaded it. The two sides now ask
        /// the same question (`CandleEngine::files_exist` runs exactly this
        /// `gguf_has_tokenizer` call when no `tokenizer.json` is given); if they
        /// disagreed, discovery would refuse packages the loader can handle.
        pub fn is_complete(&self) -> bool {
            self.tokenizer.is_some() || self.gguf_tokenizer
        }

        /// What to print next to the package, in one phrase.
        pub fn tokenizer_note(&self) -> &'static str {
            if self.tokenizer.is_some() {
                "tokenizer: tokenizer.json"
            } else if self.gguf_tokenizer {
                "tokenizer: inside the .gguf"
            } else {
                "tokenizer: MISSING — this package cannot be selected"
            }
        }
    }

    /// The model roots, IN PRIORITY ORDER.
    ///
    /// THIS IS NOT THE CONFIG DIRECTORY and it DELIBERATELY does not tie into
    /// `tacet_kernel::env::config_dir`. That is where SETTINGS live (`mcp.json`,
    /// `memory.json` — kilobytes of text, right to travel with a roaming
    /// profile). Model weights are not a setting, they are GIGABYTES OF DATA;
    /// putting them in `%APPDATA%` (the roaming profile) or `$XDG_CONFIG_HOME`
    /// would bloat the user's network profile or their backup. Hence a separate
    /// notion of a root; not duplication.
    ///
    /// On Unix the second root is `$XDG_DATA_HOME/tacet/models`: in XDG, large
    /// reproducible DATA goes there. XDG's `~/.local/share` default WAS NOT
    /// ADDED — if the user has not set the variable, `~/models` is already first
    /// in line, and creating an unmeasured third directory would make the
    /// question of where packages land even murkier.
    ///
    /// THE WINDOWS ROOTS ARE UNMEASURED: this machine has no rustup and
    /// `cargo check` could not be run for another target. The paths look like
    /// they compile on Windows (they only use `std::env`) but WERE NEVER RUN.
    /// Contrary to the old code this is not a regression but progress: the
    /// previous state searched no root at all on Windows.
    pub fn model_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let mut add = |p: PathBuf| {
            if !roots.contains(&p) {
                roots.push(p);
            }
        };
        if cfg!(windows) {
            if let Some(p) = absolute_env("USERPROFILE") {
                add(p.join("models"));
            }
            if let Some(p) = absolute_env("LOCALAPPDATA") {
                add(p.join("Tacet").join("models"));
            }
        } else {
            if let Some(p) = absolute_env("HOME") {
                add(p.join("models"));
            }
            if let Some(p) = absolute_env("XDG_DATA_HOME") {
                add(p.join("tacet").join("models"));
            }
        }
        roots
    }

    /// An environment variable that is non-empty and carries an ABSOLUTE path.
    ///
    /// A relative value IS IGNORED (the XDG rule): a relative root would tie the
    /// model search to the user's current working directory — opening `tacet`
    /// from another folder would make the model "disappear".
    ///
    /// THE LOCAL COPY WAS DELETED. The same rule lived here and in
    /// `tacet_kernel::env`, and the copies had already drifted: the version
    /// there applied the rule to `XDG_CONFIG_HOME` but not to `TACET_HOME`, the
    /// one variable a user actually types by hand. One home, one rule.
    use tacet_kernel::env::absolute_env;

    /// Scans the packages in the given roots.
    ///
    /// DETERMINISTIC — not the tests' need but THE USER'S:
    /// * packages are sorted BY NAME (`read_dir` order depends on the file
    ///   system),
    /// * if a folder holds several `.gguf` files the first BY FILE NAME is
    ///   picked,
    /// * if the same name exists in two roots the EARLIER root wins and the
    ///   other is dropped.
    ///
    /// All three fix the old "take the first one you find" behaviour: that state
    /// could run the same command with a different weight on two machines (or on
    /// the same machine after adding a file) and silently made measurement
    /// results incomparable.
    pub fn scan(roots: &[PathBuf]) -> Vec<ModelPackage> {
        let mut packages: Vec<ModelPackage> = Vec::new();
        for root in roots {
            let Ok(entries) = std::fs::read_dir(root) else {
                continue; // root missing or unreadable: not an error, "empty"
            };
            let mut candidates: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            candidates.sort();
            for dir in candidates {
                let Some(package) = package_from_dir(root, &dir) else {
                    continue;
                };
                // THE FIRST ROOT WINS: `~/models` is where the user put things by
                // hand, it comes before what was downloaded into the XDG root.
                if packages.iter().any(|p| p.name == package.name) {
                    continue;
                }
                packages.push(package);
            }
        }
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        packages
    }

    /// Turns a single folder into a package. With NO `.gguf` there is no package.
    fn package_from_dir(root: &Path, dir: &Path) -> Option<ModelPackage> {
        let name = dir.file_name()?.to_string_lossy().into_owned();
        let mut ggufs: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file())
            // The extension comparison is CASE INSENSITIVE: downloaded files
            // sometimes arrive as `.GGUF` and the user cannot be expected to know
            // that.
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("gguf"))
            })
            .collect();
        ggufs.sort();
        let gguf = ggufs.into_iter().next()?;
        let gguf_bytes = gguf.metadata().map(|m| m.len()).unwrap_or(0);
        let tok = dir.join("tokenizer.json");
        let tokenizer = tok.is_file().then_some(tok);
        // Asked ONLY when the answer can change the outcome (see the field's
        // note): with a `tokenizer.json` present the file wins either way.
        let gguf_tokenizer = tokenizer.is_none() && tacet_engine::gguf_has_tokenizer(&gguf);
        Some(ModelPackage {
            name,
            dir: dir.to_path_buf(),
            gguf,
            gguf_bytes,
            tokenizer,
            gguf_tokenizer,
            root: root.to_path_buf(),
        })
    }

    /// All installed packages (the default roots).
    pub fn catalog() -> Vec<ModelPackage> {
        scan(&model_roots())
    }

    /// What the engine needs: the weights, and a tokenizer file ONLY IF one was
    /// named. `None` on the second field means "read it out of the GGUF".
    pub type Weights = (String, Option<String>);

    /// The weights given DIRECTLY through environment variables.
    ///
    /// `TACET_TOKENIZER` USED TO BE MANDATORY HERE and it no longer is. The old
    /// rule ("both or neither") existed because half a pair could not load; now
    /// it can, because the vocabulary is in the `.gguf`. What has NOT changed is
    /// the direction of the override: a named `tokenizer.json` still wins, and
    /// if the named file does not exist the load FAILS instead of quietly using
    /// the one inside the weights (`ModelSetting::new`'s own rule — a typo must
    /// not turn into an unexplainable difference in output).
    ///
    /// `TACET_TOKENIZER` ALONE, with no `TACET_MODEL`, is still nothing: there
    /// are no weights to attach it to.
    ///
    /// This branch comes BEFORE the catalog — an explicit request is ahead of
    /// discovery.
    pub fn pair_from_env() -> Option<Weights> {
        let m = tacet_kernel::env_var(MODEL_VARIABLE)?;
        let t = tacet_kernel::env_var(TOKENIZER_VARIABLE);
        Some((
            m.to_string_lossy().into_owned(),
            t.map(|t| t.to_string_lossy().into_owned()),
        ))
    }

    /// The weights for `name` from the given package list.
    ///
    /// SEPARATE AND PURE: so the discovery logic can be tested without touching
    /// environment variables. Environment variables are PROCESS-WIDE and tests
    /// running in parallel step on each other.
    ///
    /// A package with NEITHER tokenizer is still refused (`is_complete`) — it
    /// is refused HERE rather than at load time so the user gets the catalog
    /// report instead of a 2.5 GB wait ending in an error.
    pub fn to_pair(packages: &[ModelPackage], name: &str) -> Option<Weights> {
        let p = packages.iter().find(|p| p.name == name)?;
        if !p.is_complete() {
            return None;
        }
        Some((
            p.gguf.to_string_lossy().into_owned(),
            p.tokenizer
                .as_ref()
                .map(|t| t.to_string_lossy().into_owned()),
        ))
    }

    /// PRODUCTION DISCOVERY: environment first, then the catalog.
    ///
    /// THE ARCHITECTURE IS NOT GUESSED HERE. The folder name ("qwen3-4b") is
    /// only a label; which module gets loaded is told by the GGUF metadata (see
    /// `Architecture::resolve`). If the name and the content diverge — if the
    /// user puts another weight in the folder — the right thing is to follow the
    /// content.
    pub fn resolve_pair(name: &str) -> Option<Weights> {
        if let Some(p) = pair_from_env() {
            return Some(p);
        }
        to_pair(&catalog(), name)
    }

    // -----------------------------------------------------------------------
    // The remote catalog (packages.json)
    // -----------------------------------------------------------------------

    /// The description of a downloadable package.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RemotePackage {
        pub name: String,
        pub files: Vec<RemoteFile>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RemoteFile {
        /// The name it will take on disk (`model.gguf`, `tokenizer.json`).
        pub name: String,
        pub url: String,
        /// The size declared by the catalog. Shown on the approval screen.
        pub bytes: Option<u64>,
        /// The SHA-256 declared by the publisher. If `None`, the digest
        /// COMPUTED on the first download is shown to the user and written next
        /// to the package (TOFU).
        pub sha256: Option<String>,
    }

    /// The catalog shipped with the binary, so a fresh install can fetch a
    /// working model without writing a JSON file first.
    ///
    /// THIS USED TO BE EMPTY, and the reason it was empty still stands as the bar
    /// every entry here had to clear: an invented address sends the user to a
    /// mirror nobody chose, and an invented digest fails verification on the
    /// first download and teaches the user to switch verification off. So none of
    /// the values below were written from memory. Each URL was requested and
    /// answered 200 without credentials, and each `content-length` matched the
    /// size recorded here. The digests are the registry's own `lfs.oid`, which
    /// for a Hugging Face LFS object IS the SHA-256 of the content — a fact that
    /// can be checked without downloading gigabytes, and which the first real
    /// download then confirms.
    ///
    /// `sha256: None` on the Qwen2.5 tokenizer is NOT an oversight: that file is
    /// stored inline rather than through LFS, so the registry publishes no
    /// digest for it. Rather than invent one, the download path falls back to
    /// trust-on-first-use — it computes the digest, shows it, and records it.
    ///
    /// A user's own `packages.json` still wins by name: this is a default, not a
    /// lock. And nothing here downloads on its own — the approval gate prints the
    /// address and the size and waits for a keypress.
    pub fn embedded_catalog() -> Vec<RemotePackage> {
        fn package(name: &str, files: [(&str, &str, u64, Option<&str>); 2]) -> RemotePackage {
            RemotePackage {
                name: name.to_string(),
                files: files
                    .into_iter()
                    .map(|(file, url, bytes, sha)| RemoteFile {
                        name: file.to_string(),
                        url: url.to_string(),
                        bytes: Some(bytes),
                        sha256: sha.map(str::to_string),
                    })
                    .collect(),
            }
        }

        vec![
            // The default (`DEFAULT_MODEL`). Q4_K_M: the smallest quantisation
            // that still answers well enough to be worth shipping as the one a
            // first-time user gets.
            package(
                "qwen3-4b",
                [
                    (
                        "model.gguf",
                        "https://huggingface.co/Qwen/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf",
                        2_497_280_256,
                        Some("7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5"),
                    ),
                    // The tokenizer lives in the base repository, not the GGUF
                    // one: a GGUF carries its vocabulary internally, but this
                    // engine wants a `tokenizer.json` on disk.
                    (
                        "tokenizer.json",
                        "https://huggingface.co/Qwen/Qwen3-4B/resolve/main/tokenizer.json",
                        11_422_654,
                        Some("aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4"),
                    ),
                ],
            ),
            // A smaller second option for machines where 2.5 GB of weights is
            // the constraint rather than the quality.
            package(
                "qwen2.5-3b",
                [
                    (
                        "model.gguf",
                        "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
                        2_104_932_768,
                        Some("626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d"),
                    ),
                    (
                        "tokenizer.json",
                        "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct/resolve/main/tokenizer.json",
                        7_031_645,
                        None,
                    ),
                ],
            ),
        ]
    }

    /// The full path of `packages.json` (in the config directory — this is a
    /// SETTING, not the weight itself; for the root distinction see
    /// `model_roots`).
    pub fn remote_catalog_path() -> Option<PathBuf> {
        tacet_kernel::env::config_path(CATALOG_FILE)
    }

    /// The example shown to the user. There is NO real URL: the field names and
    /// the shape are shown, the address belongs to the user.
    pub const EXAMPLE_CATALOG: &str = r#"{
  "packages": [
    {
      "name": "qwen3-4b",
      "files": [
        { "name": "model.gguf",     "url": "https://<your-own-mirror>/qwen3-4b.gguf", "bytes": 2497281120 },
        { "name": "tokenizer.json", "url": "https://<your-own-mirror>/tokenizer.json" }
      ]
    }
  ]
}"#;

    /// Reads the remote catalog: the user's `packages.json` MERGED over the
    /// embedded defaults. `Err` = the file EXISTS but is broken — not silently
    /// swallowed.
    ///
    /// Merged rather than replaced, and by NAME. Writing one entry of your own
    /// used to hide every default, so a user who added a private mirror silently
    /// lost `qwen3-4b` and had no way to tell that their file was the cause.
    /// Same name = yours wins; that is the override anyone writing the file
    /// actually means.
    pub fn read_remote_catalog() -> Result<Vec<RemotePackage>, String> {
        let Some(path) = remote_catalog_path() else {
            return Ok(embedded_catalog());
        };
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(embedded_catalog()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut merged =
            parse_remote_catalog(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
        let taken: std::collections::HashSet<String> =
            merged.iter().map(|p| p.name.clone()).collect();
        merged.extend(
            embedded_catalog()
                .into_iter()
                .filter(|p| !taken.contains(&p.name)),
        );
        Ok(merged)
    }

    /// A catalog name reduced to a SINGLE plain path component, or an error.
    ///
    /// WHY THIS GATE EXISTS: a catalog name BECOMES A PATH at download time —
    /// `root.join(package.name).join(file.name)`. `PathBuf::join` DISCARDS
    /// everything to its left when the joined component is absolute, so a name
    /// of `/Users/u/.zshenv` makes the download target exactly that file, and a
    /// name of `../../x` walks out of the model root (the downloader creates the
    /// target's parent, so the escape does not even need the directory to
    /// exist). `packages.json` is written by hand and gets pasted around, which
    /// makes it the lowest-privilege-looking file that can write anywhere on the
    /// disk. This is the same rule `ToolContext::resolve_path` enforces for
    /// tools; it has to hold here too.
    ///
    /// A BROKEN CATALOG IS AN ERROR, NOT A SKIPPED ENTRY: this file already
    /// refuses to swallow a malformed catalog silently, and a name that was
    /// quietly rewritten would be worse — the user would not learn that their
    /// override does not do what it says.
    fn plain_name(field: &str, value: &str) -> Result<String, String> {
        use std::path::{Component, Path};
        let mut components = Path::new(value).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(p)), None) if !value.is_empty() => {
                Ok(p.to_string_lossy().into_owned())
            }
            _ => Err(format!(
                "'{value}': the {field} must be a plain name — no '/', no '\\', no '..' and no absolute path"
            )),
        }
    }

    /// SEPARATE AND PUBLIC: so it can be tested without touching the file system.
    pub fn parse_remote_catalog(raw: &str) -> Result<Vec<RemotePackage>, String> {
        let root: serde_json::Value =
            serde_json::from_str(raw).map_err(|e| format!("JSON could note be read: {e}"))?;
        let array = root
            .get("packages")
            .and_then(|p| p.as_array())
            .ok_or_else(|| "no `packages` array".to_string())?;
        let mut output = Vec::new();
        for p in array {
            let name = p
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or_else(|| "the package has no `name` field".to_string())?;
            // THE PACKAGE NAME IS A DIRECTORY NAME (`root.join(name)`), so it
            // goes through the same gate as the file names below.
            let name = &plain_name("package name", name)?;
            let file_array = p
                .get("files")
                .and_then(|f| f.as_array())
                .ok_or_else(|| format!("'{name}': no `files` array"))?;
            let mut files = Vec::new();
            for f in file_array {
                let fname = f
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| format!("'{name}': the file has no `name` field"))?;
                // THE FILE NAME IS THE DOWNLOAD TARGET (`dir.join(&f.name)`):
                // an absolute name would make `join` throw away the model root
                // entirely and write anywhere the user can write.
                let fname =
                    &plain_name("file name", fname).map_err(|e| format!("'{name}': {e}"))?;
                let url = f
                    .get("url")
                    .and_then(|u| u.as_str())
                    .ok_or_else(|| format!("'{name}/{fname}': no `url` field"))?;
                files.push(RemoteFile {
                    name: fname.to_string(),
                    url: url.to_string(),
                    bytes: f.get("bytes").and_then(serde_json::Value::as_u64),
                    sha256: f
                        .get("sha256")
                        .and_then(|s| s.as_str())
                        .map(str::to_string)
                        .filter(|s| !s.is_empty()),
                });
            }
            output.push(RemotePackage {
                name: name.to_string(),
                files,
            });
        }
        Ok(output)
    }
}

/// Human-readable bytes. Packages are gigabytes: a raw number tells the user
/// nothing.
fn byte_text(b: u64) -> String {
    const UNIT: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut d = b as f64;
    let mut i = 0;
    while d >= 1024.0 && i + 1 < UNIT.len() {
        d /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{d:.1} {}", UNIT[i])
    }
}

/// Sets up the engine according to the choice. `Auto`: candle if a model exists,
/// fake otherwise (with a message).
fn setup_engine(
    choice: EngineChoice,
    script: Vec<String>,
    model_name: &str,
    color: &Color,
) -> Result<Arc<dyn EngineProvider>, String> {
    let fake = |s: Vec<String>| -> Arc<dyn EngineProvider> {
        Arc::new(FakeEngine::script(s).with_default("Understood. (fake engine)"))
    };
    match choice {
        EngineChoice::Fake => Ok(fake(script)),
        EngineChoice::Candle => match model_package::resolve_pair(model_name) {
            Some((m, t)) => candle_engine_from_path(&m, t.as_deref()),
            // `--engine candle` is an EXPLICIT request: with no model, erroring
            // out is right, not falling back to fake (see the `Auto` branch,
            // which does the opposite).
            //
            // THE CATALOG IS STILL PRINTED: the error message is one line, while
            // what the user needs is "which roots were searched, what was found".
            // This used to print a SINGLE guess, `~/models/<name>`.
            None => {
                model_not_found_report(model_name, color);
                Err(format!("local model note found: {model_name}"))
            }
        },
        EngineChoice::Auto => match model_package::resolve_pair(model_name) {
            Some((m, t)) => match candle_engine_from_path(&m, t.as_deref()) {
                Ok(engine) => {
                    eprintln!("{}", color.paint(DIM, &format!("(model: {m})")));
                    Ok(engine)
                }
                // If the candle feature is off or loading fails: falling back to
                // fake SILENTLY would be wrong — the user expects a real model.
                Err(e) => {
                    eprintln!(
                        "{}",
                        color.paint(YELLOW, &format!("(the real model could note be used: {e})"))
                    );
                    eprintln!(
                        "{}",
                        color.paint(DIM, "(fell back to FakeEngine — answers are fixed)")
                    );
                    Ok(fake(script))
                }
            },
            None => {
                model_not_found_report(model_name, color);
                eprintln!(
                    "{}",
                    color.paint(DIM, "(FakeEngine for now — answers are fixed)")
                );
                Ok(fake(script))
            }
        },
    }
}

/// Prints THE CATALOG when no model is found.
///
/// WHY SO MUCH DETAIL: this is the wall the user hits most often and the old
/// state gave them a one-line guess ("put it under ~/models/<name>"). That line
/// left three questions unanswered: which directories WERE SEARCHED, what was
/// found there, and why none of the finds could be selected. All three are
/// written here.
///
/// IF THERE IS AN ENV OVERRIDE IT IS SAID FIRST: showing the catalog while
/// `TACET_MODEL` is set would mislead — in that case discovery never ran at all.
fn model_not_found_report(requested: &str, color: &Color) {
    if let Some((m, t)) = model_package::pair_from_env() {
        // The tokenizer line reports what was ACTUALLY asked for. Printing a
        // `TACET_TOKENIZER` value the user never set would send them looking for
        // a variable that is not in their environment.
        let tokenizer_line = match &t {
            Some(t) => format!("\n   tokenizer: {t}"),
            None => format!(
                "\n   tokenizer: not set ({TOKENIZER_VARIABLE}) — the one inside the .gguf would be used"
            ),
        };
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "({MODEL_VARIABLE} is set but the files could note be loaded:\n   gguf : {m}{tokenizer_line})"
                )
            )
        );
        return;
    }

    let roots = model_package::model_roots();
    eprintln!(
        "{}",
        color.paint(
            YELLOW,
            &format!("(model package note found: '{requested}')")
        )
    );
    if roots.is_empty() {
        // Neither HOME/USERPROFILE nor XDG_DATA_HOME/LOCALAPPDATA resolved.
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "  no root to search — point at the files directly with {}/{}",
                    MODEL_VARIABLE, TOKENIZER_VARIABLE
                )
            )
        );
        return;
    }
    for r in &roots {
        eprintln!(
            "{}",
            color.paint(DIM, &format!("  searched: {}", r.display()))
        );
    }

    let packages = model_package::scan(&roots);
    if packages.is_empty() {
        // NO PACKAGES AT ALL. What to suggest depends on THE STATE OF THE
        // CATALOG: `tacet models download` now exists but downloads ONLY from the
        // user's own `packages.json` (the embedded catalog is deliberately
        // empty). So suggesting the command with an empty catalog would send the
        // user to a line that does nothing.
        eprintln!("{}", color.paint(DIM, "  no packages at all."));
        let catalog = model_package::read_remote_catalog();
        match &catalog {
            Ok(c) if !c.is_empty() => {
                let names: Vec<&str> = c.iter().map(|p| p.name.as_str()).collect();
                eprintln!(
                    "{}",
                    color.paint(DIM, &format!("  downloadable (packages.json): {}", names.join(", ")))
                );
                eprintln!(
                    "{}",
                    color.paint(DIM, &format!("  to download: tacet models download {}", names[0]))
                );
            }
            // A BROKEN CATALOG IS NOT PASSED OVER IN SILENCE: the user wrote the
            // file, and the sentence "no packages at all" does not tell them the
            // file was not read.
            Err(e) => eprintln!("{}", color.paint(YELLOW, &format!("  packages.json could note be read: {e}"))),
            Ok(_) => match model_package::remote_catalog_path() {
                Some(p) => eprintln!(
                    "{}",
                    color.paint(
                        DIM,
                        &format!(
                            "  you can write a download source into {}; for the shape: tacet models list --json",
                            p.display()
                        )
                    )
                ),
                None => eprintln!("{}", color.paint(DIM, "  the config directory could note be resolved.")),
            },
        }
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "  or put <name>/*.gguf + tokenizer.json in a folder: {}",
                    roots[0].display()
                )
            )
        );
        return;
    }

    eprintln!(
        "{}",
        color.paint(DIM, "  what was found (tacet models list):")
    );
    for p in &packages {
        let note = if p.is_complete() {
            ""
        } else {
            "  [no tokenizer, in the folder or in the .gguf — cannot be selected]"
        };
        eprintln!("{}", color.paint(DIM, &format!("    {}{note}", p.name)));
    }
    let selectable: Vec<&str> = packages
        .iter()
        .filter(|p| p.is_complete())
        .map(|p| p.name.as_str())
        .collect();
    if !selectable.is_empty() {
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!("  to select: tacet --model {}", selectable[0])
            )
        );
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
fn session_catalog(
    store: &Arc<SharedStore>,
    memory: &SharedMemory,
    color: &Color,
) -> (ToolCatalog, Option<Arc<CodeState>>) {
    // THE LIST ITSELF IS NO LONGER HERE (see tacet-tools/src/catalog.rs). The
    // shell and eval must see the same list: the tool SELECTION measurement
    // derives from the catalog the model sees; if two lists diverge, what is
    // measured is not the selection the application makes. The shell's only
    // remaining job here is telling the user WHY when run_code is not found.
    let (c, code_state, diagnosis) = tacet_tools::catalog::production_catalog(store, memory, None);
    if let Some(d) = diagnosis {
        eprintln!("{}", color.paint(DIM, &format!("({})", d.0)));
    }
    (c, code_state)
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
    let (mut c, cs) = session_catalog(store, memory, color);
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
/// DERIVED FROM THE WINDOW, not chosen for looks. `TokenCounter::estimate`
/// charges roughly one token per three bytes (biased high on purpose), and the
/// prompt half of the 4096-token window is `prompt_cap()` ≈ 3072 tokens — of
/// which the system block and the tool descriptions already eat ~2300 on a full
/// catalog. 8 KiB of pasted text is ~2700 estimated tokens: still larger than
/// the room actually left, which is deliberate. The point of the cap is not to
/// make the paste fit (truncation handles that, and it is allowed to bite here)
/// but to stop a `cat 10mb.log |` from making the shell allocate and hash
/// megabytes before the counter ever sees them.
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
///     directory                       bytes   tokens   % of 4096
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
/// are the bulkiest thing in a transcript and the 4096-token window is tight —
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

// ---------------------------------------------------------------------------
// chat
// ---------------------------------------------------------------------------

/// Everything `chat` needs. A STRUCT, not nine positional arguments: the list
/// had already reached six `&str`/`bool`/`Option<String>` values where a swapped
/// pair would still compile.
struct ChatRun {
    choice: EngineChoice,
    script: Vec<String>,
    show_prompt: bool,
    dir: String,
    single_message: Option<String>,
    model_name: String,
    json: bool,
    continue_session: bool,
    session_id: Option<String>,
}

#[allow(clippy::too_many_lines)]
fn chat(run: ChatRun) -> ExitCode {
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

    let engine = match setup_engine(choice, script, model_name, &color) {
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
    let mcp_load = mcp::load_from_default();
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

    let router = Router::new();
    let counter = TokenCounter::default();
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
    // the 4096 window: when the budget filled, old turns dropped SILENTLY (see
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
            println!("{}{}", color.paint(BOLD, "Tacet"), color.paint(BRASS, "."));
            println!(
                "{}",
                color.paint(
                    DIM,
                    &format!(
                        "{} · {} tools · /help",
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
                         the 4096-token window cannot hold more)",
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
        if message.starts_with('/') {
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
            let cleared = message.trim() == "/clear";
            // An /addon verb with arguments changes the registry; the running
            // session must see the change (the transcript that forced this:
            // `/addon on web-search` said "opened" while the session kept
            // answering "the addon is CLOSED" until a restart).
            let addon_touched = message.trim_start().starts_with("/addon ");
            match slash(
                &message,
                &catalog,
                &memory,
                &mut history,
                &engine,
                &color,
                &last_artifact,
            ) {
                SlashResult::Quit => break,
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
                                &format!("(catalog refreshed — {} tools)", catalog.tools().len())
                            )
                        );
                    }
                    if single_message.is_some() {
                        break;
                    }
                    continue;
                }
                SlashResult::Unknown => {
                    println!("{}", color.paint(DIM, "(unknown command; /help)"));
                    if single_message.is_some() {
                        break;
                    }
                    continue;
                }
            }
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
        // `--show-prompt` output. The double write eats room in the 4096 window
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
        let asked = match &piped {
            Some(p) => format!("{}\n\n{message}", stdin_fence(p)),
            None => message.clone(),
        };

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
        for _ in 0..tacet_eval::MAX_TURNS {
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
            let mut prompt = Prompt::new(&system, question)
                .with_tools(&selected)
                .with_history(previous);
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
            // nothing.
            let mut indicator = TurnIndicator::start(Arc::clone(&screen), &CANCEL, "thinking");

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
            let listener = |chunk: &str| {
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
                    constraint
                        .as_ref()
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
                    SamplingSetting {
                        cancel: Some(&CANCEL),
                        max_tokens: counter.generation_cap(&prompt),
                        ..Default::default()
                    },
                    &listener,
                ),
            ) {
                Ok(g) => g,
                Err(e) => {
                    indicator.finish();
                    eprintln!("\nengine error: {e}");
                    turn_error = Some(e.to_string());
                    any_turn_failed = true;
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
                break;
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
                break;
            };
            indicator.finish();
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
                break;
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
            println!(
                "{}",
                serde_json::json!({
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
                    // ABSENT ON SUCCESS, so `has("error")` is the check a script
                    // makes. Present and non-null means `answer` is not an
                    // answer — do not treat an empty string as one.
                    "error": turn_error,
                })
            );
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
///   * `context` — the LAST prompt's place in the 4096 window. This is the
///     critical one: when the window fills, old turns drop SILENTLY (see
///     `TokenCounter::truncate`) and the user only noticed it when the model
///     "forgot" something.
///
/// The numbers ARE ESTIMATES (see `TokenCounter::estimate` — deliberately biased
/// high); no separate counter was invented.
fn status_line(
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
    format!(
        "this turn {}+{} · session {session} · context {context}/{CONTEXT_BUDGET} tokens{fullness}",
        turn_prompt, turn_generation
    )
}

// ---------------------------------------------------------------------------
// Slash commands
// ---------------------------------------------------------------------------

enum SlashResult {
    Quit,
    Handled,
    Unknown,
}

fn slash(
    command: &str,
    catalog: &ToolCatalog,
    memory: &SharedMemory,
    history: &mut Vec<Turn>,
    engine: &Arc<dyn EngineProvider>,
    color: &Color,
    last_artifact: &Option<std::path::PathBuf>,
) -> SlashResult {
    let name = command.split_whitespace().next().unwrap_or("");
    match name {
        // `/exit` IS KEPT AS AN ALIAS: it is the habit carried over from other
        // shells and refusing it costs nothing.
        "/quit" | "/exit" => SlashResult::Quit,
        "/help" => {
            println!("{}", color.paint(BOLD, "commands"));
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
fn report_mcp(load: &mcp::LoadOutcome, color: &Color) {
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

// ---------------------------------------------------------------------------
// candle engine setup
// ---------------------------------------------------------------------------

/// `tokenizer: None` means "the vocabulary is inside the weights". The choice is
/// made ONE LEVEL UP, in discovery (`model_package::to_pair`), so that the two
/// places that answer "is this package usable" — the catalog report the user
/// reads and the loader — cannot drift apart.
#[cfg(feature = "candle")]
fn candle_engine_from_path(
    model: &str,
    tokenizer: Option<&str>,
) -> Result<Arc<dyn EngineProvider>, String> {
    let setting = match tokenizer {
        Some(t) => tacet_engine::ModelSetting::new(model, t),
        None => tacet_engine::ModelSetting::from_gguf(model),
    };
    // File existence is checked BEFORE a 2.5 GB load; learning about a missing
    // file at the end of that wait is a pointless delay.
    tacet_engine::CandleEngine::files_exist(&setting).map_err(|e| e.to_string())?;
    let engine = tacet_engine::CandleEngine::load(&setting).map_err(|e| e.to_string())?;
    // WHICH ARCHITECTURE was loaded is printed. Had it stayed silent, a model
    // running with the wrong template would look like "it gives odd answers" and
    // be hard to diagnose.
    //
    // WHICH TOKENIZER is printed for the same reason and it is the sharper of
    // the two: the two sources are indistinguishable from the output — a
    // vocabulary rebuilt from the wrong place does not error, it produces text
    // that reads like broken weights.
    eprintln!(
        "(architecture: {}, template: {:?}, tokenizer: {})",
        engine.architecture().name(),
        engine.architecture().template(),
        engine.tokenizer_source().name()
    );
    Ok(Arc::new(engine) as Arc<dyn EngineProvider>)
}

#[cfg(not(feature = "candle"))]
fn candle_engine_from_path(
    _model: &str,
    _tokenizer: Option<&str>,
) -> Result<Arc<dyn EngineProvider>, String> {
    Err("this binary was built without the `candle` feature".into())
}

// ---------------------------------------------------------------------------
// eval
// ---------------------------------------------------------------------------

fn eval(json: bool, threshold: f64) -> ExitCode {
    let report = tacet_eval::run(&tacet_eval::all(), &FakeSelector);
    if json {
        println!("{}", report.json());
    } else {
        print!("{}", report.table());
    }
    if report.success_rate + f64::EPSILON >= threshold {
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "threshold not met: {:.3} < {threshold:.3}",
            report.success_rate
        );
        ExitCode::FAILURE
    }
}

/// Runs the tool selection set with a REAL model.
///
/// IT DOES NOT FALL BACK TO THE FAKE ENGINE. For the `chat` command, falling
/// back to FakeEngine silently is the right behaviour (the user at least sees the
/// shell); not here: a SELECTION measurement run with the fake engine measures
/// its own script and prints "accuracy 100%". A wrong number is worse than no
/// number.
///
/// THE THRESHOLD IS APPLIED ONLY TO IRRELEVANCE. Tool accuracy depends on model
/// capacity and rises over time; irrelevance is a limit that CANNOT BE BROKEN —
/// an assistant that calls a tool on a greeting is unusable whatever its
/// accuracy. That is why the exit code is tied to irrelevance.
fn eval_tool_selection(
    json: bool,
    threshold: f64,
    model_name: &str,
    only: Option<&str>,
) -> ExitCode {
    let color = Color::setup();
    let engine = match model_package::resolve_pair(model_name) {
        Some((m, t)) => match candle_engine_from_path(&m, t.as_deref()) {
            Ok(engine) => {
                eprintln!("{}", color.paint(DIM, &format!("(model: {m})")));
                engine
            }
            Err(e) => {
                eprintln!("error: the real model could not be loaded: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => {
            // The catalog is printed HERE TOO: before the measurement runs the
            // user needs to see which package they could select.
            model_not_found_report(model_name, &color);
            eprintln!("error: the tool selection measurement REQUIRES a real model.");
            return ExitCode::FAILURE;
        }
    };

    let mut cases = tacet_eval::selection_cases();
    if let Some(pattern) = only {
        cases.retain(|c| c.name.contains(pattern));
        if cases.is_empty() {
            eprintln!("error: no case matches the pattern '{pattern}'");
            return ExitCode::FAILURE;
        }
    }
    eprintln!(
        "{}",
        color.paint(
            DIM,
            &format!("({} cases running — takes minutes)", cases.len())
        )
    );

    let report = tacet_eval::run_selection(&cases, &engine);
    if json {
        println!("{}", report.json());
    } else {
        print!("{}", report.table());
    }
    if report.irrelevance_rate() + f64::EPSILON >= threshold {
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "irrelevance threshold not met: {:.3} < {threshold:.3}",
            report.irrelevance_rate()
        );
        ExitCode::FAILURE
    }
}

// ---------------------------------------------------------------------------
// tools
// ---------------------------------------------------------------------------

fn tools(print_schema: bool) -> ExitCode {
    let color = Color::setup();
    let store = Arc::new(SharedStore::new());
    let memory = SharedMemory::in_memory();
    let (mut catalog, _) = session_catalog(&store, &memory, &color);
    // MCP tools must be visible HERE TOO: this command is the verbatim source of
    // "what the prompt says"; it must not print something different from the
    // catalog chat sees.
    let mcp_load = mcp::load_from_default();
    let _ = mcp::feed_catalog(&mut catalog, &mcp_load);
    report_mcp(&mcp_load, &color);
    for tool in catalog.tools() {
        println!(
            "{}{}",
            tool.name(),
            if tool.taints_session() {
                "  [taints]"
            } else {
                ""
            }
        );
        println!("  {}", tool.description());
        if print_schema {
            println!("  args: {}", tool.schema().json_schema());
        } else {
            for field in tool.schema().fields() {
                println!(
                    "  - {}{}: {}",
                    field.name,
                    if field.required { "*" } else { "" },
                    kind_text(&field.schema)
                );
            }
        }
        println!();
    }
    println!("{} tools", catalog.tools().len());
    // WHY WHAT IS MISSING IS MISSING IS ALSO THIS COMMAND'S JOB. The user looks at
    // the list asking "where is web_search"; a closed gate must not be as silent
    // as an empty list.
    if !tacet_web::addon::web_search_is_open() {
        println!(
            "{}",
            color.paint(DIM, &format!("({})", addon::closed_gate_message(false)))
        );
    }
    ExitCode::SUCCESS
}

fn kind_text(schema: &ArgSchema) -> String {
    use tacet_kernel::SchemaKind::*;
    match &schema.kind {
        Object { fields } => format!("object({} fields)", fields.len()),
        Array { .. } => "array".into(),
        Text { .. } => "text".into(),
        Choice { choices } => format!("choice[{}]", choices.join("|")),
        Number { is_integer, .. } => if *is_integer { "integer" } else { "number" }.into(),
        Bool => "bool".into(),
    }
}

// ---------------------------------------------------------------------------
// package — skill packages
// ---------------------------------------------------------------------------

/// `tacet package list` — prints the installed skill packages.
///
/// WHY IT EXISTS: the skill layer is silent. A matching skill goes into the
/// prompt and the user can only see what was loaded with `--show-prompt`, and
/// only if that skill matched ON THAT TURN. A broken `.md` file dropped into the
/// user directory is DELIBERATELY skipped in silence (see
/// `SkillStore::load_from_dir`) — the right trade, but it left the answer to "I
/// put my file there and it doesn't work" nowhere. This command is where that
/// answer lives.
///
/// THE DIRECTORY ADDRESS IS PRINTED TOO: the config directory varies by platform
/// (XDG / `%APPDATA%` / `TACET_HOME`) and the user cannot be expected to guess
/// WHERE to put the file.
fn package_list(json: bool) -> ExitCode {
    let color = Color::setup();
    let mut store = SkillStore::default_set();
    let embedded = store.count();

    // The user directory: `tacet_skills::user_dir` -> `tacet_kernel::env`. The path
    // is NOT computed HERE; it comes from the same single source as memory and
    // MCP.
    let dir = tacet_skills::user_dir();
    let mut loaded = 0usize;
    if let Some(d) = &dir
        && d.is_dir()
    {
        loaded = store.load_from_dir(d);
    }

    if json {
        let records: Vec<serde_json::Value> = store
            .all()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "source": if s.is_users { "user" } else { "embedded" },
                    "triggers": s.triggers,
                    "tools": s.tools,
                    "body_chars": s.text.chars().count(),
                })
            })
            .collect();
        let output = serde_json::json!({
            "dir": dir.as_ref().map(|d| d.display().to_string()),
            "embedded": embedded,
            "user": loaded,
            "packages": records,
        });
        println!("{output}");
        return ExitCode::SUCCESS;
    }

    match &dir {
        Some(d) => {
            let note = if d.is_dir() { "" } else { " (missing)" };
            println!(
                "{}",
                color.paint(DIM, &format!("user directory: {}{note}", d.display()))
            );
        }
        // Neither `TACET_HOME` nor `HOME`/`APPDATA` could be resolved: this is not
        // an error but a "user skills cannot be loaded" state, and it has to be
        // said.
        None => println!(
            "{}",
            color.paint(
                DIM,
                "user directory: could not be resolved (TACET_HOME can be set)"
            )
        ),
    }
    println!();

    for s in store.all() {
        let source = if s.is_users { "user" } else { "embedded" };
        println!(
            "{}  {}",
            color.paint(BOLD, &s.name),
            color.paint(DIM, source)
        );
        println!(
            "  {} {}",
            color.paint(DIM, "triggers:"),
            s.triggers.join(", ")
        );
        if !s.tools.is_empty() {
            // A skill is never selected if the tools it MANDATES are not in the
            // catalog; a missing tool is the most common answer to "why does this
            // skill never match".
            println!(
                "  {} {}",
                color.paint(DIM, "required tool:"),
                s.tools.join(", ")
            );
        }
        println!(
            "  {}",
            color.paint(DIM, &format!("{} characters", s.text.chars().count()))
        );
        println!();
    }

    println!(
        "{embedded} embedded · {loaded} user · {} total",
        store.count()
    );
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// model — model (weight) packages
// ---------------------------------------------------------------------------

/// `tacet models list` — prints the installed model packages.
///
/// WHY IT EXISTS: model discovery was silent. The shell either said
/// "(model: /long/path.gguf)" or "not found"; NOTHING IN BETWEEN was visible —
/// which roots were scanned, what else is there, whether a half package exists,
/// which `.gguf` was picked. This is where the answer to "my folder is right
/// there but it doesn't see it" lives.
///
/// NO NETWORK: this command is entirely local. The remote catalog
/// (`packages.json`) is only READ and which packages have a source is shown; no
/// address is called.
fn model_list(json: bool, selected_name: &str) -> ExitCode {
    let color = Color::setup();
    let roots = model_package::model_roots();
    let packages = model_package::scan(&roots);
    let env = model_package::pair_from_env();
    let remote = model_package::read_remote_catalog();

    if json {
        let records: Vec<serde_json::Value> = packages
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "dir": p.dir.display().to_string(),
                    "gguf": p.gguf.display().to_string(),
                    "gguf_bytes": p.gguf_bytes,
                    "tokenizer": p.tokenizer.as_ref().map(|t| t.display().to_string()),
                    // A SEPARATE FIELD rather than a fabricated `tokenizer`
                    // path: there is no file to name, and writing the .gguf's
                    // path into a field called `tokenizer` would make a script
                    // hand that path to `TACET_TOKENIZER`.
                    "gguf_tokenizer": p.gguf_tokenizer,
                    "complete": p.is_complete(),
                    "root": p.root.display().to_string(),
                    // "Selected": if there is an env override NONE is selected —
                    // in that case discovery is not in play at all.
                    "selected": env.is_none() && p.name == selected_name && p.is_complete(),
                })
            })
            .collect();
        let output = serde_json::json!({
            "roots": roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>(),
            "requested": selected_name,
            "env_override": env.as_ref().map(|(m, t)| serde_json::json!({ "gguf": m, "tokenizer": t })),
            "packages": records,
            "remote_catalog": {
                "path": model_package::remote_catalog_path().map(|p| p.display().to_string()),
                "error": remote.as_ref().err(),
                "packages": remote.as_ref().map(|r| r.iter().map(|p| p.name.clone()).collect::<Vec<_>>()).unwrap_or_default(),
                "example": model_package::EXAMPLE_CATALOG,
            },
        });
        println!("{output}");
        return ExitCode::SUCCESS;
    }

    if roots.is_empty() {
        println!("{}", color.paint(DIM, "model root: could not be resolved"));
    } else {
        for r in &roots {
            let note = if r.is_dir() { "" } else { " (missing)" };
            println!(
                "{}",
                color.paint(DIM, &format!("model root: {}{note}", r.display()))
            );
        }
    }
    if let Some((m, t)) = &env {
        // With an override in place the catalog IS NOT SILENCED, but the warning
        // comes first: none of the list below is being used.
        let tokenizer_line = match t {
            Some(t) => format!("  tokenizer : {t}"),
            None => format!("  tokenizer : inside the .gguf ({TOKENIZER_VARIABLE} not set)"),
        };
        println!(
            "{}",
            color.paint(
                YELLOW,
                &format!("{MODEL_VARIABLE} set — discovery disabled:\n  gguf      : {m}\n{tokenizer_line}")
            )
        );
    }
    println!();

    if packages.is_empty() {
        println!("{}", color.paint(DIM, "(no model package installed)"));
    }
    for p in &packages {
        let selected = env.is_none() && p.name == selected_name && p.is_complete();
        let mark = if selected {
            color.paint(BOLD, " ← selected")
        } else {
            String::new()
        };
        println!(
            "{}  {}{}",
            color.paint(BOLD, &p.name),
            color.paint(DIM, &byte_text(p.gguf_bytes)),
            mark
        );
        println!("  {}", color.paint(DIM, &p.gguf.display().to_string()));
        // A HALF PACKAGE IS SAID PLAINLY: the `.gguf` is there but no engine can
        // be set up, and the user would only learn that by trying
        // `--engine candle` and getting an error. A `.gguf` carrying its own
        // vocabulary is NOT half and is no longer described as if it were.
        let note = p.tokenizer_note();
        println!(
            "  {}",
            color.paint(if p.is_complete() { DIM } else { YELLOW }, note)
        );
        println!();
    }

    let complete = packages.iter().filter(|p| p.is_complete()).count();
    println!("{} packages · {complete} usable", packages.len());
    if env.is_none()
        && !packages
            .iter()
            .any(|p| p.name == selected_name && p.is_complete())
    {
        println!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "the requested '{selected_name}' is not usable — chat falls back to FakeEngine"
                )
            )
        );
    }

    // THE REMOTE CATALOG: if the file exists it says what it recognises, if it is
    // broken it DOES NOT STAY SILENT, if it is missing it shows where to write it.
    println!();
    match &remote {
        Err(e) => println!(
            "{}",
            color.paint(YELLOW, &format!("packages.json could not be read: {e}"))
        ),
        Ok(r) if r.is_empty() => match model_package::remote_catalog_path() {
            Some(p) => {
                println!(
                    "{}",
                    color.paint(DIM, &format!("no download catalog: {}", p.display()))
                );
                println!("{}", color.paint(DIM, "shape:"));
                for line in model_package::EXAMPLE_CATALOG.lines() {
                    println!("{}", color.paint(DIM, &format!("  {line}")));
                }
            }
            None => println!(
                "{}",
                color.paint(DIM, "the config directory could not be resolved")
            ),
        },
        Ok(r) => {
            let names: Vec<&str> = r.iter().map(|p| p.name.as_str()).collect();
            println!(
                "{}",
                color.paint(
                    DIM,
                    &format!("in the download catalog: {}", names.join(", "))
                )
            );
            // THE COMMAND NOW EXISTS, which is why it is SUGGESTED. In the
            // previous round this only said "here is what I recognise", because
            // `model download` did not exist and suggesting a nonexistent command
            // would send the user to something that does nothing.
            if let Some(first) = names.first() {
                println!(
                    "{}",
                    color.paint(DIM, &format!("to download: tacet models download {first}"))
                );
            }
        }
    }
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// model download — THE PRODUCTION CALLER of `tacet_web::download`
// ---------------------------------------------------------------------------

/// The terminal end of the download approval gate.
///
/// THIS SIDE ASKS THE QUESTION, `tacet-web` is the side that goes on the network.
/// Without the split, either the network layer would read stdin or the shell
/// would open a socket. `tacet_web::download` does not even set up the ureq agent
/// before this gate returns `true`.
struct TerminalDownloadApproval {
    color: Color,
    /// `--no-approval`: no question is asked. WHAT IS BEING DOWNLOADED IS STILL
    /// PRINTED — even in script mode the record has to land in the log.
    no_approval: bool,
    /// What is said when the plan carries NO expected digest.
    ///
    /// WHY THIS IS A FIELD AND NOT ONE FIXED SENTENCE: two callers share this
    /// gate and the truth differs between them. On the MODEL path there is a
    /// catalog with a `sha256` field, so "first trust" is real — the user can
    /// paste the computed digest into `packages.json` and the next download is
    /// verified. On the UPDATE path there is no catalog, no field to fill, and
    /// every release has a different digest, so verification NEVER happens.
    /// Printing the model sentence there told the user a TOFU chain existed
    /// when none did, which is exactly the belief that makes an unverified
    /// binary look checked.
    no_digest_note: &'static str,
}

/// The model path: the catalog HAS a digest field, so first trust is real.
const TOFU_NOTE_CATALOG: &str = "no sha256 in the catalog — the downloaded file's digest will be COMPUTED and shown (first trust)";

/// The update path: there is no catalog and no digest to compare against, now
/// or later. SAID PLAINLY, because the closing `sha256:` line looks identical
/// to the output of a verified download.
const TOFU_NOTE_NO_PUBLISHER: &str = "no published digest for this binary — its digest will be COMPUTED and SHOWN, NOT COMPARED. Nothing is remembered for next time either: a new version has a new digest, so this download rests on TLS alone";

impl tacet_web::DownloadApproval for TerminalDownloadApproval {
    fn approve(&self, plan: &tacet_web::DownloadPlan, existing_bytes: u64) -> bool {
        let size = match plan.expected_bytes {
            Some(b) => byte_text(b),
            // If the catalog declared no size, NO FAKE NUMBER IS PRODUCED. The
            // user sees "size unknown" and decides; an estimated figure would make
            // the only quantitative fact their approval rests on a fabrication.
            None => "size unknown".to_string(),
        };
        eprintln!();
        eprintln!(
            "  {} {}  ({size})",
            self.color.paint(BOLD, "to download:"),
            plan.name
        );
        eprintln!("    source: {}", plan.url);
        eprintln!("    target: {}", plan.target.display());
        if existing_bytes > 0 {
            eprintln!(
                "    {}",
                self.color.paint(
                    DIM,
                    &format!(
                        "a half file exists: {} — it will be resumed",
                        byte_text(existing_bytes)
                    )
                )
            );
        }
        if plan.expected_sha256.is_none() {
            // TOFU IS SAID PLAINLY. Giving the impression that "the digest was
            // verified" would hide that the first download is unprotected.
            eprintln!("    {}", self.color.paint(YELLOW, self.no_digest_note));
        }
        if self.no_approval {
            eprintln!(
                "    {}",
                self.color
                    .paint(DIM, "--no-approval: downloading without asking")
            );
            return true;
        }
        eprint!("  Download it? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
    }
}

/// The terminal end of the download progress.
///
/// ONE LINE, overwritten with `\r`: a 2.5 GB download printing five lines a
/// second would make the terminal unusable. A speed readout WAS NOT ADDED —
/// showing an unmeasured time estimate ("3 min left") would break this repo's
/// "do not write what you did not measure" rule at the interface level.
struct TerminalDownloadProgress {
    color: Color,
}

/// The RIGHT side of the progress line. SEPARATE AND PURE: so it can be tested
/// independently of the drawing — what needs measuring is not the caret's place
/// but that NO PERCENTAGE IS PRODUCED when the total is unknown.
fn progress_text(downloaded: u64, total: Option<u64>) -> String {
    match total {
        // The `t > 0` condition is not decoration, it is the answer to dividing by
        // zero: on a server declaring length 0 it would print `%NaN`.
        Some(t) if t > 0 => {
            format!(
                "{} / {}  ({:.0}%)",
                byte_text(downloaded),
                byte_text(t),
                (downloaded as f64 / t as f64) * 100.0
            )
        }
        // If the server gave no `Content-Length`, a percentage IS NOT INVENTED.
        _ => byte_text(downloaded),
    }
}

impl TerminalDownloadProgress {
    fn line(&self, name: &str, downloaded: u64, total: Option<u64>) {
        eprint!(
            "\r  {} {}   ",
            self.color.paint(DIM, name),
            progress_text(downloaded, total)
        );
        let _ = std::io::stderr().flush();
    }
}

impl tacet_web::Progress for TerminalDownloadProgress {
    fn started(&self, name: &str, downloaded: u64, total: Option<u64>) {
        self.line(name, downloaded, total);
    }
    fn advanced(&self, downloaded: u64, total: Option<u64>) {
        self.line("", downloaded, total);
    }
    fn digesting(&self, bytes: u64) {
        // IT HAS TO BE REPORTED: the SHA-256 of a GB-sized file takes seconds and
        // if the line stayed at "download finished" the program would look hung.
        eprint!(
            "\r  {}   ",
            self.color
                .paint(DIM, &format!("computing digest ({})…", byte_text(bytes)))
        );
        let _ = std::io::stderr().flush();
    }
    fn finished(&self, _outcome: &tacet_web::DownloadOutcome) {
        eprintln!();
    }
}

/// The root the download lands in: the FIRST of `model_roots()`.
///
/// WHY THE FIRST, NOT "the first directory that exists": `scan` takes its
/// priority order from the same list and if the same name exists in two roots the
/// EARLIER root wins. Downloading into the second root would leave the downloaded
/// package IN THE SHADOW of a half folder with the same name in the first root —
/// the user would say "I downloaded it but it doesn't show up".
fn download_root() -> Option<std::path::PathBuf> {
    model_package::model_roots().into_iter().next()
}

/// `tacet models download <name>` — downloads the package from `packages.json`.
///
/// A PRODUCTION CALL: this function really does call `tacet_web::download`. The
/// module sat "tested but not wired" for a whole round; it is written out step by
/// step so whoever looks here can see the entire chain in one place:
/// catalog → package → root → per-file plan → approval → download → digest report.
fn model_download(name: &str, no_approval: bool) -> ExitCode {
    let color = Color::setup();
    let catalog = match model_package::read_remote_catalog() {
        Ok(c) => c,
        // A BROKEN CATALOG IS NOT SILENTLY SWALLOWED: if the file EXISTS but
        // cannot be read, saying "package not found" would send the user looking
        // in the wrong place.
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(YELLOW, &format!("packages.json could not be read: {e}"))
            );
            return ExitCode::FAILURE;
        }
    };

    let Some(package) = catalog.iter().find(|p| p.name == name) else {
        eprintln!(
            "{}",
            color.paint(YELLOW, &format!("'{name}' is not in the download catalog"))
        );
        if catalog.is_empty() {
            match model_package::remote_catalog_path() {
                Some(p) => {
                    eprintln!(
                        "{}",
                        color.paint(
                            DIM,
                            &format!("  the catalog is empty or missing: {}", p.display())
                        )
                    );
                    eprintln!("{}", color.paint(DIM, "  shape:"));
                    for line in model_package::EXAMPLE_CATALOG.lines() {
                        eprintln!("{}", color.paint(DIM, &format!("    {line}")));
                    }
                }
                None => eprintln!(
                    "{}",
                    color.paint(DIM, "  the config directory could not be resolved")
                ),
            }
        } else {
            let names: Vec<&str> = catalog.iter().map(|p| p.name.as_str()).collect();
            eprintln!(
                "{}",
                color.paint(DIM, &format!("  in the catalog: {}", names.join(", ")))
            );
        }
        return ExitCode::FAILURE;
    };

    if package.files.is_empty() {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!("the `files` list of package '{name}' is empty")
            )
        );
        return ExitCode::FAILURE;
    }

    let Some(root) = download_root() else {
        eprintln!(
            "{}",
            color.paint(YELLOW, "the download root could not be resolved")
        );
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "  neither HOME/USERPROFILE nor XDG_DATA_HOME/LOCALAPPDATA resolved — point at the files directly with {}/{}",
                    MODEL_VARIABLE, TOKENIZER_VARIABLE
                )
            )
        );
        return ExitCode::FAILURE;
    };
    let dir = root.join(name);

    println!(
        "{}",
        color.paint(BOLD, &format!("{name} → {}", dir.display()))
    );
    println!(
        "{}",
        color.paint(DIM, &format!("{} files", package.files.len()))
    );

    let approval = TerminalDownloadApproval {
        color: Color::setup(),
        no_approval,
        no_digest_note: TOFU_NOTE_CATALOG,
    };
    let progress = TerminalDownloadProgress {
        color: Color::setup(),
    };
    // TOFU RECORDS: the computed digest of the files that have NO expected digest.
    // Printed together at the end so the user can paste them into
    // `packages.json` — on the second download the verification becomes real.
    let mut tofu: Vec<(String, String)> = Vec::new();

    for f in &package.files {
        let plan = tacet_web::DownloadPlan {
            name: f.name.clone(),
            url: f.url.clone(),
            target: dir.join(&f.name),
            expected_bytes: f.bytes,
            expected_sha256: f.sha256.clone(),
        };
        // DEFENCE IN DEPTH, AND IT IS THE LAST GATE THAT SEES A REAL PATH.
        // `parse_remote_catalog` already refuses a name that is not a plain
        // component, but THIS is the value that reaches the file system, and a
        // download that escapes the model root writes an executable file
        // wherever the user can write (`~/.zshenv` runs at the next shell). The
        // check is cheap and it survives any future edit that builds `target`
        // some other way.
        if !plan.target.starts_with(&dir) {
            eprintln!();
            eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    &format!(
                        "'{}': the download target falls outside the model directory — refused",
                        f.name
                    )
                )
            );
            return ExitCode::FAILURE;
        }
        match tacet_web::download(&plan, &approval, &progress) {
            Ok(o) => {
                let note = if o.already_present {
                    // NO NETWORK CALL WAS MADE: the file was in place, it was only
                    // digested. A second run of the command is thus a VERIFICATION
                    // round.
                    "already present — no network call"
                } else if o.resumed {
                    "resumed from a half file"
                } else {
                    "downloaded"
                };
                let digest_note = if o.digest_verified {
                    "sha256 verified"
                } else {
                    "sha256 not in the catalog"
                };
                println!(
                    "  {} {}  ({}, {note}, {digest_note})",
                    color.paint(BOLD, "✓"),
                    f.name,
                    byte_text(o.bytes)
                );
                if !o.digest_verified {
                    tofu.push((f.name.clone(), o.sha256.clone()));
                }
            }
            Err(e) => {
                // WE STOP AT THE FIRST FAILURE: carrying on with a half package
                // would download the remaining files too and give the user the
                // impression that it is "ready".
                eprintln!();
                eprintln!("{}", color.paint(YELLOW, &format!("{}: {e}", f.name)));
                return ExitCode::FAILURE;
            }
        }
    }

    if !tofu.is_empty() {
        println!();
        println!(
            "{}",
            color.paint(YELLOW, "these files had no digest in the catalog — the first download WAS NOT VERIFIED (TOFU).")
        );
        println!(
            "{}",
            color.paint(
                DIM,
                "if you write them into packages.json, later downloads are verified:"
            )
        );
        for (file, digest) in &tofu {
            println!(
                "{}",
                color.paint(DIM, &format!("  \"{file}\": \"sha256\": \"{digest}\""))
            );
        }
    }

    // THE RESULT IS STATED IN TERMS OF USABILITY. "Download finished" is not
    // enough: a package missing its `tokenizer.json` sits on disk but cannot set
    // up an engine, and the user would only learn that by trying `--engine candle`
    // and getting an error. The catalog is RESCANNED; the claim comes from the
    // file system.
    println!();
    let rescan = model_package::scan(&[root]);
    match rescan.iter().find(|p| p.name == name) {
        Some(p) if p.is_complete() => {
            println!(
                "{}",
                color.paint(BOLD, &format!("ready: tacet --model {name}"))
            );
            ExitCode::SUCCESS
        }
        Some(_) => {
            println!(
                "{}",
                color.paint(
                    YELLOW,
                    "the package is HALF: no tokenizer.json — it cannot be selected"
                )
            );
            ExitCode::FAILURE
        }
        None => {
            println!(
                "{}",
                color.paint(
                    YELLOW,
                    "the package does not show up in the scan: no `.gguf` file in the folder"
                )
            );
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// grammar
// ---------------------------------------------------------------------------

fn grammar(name: &str, try_input: Option<&str>) -> ExitCode {
    let color = Color::setup();
    let store = Arc::new(SharedStore::new());
    let memory = SharedMemory::in_memory();
    let (catalog, _) = session_catalog(&store, &memory, &color);
    if write_grammar(name, &catalog, try_input, &color) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The short form called from the shell (`/grammar <tool>`); no trial input.
fn print_grammar(name: &str, catalog: &ToolCatalog, color: &Color) {
    write_grammar(name, catalog, None, color);
}

/// The SINGLE source of the grammar output. The subcommand and the slash command
/// have to print the same text: if the two diverge, "the diagnostic output" tells
/// two different truths and which one is right becomes unclear.
fn write_grammar(
    name: &str,
    catalog: &ToolCatalog,
    try_input: Option<&str>,
    color: &Color,
) -> bool {
    let Some(tool) = catalog.find(name) else {
        eprintln!(
            "unknown tool: {name}\ncatalog: {}",
            catalog.names().join(", ")
        );
        return false;
    };
    let _ = color;

    let schema = tool.schema();
    let grammar = Grammar::compile(&schema);
    println!("tool   : {name}");
    println!("schema : {}", schema.json_schema());

    let state = grammar.state();
    let allowed = state.allowed_prefixes();
    let chars: String = allowed.chars().collect();
    println!("\nallowed starting characters: {chars:?}");
    println!("text body open  : {}", allowed.is_text_body());
    println!("space free      : {}", allowed.is_space_free());
    println!("can end here    : {}", allowed.can_finish());

    if let Some(example) = try_input {
        let mut s = grammar.state();
        println!("\ntrial  : {example}");
        match s.advance(example) {
            Ok(()) => {
                println!(
                    "accept : {}",
                    if s.is_done() {
                        "yes (complete)"
                    } else {
                        "partial"
                    }
                );
                let rest: String = s.allowed_prefixes().chars().collect();
                println!("next   : {rest:?}");
            }
            Err(e) => {
                println!("REJECT : {e}");
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// When the window fills the user IS WARNED — the truncation must not stay
    /// silent.
    #[test]
    fn the_status_line_says_the_window_is_full() {
        let c = TokenCounter::default();
        let s = status_line(4000, 10, 4010, c.prompt_cap(), &c);
        assert!(s.contains("window full"), "{s}");
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
    fn quoted_words(block: &str) -> Vec<String> {
        block
            .split('"')
            .skip(1)
            .step_by(2)
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

        let mut checked_values = 0;
        for skill in skills.all() {
            for name in &skill.tools {
                let tool = catalog.find(name).unwrap_or_else(|| {
                    panic!(
                        "skill '{}' commands tool '{name}', which is not in the catalog: {:?}",
                        skill.name,
                        catalog.names()
                    )
                });
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
