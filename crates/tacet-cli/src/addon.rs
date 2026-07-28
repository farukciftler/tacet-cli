//! `tacet addon ...` — addon install, list and try.
//!
//! WHAT IT DOES: manages the user's `addons.json` registry and runs the install
//! flow of whichever addon was named.
//!
//! THE FLOW IS DRIVEN BY THE DEFINITION, NOT BY THE NAME. This file used to open
//! with `if name != WEB_SEARCH { reject }`, which made the "addon system" a
//! single install flow with one name compiled into it — a second addon could not
//! be added without editing install, list, gate and try in four places. What an
//! addon is now lives in `tacet_web::addon::DEFINITIONS`, and this file walks
//! that table: it asks the questions the definition names, checks the answers
//! against the SHAPE the definition names, and writes them. Adding an addon here
//! costs a row in that table.
//!
//! ONE ADDON STILL HAS A FLOW OF ITS OWN: `web-search` can bring up a local
//! SearXNG with docker, so it keeps its two paths — (a) the container, (b) the
//! user's own server address. That is a genuine difference in kind (nothing else
//! installs a server), not a name being special.
//!
//! THIRD-PARTY EXTENSION IS MCP, and `list` says so. The names in this table are
//! the ones the build ships; a user's own tool goes in `mcp.json` and needs no
//! build of ours (see `MCP_NOTE`).
//!
//! THE NETWORK MONOPOLY IS PRESERVED. This file OPENS NO SOCKET and does not
//! pull `ureq`; verification goes through
//! `tacet_web::WebSearchClient::health()`. The terminal side (asking the
//! question, printing progress) is the shell's job, the side that goes on the
//! network is `tacet-web`. The split is identical to `model download`.
//!
//! WHAT WAS MEASURED, WHAT WAS NOT (the docker side):
//!
//! * MEASURED — `docker_version` and `compose_command`: run on this machine
//!   (Docker 29.6.1 found; `docker compose` MISSING, `docker-compose` 5.3.1
//!   PRESENT — that is exactly the rationale for trying both forms). The approval
//!   screen was run too and was seen to exit with "cancelled" on EOF.
//! * MEASURED NOTHING — `compose_up` and `wait_until_ready`: the image was never
//!   pulled, no container was ever started, the health probe never ran against a
//!   local instance. These steps download hundreds of MB on the user's machine
//!   and leave a persistent container behind; the approval is the user's, not the
//!   development session's.
//! * MEASURED — the address path (b): install against a real SearXNG instance,
//!   verification (30 results), the registry write, the catalog gate opening and
//!   closing, and the `try` command were run end to end.
//!
//! NO UNIT TEST WAS WRITTEN FOR THE NETWORK AND DOCKER — it would have produced
//! a false green. The tests only measure the CONTENT of the written config
//! (`compose_text`, `settings_text`), the address rule and the registry round
//! trip.
//!
//! THE APPROVAL GATE. Pulling a docker image and starting a container is a
//! persistent change on the user's machine; the image name, the files to be
//! written and the command to be run are shown as they are BEFORE THE APPROVAL.
//! `--no-approval` is for scripts only and follows the same pattern as the same
//! flag in `model download`.

use crate::ui::{BOLD, Color, DIM, YELLOW};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::process::{Command, Stdio};
use tacet_web::addon::{ADDRESS_KEY, Addon, WEB_SEARCH};

/// The port the local SearXNG will bind to.
///
/// NOT 8080: that is the most-collided port on a developer machine and the first
/// "it doesn't work" complaint would be another server holding it. 8888 is a less
/// crowded choice that does not carry SearXNG's own in-container port (8080) to
/// the outside.
const LOCAL_PORT: u16 = 8888;

/// It binds to LOOPBACK only.
///
/// The difference between `127.0.0.1:8888:8080` and `8888:8080` is security: the
/// latter opens the search server to EVERYONE on the same network. So that the
/// user's queries stay on their own machine, the address is pinned to loopback
/// explicitly.
const LOCAL_ADDRESS: &str = "http://localhost:8888";

/// The image to pull — shown to the user BEFORE the approval.
const IMAGE: &str = "searxng/searxng:latest";

/// The subdirectory (under the config directory) the compose files go into.
const SEARXNG_DIR: &str = "searxng";

/// THE LIST IN THIS COMMAND IS NOT THE EXTENSION POINT — and the user has to be
/// told, in the place they came looking.
///
/// The addons here are the ones THIS BUILD ships: adding a sixth means a row in
/// `tacet_web::addon::DEFINITIONS` and a Rust build. Anybody wanting to plug
/// their OWN tool into Tacet does it with an MCP server, which needs no build
/// and no permission from us. A user who types `tacet addon list` looking for
/// "plugins" and sees five fixed names concludes the system is closed; that
/// conclusion is wrong and it is this command's fault for not saying so.
const MCP_NOTE: &str = "your own tools plug in through MCP, not through this list.";

/// Where the MCP door actually is. A hint that does not name the file leaves
/// the reader to search for it.
///
/// IT NAMES A FILE, NOT A COMMAND. There is no `tacet mcp` subcommand in this
/// build, and this file has already paid for suggesting a command the reader
/// cannot run (see `SHELL_CLOSED`): the user types it, gets a usage error, and
/// the hint has cost them a turn instead of saving one.
const MCP_WHERE: &str =
    "add a server to `mcp.json` in the config directory; its tools join the catalog at startup.";

/// The ONE sentence told to the user when the addon is not installed.
///
/// A constant, because the same sentence appears in three separate places: when a
/// web intent is sensed in a chat turn, in the `tacet tools` output, and in
/// `tacet addon try`. Written as three copies, one would get updated and the
/// others would go stale.
pub const ADDON_MISSING: &str =
    "the web search addon is not installed: `tacet addon install web-search`";

/// The same two sentences as typed INSIDE the shell.
///
/// MEASURED, from a real session: the gate fired mid-chat and told the user to
/// run `tacet addon open web-search`. Inside the shell that is not a command —
/// there the verb is `/addon on` — so the user typed the suggestion, got a usage
/// error, and then typed it as a chat message, which the model answered as if it
/// were a question. A hint that names a command the reader cannot run is worse
/// than no hint: it costs a turn and it teaches the wrong verb.
const SHELL_MISSING: &str = "the web search addon is not installed: `/addon install web-search`";
const SHELL_CLOSED: &str = "the web search addon is CLOSED: `/addon on web-search`";

/// INSTALLED BUT CLOSED is a separate state and needs a separate sentence.
///
/// In the first version both were told "not installed" and this WAS MEASURED: to
/// a user who had closed the addon with `tacet addon close`, `tacet tools` said
/// "not installed" — a wrong sentence that pushes the user to reinstall what is
/// already installed. The suggested command was wrong too (`install`, when the
/// right one is `open`).
const ADDON_CLOSED: &str = "the web search addon is CLOSED: `tacet addon open web-search`";

/// Is the web addon INSTALLED at all (open or closed)? The gate message below
/// distinguishes missing from closed; the chat's offer-to-open needs the same
/// distinction — offering to "open" something that is not installed would be
/// the exact wrong-sentence bug the constant above documents.
pub fn web_installed() -> bool {
    tacet_web::addon::read()
        .map(|r| r.find(WEB_SEARCH).is_some())
        .unwrap_or(false)
}

