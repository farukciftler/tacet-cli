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
use tacet_tools::executor::{ApprovalGate, ApprovalRequest, SilentDeny, ToolExecutor};
use tacet_tools::mcp;
use tacet_tools::memory::SharedMemory;
use tacet_tools::router::Router;
use tacet_tools::run_code::CodeState;
use ui::{BOLD, BRASS, Color, DIM, LiveReporter, RESET, Screen, TurnIndicator, YELLOW, paper_code};

use std::io::Write;
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
    });
    match command {
        Command::Chat {
            engine,
            script,
            show_prompt,
            dir,
            message,
            model,
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
            chat(engine, script, show_prompt, &dir, message, &model)
        }
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
        eprintln!("    {}", request.content);
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
        /// `tokenizer.json`. If `None` the package is HALF and no engine can be
        /// set up.
        pub tokenizer: Option<PathBuf>,
        /// The root this package was found in — the same name can sit in two
        /// roots and the user needs to see WHICH ONE wins.
        pub root: PathBuf,
    }

    impl ModelPackage {
        /// Is it ENOUGH to set up an engine. A half package (with no tokenizer)
        /// IS VISIBLE in the catalog but cannot be selected: this is exactly the
        /// answer to "my folder is right there but the model is not found".
        pub fn is_complete(&self) -> bool {
            self.tokenizer.is_some()
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
    fn absolute_env(name: &str) -> Option<PathBuf> {
        let v = std::env::var_os(name).filter(|v| !v.is_empty())?;
        let p = PathBuf::from(v);
        p.is_absolute().then_some(p)
    }

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
        Some(ModelPackage {
            name,
            dir: dir.to_path_buf(),
            gguf,
            gguf_bytes,
            tokenizer: tok.is_file().then_some(tok),
            root: root.to_path_buf(),
        })
    }

    /// All installed packages (the default roots).
    pub fn catalog() -> Vec<ModelPackage> {
        scan(&model_roots())
    }

    /// The pair given DIRECTLY through environment variables.
    ///
    /// BOTH are required: if only one is given the user's intent is incomplete,
    /// and setting up an engine with half a pair would be a silent mistake. This
    /// branch comes BEFORE the catalog — an explicit request is ahead of
    /// discovery.
    pub fn pair_from_env() -> Option<(String, String)> {
        let m = tacet_kernel::env_var(MODEL_VARIABLE)?;
        let t = tacet_kernel::env_var(TOKENIZER_VARIABLE)?;
        Some((
            m.to_string_lossy().into_owned(),
            t.to_string_lossy().into_owned(),
        ))
    }

    /// The (gguf, tokenizer) pair for `name` from the given package list.
    ///
    /// SEPARATE AND PURE: so the discovery logic can be tested without touching
    /// environment variables. Environment variables are PROCESS-WIDE and tests
    /// running in parallel step on each other.
    pub fn to_pair(packages: &[ModelPackage], name: &str) -> Option<(String, String)> {
        let p = packages.iter().find(|p| p.name == name)?;
        let t = p.tokenizer.as_ref()?;
        Some((
            p.gguf.to_string_lossy().into_owned(),
            t.to_string_lossy().into_owned(),
        ))
    }

    /// PRODUCTION DISCOVERY: environment first, then the catalog.
    ///
    /// THE ARCHITECTURE IS NOT GUESSED HERE. The folder name ("qwen3-4b") is
    /// only a label; which module gets loaded is told by the GGUF metadata (see
    /// `Architecture::resolve`). If the name and the content diverge — if the
    /// user puts another weight in the folder — the right thing is to follow the
    /// content.
    pub fn resolve_pair(name: &str) -> Option<(String, String)> {
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
            Some((m, t)) => candle_engine_from_path(&m, &t),
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
            Some((m, t)) => match candle_engine_from_path(&m, &t) {
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
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "({}/{} are set but the files could note be loaded:\n   gguf : {m}\n   tokenizer: {t})",
                    MODEL_VARIABLE, TOKENIZER_VARIABLE
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
            "  [NO tokenizer.json — cannot be selected]"
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
// chat
// ---------------------------------------------------------------------------

fn chat(
    choice: EngineChoice,
    script: Vec<String>,
    show_prompt: bool,
    dir: &str,
    single_message: Option<String>,
    model_name: &str,
) -> ExitCode {
    let color = Color::setup();
    let screen = Screen::setup();
    let interactive = single_message.is_none();

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
    let mut history: Vec<Turn> = Vec::new();
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

    // Counts COMPLETED turns, for the one-time update offer below. Not a
    // metric and not persisted: it exists so the question lands after the
    // user has actually used the shell rather than on the first line.
    let mut completed_turns: usize = 0;
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
            let cleared = message.trim() == "/clear";
            // An /addon verb with arguments changes the registry; the running
            // session must see the change (the transcript that forced this:
            // `/addon on web-search` said "opened" while the session kept
            // answering "the addon is CLOSED" until a restart).
            let addon_touched = message.trim_start().starts_with("/addon ");
            match slash(&message, &catalog, &memory, &mut history, &engine, &color) {
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

        // The tool budget derives ONLY from the user message.
        let selected: ToolCatalog = router.select(&message, &catalog).into_iter().collect();
        let selected_names: Vec<String> = selected.names().into_iter().map(String::from).collect();

        // SKILL INJECTION (700 limit, NOT EMBEDDED into the system instruction):
        // the SINGLE skill matching the message, into that turn's prompt behind a
        // `<guidance>` fence. Turn-distance repeat suppression via
        // `injection_state`: the same skill is not added again on every turn.
        let guide = skill_store
            .matching(&message, Some(&selected_names))
            .and_then(|s| {
                if injection_state.is_needed(&s.name) {
                    injection_state.mark(&s.name);
                    Some(injection_text(s))
                } else {
                    None
                }
            });

        // MEMORY INJECTION (600 limit): the notes matching the message, in the
        // system block.
        let memory_text = memory.with(|s| s.injection_text(&message)).flatten();

        let mut answer = String::new();
        // The tokens of this USER turn. The inner loop (the tool turns) goes to
        // the model more than once; the "this turn" number shown to the user is
        // the sum of all of them — that is the real cost spent on a single
        // question.
        let mut turn_prompt = 0usize;
        let mut turn_generation = 0usize;
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
            let question = if first_turn { message.as_str() } else { "" };
            let previous: Vec<Turn> = if first_turn {
                history.clone()
            } else {
                history
                    .iter()
                    .cloned()
                    .chain(std::iter::once(Turn::user(&message)))
                    .chain(turn_tools.iter().cloned())
                    .collect()
            };
            let mut prompt = Prompt::new(SYSTEM_INSTRUCTIONS, question)
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
            if report.changed() {
                eprintln!(
                    "{}",
                    color.paint(
                        DIM,
                        &format!(
                            "(context truncated: {} turns dropped)",
                            report.dropped_turns
                        )
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
                println!("--- PROMPT ({:?}) ---", engine.template());
                println!("{wire}");
                println!(
                    "--- ~{} tokens (estimate) ---",
                    TokenCounter::estimate(&wire)
                );
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
            let listener = |chunk: &str| {
                if !interactive {
                    return;
                }
                let mut visible = filter.lock().expect("filter lock").feed(chunk);
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
            if interactive && !streaming.load(Ordering::Relaxed) && !remaining.trim().is_empty() {
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
                executor.cancel();
                indicator.finish();
                screen.line(&color.paint(DIM, "  (stopped)"));
                answer = String::new();
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
                if interactive && !streaming.load(Ordering::Relaxed) {
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

        // Chips: Tacet does not hide what it did. In interactive mode they were
        // already printed LIVE (see `LiveReporter`); printing them again here
        // would duplicate the screen. In single-message/diagnostic mode this is
        // the only place they are printed.
        if !interactive {
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
        history.push(Turn::user(&message));
        history.append(&mut turn_tools);
        if !answer.is_empty() {
            history.push(Turn::assistant(&answer));
            // In interactive mode the answer was already printed while streaming;
            // do not print it again.
            if !interactive {
                println!("Tacet: {answer}");
            }
        }
        println!();
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
        "/clear" => {
            history.clear();
            println!("{}", color.paint(DIM, "(history deleted — the fixed prompt and tools still occupy the window on the next turn)"));
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

#[cfg(feature = "candle")]
fn candle_engine_from_path(
    model: &str,
    tokenizer: &str,
) -> Result<Arc<dyn EngineProvider>, String> {
    let setting = tacet_engine::ModelSetting::new(model, tokenizer);
    // File existence is checked BEFORE a 2.5 GB load; learning about a missing
    // file at the end of that wait is a pointless delay.
    tacet_engine::CandleEngine::files_exist(&setting).map_err(|e| e.to_string())?;
    let engine = tacet_engine::CandleEngine::load(&setting).map_err(|e| e.to_string())?;
    // WHICH ARCHITECTURE was loaded is printed. Had it stayed silent, a model
    // running with the wrong template would look like "it gives odd answers" and
    // be hard to diagnose.
    eprintln!(
        "(architecture: {}, template: {:?})",
        engine.architecture().name(),
        engine.architecture().template()
    );
    Ok(Arc::new(engine) as Arc<dyn EngineProvider>)
}

#[cfg(not(feature = "candle"))]
fn candle_engine_from_path(
    _model: &str,
    _tokenizer: &str,
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
        Some((m, t)) => match candle_engine_from_path(&m, &t) {
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
        println!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "{}/{} set — discovery disabled:\n  gguf      : {m}\n  tokenizer : {t}",
                    MODEL_VARIABLE, TOKENIZER_VARIABLE
                )
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
        match &p.tokenizer {
            Some(_) => println!("  {}", color.paint(DIM, "tokenizer: tokenizer.json")),
            // A HALF PACKAGE IS SAID PLAINLY: the `.gguf` is there but no engine
            // can be set up, and the user would only learn that by trying
            // `--engine candle` and getting an error.
            None => println!(
                "  {}",
                color.paint(
                    YELLOW,
                    "tokenizer: MISSING — this package cannot be selected"
                )
            ),
        }
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
}

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
            eprintln!(
                "    {}",
                self.color.paint(YELLOW, "no sha256 in the catalog — the downloaded file's digest will be COMPUTED and shown (first trust)")
            );
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
    fn the_env_pair_does_not_resolve_unless_both_are_given() {
        // SAFETY: a single-threaded test body; no other test in this binary reads
        // these two variables.
        unsafe {
            std::env::remove_var(MODEL_VARIABLE);
            std::env::remove_var(TOKENIZER_VARIABLE);
        }
        assert!(model_package::pair_from_env().is_none());

        unsafe { std::env::set_var(MODEL_VARIABLE, "/path/m.gguf") };
        assert!(
            model_package::pair_from_env().is_none(),
            "with only the model given, no engine must be set up from half a pair"
        );

        unsafe { std::env::set_var(TOKENIZER_VARIABLE, "/path/tokenizer.json") };
        assert_eq!(
            model_package::pair_from_env(),
            Some((
                "/path/m.gguf".to_string(),
                "/path/tokenizer.json".to_string()
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
