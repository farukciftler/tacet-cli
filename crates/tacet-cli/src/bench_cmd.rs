//! `tacet bench` — run a benchmark somebody else wrote.
//!
//! THE DIFFERENCE FROM `eval --tool-selection`, in one line: that suite is
//! compiled in and measures THIS project; a benchmark is a file and measures the
//! machine it is run on, with the tools that machine actually has — the user's
//! MCP servers, their addons, their language.
//!
//! WHICH CATALOG, and it is the whole reason this command exists rather than a
//! flag on `eval`. The suite deliberately runs against a narrow, reproducible
//! catalog: no network, nothing host-dependent, so the published number does not
//! move when a reader installs an addon. A benchmark runs against
//! `session_catalog` plus MCP — the same list the interactive shell builds —
//! because "does my assistant call MY tool" is unanswerable against a catalog
//! that does not contain it.
//!
//! `check` EXISTS BECAUSE A BAD CASE IS PERMANENT. A question whose expected
//! tool is absent, or which no routing would ever put in front of the model,
//! scores as a model failure every time it is run and reads as one forever. So
//! the file is checked before a token is generated, and the check needs no
//! weights: parse, then the host's catalog, then the router.

use crate::model_package;
use crate::ui::{BOLD, Color, DIM, YELLOW};
use crate::{candle_engine_from_path, model_not_found_report, session_catalog};
use std::process::ExitCode;
use std::sync::Arc;
use tacet_eval::bench::{BenchFile, BenchScore};
use tacet_kernel::ToolCatalog;
use tacet_tools::memory::SharedMemory;
use tacet_tools::router::Router;

/// Reads the file, or explains what is wrong with it and stops.
fn read(path: &str) -> Result<BenchFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    BenchFile::parse(&text).map_err(|e| format!("{path}: {e}"))
}

/// The catalog this machine actually has: the shell's own list, plus whatever
/// the configured MCP servers offer.
///
/// `can_ask` is false — a benchmark is a measurement, and a tool that would stop
/// to ask the user a question cannot be part of one. It is the same choice
/// `chat --message` makes for the same reason.
fn host_catalog(store: &Arc<tacet_tools::data_store::SharedStore>, color: &Color) -> ToolCatalog {
    let memory = SharedMemory::in_memory();
    let (mut catalog, _) = session_catalog(store, &memory, color, false);
    let load = tacet_tools::mcp::load_from_default();
    let _ = tacet_tools::mcp::feed_catalog(&mut catalog, &load);
    catalog
}

/// `tacet bench check <file>` — everything that can be known without a model.
///
/// THREE QUESTIONS, in the order that makes the answers cheap:
///
///   1. Is the file well formed? (`BenchFile::parse` — the author's mistake.)
///   2. Does this machine have the tools it needs? (`requires` — the host's.)
///   3. Would the expected tool even REACH the model? The router shows nine of
///      the catalog to the model, so a case whose tool falls outside those nine
///      is measuring the router and reporting the model. This is the check
///      nobody writes by hand and the one that catches the most.
pub fn bench_check(path: &str, portable: bool) -> ExitCode {
    let color = Color::setup();
    let file = match read(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &e));
            return ExitCode::FAILURE;
        }
    };

    let store = Arc::new(tacet_tools::data_store::SharedStore::new());
    let catalog = if portable {
        // The DEFAULT catalog: built-in tools with the web addon open, no MCP,
        // no discovery of this machine's sandbox. What a fresh install sees.
        let memory = SharedMemory::in_memory();
        tacet_tools::catalog::production_catalog_with(&store, &memory, Some(0), true).0
    } else {
        host_catalog(&store, &color)
    };
    let names: Vec<String> = catalog.names().into_iter().map(String::from).collect();

    println!("{}", color.paint(BOLD, &format!("  {}", file.name)));
    println!(
        "  {} cases · {} tools required · {}",
        file.cases.len(),
        file.requires.len(),
        file.language.as_deref().unwrap_or("no language declared")
    );
    // WHICH CATALOG THIS ANSWER IS ABOUT. The router shows nine of however many
    // exist, so the same file checks differently on a machine with MCP servers
    // attached — which is not a flaw, it is the question a benchmark asks. It
    // just has to be said out loud.
    println!(
        "{}",
        color.paint(
            DIM,
            &format!(
                "  checked against {} tools ({})",
                names.len(),
                if portable {
                    "the default catalog — what a fresh install sees"
                } else {
                    "this machine, addons and MCP included; add --portable for the default"
                }
            )
        )
    );

    if let Some(missing) = file.missing_from(&names) {
        println!();
        eprintln!("{}", color.paint(YELLOW, &missing.to_string()));
        return ExitCode::FAILURE;
    }

    // A `forbidden` ASSERTION ABOUT A TOOL THIS MACHINE DOES NOT HAVE CANNOT
    // FAIL, so it is reported rather than silently counted as a pass. This is
    // also what catches a benchmark that is accidentally not portable: the first
    // drafted set forbade `serverim_disk_durumu`, an MCP tool that exists on
    // exactly one machine.
    let vacuous: Vec<(String, String)> = file
        .forbidden_tools()
        .into_iter()
        .filter(|(_, t)| !names.iter().any(|n| n == t))
        .collect();

    // THE ROUTING CHECK. `Router::select` is what decides which nine tools the
    // model is shown, and it takes no model itself, so this is free.
    let router = Router::new();
    let mut unreachable: Vec<String> = Vec::new();
    let mut buried: Vec<String> = Vec::new();
    for case in &file.cases {
        for step in &case.steps {
            let Some(want) = step.expect.as_deref() else {
                continue;
            };
            let selected = router.select(&step.message, &catalog);
            let shown: Vec<&str> = selected.iter().map(|t| t.name()).collect();
            match shown.iter().position(|n| *n == want) {
                None => unreachable.push(format!("{} · {want} is not in the nine", case.name)),
                // NOT AN ERROR, A WARNING, and the distinction is measured: the
                // model takes the first plausible tool on the list, so a tool at
                // rank 6 is shown and rarely chosen. A case can legitimately be
                // hard; it should just not be hard by accident.
                Some(rank) if rank >= 5 => {
                    buried.push(format!(
                        "{} · {want} is shown at rank {}",
                        case.name,
                        rank + 1
                    ));
                }
                Some(_) => {}
            }
        }
    }

    println!();
    if unreachable.is_empty() {
        println!(
            "{}",
            color.paint(BOLD, "  every expected tool reaches the model")
        );
    } else {
        println!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "  {} step(s) expect a tool the router does not show. A tool that is not \
in the prompt CANNOT be called, so these measure the router and report the model:",
                    unreachable.len()
                )
            )
        );
        for u in &unreachable {
            println!("    {u}");
        }
    }
    if !buried.is_empty() {
        println!();
        println!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "  {} step(s) expect a tool shown below rank 5 — reachable, but the \
model takes the first plausible name on the list:",
                    buried.len()
                )
            )
        );
        for b in buried.iter().take(20) {
            println!("    {b}");
        }
    }

    if !vacuous.is_empty() {
        println!();
        println!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "  {} \"forbidden\" assertion(s) name a tool this machine does not have, \
so they cannot fail here — the case still measures its real claim, but this half of it \
is not being tested:",
                    vacuous.len()
                )
            )
        );
        let mut shown: Vec<&str> = Vec::new();
        for (case, tool) in vacuous.iter().take(200) {
            if !shown.contains(&tool.as_str()) {
                shown.push(tool);
                println!("    {tool}  (first in {case})");
            }
        }
    }

    if unreachable.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `tacet bench run <file> --model <name>` — the measurement.