/// The right sentence to print while the gate is closed: is it not installed, or
/// closed — and in the verb the reader can actually type.
///
/// `in_shell` picks the wording, not the meaning: the state is read once, here,
/// so the two forms cannot drift apart.
pub fn closed_gate_message(in_shell: bool) -> &'static str {
    let installed = matches!(tacet_web::addon::read(), Ok(r) if r.find(WEB_SEARCH).is_some());
    match (installed, in_shell) {
        (true, true) => SHELL_CLOSED,
        (true, false) => ADDON_CLOSED,
        // An unreadable registry counts as "missing" too (the gate counts it that
        // way as well).
        (false, true) => SHELL_MISSING,
        (false, false) => ADDON_MISSING,
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

pub fn list(json: bool) -> ExitCode {
    let color = Color::setup();
    let path = tacet_web::addon::registry_path();
    let record = match tacet_web::addon::read() {
        Ok(r) => r,
        // A BROKEN REGISTRY IS NOT SILENTLY SWALLOWED: saying "no addons at all"
        // would push the user to reinstall what they had installed.
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    &format!("the addon registry could not be read: {e}")
                )
            );
            return ExitCode::FAILURE;
        }
    };

    if json {
        let records: Vec<serde_json::Value> = record
            .all()
            .iter()
            .map(|a| {
                // THE SECRETS ARE COVERED HERE TOO. `--json` is the output a
                // user pipes into a file and pastes into a bug report; a db
                // password printed here has left the machine by the time
                // anybody notices.
                let settings: serde_json::Map<String, serde_json::Value> = a
                    .settings
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            serde_json::Value::String(tacet_web::addon::shown_value(&a.name, k, v)),
                        )
                    })
                    .collect();
                serde_json::json!({
                    "name": a.name,
                    "kind": a.kind,
                    "state": a.state_text(),
                    "settings": settings,
                    "tools": tacet_web::addon::definition(&a.name).map(|d| d.tools),
                })
            })
            .collect();
        let available: Vec<serde_json::Value> = tacet_web::addon::DEFINITIONS
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "summary": d.summary,
                    "tools": d.tools,
                    "effect": d.effect,
                    "network": d.network,
                    "installed": record.find(d.name).is_some(),
                    "open": record.is_open(d.name),
                })
            })
            .collect();
        let output = serde_json::json!({
            "registry": path.as_ref().map(|p| p.display().to_string()),
            "addons": records,
            "available": available,
            "third_party": MCP_NOTE,
            "web_search_open": record.is_open(WEB_SEARCH),
        });
        println!("{output}");
        return ExitCode::SUCCESS;
    }

    match &path {
        Some(p) => {
            let note = if p.is_file() { "" } else { " (missing)" };
            println!(
                "{}",
                color.paint(DIM, &format!("registry: {}{note}", p.display()))
            );
        }
        None => println!(
            "{}",
            color.paint(
                DIM,
                "registry: the config directory could not be resolved (TACET_HOME can be set)"
            )
        ),
    }
    println!();

    if record.is_empty() {
        println!("{}", color.paint(DIM, "no addon installed."));
    } else {
        println!("{}", color.paint(BOLD, "installed"));
        for a in record.all() {
            let state = if a.open { "open" } else { "closed" };
            println!(
                "  {}  {}",
                color.paint(BOLD, &a.name),
                color.paint(DIM, state)
            );
            match tacet_web::addon::definition(&a.name) {
                // WHAT IT DOES, not which tools it has: `workspace` has none and
                // a "tools:" line would be blank for it.
                Some(d) => println!("    {}", color.paint(DIM, d.effect)),
                // A record this build has no definition for — an addon removed
                // in an upgrade, or a hand-written line. It is SHOWN rather than
                // hidden: a record that gates nothing but sits in the file is
                // exactly what a user needs to be told about.
                None => println!(
                    "    {}",
                    color.paint(
                        YELLOW,
                        "this build does not know this addon; it opens no tool."
                    )
                ),
            }
            for (key, value) in &a.settings {
                let shown = tacet_web::addon::shown_value(&a.name, key, value);
                let mut lines = shown.split(tacet_web::addon::VALUE_SEPARATOR);
                if let Some(first) = lines.next() {
                    println!("    {} {first}", color.paint(DIM, &format!("{key}:")));
                }
                for line in lines {
                    println!("    {}  {line}", " ".repeat(key.len()));
                }
            }
        }
    }

    // WHAT ELSE COULD BE INSTALLED. Without this the command answers "what have
    // I got" and never "what is there" — and the second question is the one a
    // user opens this list with.
    let missing: Vec<_> = tacet_web::addon::DEFINITIONS
        .iter()
        .filter(|d| record.find(d.name).is_none())
        .collect();
    if !missing.is_empty() {
        println!();
        println!("{}", color.paint(BOLD, "not installed"));
        for d in missing {
            let network = if d.network { "  (network)" } else { "" };
            println!(
                "  {:<11} {}{}",
                d.name,
                color.paint(DIM, d.summary),
                color.paint(YELLOW, network)
            );
            println!(
                "{}",
                color.paint(
                    DIM,
                    &format!("              tacet addon install {}", d.name)
                )
            );
        }
    }

    // THE EFFECT ON THE TOOL CATALOG IS STATED PLAINLY: what the user really
    // wants to learn from this command is "does web search work".
    println!();
    let open = record.is_open(WEB_SEARCH);
    println!(
        "{}",
        color.paint(
            DIM,
            if open {
                "the web_search/web_fetch tools ARE VISIBLE in the catalog."
            } else {
                "the web_search/web_fetch tools are NOT in the catalog (the addon is not installed/open)."
            }
        )
    );
    // WHERE THIRD-PARTY EXTENSION ACTUALLY LIVES. Measured behaviour, not a
    // guess: a user looking for "plugins" comes to this command, finds a list
    // of five names that only this build can grow, and leaves without ever
    // learning that MCP is the door for everything else.
    println!("{}", color.paint(DIM, MCP_NOTE));
    println!("{}", color.paint(DIM, &format!("  {MCP_WHERE}")));
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

/// `value` is the `--address` flag. It is NAMED for web search because that is
/// the addon it was written for, but it is read here as "the one setting, given
/// on the command line" — the only way to install a settings-taking addon from
/// a script. It applies only to a definition with EXACTLY ONE setting; with
/// more than one there is no way to tell which was meant, so the install stays
/// interactive rather than guess.
pub fn install(name: &str, value: Option<String>, local: bool, no_approval: bool) -> ExitCode {
    let color = Color::setup();
    let Some(def) = tacet_web::addon::definition(name) else {
        eprintln!(
            "{}",
            color.paint(YELLOW, &format!("unknown addon: '{name}'"))
        );
        eprintln!("{}", color.paint(DIM, "  installable:"));
        for d in tacet_web::addon::DEFINITIONS {
            eprintln!(
                "{}",
                color.paint(DIM, &format!("   • {:<11} {}", d.name, d.summary))
            );
        }
        eprintln!("{}", color.paint(DIM, &format!("  {MCP_NOTE}")));
        return ExitCode::FAILURE;
    };

    // `--local` MEANS "bring up the container", and only one addon has one.
    // Accepting it silently elsewhere would let a user believe they had asked
    // for something local when the flag did nothing at all.
    if local && def.name != WEB_SEARCH {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!("--local belongs to '{WEB_SEARCH}' — '{name}' sets up no local server")
            )
        );
        return ExitCode::FAILURE;
    }

    if def.name != WEB_SEARCH {
        return install_generic(&color, def, value, no_approval);
    }

    // IF THE FLAGS CLASH WE STOP: silently picking one of the two could have
    // installed a local container instead of the user's server.
    if local && value.is_some() {
        eprintln!(
            "{}",
            color.paint(YELLOW, "--local and --address cannot be given together")
        );
        return ExitCode::FAILURE;
    }

    let choice = if local {
        InstallPath::Local
    } else if let Some(a) = value {
        InstallPath::Address(a)
    } else {
        match ask_path(&color) {
            Some(p) => p,
            None => return ExitCode::FAILURE,
        }
    };

    match choice {
        InstallPath::Local => install_local(&color, no_approval),
        InstallPath::Address(a) => install_with_address(&color, &a),
    }
}

