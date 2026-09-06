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
    let mut load = tacet_tools::mcp::load_from_default();
    let _ = tacet_tools::mcp::feed_catalog(&mut catalog, &mut load);
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
pub fn bench_run(path: &str, model_name: &str, json: bool, skip_missing: bool) -> ExitCode {
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
    let mut file = file;
    if let Some(missing) = file.missing_from(&names) {
        if !skip_missing {
            eprintln!("{}", color.paint(YELLOW, &missing.to_string()));
            return ExitCode::FAILURE;
        }
        // SET ASIDE, AND COUNTED OUT LOUD. A case that needs an absent tool
        // cannot be scored; a case that does not is unaffected by its absence.
        let before = file.cases.len();
        let absent = &missing.0;
        file.cases.retain(|c| {
            !c.steps.iter().any(|s| {
                s.expect
                    .as_ref()
                    .is_some_and(|t| absent.iter().any(|m| m == t))
            })
        });
        let skipped = before - file.cases.len();
        file.requires.retain(|r| !absent.iter().any(|m| m == r));
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "this machine has no {} — {skipped} of {before} cases need one of them \
and were SET ASIDE. The {} below are the ones this host can actually answer.",
                    absent.join(", "),
                    file.cases.len()
                )
            )
        );
        if file.cases.is_empty() {
            eprintln!("nothing left to run");
            return ExitCode::FAILURE;
        }
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
            let mut load = tacet_tools::mcp::load_from_default();
            let _ = tacet_tools::mcp::feed_catalog(&mut c, &mut load);
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

/// THE FOUR NUMBERS — and the first two are the whole point.
///
/// `bench gap` asks one question the rest of this repository asserts and never
/// measured on a small model: HOW MUCH OF THE WORK IS THE GRAMMAR DOING.
///
///   * VALID call rate — did anything parseable come out. Constrained, this
///     should be 100% by construction: a call that has started cannot finish
///     invalidly. That is the claim on the front page, and here it is a
///     measurement instead of a sentence.
///   * CORRECT call rate — was it the RIGHT tool. Valid is not correct, and the
///     distance between the two lines is the honest limit of constrained
///     decoding: the automaton guarantees syntax and says nothing about
///     judgement. On a 270M model that distance is the finding.
///   * TIME TO FIRST TOKEN — prefill, in one number.
///   * DECODE tok/s — everything after the first token, which is where a long
///     answer's cost actually lives.
///
/// WHY BOTH RUNS USE THE SAME PROMPT AND THE SAME SAMPLER: the only difference
/// between the two columns must be the mask. Anything else and the gap measures
/// two changes at once.
///
/// PEAK MEMORY is read from `/proc/self/status` where the kernel offers it and
/// reported as unavailable otherwise, rather than pulled in through a new
/// dependency. macOS has no `/proc`, so on a Mac the column is honest about
/// being empty instead of printing a zero.
/// THE SAME CEILING ON BOTH COLUMNS, and it exists because the measurement did
/// not terminate without it.
///
/// MEASURED, by hanging: Qwen3-0.6B with the grammar OFF does not stop. The
/// engine's default ceiling is the context share — tens of thousands of tokens —
/// and a 600M model asked for a tool call will happily fill it, so one
/// unconstrained generation took minutes and 88 of them never finished. That is
/// itself half of what this command is for: the runaway is the failure the
/// constraint prevents, and a run that hangs cannot report it.
///
/// 256 IS GENEROUS AND THE SAME ON BOTH SIDES. A tool call is about twenty
/// tokens and the largest legitimate one measured in this repository is 1523,
/// which is a bulk-content `create_document` and not the shape being asked for
/// here. What a model does past 256 tokens on "what is 15% off 80" is not an
/// answer that arrived late; it is an answer that never arrives. Equal on both
/// sides is the part that matters: a cap that bound one column and not the other
/// would measure the cap.
const GAP_CAP: usize = 256;

