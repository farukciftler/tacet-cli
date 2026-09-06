//! THE COMMANDS THAT EXPLAIN THE PROGRAM TO ITS USER.
//!
//! `why`, `doctor`, `tools`, `grammar`, `sessions` and the `mcp` sub-commands
//! share one property that makes them a module rather than a pile: none of them
//! runs a model, and none of them changes anything. They exist because three
//! separate defects in this codebase were found by dumping a prompt and looking
//! at which tools were in it — sixty seconds of work that beat an hour of model
//! runs — and a diagnostic that valuable should be a command, not something a
//! maintainer has to improvise.
//!
//! `why` IS THE ONE TO REACH FOR FIRST when a tool "stops being called". The
//! router shows the model at most nine tools; a tool that is not among them
//! cannot be called however well the model reasons, and this is where that is
//! visible.

use crate::chat::report_mcp;
use crate::engine_setup::byte_text;
use crate::engine_setup::model_package;
use crate::ui::{BOLD, BRASS, Color, DIM, YELLOW};
use crate::{TerminalAsk, addon, backend, config, session_catalog, total_ram_bytes};
use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;
use tacet_grammar::Grammar;
use tacet_kernel::{ArgSchema, ToolCatalog};
use tacet_skills::SkillStore;
use tacet_tools::data_store::SharedStore;
use tacet_tools::mcp;
use tacet_tools::memory::SharedMemory;
use tacet_tools::router::Router;

/// `tacet why "<message>"` — the router's reasoning, in the order it happens.
///
/// It runs NO MODEL and opens NO socket: this is the layer BEFORE the model,
/// and the whole point is that it answers in milliseconds.
pub fn why(message: &str) -> ExitCode {
    let color = Color::setup();
    let store = Arc::new(SharedStore::new());
    let memory = SharedMemory::in_memory();
    // `can_ask = true`: this command INSPECTS the catalog and never runs a
    // tool, so no confirmation of its own is ever asked. What it must report
    // is the catalog an ordinary session is given — see `session_catalog`.
    let (mut catalog, _) = session_catalog(&store, &memory, &color, true);
    // The remote tools count too — they are the ones most likely to be missing
    // from a budget, since no profile knows their names.
    let mut mcp_load = mcp::load_from_default();
    let mcp_names = mcp::feed_catalog(&mut catalog, &mut mcp_load);
    let router = Router::new().reserving(mcp_names);
    let explanation = router.explain(message, &catalog);

    println!();
    println!("  {} {message}", color.paint(BRASS, "›"));
    println!();

    println!("  {}", color.paint(BOLD, "what the message scored"));
    let mut any = false;
    for (profile, score, fired) in &explanation.profiles {
        if *score == 0 {
            continue;
        }
        any = true;
        println!(
            "    {:<9} {:>4}   {}",
            profile.name(),
            score,
            color.paint(DIM, &fired.join(", "))
        );
    }
    if !any {
        // THE FAILURE MODE THIS COMMAND WAS BUILT FOR.
        println!(
            "    {}",
            color.paint(
                YELLOW,
                "nothing matched — every tool scores zero and the budget fills with the \
                 head of the catalog, whatever the message was about"
            )
        );
    }

    println!();
    println!(
        "  {} ({} of {})",
        color.paint(BOLD, "tools the model will see"),
        explanation.selected.len(),
        explanation.selected.len() + explanation.dropped.len()
    );
    for (i, (name, score, overlap)) in explanation.selected.iter().enumerate() {
        let reason = match (score, overlap) {
            (0, 0) => "catalog order".to_string(),
            (0, o) => format!("shares {o} with the message"),
            (s, 0) => format!("profile {s}"),
            (s, o) => format!("profile {s} · shares {o}"),
        };
        println!("    {}. {:<26} {}", i + 1, name, color.paint(DIM, &reason));
    }

    if !explanation.dropped.is_empty() {
        println!();
        println!("  {}", color.paint(BOLD, "left out"));
        println!(
            "    {}",
            color.paint(DIM, &crate::ui::one_line(&explanation.dropped.join(", ")))
        );
        println!(
            "    {}",
            color.paint(
                DIM,
                "a tool that is not on the list above cannot be called, however well the \
                 model reasons"
            )
        );
    }
    println!();
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// MCP connections
// ---------------------------------------------------------------------------

/// The configured connections. READS THE FILE ONLY — no socket is opened, so
/// this is safe to run when you just want to know what is configured.
pub fn mcp_list(json: bool) -> ExitCode {
    let color = Color::setup();
    let connections = match mcp::connections() {
        Ok(list) => list,
        Err(message) => {
            eprintln!("  {}", color.paint(YELLOW, &message));
            return ExitCode::FAILURE;
        }
    };
    if json {
        let rows: Vec<serde_json::Value> = connections
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "url": c.url,
                    "enabled": c.enabled,
                    "spec": c.spec,
                    "spec_understood": c.spec_understood,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".into())
        );
        return ExitCode::SUCCESS;
    }
    if connections.is_empty() {
        println!("  no connections — Tacet talks to nothing until you add one");
        return ExitCode::SUCCESS;
    }
    for c in &connections {
        let state = if c.enabled { "on" } else { "off" };
        println!(
            "  {}  {}  {}",
            color.paint(BOLD, &c.name),
            color.paint(DIM, &format!("{state} · spec {}", c.spec)),
            color.paint(DIM, &c.url),
        );
        if !c.spec_understood {
            println!(
                "    {}",
                color.paint(YELLOW, "that spec value is not recognised — auto is used")
            );
        }
    }
    ExitCode::SUCCESS
}