/// THE INSTALL FLOW OF EVERY ADDON THAT IS NOT `web-search`.
///
/// ONE FLOW, NOT FIVE. What differs between `shell`, `workspace`, `http`, `db`
/// and `clipboard` is entirely in the DEFINITION — which questions get asked,
/// what shape the answers must have, whether an answer is a secret. The steps
/// are the same for all of them and in this order: say what it does → ask →
/// CHECK THE SHAPE → show what will be stored → take the approval → write. The
/// order is the point: the approval question is asked when there is something
/// concrete to approve, and nothing is written before the answer.
fn install_generic(
    color: &Color,
    def: &tacet_web::addon::Definition,
    value: Option<String>,
    no_approval: bool,
) -> ExitCode {
    println!("{}", color.paint(BOLD, &format!("{} addon", def.name)));
    println!("{}", color.paint(DIM, def.summary));
    println!(
        "{}",
        color.paint(DIM, &format!("once installed: {}", def.effect))
    );
    if def.network {
        println!(
            "{}",
            color.paint(
                YELLOW,
                "THIS ONE GOES ON THE NETWORK: data leaves this machine."
            )
        );
    }
    println!("{}", color.paint(YELLOW, def.warning));
    println!();

    // A flag was given but there is nowhere to put it. Silence here would make
    // `tacet addon install clipboard --address https://x` look like it did
    // something with the address.
    if value.is_some() && def.settings.len() != 1 {
        let complaint = if def.settings.is_empty() {
            format!("'{}' takes no settings", def.name)
        } else {
            format!(
                "'{}' takes {} settings — install it without the flag and answer the questions",
                def.name,
                def.settings.len()
            )
        };
        eprintln!("{}", color.paint(YELLOW, &complaint));
        return ExitCode::FAILURE;
    }

    let mut collected: Vec<(&'static str, String)> = Vec::new();
    for spec in def.settings {
        let raw = match &value {
            Some(v) => {
                if spec.secret {
                    // MEASURED FROM THE SHELL'S SIDE, not from ours: a value
                    // typed on the command line is in the shell's history file
                    // and in the process list while the command runs. We cannot
                    // take it back, so we say it.
                    eprintln!(
                        "{}",
                        color.paint(
                            YELLOW,
                            "the value was given on the command line: it is in your shell history now."
                        )
                    );
                }
                // The flag carries many values separated by commas. THE PROMPT
                // TAKES ONE PER LINE and has no such limit — a value with a
                // comma in it (a directory can have one) has to be typed at the
                // prompt.
                if spec.many {
                    v.split(',').map(|s| s.trim().to_string()).collect()
                } else {
                    vec![v.trim().to_string()]
                }
            }
            None => match ask_setting(color, spec) {
                Some(values) => values,
                // EOF or a cancelled prompt: nothing is written.
                None => {
                    eprintln!("{}", color.paint(DIM, "cancelled — nothing was written."));
                    return ExitCode::FAILURE;
                }
            },
        };

        let mut values: Vec<String> = raw
            .into_iter()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect();
        if values.is_empty() {
            if spec.required {
                eprintln!(
                    "{}",
                    color.paint(
                        YELLOW,
                        &format!("'{}' is required — nothing was written.", spec.prompt)
                    )
                );
                return ExitCode::FAILURE;
            }
            continue;
        }
        // SORTED AND DEDUPLICATED, so the same answers always produce the same
        // file and a list does not grow a second copy of an entry on reinstall.
        values.sort();
        values.dedup();

        for v in &values {
            if let Err(e) = spec.shape.check(v) {
                eprintln!("{}", color.paint(YELLOW, &format!("not accepted: {e}")));
                eprintln!("{}", color.paint(DIM, &format!("  {}", spec.help)));
                eprintln!("{}", color.paint(DIM, "  nothing was written."));
                return ExitCode::FAILURE;
            }
            // What the SHAPE cannot know: whether the thing is THERE on this
            // machine. Some of that is a refusal and some of it is a warning —
            // `machine_check` decides which, and the difference is whether the
            // layer that will use the value refuses it too.
            match machine_check(spec, v) {
                Ok(warnings) => {
                    for warning in warnings {
                        eprintln!("{}", color.paint(YELLOW, &format!("  ! {warning}")));
                    }
                }
                Err(e) => {
                    eprintln!("{}", color.paint(YELLOW, &format!("not accepted: {e}")));
                    eprintln!("{}", color.paint(DIM, "  nothing was written."));
                    return ExitCode::FAILURE;
                }
            }
        }
        collected.push((spec.key, tacet_web::addon::join_values(&values)));
    }

    // WHAT WILL BE WRITTEN, BEFORE THE APPROVAL — with the secrets covered up:
    // the approval screen is the most-screenshotted screen there is.
    println!();
    println!("{}", color.paint(BOLD, "this will be recorded:"));
    println!("  addon : {}", def.name);
    println!("  state : open");
    if collected.is_empty() {
        println!("{}", color.paint(DIM, "  (no settings)"));
    }
    for (key, joined) in &collected {
        let shown = tacet_web::addon::shown_value(def.name, key, joined);
        let mut lines = shown.split(tacet_web::addon::VALUE_SEPARATOR);
        if let Some(first) = lines.next() {
            println!("  {key}: {first}");
        }
        // The continuation lines are indented UNDER the first value, not under
        // the key: a list of six commands read as one line each is the only way
        // the approval screen shows what is actually being allowed.
        for line in lines {
            println!("  {}  {line}", " ".repeat(key.len()));
        }
    }
    if !take_approval(color, no_approval) {
        println!("{}", color.paint(DIM, "cancelled — nothing was written."));
        return ExitCode::FAILURE;
    }

    save(color, def, collected)
}

/// Asks ONE setting. `None` = the user cancelled or the input ended (EOF): the
/// caller writes nothing.
///
/// MANY VALUES ARE TAKEN ONE PER LINE and an empty line ends the list. Not
/// comma-separated, because a comma is a legal character in a directory name
/// and this is the only place the user can type one.
fn ask_setting(color: &Color, spec: &tacet_web::addon::Setting) -> Option<Vec<String>> {
    println!("{}", color.paint(BOLD, spec.prompt));
    println!("{}", color.paint(DIM, &format!("  {}", spec.help)));
    if spec.secret {
        println!(
            "{}",
            color.paint(
                DIM,
                "  it is stored in a file only you can read, and is not printed back."
            )
        );
    }
    if !spec.many {
        print!("> ");
        let _ = std::io::stdout().flush();
        return read_line().map(|l| vec![l.trim().to_string()]);
    }

    println!(
        "{}",
        color.paint(DIM, "  one per line; an empty line ends the list.")
    );
    let mut values = Vec::new();
    loop {
        print!("> ");
        let _ = std::io::stdout().flush();
        // EOF with nothing collected is a cancellation; EOF after some lines is
        // the end of a pipe and what came before it counts.
        let Some(line) = read_line() else {
            return (!values.is_empty()).then_some(values);
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            return Some(values);
        }
        values.push(line);
    }
}

/// What the machine can say about a value that its SHAPE cannot.
///
/// `Err` REFUSES the install, `Ok(warnings)` lets it through with a note. Which
/// of the two a check gets is decided by ONE question: does the layer that will
/// actually use the value refuse it as well?
///
/// * A DIRECTORY IS A REFUSAL, and the rule is not written here — it is
///   `tacet_tools::workspace::validate_root`, which refuses a missing root, a
///   root that is not a directory, and a root at or above the home directory
///   ("see everything"). It is CALLED rather than copied: a second opinion about
///   what a legal root is would let the install accept a root the file layer
///   then rejects, and the user would be looking at a configured workspace that
///   answers "no such file".
/// * A COMMAND IS A WARNING. `PATH` is not the same in every shell and a program
///   installed tomorrow is a legitimate entry; refusing it would send the user
///   round a loop over a guess.
fn machine_check(spec: &tacet_web::addon::Setting, value: &str) -> Result<Vec<String>, String> {
    use tacet_web::addon::Shape;
    match spec.shape {
        Shape::Directory => tacet_tools::workspace::validate_root(value)
            .map(|_| Vec::new())
            .map_err(|e| e.to_string()),
        Shape::CommandName => Ok(if on_path(value) {
            Vec::new()
        } else {
            vec![format!("{value}: not found on PATH")]
        }),
        _ => Ok(Vec::new()),
    }
}

/// Is there an executable by this name on `PATH`.
///
/// NO PROCESS IS STARTED. Running the command to find out whether it exists is
/// how a "check" turns into an execution of something the user has not approved
/// yet; the directories on `PATH` are read instead. The executable BIT is what
/// is asked for, not merely existence: a data file named `git` sitting in a
/// `PATH` directory is not the command.
fn on_path(command: &str) -> bool {
    let Some(path) = tacet_kernel::env_var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(command);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(&candidate)
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            candidate.is_file()
        }
    })
}

/// The two install paths. The trailing underscore keeps it clear of
/// `std::path::Path`, which this file also uses.
enum InstallPath {
    Local,
    Address(String),
}

/// It ASKS for the two paths. This is what the user's request said verbatim:
/// "let there be either setting SearXNG up as a local server, or entering another
/// server address".
fn ask_path(color: &Color) -> Option<InstallPath> {
    println!("{}", color.paint(BOLD, "web search addon"));
    println!(
        "{}",
        color.paint(DIM, "search runs through your own SearXNG server.")
    );
    println!();
    println!("  1) set up a local SearXNG (docker required)");
    println!("  2) enter my own server address");
    print!("choice [1/2]: ");
    let _ = std::io::stdout().flush();
    let choice = read_line()?;
    match choice.trim() {
        "1" => Some(InstallPath::Local),
        "2" => {
            print!("SearXNG address (https://... or http://localhost:...): ");
            let _ = std::io::stdout().flush();
            let a = read_line()?;
            Some(InstallPath::Address(a.trim().to_string()))
        }
        other => {
            eprintln!(
                "{}",
                color.paint(YELLOW, &format!("invalid choice: '{other}'"))
            );
            None
        }
    }
}

fn read_line() -> Option<String> {
    let mut s = String::new();
    // 0 bytes = EOF: `None`, so piped input does not enter an infinite loop.
    match std::io::stdin().read_line(&mut s) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(s),
    }
}

