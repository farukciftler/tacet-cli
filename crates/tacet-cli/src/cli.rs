//! THE COMMAND LINE ITSELF — what `tacet` accepts, and nothing about what it
//! then does.
//!
//! WHY IT IS ITS OWN FILE: it is five hundred lines of DECLARATION. Every one
//! of them is a promise to a user who has typed the command before, and the
//! test module at the bottom is there because those promises have been broken
//! by accident: `tacet -m "..."`, `tacet --continue`, `tacet update --install`
//! are all shapes somebody has in a script somewhere. Keeping the surface in
//! one file makes "did this change what the program accepts" a question with a
//! one-file answer.
//!
//! THE SUBCOMMAND IS OPTIONAL, and that is the single most important line in
//! here: a bare `tacet` opens the interactive shell. It used to require
//! `tacet chat`, and a user who typed the program's own name got clap's usage
//! text — the wrong answer for an assistant's first screen.

use crate::{DEFAULT_MODEL, DEFAULT_THRESHOLD};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "tacet",
    version,
    about = "Tacet — the terminal shell of the on-device assistant"
)]
pub struct Shell {
    /// THE SUBCOMMAND IS OPTIONAL. If not given, the interactive shell opens;
    /// the slash commands inside it (`/eval`, `/tools`, `/grammar`, ...) reach
    /// the same jobs from there. The subcommands were not removed: scripts and
    /// the `--message` diagnostics depend on them, and a shell staying
    /// scriptable matters more than decoration.
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
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
        /// Sampling temperature. 0 (the default) is GREEDY and reproducible;
        /// raise it to get a different path through the model on a retry.
        ///
        /// NOT A QUALITY DIAL. Above 0 the same question stops giving the same
        /// answer, which is the point on a retry and a problem in a measurement
        /// — `tacet eval` deliberately never touches this.
        #[arg(long, value_name = "0.0-2.0")]
        temperature: Option<f32>,
        /// Sampling seed. Same seed + same prompt + same temperature = the same
        /// output; only meaningful with `--temperature` above 0.
        #[arg(long, value_name = "N")]
        seed: Option<u64>,
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
        #[arg(long)]
        tool_selection: bool,
        /// With --tool-selection: run the TURKISH selection set instead of the
        /// English one.
        #[arg(long)]
        turkish: bool,
        /// The local model folder (`~/models/<name>`).
        ///
        /// WITH `--tool-selection` (or any of the measurement flags) it is the
        /// model whose choices are measured, and it defaults to `DEFAULT_MODEL`.
        ///
        /// WITHOUT them, giving a model switches the LOGIC set from `FakeEngine`
        /// to that model — which is the only way `EvalCase::grounded` can ever
        /// fire. The flag had a `default_value` before, so there was no way to
        /// express "no model" and the logic set was permanently pinned to the
        /// fake engine: the case list carried a claim about the model's SENTENCE
        /// that nothing could reach. An `Option` is what tells the two apart.
        #[arg(long)]
        model: Option<String>,
        /// Run only the cases whose name contains this string.
        #[arg(long)]
        only: Option<String>,
        /// Enforce quantization match (Item 14: e.g. Q4_K_M)
        #[arg(long)]
        require_quant: Option<String>,
        /// Override tool budget for measurement (Item 10)
        #[arg(long)]
        budget: Option<usize>,
        /// Run a tool budget sweep (Item 10: e.g. "6,9,12,0")
        #[arg(long)]
        budget_sweep: Option<String>,
        /// Format-only gate test for CI (Item 12)
        #[arg(long)]
        format_gate: bool,
        /// Force tool name prefix constraint (Item 13)
        #[arg(long)]
        force_tool_name: bool,
        /// Measure THE ROUTER instead of the model: for every case in the
        /// selection suites, was the expected tool inside the budget shown to
        /// the model, and at what position.
        ///
        /// NO WEIGHTS ARE LOADED and the run is milliseconds. The router sets
        /// the ceiling on every `--tool-selection` number — a tool that is not
        /// in the prompt cannot be called however well the model reasons — so
        /// this is the cheap measurement that explains the expensive one.
        /// `--turkish` picks the Turkish suite; without it BOTH are routed,
        /// because a router regression is usually visible in only one language.
        #[arg(long)]
        routing: bool,
        /// With `--routing`: add this many synthetic REMOTE tools to the
        /// catalog before routing.
        ///
        /// WHY IT MATTERS: the built-in catalog is thirteen tools against a
        /// budget of nine, so only four can ever be crowded out. Connect one
        /// MCP server and it is thirty-odd — which is the situation the budget
        /// exists for and the only one where a tool realistically falls out of
        /// the prompt. `--pressure 20` is one ordinary server.
        #[arg(long, default_value_t = 0)]
        routing_pressure: usize,
        /// Compare two `--json` reports and say whether the difference is real:
        /// `--compare before.json after.json`.
        ///
        /// A sign test over the cases that MOVED, plus a paired bootstrap
        /// interval. Works on any of the three reports this command produces.
        #[arg(long, num_args = 2, value_names = ["BEFORE", "AFTER"])]
        compare: Option<Vec<String>>,
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
    /// CLOSED BY DEFAULT. On a fresh install NONE of the addon tools are in the
    /// catalog — not `web_search`/`web_fetch`, not `shell`, `http`, `db` or
    /// `clipboard`; the "data does not leave the device" default is applied not
    /// as a setting but as the ABSENCE of the tool.
    ///
    /// THE SIX NAMES ARE THIS BUILD'S WHOLE LIST. Third-party extension is MCP
    /// (`mcp.json` in the config directory), not this command.
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
    /// Packages the LAST conversation into an anonymised, local report the
    /// user can READ and then paste into a GitHub issue THEMSELVES.
    ///
    /// The privacy-compatible learning loop: Tacet has no telemetry, so the
    /// only way "which prompts fail" can ever reach the project is the user
    /// choosing to hand one over. This command sends NOTHING: it writes a
    /// markdown file next to the user, scrubbed of the obvious identifiers,
    /// for their own eyes first.
    Feedback {
        /// How many recent turns to include.
        #[arg(long, default_value_t = 6)]
        turns: usize,
    },
    /// Health check: hardware, engine features, models, config — and what
    /// this machine can comfortably run.
    Doctor,
    /// The receipt chain: what ran, when, verified against tampering.
    ///
    /// The receipts are written by PURE CODE as tools execute (the model is
    /// never in that loop) into an append-only, hash-chained file; this
    /// command prints the tail and re-verifies the whole chain every run.
    Log {
        #[arg(long)]
        json: bool,
        /// How many recent receipts to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Why a message reaches — or does not reach — a tool. NO MODEL RUNS.
    ///
    /// WHY IT EXISTS: the three Turkish defects fixed this week were found by
    /// dumping a prompt and looking at which tools were in it. That took a
    /// minute and told us more than an hour of model runs, because the score
    /// says WHICH case failed while this says WHY. The most useful diagnostic
    /// in the project should be a command, not something you have to know to
    /// improvise.
    Why {
        /// The message, exactly as a user would type it.
        message: String,
    },
    /// The MCP connections in `mcp.json` — list them, or try one for real.
    ///
    /// `try` is the ONE command in this program that deliberately opens a
    /// socket to a third party without a conversation around it: it exists so
    /// "is this server reachable, and which revision does it speak" has an
    /// answer that is three numbers rather than an adjective.
    Mcp {
        #[command(subcommand)]
        job: McpJob,
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

    /// Updates this binary to the newest release.
    ///
    /// NOTHING CHECKS BY ITSELF. There is no start-up check and no timer: a
    /// program whose promise is that it stays off the network cannot quietly
    /// go online to ask about itself. This runs when it is typed.
    ///
    /// THE BARE COMMAND NOW UPDATES, and the reason for the change is that the
    /// old default was a trap of the most ordinary kind: `tacet update` printed
    /// that a newer version existed and did nothing, so the user typed the
    /// command that sounds like the whole job and got half of it. Every other
    /// tool with this word installs.
    ///
    /// WHAT DID NOT CHANGE IS THE CONSENT. The download still names the file,
    /// its size and its digest and still asks before writing anything —
    /// "update" is now the default INTENT, never a silent action. `--check`
    /// keeps the old look-only behaviour for scripts that want it.
    Update {
        /// Only look. Prints whether a newer release exists and writes nothing.
        #[arg(long)]
        check: bool,
        /// Deprecated: the bare `tacet update` does this now. Kept so the older
        /// spelling in READMEs and muscle memory keeps working.
        #[arg(long, hide = true)]
        install: bool,
        /// Skips the question. What is being downloaded is still printed.
        #[arg(long = "no-approval")]
        no_approval: bool,
    },
}

/// The jobs of the `config` subcommand. The shape mirrors every other list
/// command in this shell: human text by default, `--json` for scripts.
#[derive(Subcommand)]
pub enum McpJob {
    /// The configured connections. Reads the file only; NO NETWORK.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Talks to one connection: discovery, the tool list, the round trip.
    /// **GOES ON THE NETWORK.**
    Try {
        /// The connection's name in `mcp.json`.
        name: String,
        /// Also call this remote tool. Nothing is called unless you name it —
        /// "has no effects" is not something a description can be trusted for.
        #[arg(long)]
        call: Option<String>,
        /// The arguments for `--call`, as JSON. Defaults to `{}`.
        #[arg(long, default_value = "{}")]
        args: String,
    },
    /// Logs in to a connection that has an `auth` block (OAuth, M3).
    ///
    /// Tacet prints the authorization URL and waits for the redirect URL to be
    /// pasted back. It does NOT open a browser and does NOT listen on a port:
    /// a program whose promise is that it does nothing behind your back does
    /// not open windows or sockets for you.
    Login {
        /// The connection's name in `mcp.json`.
        name: String,
    },
    /// Forgets a connection's stored token, on this machine only.
    Logout {
        /// The connection's name in `mcp.json`.
        name: String,
    },
}

#[derive(Subcommand)]
pub enum ConfigJob {
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
pub enum AddonJob {
    /// What is installed, which one is open, what its address is.
    List {
        /// JSON instead of a human table — for scripts.
        #[arg(long)]
        json: bool,
    },
    /// Installs an addon: web-search, shell, workspace, http, db, clipboard.
    ///
    /// A FLAGLESS CALL ASKS its questions; the flags are for scripts and skip
    /// them. `tacet addon list` prints the six names with a line each.
    Install {
        /// The addon name (`tacet addon list` prints them).
        name: String,
        /// The addon's ONE setting, given on the command line — a SearXNG
        /// address for web-search, the allowed commands for shell, the
        /// directories for workspace. Only for a definition with exactly one
        /// setting; with several the install stays interactive. `--address` is
        /// the old name and still works.
        #[arg(long = "value", alias = "address", value_name = "VALUE")]
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
pub enum PackageJob {
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
pub enum ModelJob {
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
pub enum EngineChoice {
    /// candle if a local model exists, fake otherwise — automatic.
    Auto,
    Fake,
    Candle,
}

#[cfg(test)]
mod command_line {
    use super::*;
    use crate::config;
    use clap::CommandFactory;

    /// The argument table is a contract with everyone's muscle memory, and clap
    /// only notices a broken one at RUN time — a conflicting flag or a bad
    /// default panics the first user, not the build. This asks it to check.
    #[test]
    fn the_argument_table_is_well_formed() {
        Shell::command().debug_assert();
    }

    /// THE FLAGS PARSE, and `chat` is what carries them.
    #[test]
    fn the_sampling_flags_reach_the_chat_command() {
        let parsed =
            Shell::try_parse_from(["tacet", "chat", "--temperature", "0.7", "--seed", "5"])
                .expect("parses");
        match parsed.command {
            Some(Command::Chat {
                temperature, seed, ..
            }) => {
                assert_eq!(temperature, Some(0.7));
                assert_eq!(seed, Some(5));
            }
            other => panic!("expected a chat command, got {}", other.is_some()),
        }
    }

    /// EVERY CONFIG KEY IS A FLAG. `config.rs` opens with that rule and the two
    /// sampling keys are the newest chance to break it — a key with no flag
    /// behind it is a second settings system growing quietly.
    #[test]
    fn the_sampling_config_keys_have_flags_behind_them() {
        let known: Vec<&str> = config::known_keys().iter().map(|(k, _)| *k).collect();
        assert!(known.contains(&"temperature"), "{known:?}");
        assert!(known.contains(&"seed"), "{known:?}");
        let flags: Vec<String> = Shell::command()
            .find_subcommand("chat")
            .expect("chat subcommand")
            .get_arguments()
            .map(|a| a.get_id().to_string())
            .collect();
        assert!(flags.contains(&"temperature".to_string()), "{flags:?}");
        assert!(flags.contains(&"seed".to_string()), "{flags:?}");
    }

    /// `tacet update` INSTALLS, and the older spelling still parses.
    ///
    /// The default changed deliberately: the old bare command printed that a
    /// newer version existed and did nothing, so the word that sounds like the
    /// whole job did half of it. What did not change is that the download still
    /// asks before writing — `--no-approval` is the only way past the question.
    #[test]
    fn update_installs_by_default_and_can_still_only_look() {
        let bare = Shell::try_parse_from(["tacet", "update"]).expect("parses");
        match bare.command {
            Some(Command::Update {
                check,
                install,
                no_approval,
            }) => {
                assert!(!check, "the bare command is not a check");
                assert!(!install, "the old flag is not implied");
                assert!(!no_approval, "the question is asked unless waived");
            }
            _ => panic!("update did not parse as the update command"),
        }

        let looking = Shell::try_parse_from(["tacet", "update", "--check"]).expect("parses");
        assert!(matches!(
            looking.command,
            Some(Command::Update { check: true, .. })
        ));

        // The spelling in older READMEs must not become an error.
        let old = Shell::try_parse_from(["tacet", "update", "--install"]).expect("parses");
        assert!(matches!(
            old.command,
            Some(Command::Update { install: true, .. })
        ));
    }
}