/// Did a generation actually BEGIN a tool call?
///
/// The grammar arms once a call has begun, so a generation that never begins one
/// is a generation the automaton was never given a chance to constrain. Without
/// this distinction the valid-call rate reads as a refutation of the guarantee
/// when it is really a count of how often a small model answers in prose.
///
/// `name(` ALONE WAS NOT ENOUGH, and getting it wrong put a finding on the front
/// page that sat there as "unexplained" for a day. An unconstrained Qwen3-0.6B
/// does not answer in prose OR call a tool; about a third of its turns PARROT THE
/// SIGNATURE back:
///
/// ```text
/// (time(kind: "clock", target?: "what time it is"))
/// calendar(kind: 'date', target?: text).
/// ```
///
/// The `?:` is copied straight out of the tool description. That is not a call
/// and never becomes one, but it contains `time(`, so a substring test counted it
/// as a start — and counted it only in the unconstrained column, because the mask
/// forbids that shape. The measurement was reading "the grammar stopped the model
/// parroting the schema" as "the grammar stopped the model calling a tool", and
/// reported the 0.6B starting 15 points FEWER calls with the grammar on.
///
/// Requiring the brace separates them: Tacet's call format is `name({...})`, and
/// no echo of a signature reaches it. Whitespace between the paren and the brace
/// is allowed because a model that writes `calculate( {"expression":"2+2"})` has
/// begun a call by any reading; a newline there is the same.
fn started_a_call(text: &str, names: &[&str]) -> bool {
    names.iter().any(|name| {
        text.match_indices(&format!("{name}("))
            .any(|(i, m)| text[i + m.len()..].trim_start().starts_with('{'))
    })
}