/// (b) THE USER'S OWN ADDRESS.
///
/// Three steps, and the order matters: (1) the address must pass THE RULE —
/// `https` is mandatory, plain `http` only on a local network (the rule lives in
/// `tacet_web::address_is_valid`, in one place); (2) it must be verified WITH A
/// REAL QUERY — SearXNG can be up while keeping the JSON format off, and that
/// state looks like "200 OK"; (3) only then is it written to the registry.
/// Writing first and trying afterwards would leave a non-working addon marked
/// "installed".
fn install_with_address(color: &Color, address: &str) -> ExitCode {
    let address = address.trim().trim_end_matches('/');
    // THE BEST PROTECTION FOR A SECRET IS NOT TO CREATE IT.
    //
    // `https://user:password@host/searxng` is a perfectly ordinary thing to
    // type for someone who put their SearXNG behind basic auth, and the address
    // rule below does not look at it — it only checks the scheme. The password
    // would then be written into `addons.json` in clear, printed by `addon
    // list`, printed again by `addon try --json` (the output people paste into
    // bug reports), and carried in the chip's raw request URL. Refusing it here
    // means there is no plain-text credential on disk to protect in the first
    // place.
    if let Some(user_info) = userinfo(address) {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                "the address must not carry a user name or password ('user:pass@host')"
            )
        );
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "  '{user_info}@' was found in the address, and it would be stored IN CLEAR in addons.json",
                )
            )
        );
        eprintln!(
            "{}",
            color.paint(
                DIM,
                "  the addon registry is not a password store — put the credential in a proxy or a header instead."
            )
        );
        return ExitCode::FAILURE;
    }
    if let Err(e) = tacet_web::address_is_valid(address) {
        eprintln!(
            "{}",
            color.paint(YELLOW, &format!("the address was not accepted: {e}"))
        );
        eprintln!(
            "{}",
            color.paint(
                DIM,
                "  https is mandatory. Plain http only on a local network (localhost, 127.0.0.1, 192.168.*):",
            )
        );
        eprintln!(
            "{}",
            color.paint(
                DIM,
                "  a query going unencrypted to a remote server is read at every hop in between."
            )
        );
        return ExitCode::FAILURE;
    }

    println!("{}", color.paint(DIM, &format!("trying: {address}")));
    match verify(address) {
        Ok(n) => {
            println!(
                "{}",
                color.paint(BOLD, &format!("✓ the server answered ({n} results)"))
            );
        }
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(YELLOW, &format!("✗ could not be verified: {e}"))
            );
            eprintln!(
                "{}",
                color.paint(
                    DIM,
                    "  the addon WAS NOT INSTALLED — a non-working address is not recorded."
                )
            );
            return ExitCode::FAILURE;
        }
    }

    write_registry(color, address)
}

/// The userinfo part of a URL — everything before the `@` in the authority.
///
/// ONLY THE AUTHORITY IS EXAMINED: an `@` in a path or a query is not a
/// credential (`https://host/a@b`), and treating it as one would reject
/// perfectly good addresses.
fn userinfo(address: &str) -> Option<&str> {
    let after_scheme = address.split_once("://").map(|(_, rest)| rest)?;
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    authority.rsplit_once('@').map(|(user, _)| user)
}

/// Verifies WITH A REAL QUERY. The network path WAS NOT MEASURED on this machine
/// (there was no running SearXNG instance).
fn verify(address: &str) -> Result<usize, tacet_web::WebError> {
    tacet_web::WebSearchClient::with_address(address).health()
}

/// The web search record — the verified address and nothing else.
fn write_registry(color: &Color, address: &str) -> ExitCode {
    let def = tacet_web::addon::definition(WEB_SEARCH)
        .expect("the web-search definition is in the table");
    save(color, def, vec![(ADDRESS_KEY, address.to_string())])
}

/// WRITES THE RECORD — the one place any addon becomes installed.
///
/// Read, replace, write: `Record::add` replaces the record with the same name,
/// so a reinstall updates the settings rather than leaving two records behind
/// with no answer to which one is in force.
fn save(
    color: &Color,
    def: &tacet_web::addon::Definition,
    settings: Vec<(&'static str, String)>,
) -> ExitCode {
    let mut record = match tacet_web::addon::read() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    &format!("the addon registry could not be read: {e}")
                )
            );
            return ExitCode::FAILURE;
        }
    };
    let mut addon = Addon::new(def.name, def.name);
    for (key, value) in settings {
        addon = addon.with_setting(key, value);
    }
    record.add(addon);
    match tacet_web::addon::write(&record) {
        Ok(path) => {
            println!(
                "{}",
                color.paint(DIM, &format!("registry: {}", path.display()))
            );
            println!(
                "{}",
                color.paint(
                    BOLD,
                    &format!("the {} addon is installed and open.", def.name)
                )
            );
            println!("{}", color.paint(DIM, def.effect));
            if def.network {
                println!(
                    "{}",
                    color.paint(
                        DIM,
                        "every call that takes data out still asks for approval in a tainted session.",
                    )
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(YELLOW, &format!("the registry could not be written: {e}"))
            );
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// (a) local SearXNG — docker
// ---------------------------------------------------------------------------

/// IF DOCKER IS MISSING WE DO NOT FAIL SILENTLY: what was looked for, what could
/// not be found and what the alternative is are written out one by one.
fn install_local(color: &Color, no_approval: bool) -> ExitCode {
    let Some(version) = docker_version() else {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                "docker not found — a local SearXNG cannot be set up."
            )
        );
        eprintln!(
            "{}",
            color.paint(
                DIM,
                "  `docker --version` did not run (not on PATH, or the daemon is down)."
            )
        );
        eprintln!("{}", color.paint(DIM, "  options:"));
        eprintln!(
            "{}",
            color.paint(
                DIM,
                "   • install Docker Desktop / docker engine and repeat this command"
            )
        );
        eprintln!(
            "{}",
            color.paint(
                DIM,
                "   • use your own server: tacet addon install web-search --address https://..."
            )
        );
        return ExitCode::FAILURE;
    };
    println!("{}", color.paint(DIM, &format!("docker: {version}")));

    let Some(compose) = compose_command() else {
        eprintln!(
            "{}",
            color.paint(YELLOW, "docker is there but compose is not.")
        );
        eprintln!(
            "{}",
            color.paint(
                DIM,
                "  neither `docker compose version` nor `docker-compose --version` ran."
            )
        );
        eprintln!(
            "{}",
            color.paint(
                DIM,
                "   • use your own server: tacet addon install web-search --address https://..."
            )
        );
        return ExitCode::FAILURE;
    };

    let Some(dir) = searxng_dir() else {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                "the config directory could not be resolved (TACET_HOME can be set)"
            )
        );
        return ExitCode::FAILURE;
    };

    // EVERYTHING IS SHOWN BEFORE THE APPROVAL: which image will come down, which
    // files will be written, which command will run, where the server will bind.
    println!();
    println!("{}", color.paint(BOLD, "a local SearXNG will be set up:"));
    println!("  image     : {IMAGE}");
    println!("  directory : {}", dir.display());
    println!("  files     : docker-compose.yml, settings.yml");
    println!("  command   : {} up -d", compose.join(" "));
    println!("  address   : {LOCAL_ADDRESS}  (binds to 127.0.0.1 only)");
    println!(
        "{}",
        color.paint(
            DIM,
            "  the image is a few hundred MB and is pulled by docker."
        )
    );
    if !take_approval(color, no_approval) {
        println!("{}", color.paint(DIM, "cancelled — nothing was written."));
        return ExitCode::FAILURE;
    }

    if let Err(e) = write_config(&dir) {
        eprintln!(
            "{}",
            color.paint(YELLOW, &format!("the config could not be written: {e}"))
        );
        return ExitCode::FAILURE;
    }
    println!(
        "{}",
        color.paint(DIM, &format!("written: {}", dir.display()))
    );

    match compose_up(&compose, &dir) {
        Ok(()) => println!(
            "{}",
            color.paint(DIM, "the container was started; waiting for it to come up…")
        ),
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(YELLOW, &format!("docker compose failed: {e}"))
            );
            eprintln!(
                "{}",
                color.paint(
                    DIM,
                    &format!(
                        "  try by hand: cd {} && {} up -d",
                        dir.display(),
                        compose.join(" ")
                    )
                )
            );
            return ExitCode::FAILURE;
        }
    }

    // THE HEALTH QUERY goes through tacet-web; this file opens no socket.
    match wait_until_ready(color) {
        Some(n) => {
            println!(
                "{}",
                color.paint(BOLD, &format!("✓ SearXNG answered ({n} results)"))
            );
            write_registry(color, LOCAL_ADDRESS)
        }
        None => {
            eprintln!(
                "{}",
                color.paint(YELLOW, "✗ SearXNG did not answer within the expected time.")
            );
            eprintln!(
                "{}",
                color.paint(DIM, &format!("  logs: {} logs", compose.join(" ")))
            );
            eprintln!(
                "{}",
                color.paint(
                    DIM,
                    "  the addon WAS NOT INSTALLED — a non-working address is not recorded."
                )
            );
            ExitCode::FAILURE
        }
    }
}