pub fn bench_run(path: &str, model_name: &str, json: bool) -> ExitCode {
    let color = Color::setup();
    let file = match read(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &e));
            return ExitCode::FAILURE;
        }
    };

    // THE HOST CHECK RUNS BEFORE THE WEIGHTS ARE OPENED. Loading 2.5 GB and then
    // discovering the benchmark needs a tool this machine does not have wastes a
    // minute and, worse, tempts whoever is watching to run it anyway.
    let probe_store = Arc::new(tacet_tools::data_store::SharedStore::new());
    let probe = host_catalog(&probe_store, &color);
    let names: Vec<String> = probe.names().into_iter().map(String::from).collect();
    if let Some(missing) = file.missing_from(&names) {
        eprintln!("{}", color.paint(YELLOW, &missing.to_string()));
        return ExitCode::FAILURE;
    }

    let engine = match model_package::resolve_pair(model_name) {
        Some((m, t)) => match candle_engine_from_path(&m, t.as_deref()) {
            Ok(engine) => {
                eprintln!("{}", color.paint(DIM, &format!("(model: {m})")));
                engine
            }
            Err(e) => {
                eprintln!("error: the model could not be loaded: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => {
            model_not_found_report(model_name, &color);
            return ExitCode::FAILURE;
        }
    };

    let name = file.name.clone();
    let cases = file.into_cases();
    let report = tacet_eval::tool_selection::run_selection_in(
        &cases,
        &engine,
        None,
        false,
        &|env, memory| {
            let (mut c, _) = session_catalog(&env.store, memory, &Color::setup(), false);
            let load = tacet_tools::mcp::load_from_default();
            let _ = tacet_tools::mcp::feed_catalog(&mut c, &load);
            c
        },
    );

    if json {
        println!("{}", report.json());
        return ExitCode::SUCCESS;
    }

    let score = BenchScore::from_counts(
        (report.tool_passed, report.tool_total),
        (report.irrelevance_passed, report.irrelevance_total),
        (report.step_passed, report.step_total),
        (report.answer_passed, report.answer_total),
    );
    let axis = |label: &str, v: Option<f64>, p: usize, t: usize| {
        match v {
            Some(v) => format!("  {label:<14} {p:>4}/{t:<4}  {:>5.1}%", 100.0 * v),
            // An axis with no cases is printed as such rather than as 0%,
            // because the two mean opposite things.
            None => format!("  {label:<14}    —          not measured"),
        }
    };
    println!();
    println!("{}", color.paint(BOLD, &format!("  {name}")));
    println!(
        "{}",
        axis(
            "irrelevance",
            score.irrelevance,
            report.irrelevance_passed,
            report.irrelevance_total
        )
    );
    println!(
        "{}",
        axis("tool", score.tool, report.tool_passed, report.tool_total)
    );
    println!(
        "{}",
        axis("step", score.step, report.step_passed, report.step_total)
    );
    println!(
        "{}",
        axis(
            "answer",
            score.answer,
            report.answer_passed,
            report.answer_total
        )
    );
    println!();
    println!(
        "{}",
        color.paint(BOLD, &format!("  SCORE  {:.1} / 100", score.out_of_100()))
    );
    println!(
        "{}",
        color.paint(
            DIM,
            "  weights: irrelevance 0.40 · tool 0.30 · step 0.20 · answer 0.10 — \
the safety axis is heaviest on purpose; an axis with no cases is left out, not zeroed"
        )
    );
    ExitCode::SUCCESS
}