pub fn bench_gap(path: &str, model_name: &str) -> ExitCode {
    let color = Color::setup();
    let file = match read(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &e));
            return ExitCode::FAILURE;
        }
    };
    let store = Arc::new(tacet_tools::data_store::SharedStore::new());
    let catalog = host_catalog(&store, &color);
    let names: Vec<String> = catalog.names().into_iter().map(String::from).collect();
    if let Some(missing) = file.missing_from(&names) {
        eprintln!("{}", color.paint(YELLOW, &missing.to_string()));
        return ExitCode::FAILURE;
    }
    let engine = match model_package::resolve_pair(model_name) {
        Some((m, t)) => match candle_engine_from_path(&m, t.as_deref()) {
            Ok(e) => e,
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
    let Some(vocab) = engine.vocab() else {
        eprintln!("error: this engine exposes no vocabulary, so no constraint can be built");
        return ExitCode::FAILURE;
    };

    let router = tacet_tools::router::Router::new();
    let mut rows: Vec<GapRow> = Vec::new();
    let mut cases = 0usize;

    for case in &file.cases {
        for step in &case.steps {
            let Some(want) = step.expect.as_deref() else {
                continue;
            };
            cases += 1;
            let selected: tacet_kernel::ToolCatalog =
                router.select(&step.message, &catalog).into_iter().collect();
            let constraint = tacet_grammar::CallConstraint::new(&vocab, &selected);
            let prompt = tacet_engine::Prompt::new(tacet_eval::SYSTEM_INSTRUCTIONS, &step.message)
                .with_tools(&selected);
            for (armed, label) in [(false, "off"), (true, "on")] {
                let c: Option<&dyn tacet_engine::Constrainer> = if armed {
                    Some(&constraint as &dyn tacet_engine::Constrainer)
                } else {
                    None
                };
                let started = std::time::Instant::now();
                // AN ATOMIC AND NOT A `Cell`, because the listener contract is
                // `Fn(&str) + Send + Sync`: the engine may hand fragments over
                // from whatever thread produced them.
                let first = std::sync::atomic::AtomicU64::new(0);
                let listener = |_: &str| {
                    let _ = first.compare_exchange(
                        0,
                        started.elapsed().as_micros().max(1) as u64,
                        std::sync::atomic::Ordering::Relaxed,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                };
                let Ok(produced) = tacet_engine::wait(engine.generate_streaming(
                    &prompt,
                    c,
                    tacet_engine::SamplingSetting {
                        max_tokens: GAP_CAP,
                        ..Default::default()
                    },
                    &listener,
                )) else {
                    continue;
                };
                let total = started.elapsed();
                let ttft = match first.load(std::sync::atomic::Ordering::Relaxed) {
                    0 => total,
                    us => std::time::Duration::from_micros(us),
                };
                let call = tacet_tools::executor::ToolCall::parse(&produced.text);
                let names: Vec<&str> = selected.tools().iter().map(|t| t.name()).collect();
                let started = started_a_call(&produced.text, &names);
                // A DIAGNOSTIC, not a feature: `TACET_GAP_DUMP=<n>` prints the
                // first n generations of each column. It is kept because it is
                // what found the signature-echo artefact above. Two rounds of
                // reasoning about the mask and the sampler produced nothing; the
                // cause was visible in the first screen of raw output. When a
                // rate cannot be explained, read the generations.
                if let Some(n) = tacet_kernel::env_var("TACET_GAP_DUMP")
                    .and_then(|v| v.to_string_lossy().parse::<usize>().ok())
                    && rows.iter().filter(|r| r.armed == label).count() < n
                {
                    eprintln!(
                        "\n--- [{label}] {} | started={started} valid={} ---\n{}",
                        case.name,
                        call.is_some(),
                        produced.text.chars().take(280).collect::<String>()
                    );
                }
                rows.push(GapRow {
                    armed: label,
                    started,
                    valid: call.is_some(),
                    correct: call.as_ref().is_some_and(|c| c.name == want),
                    tokens: produced.token_count,
                    ttft_ms: ttft.as_secs_f64() * 1000.0,
                    decode_s: (total.saturating_sub(ttft)).as_secs_f64(),
                });
            }
        }
    }

    let summarise = |armed: &str| {
        let r: Vec<&GapRow> = rows.iter().filter(|r| r.armed == armed).collect();
        let n = r.len().max(1) as f64;
        let started_n = r.iter().filter(|r| r.started).count();
        let started = 100.0 * started_n as f64 / n;
        let valid_n = r.iter().filter(|r| r.valid).count();
        let valid = 100.0 * valid_n as f64 / n;
        // THE LINE THE GUARANTEE IS ABOUT: of the generations that actually
        // began a call, how many parsed. Constrained, this is the 100% the
        // front page claims; unconstrained it is the model's own syntax.
        let valid_given_started = if started_n > 0 {
            100.0 * valid_n.min(started_n) as f64 / started_n as f64
        } else {
            f64::NAN
        };
        let correct = 100.0 * r.iter().filter(|r| r.correct).count() as f64 / n;
        let ttft = r.iter().map(|r| r.ttft_ms).sum::<f64>() / n;
        let toks: usize = r.iter().map(|r| r.tokens).sum();
        // MEAN TOKENS PER GENERATION, because "started a call" moving between
        // the two columns has to be explained by something, and the first
        // candidate is that one column simply stops sooner.
        let mean_tokens = toks as f64 / n;
        let secs: f64 = r.iter().map(|r| r.decode_s).sum();
        let rate = if secs > 0.0 { toks as f64 / secs } else { 0.0 };
        (
            started,
            valid,
            valid_given_started,
            correct,
            ttft,
            rate,
            mean_tokens,
        )
    };
    let (s_off, v_off, g_off, c_off, t_off, r_off, m_off) = summarise("off");
    let (s_on, v_on, g_on, c_on, t_on, r_on, m_on) = summarise("on");

    println!();
    println!(
        "{}",
        color.paint(
            BOLD,
            &format!("  {} · {model_name} · {cases} calls", file.name)
        )
    );
    // A FILE WITH NOTHING TO MEASURE SAYS SO. `bench gap` only looks at steps
    // that EXPECT a tool, so an irrelevance file holds no calls at all — and
    // printing 0.0% on every row for that is a lie in the shape of a result.
    if cases == 0 {
        println!(
            "{}",
            color.paint(
                DIM,
                "  no step in this file expects a tool, so there is no call for the grammar to constrain and nothing to measure. `bench gap` reads tool cases; `bench run` is what scores an irrelevance file."
            )
        );
        return ExitCode::SUCCESS;
    }
    println!("                     grammar OFF   grammar ON    gap");
    println!(
        "  started a call     {s_off:>9.1}%   {s_on:>9.1}%   {:>+6.1}",
        s_on - s_off
    );
    println!(
        "  valid IF started   {g_off:>9.1}%   {g_on:>9.1}%   {:>+6.1}",
        g_on - g_off
    );
    println!(
        "  valid call rate    {v_off:>9.1}%   {v_on:>9.1}%   {:>+6.1}",
        v_on - v_off
    );
    println!(
        "  correct call rate  {c_off:>9.1}%   {c_on:>9.1}%   {:>+6.1}",
        c_on - c_off
    );
    println!("  time to 1st token  {t_off:>8.0}ms   {t_on:>8.0}ms");
    println!("  decode             {r_off:>6.1} tok/s   {r_on:>6.1} tok/s");
    println!(
        "  tokens per answer  {m_off:>9.0}    {m_on:>9.0}    {:>+6.0}",
        m_on - m_off
    );
    match peak_memory_mib() {
        Some(mib) => println!("  peak resident      {mib} MiB"),
        None => println!(
            "{}",
            color.paint(
                DIM,
                "  peak resident      not available on this OS (no /proc)"
            )
        ),
    }
    println!(
        "{}",
        color.paint(
            DIM,
            "  READ THE FIRST TWO LINES TOGETHER. The automaton arms after `name(`, so it \
can only guarantee a generation that STARTED a call: that is the `valid IF started` row, and \
constrained it is the 100% the front page claims. The `valid call rate` beneath it is the \
same number diluted by every turn that answered in prose instead — which the grammar has no \
say over. And correct is judgement, which it never claimed."
        )
    );
    ExitCode::SUCCESS
}

struct GapRow {
    armed: &'static str,
    started: bool,
    valid: bool,
    correct: bool,
    tokens: usize,
    ttft_ms: f64,
    decode_s: f64,
}

/// Peak resident set, from the kernel, where the kernel offers it.
///
/// `/proc/self/status`'s `VmHWM` is the high-water mark in kibibytes. macOS has
/// no `/proc` and the alternative is a libc call, which this workspace does not
/// take a dependency for — so there the answer is "not available", which is
/// true, rather than zero, which is not.
fn peak_memory_mib() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
        .map(|kib| kib / 1024)
}

#[cfg(test)]
mod gap_tests {
    use super::started_a_call;

    /// THE GENERATIONS THAT BROKE THE MEASUREMENT, kept verbatim. Every string
    /// here was produced by an unconstrained Qwen3-0.6B on
    /// `benchmarks/en/arithmetic-time.json` and counted as a started call by the
    /// `name(`-only test that produced the retracted 46% → 26% figure.
    #[test]
    fn a_parroted_signature_is_not_a_started_call() {
        let names = ["time", "calendar", "calculate"];
        for echo in [
            "(time(kind: \"clock\", target?: \"what time it is\"))",
            "<time(kind=\"date\", target:\"today\")>",
            "calendar(kind: 'date', target?: text).",
            "You would use calculate(expression) for that.",
            "calculate(\"84 + 12\")",
        ] {
            assert!(
                !started_a_call(echo, &names),
                "a signature echo counted as a call start: {echo}"
            );
        }
    }

    /// And the other direction, which is the half that can rot silently: if this
    /// ever returns false the `started` column collapses to zero and the gap
    /// table reads as though no model calls anything.
    #[test]
    fn a_real_call_is_a_started_call() {
        let names = ["time", "calculate"];
        for call in [
            "calculate({\"expression\":\"45*1.2\"})",
            "Sure — calculate({\"expression\":\"84 + (84 * 0.15)\"})",
            "calculate( {\"expression\":\"2+2\"})",
            "calculate(\n  {\"expression\":\"2+2\"})",
            "time({})",
        ] {
            assert!(
                started_a_call(call, &names),
                "a real call was missed: {call}"
            );
        }
    }

    /// A call to a tool that is not in the catalog is not this catalog's call.
    /// `selected.tools()` is what the model was shown, and counting a name it
    /// invented would credit the model for reaching a tool that does not exist.
    #[test]
    fn a_tool_outside_the_catalog_does_not_count() {
        assert!(!started_a_call(
            "weather({\"city\":\"istanbul\"})",
            &["time", "calculate"]
        ));
    }
}