fn take_approval(color: &Color, no_approval: bool) -> bool {
    if no_approval {
        println!(
            "{}",
            color.paint(DIM, "  (--no-approval: no question was asked)")
        );
        return true;
    }
    print!("  Continue? [y/N] ");
    let _ = std::io::stdout().flush();
    match read_line() {
        Some(s) => matches!(s.trim().to_lowercase().as_str(), "y" | "yes"),
        None => false,
    }
}

fn searxng_dir() -> Option<PathBuf> {
    tacet_kernel::env::config_dir().map(|d| d.join(SEARXNG_DIR))
}

/// The output of `docker --version`. `None` = docker missing or not running.
///
/// MEASURED NOTHING: docker is not installed on this machine.
fn docker_version() -> Option<String> {
    let output = Command::new("docker")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Finds which form compose is installed in.
///
/// THERE ARE TWO FORMS and both are widespread in the field: `docker compose` as
/// a plugin (v2) and the separate `docker-compose` binary (v1). Trying only one
/// would mean saying "missing" even though compose IS PRESENT on the user's
/// machine.
///
/// THIS DISTINCTION IS NOT AN ASSUMPTION, IT IS A MEASUREMENT: on the development
/// machine `docker --version` = 29.6.1 but `docker compose version` FAILS; the
/// only working form is `docker-compose` (5.3.1). Had only v2 been tried, the
/// local install would have been rejected on this machine with "no compose".
fn compose_command() -> Option<Vec<String>> {
    let v2 = Command::new("docker")
        .args(["compose", "version"])
        .stdin(Stdio::null())
        .output()
        .ok()
        .is_some_and(|o| o.status.success());
    if v2 {
        return Some(vec!["docker".into(), "compose".into()]);
    }
    let v1 = Command::new("docker-compose")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()
        .is_some_and(|o| o.status.success());
    v1.then(|| vec!["docker-compose".into()])
}

/// MEASURED NOTHING. Its output is relayed to the user as is: docker's own error
/// text (port taken, daemon down, image could not be pulled) is more informative
/// than any summary we would rewrite.
fn compose_up(compose: &[String], dir: &Path) -> Result<(), String> {
    let (binary, prefixes) = compose
        .split_first()
        .ok_or("the compose command is empty")?;
    let output = Command::new(binary)
        .args(prefixes)
        .args(["up", "-d"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{binary} could not be run: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if error.is_empty() {
        format!("exit code {}", output.status)
    } else {
        error
    })
}

/// Waits for the container to come up; returns the number of results that came
/// back.
///
/// WHY POLLING: `up -d` STARTS the container but SearXNG takes a few seconds to
/// begin listening. Saying "it did not answer" on a single attempt would declare
/// a working install a failure.
///
/// MEASURED NOTHING: the duration and the attempt count were never exercised
/// against a real container.
fn wait_until_ready(color: &Color) -> Option<usize> {
    const ATTEMPTS: usize = 15;
    const INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
    for i in 1..=ATTEMPTS {
        if let Ok(n) = verify(LOCAL_ADDRESS) {
            return Some(n);
        }
        if i == 1 {
            println!(
                "{}",
                color.paint(
                    DIM,
                    "  (the first start can take minutes, including the image pull)"
                )
            );
        }
        std::thread::sleep(INTERVAL);
    }
    None
}

/// Writes the compose and settings files.
fn write_config(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("docker-compose.yml"), compose_text())?;
    let settings = dir.join("settings.yml");
    // THE SETTINGS FILE IS NOT OVERWRITTEN: the user may have changed their own
    // SearXNG settings (engine choice, language) by hand and a reinstall must not
    // delete them. The secret key must not change on every install either.
    if !settings.is_file() {
        let (key, from_os) = secret_key();
        if !from_os {
            // NOT SILENT. What the user gets in that case is a key mixed from
            // the clock, the pid and a couple of addresses; it is not
            // cryptographic and they are entitled to know before they rely on
            // it. Saying nothing is how a weak key gets mistaken for a strong
            // one.
            let color = Color::setup();
            eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    "the OS entropy source could not be read — a NON-CRYPTOGRAPHIC secret_key was written."
                )
            );
            eprintln!(
                "{}",
                color.paint(
                    DIM,
                    "  replace `secret_key` in settings.yml by hand if this instance is not single-user and loopback-only."
                )
            );
        }
        std::fs::write(settings, settings_text(&key))?;
    }
    Ok(())
}

/// The text of the compose file. A SEPARATE FUNCTION: so the correctness of its
/// content (binding to loopback, the image name, the volume mount) can be
/// measured by a test.
fn compose_text() -> String {
    format!(
        "# Generated by Tacet — `tacet addon install web-search`\n\
         # Editable by hand; `settings.yml` IS NOT OVERWRITTEN on reinstall.\n\
         services:\n\
         \x20 searxng:\n\
         \x20   image: {IMAGE}\n\
         \x20   container_name: tacet-searxng\n\
         \x20   restart: unless-stopped\n\
         \x20   ports:\n\
         \x20     - \"127.0.0.1:{LOCAL_PORT}:8080\"\n\
         \x20   volumes:\n\
         \x20     - ./:/etc/searxng:rw\n\
         \x20   environment:\n\
         \x20     - SEARXNG_BASE_URL={LOCAL_ADDRESS}/\n"
    )
}

/// The SearXNG settings file.
///
/// WITHOUT `formats: json` SEARCH DOES NOT WORK. SearXNG returns only HTML by
/// default; with JSON off the server gives "200 OK" and the client takes the HTML
/// for JSON and falls into `InvalidJson`. That trap is known by name in this repo
/// (see the `WebError::InvalidJson` comment) — the install's first job is to turn
/// it on.
///
/// `limiter: false`: the rate limit is for bot protection and on a single-user
/// local instance it only blocks our own queries.
fn settings_text(secret: &str) -> String {
    format!(
        "# Generated by Tacet. You can add your own settings here.\n\
         use_default_settings: true\n\
         server:\n\
         \x20 secret_key: \"{secret}\"\n\
         \x20 limiter: false\n\
         \x20 image_proxy: true\n\
         search:\n\
         \x20 # `json` IS MANDATORY: Tacet reads search results as JSON.\n\
         \x20 formats:\n\
         \x20   - html\n\
         \x20   - json\n"
    )
}