/// The paste flow (spec §5). Three prints and one read: the URL, the paste,
/// the file it landed in.
pub fn mcp_login(name: &str) -> ExitCode {
    let color = Color::setup();
    let step = match mcp::begin_login(name) {
        Ok(step) => step,
        Err(message) => {
            eprintln!("  {}", color.paint(YELLOW, &message));
            return ExitCode::FAILURE;
        }
    };
    println!("  open this in a browser:");
    println!();
    println!("    {}", step.url);
    println!();
    println!(
        "  {}",
        color.paint(
            DIM,
            "then paste the address you land on back here (it will look like a 404 page)"
        )
    );
    print!("  > ");
    let _ = std::io::stdout().flush();
    let mut pasted = String::new();
    if std::io::stdin().read_line(&mut pasted).is_err() || pasted.trim().is_empty() {
        eprintln!(
            "  {}",
            color.paint(YELLOW, "nothing pasted; nothing was sent")
        );
        return ExitCode::FAILURE;
    }
    match mcp::finish_login(&step, &pasted) {
        Ok(path) => {
            println!("  {} {path}", color.paint(BRASS, "saved"));
            println!(
                "  {}",
                color.paint(DIM, "the token file is private to your account (0600 on unix; on windows it is as private as your profile folder and no more)")
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("  {}", color.paint(YELLOW, &message));
            ExitCode::FAILURE
        }
    }
}

pub fn mcp_logout(name: &str) -> ExitCode {
    let color = Color::setup();
    match mcp::logout(name) {
        Ok(true) => {
            println!("  the stored token was removed from this machine");
            println!(
                "  {}",
                color.paint(
                    DIM,
                    "the authorization server was not told — revoke it there yourself if you want it gone for good"
                )
            );
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("  there was no stored token for that connection");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("  {}", color.paint(YELLOW, &message));
            ExitCode::FAILURE
        }
    }
}

/// The live smoke test (spec §9): discovery, the catalog, the round trip.
/// **GOES ON THE NETWORK, on purpose, when typed.** Three numbers, no
/// adjectives.
pub fn mcp_try(name: &str, call: Option<String>, args: &str) -> ExitCode {
    let color = Color::setup();
    let call = match call {
        None => None,
        Some(tool) => match serde_json::from_str::<serde_json::Value>(args) {
            Ok(parsed) => {
                // WHAT IS ABOUT TO BE SENT IS SHOWN FIRST, the same rule the
                // approval gate follows: this command has no gate in front of
                // it, so the transparency has to be here.
                println!(
                    "  {} {tool} {}",
                    color.paint(DIM, "→"),
                    color.paint(DIM, &crate::ui::one_line(&parsed.to_string()))
                );
                Some((tool, parsed))
            }
            Err(error) => {
                eprintln!(
                    "  {}",
                    color.paint(YELLOW, &format!("--args is not JSON: {error}"))
                );
                return ExitCode::FAILURE;
            }
        },
    };
    let outcome = match mcp::try_connection(name, Arc::new(TerminalAsk), call) {
        Ok(outcome) => outcome,
        Err(message) => {
            eprintln!("  {}", color.paint(YELLOW, &message));
            return ExitCode::FAILURE;
        }
    };
    println!(
        "  {}  {}  {}",
        color.paint(BOLD, &outcome.revision),
        color.paint(DIM, &format!("{} ms", outcome.millis)),
        color.paint(DIM, &format!("{} tools", outcome.tools.len())),
    );
    if outcome.fell_back {
        println!(
            "  {}",
            color.paint(
                YELLOW,
                "the server did not accept the current revision; the frozen path was used"
            )
        );
    }
    for (tool, description) in &outcome.tools {
        // A description the far side wrote, on its way to a terminal: one line,
        // capped, escapes gone.
        println!(
            "    {}  {}",
            tool,
            color.paint(DIM, &crate::ui::one_line(description))
        );
    }
    if outcome.tools.is_empty() {
        println!("  {}", color.paint(DIM, "(the server offers no tools)"));
    }
    if let Some((tool, text, is_error)) = &outcome.called {
        println!();
        println!(
            "  {} {}",
            color.paint(BOLD, tool),
            color.paint(
                DIM,
                if *is_error {
                    "· the server called it an error"
                } else {
                    ""
                }
            )
        );
        for line in text.lines().take(20) {
            println!("    {}", crate::ui::one_line(line));
        }
    }
    ExitCode::SUCCESS
}

/// `tacet doctor` — one screen that answers "is this machine set up right".
///
/// It DIAGNOSES AND SUGGESTS, it never changes anything: the fix commands are
/// printed for the user to run, in the same spirit as `tacet font`.
pub fn doctor() -> ExitCode {
    let color = Color::setup();
    println!("{}{}", color.paint(BOLD, "Tacet"), color.paint(BRASS, "."));
    println!();

    // The binary.
    let candle = cfg!(feature = "candle");
    let b = backend();
    println!(
        "  binary     candle: {} · backend: {}",
        if candle {
            "yes"
        } else {
            "NO — the real engine is missing"
        },
        b
    );
    if !candle {
        println!(
            "{}",
            color.paint(YELLOW, "             reinstall with: cargo install tacet-cli --features candle (metal on Apple silicon, cuda for NVIDIA GPUs)")
        );
    }

    // The machine.
    let ram = total_ram_bytes();
    match ram {
        Some(b) => println!(
            "  machine    ram: {:.1} GiB · os: {}",
            b as f64 / (1u64 << 30) as f64,
            std::env::consts::OS
        ),
        None => println!("  machine    ram: unknown · os: {}", std::env::consts::OS),
    }

    // The models.
    let roots = model_package::model_roots();
    let packages = model_package::scan(&roots);
    if packages.is_empty() {
        println!(
            "{}",
            color.paint(
                YELLOW,
                "  models     none — run: tacet models download qwen3-4b"
            )
        );
    } else {
        for p in &packages {
            println!(
                "  model      {} · {}",
                p.name,
                color.paint(DIM, &byte_text(p.gguf_bytes))
            );
        }
    }

    // The config, in one line each.
    for key in ["model", "engine", "theme", "update.check"] {
        match config::get_str(key) {
            Some(v) => println!("  config     {key} = {v}"),
            None => println!("  config     {key} {}", color.paint(DIM, "(unset)")),
        }
    }
    println!(
        "  web        {}",
        if tacet_web::addon::web_search_is_open() {
            "addon open"
        } else {
            "addon closed or not installed"
        }
    );

    // The suggestion — a rule of thumb, spelled out so it can be argued with:
    // a Q4 model wants roughly 1.5x its file size in memory while running.
    if let Some(b) = ram {
        let gib = b as f64 / (1u64 << 30) as f64;
        let (model, note) = if gib < 8.0 {
            ("qwen2.5-3b", "under 8 GiB the 3B is the comfortable choice")
        } else if gib < 16.0 {
            (
                "qwen3-4b",
                "8-16 GiB runs the 4B comfortably; the 8B will swap",
            )
        } else {
            (
                "qwen3-8b",
                "16+ GiB runs the 8B well — try: tacet config set model qwen3-8b",
            )
        };
        println!();
        println!(
            "  suggestion {} {}",
            model,
            color.paint(DIM, &format!("· {note}"))
        );
    }
    ExitCode::SUCCESS
}

/// `tacet font` — the appearance guide. See the enum doc for why this prints
/// instructions instead of applying anything: the font is the TERMINAL's
/// setting and the terminal is the user's. The colour story is told here too,
/// because it is the same question ("why doesn't it look like the site?") and
/// the same answer (your terminal owns the canvas; Tacet adapts to it).
pub fn font() -> ExitCode {
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
// tools
// ---------------------------------------------------------------------------

pub fn tools(print_schema: bool) -> ExitCode {
    let color = Color::setup();
    let store = Arc::new(SharedStore::new());
    let memory = SharedMemory::in_memory();
    // `can_ask = true`: this command INSPECTS the catalog and never runs a
    // tool, so no confirmation of its own is ever asked. What it must report
    // is the catalog an ordinary session is given — see `session_catalog`.
    let (mut catalog, _) = session_catalog(&store, &memory, &color, true);
    // MCP tools must be visible HERE TOO: this command is the verbatim source of
    // "what the prompt says"; it must not print something different from the
    // catalog chat sees.
    let mut mcp_load = mcp::load_from_default();
    let _ = mcp::feed_catalog(&mut catalog, &mut mcp_load);
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
pub fn package_list(json: bool) -> ExitCode {
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
// grammar
// ---------------------------------------------------------------------------

pub fn grammar(name: &str, try_input: Option<&str>) -> ExitCode {
    let color = Color::setup();
    let store = Arc::new(SharedStore::new());
    let memory = SharedMemory::in_memory();
    // `can_ask = true`: this command INSPECTS the catalog and never runs a
    // tool, so no confirmation of its own is ever asked. What it must report
    // is the catalog an ordinary session is given — see `session_catalog`.
    let (catalog, _) = session_catalog(&store, &memory, &color, true);
    if write_grammar(name, &catalog, try_input, &color) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The short form called from the shell (`/grammar <tool>`); no trial input.
pub fn print_grammar(name: &str, catalog: &ToolCatalog, color: &Color) {
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