/// A value for SearXNG's `secret_key` field, and whether it came from the
/// operating system's pool (`true`) or from the fallback (`false`).
///
/// ZERO DEPENDENCY: `rand` WAS NOT ADDED. On Unix 32 bytes are read from
/// `/dev/urandom` — the operating system's own pool, always better than a
/// hand-written generator. If it cannot be read (Windows, or a restricted
/// container) the fallback below runs; THAT PATH IS NOT CRYPTOGRAPHIC and is
/// not claimed to be. On a local, loopback-bound, single-user instance this
/// key's job is to sign image-proxy links; it is not an authentication key.
///
/// THE CALLER IS TOLD WHICH ONE IT GOT. On Windows the fallback runs EVERY
/// time, so "there is a fallback" is not an edge case there, it is the norm.
///
/// `read_exact`, NOT `fs::read` — and this line is a bug fix. The first version
/// called `std::fs::read("/dev/urandom")`; that function reads UNTIL END OF FILE
/// and `/dev/urandom` HAS NO END. The test suite therefore hung (measured:
/// `cargo test -p tacet-cli` never finished). In production the same call would
/// have frozen the local install forever.
fn secret_key() -> (String, bool) {
    if let Some(hex) = thirty_two_bytes_from_urandom() {
        return (hex, true);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = u128::from(std::process::id());
    let stack = &now as *const u128 as usize as u128;
    // A second, independent address: the heap and the stack move under
    // different allocators, so one does not give away the other.
    let heap = Box::new(0u8);
    let heap_addr = &*heap as *const u8 as usize as u128;
    (fallback_key(now, pid, stack, heap_addr), false)
}

/// A 64-hex-digit key from non-random inputs — MIXED, not concatenated.
///
/// WHY THIS IS A SEPARATE, PURE FUNCTION: it is the path that always runs on
/// Windows and it could not be tested at all while it was inlined behind a
/// `/dev/urandom` read that never fails on this machine. Untestable is how it
/// stayed wrong.
///
/// WHAT WAS WRONG. The old body wrote `now ^ (pid << 64)` and
/// `stack * 0x9E3779B97F4A7C15` side by side. In 2026 a nanosecond timestamp is
/// about 2^61, so it never reached the bits the pid was shifted into: the first
/// 16 hex digits WERE the pid and the next 16 WERE the install nanosecond, in
/// clear. The "secret" published the process id and the exact moment of
/// installation, and the second half was reversible because an odd multiplier
/// is invertible. The doc comment said the inputs were "mixed"; they were not.
///
/// WHAT IT DOES NOW. Each 64-bit word goes through the SplitMix64 finaliser,
/// which is an avalanche function: one input bit changes about half the output
/// bits, and the shift-xor steps are not invertible by an observer who does not
/// already know the state. Chained, so every word depends on every input. This
/// is still NOT a CSPRNG and is not offered as one — it removes a leak, it does
/// not manufacture entropy.
fn fallback_key(now: u128, pid: u128, stack: u128, heap: u128) -> String {
    /// SplitMix64's finaliser. NEVER end on the multiply: multiplication by an
    /// odd constant is invertible, which is exactly how the old form gave its
    /// input back.
    fn mix(mut x: u64) -> u64 {
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    }

    let sources = [
        now as u64,
        (now >> 64) as u64,
        pid as u64,
        stack as u64,
        heap as u64,
        // The address of a `static`: moves with ASLR, and it is independent of
        // both the stack and the heap.
        &SECRET_KEY_ANCHOR as *const u8 as usize as u64,
    ];

    let mut state: u64 = 0x243F_6A88_85A3_08D3; // the digits of pi; any nonzero seed
    let mut words = [0u64; 4];
    for word in &mut words {
        for source in sources {
            state = mix(state ^ source);
        }
        *word = state;
    }
    words.iter().map(|w| format!("{w:016x}")).collect()
}

/// Only its ADDRESS is used — see `fallback_key`.
static SECRET_KEY_ANCHOR: u8 = 0;

/// EXACTLY 32 bytes from the operating system's pool. `None` = the source is
/// missing/unreadable (Windows, a restricted container).
fn thirty_two_bytes_from_urandom() -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open("/dev/urandom").ok()?;
    let mut bytes = [0u8; 32];
    file.read_exact(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

// ---------------------------------------------------------------------------
// remove / open / close / try
// ---------------------------------------------------------------------------

pub fn remove(name: &str) -> ExitCode {
    let color = Color::setup();
    let mut record = match tacet_web::addon::read() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    &format!("the addon registry could not be read: {e}")
                )
            );
            return ExitCode::FAILURE;
        }
    };
    if !record.delete(name) {
        eprintln!(
            "{}",
            color.paint(YELLOW, &format!("not installed: '{name}'"))
        );
        return ExitCode::FAILURE;
    }
    match tacet_web::addon::write(&record) {
        Ok(_) => {
            println!("{}", color.paint(BOLD, &format!("'{name}' was removed.")));
            if name == WEB_SEARCH {
                println!(
                    "{}",
                    color.paint(
                        DIM,
                        "the web_search/web_fetch tools dropped out of the catalog."
                    )
                );
                // THE CONTAINER IS NOT STOPPED BY ITSELF. It may have been started
                // during install, but stopping it (and deleting its data) is the
                // user's decision; the command is named "remove", not "delete".
                if let Some(d) = searxng_dir()
                    && d.is_dir()
                {
                    // THE COMMAND IS WRITTEN BY MEASURING THE MACHINE. Hard-coding
                    // "docker compose" would give a recipe that DOES NOT WORK on
                    // machines where compose was installed as the old binary
                    // (`docker-compose`) — measured on this machine:
                    // `docker compose version` fails, `docker-compose --version`
                    // succeeds.
                    let command = compose_command()
                        .map(|c| c.join(" "))
                        .unwrap_or_else(|| "docker compose".to_string());
                    println!(
                        "{}",
                        color.paint(
                            DIM,
                            &format!(
                                "the local container KEEPS RUNNING — to stop it:\n  cd {} && {command} down",
                                d.display()
                            )
                        )
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(YELLOW, &format!("the registry could not be written: {e}"))
            );
            ExitCode::FAILURE
        }
    }
}

/// `open`/`close` — closing the gate without deleting the record.
///
/// WHY IT EXISTS: the registry has a `state` field and `install` always set it to
/// "open". A field that can never be "closed" is a dead field that misleads its
/// reader; it should either be removed or be changeable from production. The
/// second was chosen because turning search off WITHOUT LOSING the address and
/// the settings is a real need (while travelling, on a metered connection).
pub fn set_state(name: &str, open: bool) -> ExitCode {
    let color = Color::setup();
    let mut record = match tacet_web::addon::read() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    &format!("the addon registry could not be read: {e}")
                )
            );
            return ExitCode::FAILURE;
        }
    };
    if record.set_state(name, open).is_none() {
        eprintln!(
            "{}",
            color.paint(YELLOW, &format!("not installed: '{name}'"))
        );
        return ExitCode::FAILURE;
    }
    match tacet_web::addon::write(&record) {
        Ok(_) => {
            println!(
                "{}",
                color.paint(
                    BOLD,
                    &format!("'{name}' was {}.", if open { "opened" } else { "closed" })
                )
            );
            // WHICH TOOLS MOVED. The state word alone does not tell the user
            // what changed for the model, and that is the only thing the state
            // does — the gate is read from the catalog (`addon::is_open`), so
            // opening and closing is exactly "these tools appeared/vanished".
            // WHAT MOVED. The state word alone does not tell the user what
            // changed for the model, and that is the only thing the state does —
            // the gate is read from the catalog (`addon::is_open`), so opening
            // and closing is exactly "this appeared / this went away".
            if let Some(def) = tacet_web::addon::definition(name) {
                println!(
                    "{}",
                    color.paint(
                        DIM,
                        if open {
                            def.effect
                        } else {
                            "what it adds is no longer available to the model."
                        }
                    )
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(YELLOW, &format!("the registry could not be written: {e}"))
            );
            ExitCode::FAILURE
        }
    }
}

/// `tacet addon try <name>` — measures whether an installed addon REALLY works.
/// It goes on the network; before it does, it says what it is going to do.
pub fn try_addon(name: &str, json: bool) -> ExitCode {
    let color = Color::setup();
    let record = match tacet_web::addon::read() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    &format!("the addon registry could not be read: {e}")
                )
            );
            return ExitCode::FAILURE;
        }
    };
    let Some(a) = record.find(name) else {
        if json {
            println!(
                "{}",
                serde_json::json!({ "name": name, "installed": false, "working": false })
            );
        } else {
            eprintln!(
                "{}",
                color.paint(YELLOW, &format!("not installed: '{name}'"))
            );
            if name == WEB_SEARCH {
                eprintln!("{}", color.paint(DIM, &format!("  {ADDON_MISSING}")));
            }
        }
        return ExitCode::FAILURE;
    };
    // EVERY KIND ANSWERS THIS COMMAND, but they do not all answer it the same
    // way. Only web search has something to ask a server; the rest are measured
    // against THIS MACHINE (is the command there, is the directory there) or
    // have nothing that can be measured without doing the thing itself — and
    // saying "no probe" out loud is the honest answer, not a failure. Refusing
    // outright, which is what this did, told a user with a working `shell`
    // addon that it "cannot be tried".
    if a.kind != WEB_SEARCH {
        return try_local(&color, a, json);
    }
    // THE ADDRESS IS RESOLVED THE SAME WAY AS IN PRODUCTION:
    // `WebSearchClient::new()` also looks at the environment variable first and
    // the registry second. Had we read the registry's address directly here, on an
    // install overridden with `TACET_SEARXNG` the trial would have exercised a
    // DIFFERENT server.
    let Some(address) = tacet_web::addon::web_address() else {
        eprintln!("{}", color.paint(YELLOW, "no address is defined"));
        return ExitCode::FAILURE;
    };

    if !json {
        println!(
            "{}",
            color.paint(DIM, &format!("sending a query: {address}"))
        );
    }
    match verify(&address) {
        Ok(n) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "name": a.name, "installed": true, "open": a.open,
                        "address": address, "working": true, "results": n,
                    })
                );
            } else {
                println!("{}", color.paint(BOLD, &format!("✓ working ({n} results)")));
                if !a.open {
                    println!(
                        "{}",
                        color.paint(YELLOW, "the addon is CLOSED: the tools are not in the catalog (`tacet addon open web-search`)")
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "name": a.name, "installed": true, "open": a.open,
                        "address": address, "working": false, "error": e.to_string(),
                    })
                );
            } else {
                eprintln!("{}", color.paint(YELLOW, &format!("✗ {e}")));
            }
            ExitCode::FAILURE
        }
    }
}

/// `addon try` for the kinds that are NOT web search.
///
/// IT OPENS NO SOCKET AND STARTS NO PROCESS. What can be measured here is
/// whether what the settings NAME is present on this machine; anything beyond
/// that (running a command, connecting to the database, reading the clipboard)
/// is the thing itself, and doing the thing to find out whether it can be done
/// is how a "try" becomes an unapproved use of the addon.
fn try_local(color: &Color, a: &tacet_web::addon::Addon, json: bool) -> ExitCode {
    use tacet_web::addon::{COMMANDS_KEY, DIRECTORIES_KEY, SHELL, WORKSPACE};

    // (what was checked, is it there, why)
    let mut findings: Vec<(String, bool, String)> = Vec::new();
    let probed = match a.kind.as_str() {
        SHELL => {
            for command in a.values(COMMANDS_KEY) {
                let there = on_path(command);
                findings.push((
                    command.to_string(),
                    there,
                    if there { "on PATH" } else { "not on PATH" }.to_string(),
                ));
            }
            true
        }
        WORKSPACE => {
            for directory in a.values(DIRECTORIES_KEY) {
                let p = Path::new(directory);
                let there = p.is_dir();
                findings.push((
                    directory.to_string(),
                    there,
                    if there {
                        "a directory"
                    } else if p.exists() {
                        "there, but not a directory"
                    } else {
                        "missing"
                    }
                    .to_string(),
                ));
            }
            true
        }
        // `db` and `clipboard`: connecting to the database, or reading the
        // clipboard, IS the addon's job — there is nothing in between to
        // measure.
        _ => false,
    };

    let working = probed && findings.iter().all(|(_, ok, _)| *ok);
    if json {
        let checks: Vec<serde_json::Value> = findings
            .iter()
            .map(|(what, ok, why)| serde_json::json!({ "value": what, "ok": ok, "note": why }))
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "name": a.name, "kind": a.kind, "installed": true, "open": a.open,
                "probe": if probed { "local" } else { "none" },
                "checks": checks,
                // NOT `false` WHEN THERE IS NO PROBE. A "working: false" that
                // only means "nobody looked" is a false red, and a script
                // reading it would report a healthy addon as broken.
                "working": if probed { serde_json::Value::Bool(working) } else { serde_json::Value::Null },
            })
        );
        return if working || !probed {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    for (what, ok, why) in &findings {
        let mark = if *ok { "✓" } else { "✗" };
        let line = format!("{mark} {what} — {why}");
        if *ok {
            println!("{}", color.paint(DIM, &line));
        } else {
            eprintln!("{}", color.paint(YELLOW, &line));
        }
    }
    if !probed {
        println!(
            "{}",
            color.paint(
                DIM,
                "there is nothing to try without using the addon itself — only its state is shown."
            )
        );
    }
    println!(
        "{}",
        color.paint(
            BOLD,
            &format!(
                "'{}' is installed and {}.",
                a.name,
                if a.open { "OPEN" } else { "CLOSED" }
            )
        )
    );
    if !a.open {
        println!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "its tools are not in the catalog (`tacet addon open {}`)",
                    a.name
                )
            )
        );
    }
    if working || !probed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

// ---------------------------------------------------------------------------
// A production hint — a web request while the addon is missing
// ---------------------------------------------------------------------------

/// Does this message WANT a web search.
///
/// THE TRIGGER LIST IS NOT REWRITTEN: the router's `Web` intent profile is
/// already a list used in production and grown by measurement ("weather", "ferry
/// times", "dollar", "current"...). Setting up a second list would mean one
/// growing while the other went stale — exactly the failure this repo has already
/// lived through (the catalog list stood apart in the shell and in eval).
///
/// The DOMINANT profile is asked, not "is the score greater than zero": the words
/// "address" or "page" also appear in document requests, and advertising the addon
/// every time they do is noise the user learns to ignore.
pub fn is_web_request(message: &str) -> bool {
    tacet_tools::router::score_intent(message).dominant() == tacet_tools::router::IntentProfile::Web
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE SEARXNG TRAP: without `formats: json` search breaks silently.
    #[test]
    fn the_settings_file_turns_on_the_json_format() {
        let t = settings_text("abc");
        assert!(t.contains("formats:"), "{t}");
        assert!(t.contains("- json"), "{t}");
        assert!(t.contains("secret_key: \"abc\""), "{t}");
    }

    /// The container must bind to loopback ONLY: had `"8888:8080"` been written,
    /// the search server would have been open to everyone on the same network.
    #[test]
    fn compose_binds_to_loopback_only() {
        let t = compose_text();
        assert!(t.contains("\"127.0.0.1:8888:8080\""), "{t}");
        assert!(
            !t.contains("\"8888:8080\""),
            "the port is open to all interfaces: {t}"
        );
        assert!(
            t.contains(IMAGE),
            "the image name is not in the compose file: {t}"
        );
    }

    /// The port written in compose and the address written to the registry must be
    /// THE SAME. Both are written by hand so they can diverge, and in that state
    /// the install would say "not working".
    #[test]
    fn the_compose_port_matches_the_registry_address() {
        assert!(
            LOCAL_ADDRESS.ends_with(&LOCAL_PORT.to_string()),
            "{LOCAL_ADDRESS}"
        );
        assert!(compose_text().contains(&format!("127.0.0.1:{LOCAL_PORT}:")));
    }

    /// The local address MUST PASS `tacet-web`'s address gate. If it did not, the
    /// install would finish successfully and the first search would say "invalid
    /// address".
    #[test]
    fn the_local_address_passes_the_web_gate() {
        assert!(tacet_web::address_is_valid(LOCAL_ADDRESS).is_ok());
    }

    /// AN ADDRESS CARRYING A CREDENTIAL IS REFUSED BEFORE IT CAN BE STORED.
    ///
    /// A user who put their SearXNG behind basic auth would naturally type
    /// `https://user:pass@host/searxng`; the scheme rule accepts it, and the
    /// password then lands in `addons.json` in clear, in `addon list` output,
    /// in the `--json` output people paste into bug reports, and in the chip's
    /// request URL. What is measured here is the detector, in both directions:
    /// an `@` in a PATH is not a credential and must not cost the user a
    /// working address.
    #[test]
    fn an_address_with_a_credential_is_detected_and_a_plain_one_is_not() {
        assert_eq!(
            userinfo("https://user:pass@server.example/searxng"),
            Some("user:pass")
        );
        assert_eq!(userinfo("https://user@server.example"), Some("user"));
        // An `@` in the userinfo itself: the LAST one separates the authority.
        assert_eq!(
            userinfo("https://a@b:pass@server.example"),
            Some("a@b:pass")
        );

        assert_eq!(userinfo("https://server.example/searxng"), None);
        assert_eq!(userinfo("http://localhost:8888"), None);
        // NOT A CREDENTIAL: the `@` is in the path, and in the query.
        assert_eq!(userinfo("https://server.example/a@b"), None);
        assert_eq!(userinfo("https://server.example/s?q=a@b"), None);
        assert_eq!(userinfo("not-a-url"), None);
    }

    /// The secret key must not be empty or constant.
    #[test]
    fn a_secret_key_is_generated() {
        let (a, _) = secret_key();
        let (b, _) = secret_key();
        assert!(a.len() >= 32, "short key: {a}");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "{a}");
        assert_ne!(a, b, "the key is generated as a constant");
    }

    /// THE FALLBACK MUST NOT PUBLISH ITS INPUTS.
    ///
    /// This path runs on EVERY Windows install and in any container without
    /// `/dev/urandom`, so it is not a corner. MEASURED on the old form: the
    /// first 16 hex digits were the pid and the next 16 were the install
    /// nanosecond, written out in clear — a "secret" that hands over the
    /// process id and the exact moment of installation, with the second half
    /// reversible on top (an odd multiplier is invertible). The test that
    /// existed only checked "not empty, not constant", which the broken form
    /// passed.
    ///
    /// The inputs are HANDED IN rather than sampled: a pure function is the
    /// only way this path can be measured at all on a machine where
    /// `/dev/urandom` always answers.
    #[test]
    fn the_fallback_key_does_not_publish_its_inputs() {
        let now: u128 = 1_785_233_117_373_961_000;
        let pid: u128 = 74_204;
        let stack: u128 = 0x0000_0001_6f8a_23f0;
        let heap: u128 = 0x0000_0001_2d40_5a80;

        let k = fallback_key(now, pid, stack, heap);
        assert_eq!(k.len(), 64, "{k}");
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()), "{k}");
        for (name, value) in [("pid", pid), ("now", now), ("stack", stack), ("heap", heap)] {
            let low = (value as u64) as u128;
            assert!(
                !k.contains(&format!("{low:016x}")),
                "the {name} is written into the key: {k}"
            );
        }

        // AVALANCHE: one nanosecond of difference must change the whole key,
        // not a corner of it. Without this the install instant is recoverable
        // by walking a second's worth of candidates.
        let k2 = fallback_key(now + 1, pid, stack, heap);
        let same = k.chars().zip(k2.chars()).filter(|(a, b)| a == b).count();
        assert!(
            same < 24,
            "no avalanche: {same}/64 digits unchanged\n{k}\n{k2}"
        );
        // BOTH HALVES MUST MOVE. In the old form the second half did not depend
        // on the clock at all.
        assert_ne!(k[32..64], k2[32..64], "the second half ignores its input");
        assert_ne!(k[0..32], k2[0..32], "the first half ignores its input");

        // A different pid alone must change the key too.
        assert_ne!(k, fallback_key(now, pid + 1, stack, heap));
        // And it is deterministic, so this test measures the function rather
        // than the weather.
        assert_eq!(k, fallback_key(now, pid, stack, heap));
    }

    /// INSTALLED-BUT-CLOSED and NOT-INSTALLED-AT-ALL must get separate sentences,
    /// and THE SUGGESTED COMMAND must be one the reader can actually type where
    /// they are reading it.
    #[test]
    fn closed_and_not_installed_are_separate_sentences() {
        assert!(ADDON_MISSING.contains("addon install"));
        assert!(ADDON_CLOSED.contains("addon open"));
        assert_ne!(ADDON_MISSING, ADDON_CLOSED);
        assert_ne!(SHELL_MISSING, SHELL_CLOSED);

        // The shell forms must carry the SLASH verbs. A real session was lost to
        // this: the in-chat hint said `tacet addon open web-search`, which the
        // shell rejects — the user typed it, got a usage error, then sent it as
        // a chat message and the model answered it as a question.
        assert!(SHELL_CLOSED.contains("/addon on "), "{SHELL_CLOSED}");
        assert!(SHELL_MISSING.contains("/addon install "), "{SHELL_MISSING}");
        // And they must NOT carry the external form, or the shell teaches a verb
        // that does not work there.
        assert!(!SHELL_CLOSED.contains("tacet addon"), "{SHELL_CLOSED}");
        assert!(!SHELL_MISSING.contains("tacet addon"), "{SHELL_MISSING}");

        // Whatever state the registry is in on this machine, the chosen sentence
        // must match that state — in both wordings.
        let installed = tacet_web::addon::read()
            .map(|r| r.find(WEB_SEARCH).is_some())
            .unwrap_or(false);
        assert_eq!(
            closed_gate_message(false),
            if installed {
                ADDON_CLOSED
            } else {
                ADDON_MISSING
            }
        );
        assert_eq!(
            closed_gate_message(true),
            if installed {
                SHELL_CLOSED
            } else {
                SHELL_MISSING
            }
        );
    }

    /// THE POINT OF THE WHOLE CHANGE: a name that is not `web-search` reaches an
    /// install flow instead of a rejection.
    ///
    /// It is measured through the definition table rather than by calling
    /// `install`, because installing writes to the machine's real registry — a
    /// test that did that would change the user's configuration and would make
    /// every other test in this file depend on the order it ran in.
    #[test]
    fn five_more_addons_can_be_installed_not_just_web_search() {
        for name in [
            "web-search",
            "shell",
            "workspace",
            "http",
            "db",
            "clipboard",
        ] {
            assert!(
                tacet_web::addon::definition(name).is_some(),
                "'{name}' cannot be installed: it is in no definition"
            );
        }
        // And an unknown name is still refused — the table is a closed list, not
        // an invitation to write anything into the registry.
        assert!(tacet_web::addon::definition("web_search").is_none());
        assert!(tacet_web::addon::definition("").is_none());
    }

    /// An unknown name must not write anything, and must say what CAN be
    /// installed — a bare "unknown addon" leaves the user guessing at names.
    #[test]
    fn an_unknown_name_fails_without_writing() {
        let before = tacet_web::addon::read().ok();
        assert!(matches!(
            install("no-such-addon", None, false, true),
            code if format!("{code:?}") == format!("{:?}", ExitCode::FAILURE)
        ));
        // The registry is untouched.
        let after = tacet_web::addon::read().ok();
        assert_eq!(before, after, "a failed install changed the registry");
    }

    /// `--local` builds a container, and only one addon has one. Accepting the
    /// flag elsewhere would let a user think they asked for something local.
    /// A flag with nowhere to go is refused too, rather than ignored.
    #[test]
    fn a_flag_that_does_not_apply_is_refused_not_ignored() {
        let before = tacet_web::addon::read().ok();
        let fail = format!("{:?}", ExitCode::FAILURE);
        assert_eq!(format!("{:?}", install("shell", None, true, true)), fail);
        assert_eq!(
            format!("{:?}", install("clipboard", Some("x".into()), false, true)),
            fail,
            "clipboard takes no settings, so a value has nowhere to go"
        );
        assert_eq!(
            before,
            tacet_web::addon::read().ok(),
            "a refused install changed the registry"
        );
    }

    /// The PATH probe answers about real programs and does not start any of
    /// them. `sh` is on PATH everywhere this is built; the nonsense name is not.
    #[test]
    fn the_path_probe_finds_a_real_command_and_not_an_invented_one() {
        #[cfg(unix)]
        assert!(on_path("sh"), "sh was not found on PATH");
        assert!(!on_path("tacet-no-such-command-9182"));
        // A directory named like a command is not a command: `/usr` is not
        // executable-as-a-file, and a directory on PATH must not answer yes.
        assert!(!on_path("."));
    }

    /// A shape cannot know whether the thing is THERE. The machine check can —
    /// and the two answers are deliberately different in kind.
    ///
    /// A DIRECTORY IS REFUSED, and by the FILE LAYER'S OWN RULE
    /// (`tacet_tools::workspace::validate_root`), not by a second opinion held
    /// here. The case that makes this matter is the home directory: it passes
    /// every shape check — absolute, no `..` — and the file layer refuses it as
    /// "see everything". Had this file kept its own is-it-a-directory check, the
    /// install would have written a root the file layer then ignores, and the
    /// user would be looking at a configured workspace that answers "no such
    /// file".
    ///
    /// A COMMAND IS A WARNING: `PATH` differs between shells and a program
    /// installed tomorrow is a legitimate entry.
    #[test]
    fn a_bad_root_is_refused_by_the_file_layers_rule_and_a_missing_command_only_warns() {
        let directories = tacet_web::addon::definition("workspace")
            .unwrap()
            .setting(tacet_web::addon::DIRECTORIES_KEY)
            .unwrap();
        let missing = std::env::temp_dir().join("tacet-no-such-directory-9182");
        std::fs::remove_dir_all(&missing).ok();
        assert!(
            machine_check(directories, &missing.display().to_string()).is_err(),
            "a root that is not there was accepted"
        );
        assert_eq!(
            machine_check(directories, &std::env::temp_dir().display().to_string()),
            Ok(Vec::new())
        );
        // THE SHAPE CANNOT CATCH THIS ONE — only the file layer's rule does.
        if let Some(home) = std::env::var_os("HOME") {
            let home = home.to_string_lossy().to_string();
            assert!(
                tacet_web::addon::Shape::Directory.check(&home).is_ok(),
                "the shape has no opinion about the home directory"
            );
            assert!(
                machine_check(directories, &home).is_err(),
                "the home directory was accepted as a workspace root"
            );
        }

        let commands = tacet_web::addon::definition("shell")
            .unwrap()
            .setting(tacet_web::addon::COMMANDS_KEY)
            .unwrap();
        assert_eq!(
            machine_check(commands, "tacet-no-such-command-9182")
                .expect("a missing command must warn, not refuse")
                .len(),
            1
        );
        #[cfg(unix)]
        assert_eq!(machine_check(commands, "sh"), Ok(Vec::new()));
    }

    /// THE MCP HINT must name the door and must NOT name a command that does not
    /// exist. This file has already cost a user a turn by suggesting one (see
    /// `SHELL_CLOSED`), and there is no `tacet mcp` subcommand in this build.
    #[test]
    fn the_third_party_hint_names_mcp_and_no_invented_command() {
        assert!(MCP_NOTE.to_lowercase().contains("mcp"), "{MCP_NOTE}");
        assert!(MCP_WHERE.contains("mcp.json"), "{MCP_WHERE}");
        for hint in [MCP_NOTE, MCP_WHERE] {
            assert!(!hint.contains("tacet mcp"), "invented command: {hint}");
            assert!(!hint.contains("/mcp"), "invented shell verb: {hint}");
        }
    }

    /// THE PRODUCTION HINT trigger: it wakes on a web question, not on a greeting.
    ///
    /// THE MESSAGES STAY TURKISH ON PURPOSE. This test does not measure this
    /// file, it measures `tacet_tools::router`'s `message_triggers`, and that list
    /// is TURKISH DATA — it is matched against what the user typed and was grown
    /// by measurement ("hava", "vapur saatleri", "dolar"). Translating the case
    /// texts to English would make the test touch nothing and turn it green while
    /// measuring nothing. When the trigger list becomes multilingual, these lines
    /// move with it.
    #[test]
    fn a_web_request_is_sensed() {
        assert!(is_web_request("what is the current dollar price"));
        assert!(is_web_request("what is the weather like in Istanbul?"));
        assert!(is_web_request(
            "can you summarize this address https://example.test/post"
        ));
        assert!(!is_web_request("hello"));
        assert!(!is_web_request("multiply 17 by 45"));
        assert!(!is_web_request("create the budget table as xlsx"));
    }
}
