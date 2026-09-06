//! The `tacet eval` command family — the shell side of the three measurements.
//!
//! WHY IT IS ITS OWN FILE: these five entry points share nothing with the chat
//! loop except the process they run in, and they answer questions of a
//! different kind — not "what should the assistant do now" but "how well does
//! it do it, and can that number be believed". `main.rs` had grown to seven
//! thousand lines with this sitting in the middle of it.
//!
//! THE THREE MEASUREMENTS, and the order is the order to reach for them in:
//!
//! * `--routing` — the ROUTER's choice, no weights, milliseconds. It sets the
//!   ceiling on everything below: a tool the router leaves out of the budget is
//!   not in the prompt, so no model can call it.
//! * (no flag) — Tacet's LOGIC on the mock engine, deterministic, gates CI.
//! * `--tool-selection` — the MODEL's choice on real weights, twenty minutes.
//!
//! `--compare` is not a measurement but a verdict on two of them, and
//! `--format-gate` asks the narrow question the grammar normally hides: can
//! this model write a parseable call with no automaton helping it.

use crate::model_package;
use crate::ui::{BOLD, Color, DIM, YELLOW};
use crate::{candle_engine_from_path, model_not_found_report};
use std::process::ExitCode;
use std::sync::Arc;
use tacet_engine::{EngineProvider, Prompt, SamplingSetting, wait};
use tacet_eval::{FakeSelector, SYSTEM_INSTRUCTIONS};

// ---------------------------------------------------------------------------
// eval
// ---------------------------------------------------------------------------

/// The LOGIC set. `FakeEngine` by default; a named model switches the same case
/// list to a real one.
///
/// THE TWO MODES MEASURE DIFFERENT THINGS AND THE THRESHOLD ONLY FITS ONE.
/// With the fake engine the script pins every choice, the run is deterministic
/// and anything below 100% is a defect in Tacet's own logic — that is what
/// `--threshold` guards and what CI runs. With a real model the scripts are
/// ignored: the same cases now ask the model to pick the tools AND stay inside
/// what they returned (`EvalCase::grounded`), so failures are the model's, the
/// number moves between runs, and holding it to the CI threshold would report
/// the model as a regression in the shell. The threshold is therefore NOT
/// applied in that mode — the run is a measurement, not a gate.
pub fn eval(json: bool, threshold: f64, model_name: Option<&str>) -> ExitCode {
    let color = Color::setup();
    let selector: Box<dyn tacet_eval::EngineSelector> = match model_name {
        Some(name) => {
            let engine = match model_package::resolve_pair(name) {
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
                    model_not_found_report(name, &color);
                    eprintln!("error: --model was given, so the logic set REQUIRES that model.");
                    return ExitCode::FAILURE;
                }
            };
            eprintln!(
                "{}",
                color.paint(
                    DIM,
                    "(the scripts are IGNORED: this run measures the model, not the logic)"
                )
            );
            Box::new(tacet_eval::SingleEngine(engine))
        }
        None => Box::new(FakeSelector),
    };
    let report = tacet_eval::run(&tacet_eval::all(), selector.as_ref());
    if json {
        println!("{}", report.json());
    } else {
        print!("{}", report.table());
    }
    if model_name.is_some() || report.success_rate + f64::EPSILON >= threshold {
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "threshold not met: {:.3} < {threshold:.3}",
            report.success_rate
        );
        ExitCode::FAILURE
    }
}

/// THE FORMAT GATE — "can this model write a call this parser can read, with
/// no grammar helping it".
///
/// WHY THE GRAMMAR IS DELIBERATELY OFF (`None` below): everywhere else the
/// pushdown automaton makes a malformed call unrepresentable, which means the
/// prompt's own ability to teach the call shape is never measured. That ability
/// still matters — it decides whether the first pass of a turn is spent on a
/// call or on prose about a call — and this is the one place it is visible. So
/// the `None` is the measurement, not an oversight.
///
/// THE CATALOG IS THE ROUTER'S, NOT THE WHOLE THING. It used to pass the full
/// production catalog, which no turn in this program has ever shown a model:
/// production sends `Router::select`'s nine. A format gate over a prompt the
/// app never builds reports on a program that does not exist.
///
/// THE EXPECTED NAME IS CHECKED. It used to be bound to `_expected` and
/// ignored, so a model that answered every case with `time(...)` scored a
/// perfect gate. Two numbers are printed now and they answer different
/// questions: PARSED is the format (did anything callable come out), NAMED is
/// the choice (was it the right tool). Only the first is gated — the second is
/// what `--tool-selection` measures at length, and duplicating its threshold
/// here would fail the build twice for one cause.
///
/// THE THRESHOLD IS DERIVED FROM THE CASE COUNT. It was the literal `7` against
/// a list of 8, so adding a ninth case silently loosened the gate from 88% to
/// 78% and removing one would have made it unmeetable at 7/6.
pub fn eval_format_gate(engine: &Arc<dyn EngineProvider>) -> ExitCode {
    /// The share of cases that must produce a parseable call. 0.85 of 8 is 6.8
    /// -> 7, i.e. exactly the number that was hard-coded, now expressed as the
    /// claim it stood for.
    const FORMAT_FLOOR: f64 = 0.85;

    let cases = vec![
        ("What is 125 times 8?", "calculate"),
        ("What time is it right now?", "time"),
        ("Find file containing budget", "find_file"),
        ("Read document notes.md", "read_document"),
        (
            "Create an excel table for my shopping list",
            "create_document",
        ),
        ("Remember my birthday is May 3rd", "remember"),
        ("İstanbul'da yarın hava nasıl olacak?", "web_search"),
        ("What is on my calendar tomorrow?", "calendar"),
    ];

    let mut parsed_calls = 0;
    let mut named_right = 0;
    let total = cases.len();

    let env = match tacet_eval::Env::setup() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("format gate: failed to setup env: {e}");
            return ExitCode::FAILURE;
        }
    };
    let memory = tacet_tools::memory::SharedMemory::in_memory();
    let (catalog, _, _) = tacet_tools::catalog::production_catalog(&env.store, &memory, None);
    let router = tacet_tools::router::Router::new();

    for (msg, expected) in &cases {
        let selected: tacet_kernel::ToolCatalog =
            router.select(msg, &catalog).into_iter().collect();
        let prompt = Prompt::new(SYSTEM_INSTRUCTIONS, *msg).with_tools(&selected);
        let Ok(generation) = wait(engine.generate(&prompt, None, SamplingSetting::default()))
        else {
            continue;
        };
        let Some(call) = tacet_tools::executor::ToolCall::parse(&generation.text) else {
            continue;
        };
        parsed_calls += 1;
        if call.name == *expected {
            named_right += 1;
        }
    }

    let floor = (total as f64 * FORMAT_FLOOR).ceil() as usize;
    println!("FORMAT GATE  PARSED {parsed_calls}/{total} · NAMED {named_right}/{total}");
    if parsed_calls >= floor {
        ExitCode::SUCCESS
    } else {
        eprintln!("format gate failed: {parsed_calls}/{total} < {floor}");
        ExitCode::FAILURE
    }
}

/// COMPARES TWO EVAL RUNS AND SAYS WHETHER THE DIFFERENCE IS REAL.
///
/// WHY THIS COMMAND HAD TO BE WRITTEN: `tacet-eval`'s `analysis` module — a
/// sign test, a paired bootstrap, `cases_needed`, an AUROC, every one of them
/// pinned by hand-computed fixtures — was fully implemented and called from
/// NOWHERE outside its own tests. Its module header opens with the sentence
/// "every '+3 points' claim in this project has so far been read off two table
/// headers", and that stayed true after the module existed, because nothing
/// could reach it. Statistics nobody can run are a comment.
///
/// WHAT IT PAIRS: cases BY NAME, which is what makes the test paired and is
/// also the only honest way to compare two runs of a suite that may have
/// changed between them. Cases present in one file and not the other are
/// reported and excluded — silently dropping them would let a suite edit
/// masquerade as an improvement.
///
/// IT READS THE JSON GENERICALLY rather than through the report types. The
/// three reports in this program (`--json`, `--tool-selection --json`,
/// `--routing --json`) have different shapes and different lifetimes, and a
/// comparator that had to be recompiled whenever a field moved would go stale
/// the first time somebody added one. All three carry a list of named cases
/// with a pass/fail; that is the whole contract.
/// One case as `--compare` needs it: the name it is paired by, whether it
/// passed, and WHICH TOOLS IT TOUCHED — the last one so a case can be set aside
/// when the two runs did not have the same catalog.
#[derive(Clone)]
struct Case {
    name: String,
    passed: bool,
    tools: Vec<String>,
}

/// Which array a report's cases were read from — and therefore WHAT WAS
/// MEASURED.
///
/// `cases` is the logic and selection reports: a model ran. `outcomes` is the
/// routing report: no model ran at all, a case "passes" when the expected tool
/// merely REACHED the prompt. Pairing one against the other is not a comparison,
/// it is two different questions sharing case names — and it produced a verdict
/// on this machine: the routing report against the model baseline pairs on 155
/// names and prints `A REAL LOSS at 95%`, p = 0.0000, exit 0.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// `cases[]` — a model was run.
    Model,
    /// `outcomes[]` — the router was run and nothing else.
    Routing,
}

impl Shape {
    fn describe(self) -> &'static str {
        match self {
            Shape::Model => "a model report (`cases`)",
            Shape::Routing => "a routing report (`outcomes`) — no model ran",
        }
    }
}

/// A whole report: its cases, the shape it was read from, and the catalog the
/// run actually had. `catalog` is `None` for a report written before it was
/// recorded; the routing report DOES carry one (`RoutingReport::catalog`), and
/// a comment here used to say it did not.
struct Run {
    cases: Vec<Case>,
    shape: Shape,
    catalog: Option<Vec<String>>,
}

/// Every tool named by a case: what it EXPECTED and what was actually CALLED,
/// across all of its steps.
///
/// Both shapes of report are read here. The selection report keeps steps, each
/// with `expected` (a name or null) and `called` (a list). The routing report
/// has `expected` on the case itself and calls nothing. A field that is not
/// there contributes nothing rather than failing: this list is used to EXCLUDE,
/// so reading it pessimistically would quietly shrink the suite.
fn tools_touched(entry: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |v: Option<&serde_json::Value>| {
        if let Some(name) = v.and_then(serde_json::Value::as_str)
            && !out.iter().any(|t| t == name)
        {
            out.push(name.to_string());
        }
    };
    push(entry.get("expected"));
    if let Some(steps) = entry.get("steps").and_then(|v| v.as_array()) {
        for step in steps {
            push(step.get("expected"));
            if let Some(called) = step.get("called").and_then(|v| v.as_array()) {
                for c in called {
                    push(Some(c));
                }
            }
        }
    }
    out
}

/// The cases the two runs cannot be fairly compared on: those touching a tool
/// that one side's catalog did not have.
///
/// PURE AND SEPARATE so it can be tested by NAME rather than through an exit
/// code. The comparator's only other outcome is pass or fail, and "which cases
/// were set aside" is exactly the thing that must be right.
///
/// Empty when either report predates the `catalog` field, and empty when the two
/// catalogs agree — in both cases every paired name goes into the test, which is
/// the behaviour this comparator has always had.
fn incomparable_cases(before: &Run, after: &Run) -> Vec<String> {
    let (Some(cb), Some(ca)) = (&before.catalog, &after.catalog) else {
        return Vec::new();
    };
    if cb == ca {
        return Vec::new();
    }
    let common: Vec<&String> = cb.iter().filter(|t| ca.contains(t)).collect();
    let comparable = |c: &Case| c.tools.iter().all(|t| common.contains(&t));
    let mut out: Vec<String> = Vec::new();
    // Only PAIRED names: a case that exists on one side alone is already
    // reported as unpaired, and naming it twice would say the same thing in two
    // vocabularies.
    for (mine, theirs) in [(&before.cases, &after.cases), (&after.cases, &before.cases)] {
        for c in mine {
            if !comparable(c) && theirs.iter().any(|o| o.name == c.name) && !out.contains(&c.name) {
                out.push(c.name.clone());
            }
        }
    }
    out
}

pub fn eval_compare(before_path: &str, after_path: &str) -> ExitCode {
    let color = Color::setup();

    // THE TWO RUNS MUST BE THE SAME MODEL, and until now nothing checked.
    //
    // The comparator reads `cases` and nothing else, so it will pair two reports
    // made with DIFFERENT WEIGHTS and print a sign test as though the only thing
    // that changed was this repository. That is a number that looks authoritative
    // and measures something else.
    //
    // MEASURED, by walking into it: a CUDA run on a rented RTX 3090 was compared
    // against the checked-in Metal baseline, and the two model files differed —
    // 2 497 280 256 bytes against 2 497 281 120, both Q4_K_M, both "qwen3-4b",
    // 864 bytes and one declared context length apart (40960 vs 16954). The
    // report already carried `model_fingerprint` precisely so this could be
    // caught; nothing read it.
    //
    // IT REFUSES RATHER THAN WARNS. A warning above a verdict is a warning people
    // scroll past, and the verdict below it is wrong, not approximate. Comparing
    // across models is a legitimate thing to want — it is just not what a sign
    // test over paired cases answers.
    let fingerprint = |p: &str| -> Option<(String, String, String)> {
        let text = std::fs::read_to_string(p).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let id = v.get("identity")?;
        Some((
            id.get("model_fingerprint")?.as_str()?.to_string(),
            id.get("engine")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?")
                .to_string(),
            id.get("device")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?")
                .to_string(),
        ))
    };
    // THE DEVICE IS SAID OUT LOUD, AND NOT REFUSED.
    //
    // Different weights are refused because the verdict would be wrong. Two
    // devices are a different case: the same GGUF on Metal and on CUDA is a
    // comparison somebody legitimately wants, and this command was used to make
    // one — but it is not the same experiment, because `kv_cache_budget` differs
    // by device and the generation cap follows it. MEASURED, same weights, same
    // commit, 6 Sep 2026: the caps were ~14 000 tokens on Metal and 52 019 on a
    // rented RTX 3090, and ten cases that failed on the smaller budget passed on
    // the larger. That is a real result and it is also not "this change helped".
    //
    // So it is printed, the way the catalog difference below is printed, and the
    // reader decides.
    if let (Some((_, _, da)), Some((_, _, db))) =
        (fingerprint(before_path), fingerprint(after_path))
        && da != db
        && da != "?"
        && db != "?"
    {
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "these two runs were on DIFFERENT DEVICES ({da} -> {db}). The token budget \
follows the device's KV cache, so the two runs did not offer the model the same room to think. \
The verdict below is still paired and still honest; it is a comparison of two machines as much \
as of two builds."
                )
            )
        );
    }
    if let (Some((fa, ea, _)), Some((fb, eb, _))) =
        (fingerprint(before_path), fingerprint(after_path))
        && fa != fb
    {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "these two reports were produced by DIFFERENT WEIGHTS, so pairing \
them measures the model as much as the change:\n  before  {ea}  {}\n  after   {eb}  {}\n\
A sign test over paired cases answers whether THIS CHANGE helped, which needs one model \
held still. Re-run one side against the other's weights.",
                    // BY CHARACTER, NOT BY BYTE. `&fa[..16]` panics on a
                    // fingerprint whose sixteenth byte is inside a multi-byte
                    // character — an abort at exit 101 from the guard whose
                    // whole job is to refuse cleanly. The test that covered this
                    // used eight ASCII bytes, so the boundary was never reached.
                    fa.chars().take(16).collect::<String>(),
                    fb.chars().take(16).collect::<String>()
                )
            )
        );
        return ExitCode::FAILURE;
    }

    let load = |p: &str| -> Result<Run, String> {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("{p}: not JSON: {e}"))?;
        // `cases` is the selection and logic reports; `outcomes` is the routing
        // one. A routing case "passes" when the expected tool reached the model
        // AT ALL — the same claim the routing gate is tied to.
        let (list, shape) = match value.get("cases").and_then(|v| v.as_array()) {
            Some(l) => (l, Shape::Model),
            None => (
                value
                    .get("outcomes")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| {
                        format!("{p}: no `cases` or `outcomes` array — is this an eval report?")
                    })?,
                Shape::Routing,
            ),
        };
        let mut cases = Vec::new();
        for entry in list {
            let name = entry
                .get("name")
                .or_else(|| entry.get("case"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("{p}: a case has no name"))?;
            // A CASE WITH NEITHER `passed` NOR `rank` IS NOT A RESULT.
            //
            // The old code fell through to `false` for both, so a file of
            // `{"name": "..."}` objects — any JSON with a `cases` array — scored
            // every case as failed on BOTH sides and printed
            // `0/50 · fixed 0 · broke 0 · NOT DISTINGUISHABLE`. Every line of
            // that is arithmetically correct about data that was never read.
            let passed = match entry.get("passed").and_then(serde_json::Value::as_bool) {
                Some(b) => b,
                None => match entry.get("rank") {
                    Some(r) => !r.is_null(),
                    None => {
                        return Err(format!(
                            "{p}: case `{name}` carries neither `passed` nor `rank`, so there \
                             is no result to compare. A report this command cannot read scores \
                             every case as failed on both sides and still prints a verdict."
                        ));
                    }
                },
            };
            cases.push(Case {
                name: name.to_string(),
                passed,
                tools: tools_touched(entry),
            });
        }
        Ok(Run {
            cases,
            shape,
            catalog: value.get("catalog").and_then(|c| c.as_array()).map(|a| {
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            }),
        })
    };

    let (before, after) = match (load(before_path), load(after_path)) {
        (Ok(b), Ok(a)) => (b, a),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // TWO REPORTS OF DIFFERENT SHAPES ARE NOT TWO MEASUREMENTS OF ONE THING.
    //
    // `cases` means a model ran; `outcomes` means only the router did, and a
    // routing case "passes" when the expected tool merely reached the prompt.
    // They share case names, so nothing stopped them pairing — and the routing
    // report against the model baseline pairs on 155 of them and prints
    // `A REAL LOSS at 95%`, p = 0.0000, exit 0. The router measured, the model
    // reported, at whole-report scale.
    //
    // Refused rather than warned, for the reason the weights guard above states:
    // a warning over a verdict is a warning people scroll past.
    if before.shape != after.shape {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "these two reports measure DIFFERENT THINGS:\n  before  {}\n  after   {}\n\
A routing case passes when the expected tool reached the prompt; a model case passes when the \
model called it. Pairing them by name compares the router against the model and prints a \
verdict about neither.",
                    before.shape.describe(),
                    after.shape.describe()
                )
            )
        );
        return ExitCode::FAILURE;
    }

    // THE TWO RUNS MUST HAVE HAD THE SAME TOOLS, and until now nothing checked
    // this either.
    //
    // MEASURED, by walking into it a second time in one evening. A CUDA run on
    // a rented Linux box was compared against the checked-in Metal baseline —
    // same weights this time, so the fingerprint guard above was satisfied — and
    // the verdict read −6.0 points, twenty-one cases broken. Nineteen of the
    // twenty-one were `calendar-*`, `run_code-*` and `write_code-*`: three tools
    // that are DISCOVERED, not compiled in. The calendar bridge is macOS-only,
    // and on that box `bwrap` could not cut the network, so the sandbox tools
    // left the catalog exactly as they are designed to. The report has carried
    // `catalog` from the start; the comparator read `cases` and nothing else, so
    // a host difference was printed as a model regression. With those cases set
    // aside the same two files read +3.0.
    //
    // IT EXCLUDES RATHER THAN REFUSES, which is the opposite of the choice made
    // for the weights above, and the difference is worth stating: different
    // weights make EVERY case incomparable, so there is nothing left to report.
    // A missing tool makes exactly the cases that need it incomparable and
    // leaves the rest sound. Refusing there would throw away a legitimate
    // measurement of 164 cases to avoid a wrong one about 20.
    //
    // A CASE IS EXCLUDED IF IT TOUCHED A TOOL EITHER SIDE LACKED — `expected`
    // (what the case is about) and `called` (what actually happened) both count.
    // `called` matters because a model that reaches for an absent tool has been
    // handed a different problem, not a harder one.
    let excluded_for_tools = incomparable_cases(&before, &after);
    let (before, after) = match (&before.catalog, &after.catalog) {
        (Some(cb), Some(ca)) if cb != ca => {
            let missing_after: Vec<&String> = cb.iter().filter(|t| !ca.contains(t)).collect();
            let missing_before: Vec<&String> = ca.iter().filter(|t| !cb.contains(t)).collect();
            let say = |which: &str, list: &[&String]| {
                if list.is_empty() {
                    String::new()
                } else {
                    let names: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
                    format!("\n  not in {which}: {}", names.join(", "))
                }
            };
            eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    &format!(
                        "these two runs did not have the same tools, so {} case(s) that \
touch a tool one side lacked are EXCLUDED from the test below.{}{}\n\
A tool leaves the catalog when the host cannot support it — the calendar bridge is \
macOS-only, the sandbox tools need a working `bwrap` or `sandbox-exec` — so this is a \
difference between the two MACHINES, and counting it would report it as a difference \
between the two BUILDS.",
                        excluded_for_tools.len(),
                        say("after", &missing_after),
                        say("before", &missing_before),
                    )
                )
            );
            let keep = |c: &&Case| !excluded_for_tools.contains(&c.name);
            (
                before
                    .cases
                    .iter()
                    .filter(keep)
                    .cloned()
                    .collect::<Vec<_>>(),
                after.cases.iter().filter(keep).cloned().collect::<Vec<_>>(),
            )
        }
        _ => (before.cases, after.cases),
    };
    // Kept before the two lists are flattened into (name, passed) pairs: the
    // pairing floor below needs to know how much of each side it discarded.
    let (before_total, after_total) = (before.len(), after.len());
    let before: Vec<(String, bool)> = before.into_iter().map(|c| (c.name, c.passed)).collect();
    let after: Vec<(String, bool)> = after.into_iter().map(|c| (c.name, c.passed)).collect();

    let mut pairs: Vec<(bool, bool)> = Vec::new();
    let mut fixed_names: Vec<&str> = Vec::new();
    let mut broken_names: Vec<&str> = Vec::new();
    let mut only_before: Vec<&str> = Vec::new();
    for (name, b) in &before {
        match after.iter().find(|(n, _)| n == name) {
            Some((_, a)) => {
                pairs.push((*b, *a));
                if !*b && *a {
                    fixed_names.push(name);
                }
                if *b && !*a {
                    broken_names.push(name);
                }
            }
            None => only_before.push(name),
        }
    }
    let only_after: Vec<&str> = after
        .iter()
        .filter(|(n, _)| !before.iter().any(|(m, _)| m == n))
        .map(|(n, _)| n.as_str())
        .collect();

    // A HANDFUL OF SHARED NAMES IS NOT A PAIRING.
    //
    // The only refusal was `pairs.is_empty()`, so the logic baseline against the
    // model baseline — which is the DEFAULT pair the nightly job forms — throws
    // away 250 of 256 cases, keeps the 6 whose names happen to collide, and
    // prints a verdict from them. The nightly workflow tells its reader the
    // command "will refuse to pair" in that situation; it did not.
    //
    // The floor is both absolute and relative: six cases cannot resolve
    // anything, and a pairing that discards most of either side is measuring a
    // sub-suite nobody chose.
    const MIN_PAIRS: usize = 20;
    if pairs.is_empty() {
        eprintln!("error: the two reports share no case name — nothing to pair");
        return ExitCode::FAILURE;
    }
    let smaller = before_total.min(after_total);
    if pairs.len() < MIN_PAIRS || pairs.len() * 2 < smaller {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "these two reports pair on only {} case(s) — {} in the smaller report, \
{} in the larger.\nA verdict from that is a verdict about whichever sub-suite happened to \
share names. Pair reports from the same suite.",
                    pairs.len(),
                    smaller,
                    before_total.max(after_total)
                )
            )
        );
        return ExitCode::FAILURE;
    }

    let fixed = fixed_names.len();
    let broken = broken_names.len();
    let p = tacet_eval::analysis::sign_test(fixed, broken);
    // 2000 resamples and a FIXED SEED: the interval must not move when the
    // command is run twice on the same two files.
    let (delta, low, high) = tacet_eval::analysis::paired_bootstrap(&pairs, 2000, 0x5EED);

    let before_passed = pairs.iter().filter(|(b, _)| *b).count();
    let after_passed = pairs.iter().filter(|(_, a)| *a).count();
    let n = pairs.len();

    println!("paired on {n} cases");
    println!(
        "  before   {before_passed}/{n}  ({:.1}%)",
        100.0 * before_passed as f64 / n as f64
    );
    println!(
        "  after    {after_passed}/{n}  ({:.1}%)",
        100.0 * after_passed as f64 / n as f64
    );
    println!();
    println!("  fixed    {fixed}   {}", fixed_names.join(", "));
    println!("  broke    {broken}   {}", broken_names.join(", "));
    println!();
    println!(
        "  delta    {:+.1} points   95% CI [{:+.1}, {:+.1}]",
        delta * 100.0,
        low * 100.0,
        high * 100.0
    );
    println!("  sign test p = {p:.4}");
    if !only_before.is_empty() || !only_after.is_empty() {
        println!();
        println!(
            "  {} cases only in before, {} only in after — EXCLUDED from the test",
            only_before.len(),
            only_after.len()
        );
    }
    println!();
    // THE VERDICT IS THE POINT. A p-value printed on its own gets read as
    // whatever the reader hoped for; the module's own header states the
    // threshold (six one-way pairs), so the command states the conclusion.
    if p < 0.05 {
        let direction = if fixed > broken {
            "REAL"
        } else {
            "A REAL LOSS"
        };
        println!(
            "{}",
            color.paint(BOLD, &format!("  verdict: {direction} at 95%."))
        );
    } else {
        println!(
            "{}",
            color.paint(
                YELLOW,
                "  verdict: NOT DISTINGUISHABLE from no change at 95%."
            )
        );
        // NOTHING MOVED IS NOT A SMALL EFFECT, it is no observation at all —
        // and `cases_needed(0.0)` returns `usize::MAX` to say exactly that.
        // Printing the number would read as a suite-size requirement of
        // eighteen quintillion cases, which is true and useless.
        let points = delta.abs() * 100.0;
        if fixed == 0 && broken == 0 {
            println!(
                "           no case changed verdict — this is no evidence, not evidence of no effect."
            );
        } else {
            println!(
                "           this instrument needs {} paired cases to call a {points:.1}-point \
                 effect; this suite has {n}.",
                tacet_eval::analysis::cases_needed(points)
            );
        }
    }
    ExitCode::SUCCESS
}

/// The ROUTING measurement — the router's own choice, with no model loaded.
///
/// THE EXIT CODE IS TIED TO REACH AND NOT TO RANK, for the same reason the
/// selection gate is tied to irrelevance rather than accuracy: reach is a
/// LIMIT, not a score. A tool that is not inside the budget is not in the
/// prompt, so no model, however good, can call it — that is a defect in this
/// repository every time. Rank is a quality signal that moves whenever a tool
/// description is reworded, and gating on it would fail the build for edits
/// that made the prompt better.
pub fn eval_routing(
    json: bool,
    threshold: f64,
    turkish: bool,
    only: Option<&str>,
    pressure: usize,
) -> ExitCode {
    let color = Color::setup();
    let suite = if turkish {
        tacet_eval::Suite::Turkish
    } else {
        tacet_eval::Suite::Both
    };
    let report = match tacet_eval::run_routing_filtered(suite, only, pressure) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if report.total == 0 {
        eprintln!("error: no case matches the filter");
        return ExitCode::FAILURE;
    }

    // A tool the platform does not offer is NOT a router failure — say so
    // before the table, or the reader spends the run blaming the wrong layer.
    let missing = tacet_eval::missing_expectations(&report);
    if !missing.is_empty() {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "note: {} is not in the catalog on this machine; the cases expecting it \
                     cannot reach and are NOT counted against the router.",
                    missing.join(", ")
                )
            )
        );
    }

    if json {
        println!("{}", report.json());
    } else {
        print!("{}", report.table());
    }

    // The unreachable cases that the platform CANNOT explain — those are the
    // ones the gate is about.
    let unexplained = report
        .outcomes
        .iter()
        .filter(|o| !o.reached() && !missing.contains(&o.expected))
        .count();
    let reachable = report.total
        - report
            .outcomes
            .iter()
            .filter(|o| missing.contains(&o.expected))
            .count();
    let rate = if reachable == 0 {
        1.0
    } else {
        (reachable - unexplained) as f64 / reachable as f64
    };
    if rate + f64::EPSILON >= threshold {
        ExitCode::SUCCESS
    } else {
        eprintln!("routing reach below threshold: {rate:.3} < {threshold:.3}");
        ExitCode::FAILURE
    }
}

/// WHAT ONE SELECTION RUN IS, in one value.
///
/// The nine loose parameters this replaces were not merely a lint (`clippy` at
/// 9/7, which is what CI failed on): at the call site they were nine positional
/// values of which four were `Option` and three were `bool`, so swapping
/// `turkish` and `force_tool_name` would have compiled and measured a different
/// suite in silence. Named fields make that swap a compile error.
pub struct SelectionRun<'a> {
    pub json: bool,
    pub threshold: f64,
    pub model_name: &'a str,
    pub only: Option<&'a str>,
    pub turkish: bool,
    pub require_quant: Option<&'a str>,
    pub budget: Option<usize>,
    pub budget_sweep: Option<&'a str>,
    pub force_tool_name: bool,
}

pub fn eval_tool_selection(run: SelectionRun<'_>) -> ExitCode {
    let SelectionRun {
        json,
        threshold,
        model_name,
        only,
        turkish,
        require_quant,
        budget,
        budget_sweep,
        force_tool_name,
    } = run;
    let color = Color::setup();
    let engine = match model_package::resolve_pair(model_name) {
        Some((m, t)) => match candle_engine_from_path(&m, t.as_deref()) {
            Ok(engine) => {
                if model_package::pair_from_env().is_some() {
                    eprintln!(
                        "{}",
                        color.paint(
                            YELLOW,
                            &format!(
                                "warning: TACET_MODEL is set, so --model {model_name} was \
                                 IGNORED — this run measures {m}"
                            )
                        )
                    );
                }
                eprintln!("{}", color.paint(DIM, &format!("(model: {m})")));
                engine
            }
            Err(e) => {
                eprintln!("error: the real model could not be loaded: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => {
            model_not_found_report(model_name, &color);
            eprintln!("error: the tool selection measurement REQUIRES a real model.");
            return ExitCode::FAILURE;
        }
    };

    // BOTH LANGUAGES BY DEFAULT, which is what `--routing` has always done and
    // what this measurement did not.
    //
    // THE INSTRUMENT SAID SO ITSELF. Comparing two 115-case runs, `--compare`
    // answered "NOT DISTINGUISHABLE from no change at 95% ... this instrument
    // needs 230 paired cases to call a 2.6-point effect; this suite has 115."
    // A suite that cannot resolve the changes people actually make is not a
    // measurement, and the cheapest fix was sitting next to it: 65 Turkish cases
    // that the routing eval already runs in the same breath, while this one ran
    // one language or the other and never both.
    //
    // TURKISH IS NOT PADDING. Three of the defects fixed in this repository were
    // found only in Turkish (see `analysis.rs` and the `tr-dosya-ara` routing
    // case), because the tokenizer, the date words and the tool hints all behave
    // differently there. Running English alone measured the easier half.
    let mut cases = if turkish {
        tacet_eval::turkish_selection_cases()
    } else {
        tacet_eval::selection_suite()
    };
    if let Some(pattern) = only {
        cases.retain(|c| c.name.contains(pattern));
        if cases.is_empty() {
            eprintln!("error: no case matches the pattern '{pattern}'");
            return ExitCode::FAILURE;
        }
    }

    if let Some(sweep_str) = budget_sweep {
        let points: Vec<usize> = sweep_str
            .split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .collect();
        if points.is_empty() {
            eprintln!("error: invalid --budget-sweep format. Example: 6,9,12,0");
            return ExitCode::FAILURE;
        }
        println!("============================================================");
        println!("TOOL BUDGET SWEEP SUMMARY ({model_name})");
        println!("============================================================");
        println!(
            "{:<8} | {:<10} | {:<10} | {:<12} | {:<8}",
            "budget", "tool acc", "per-step", "irrelevance", "wall ms"
        );
        println!("------------------------------------------------------------");
        for b in points {
            let report =
                tacet_eval::run_selection_with_options(&cases, &engine, Some(b), force_tool_name);
            let b_label = if b == 0 {
                "all".to_string()
            } else {
                b.to_string()
            };
            println!(
                "{:<8} | {:<10.1}% | {:<10.1}% | {:<12.1}% | {} ms",
                b_label,
                report.tool_rate() * 100.0,
                report.answer_rate() * 100.0,
                report.irrelevance_rate() * 100.0,
                report.wall_ms
            );
        }
        println!("============================================================");
        return ExitCode::SUCCESS;
    }

    eprintln!(
        "{}",
        color.paint(
            DIM,
            &format!("({} cases running — takes minutes)", cases.len())
        )
    );

    let report = tacet_eval::run_selection_with_options(&cases, &engine, budget, force_tool_name);

    if let Some(req_q) = require_quant
        && !report
            .identity
            .quant
            .to_lowercase()
            .contains(&req_q.to_lowercase())
    {
        eprintln!(
            "error: required quantization '{req_q}' does not match engine quantization '{}'",
            report.identity.quant
        );
        return ExitCode::FAILURE;
    }

    if json {
        println!("{}", report.json());
    } else {
        print!("{}", report.table());
    }
    // A RUN WITH NO IRRELEVANCE CASE CANNOT BE GATED ON IRRELEVANCE, and the
    // distinction between the two ways that happens is the whole point:
    //
    //   * `--only write_code` — the USER asked for a subset, and there is no
    //     greeting in it. `ratio(0, 0)` is 0.0 by the deliberate rule in
    //     `ratio` ("an empty run must not look green"), so this exited FAILURE
    //     with "irrelevance threshold not met: 0.000" on a run whose four cases
    //     had all passed. The number was right and the verdict was nonsense.
    //   * NO filter — the suite itself has lost its irrelevance cases, which is
    //     exactly the silent hole `ratio` is written to catch, and it must
    //     still fail.
    //
    // The filter is what tells them apart, so the filter is what is consulted.
    if report.irrelevance_total == 0 {
        if only.is_some() {
            eprintln!(
                "note: the filter selected no irrelevance case, so the irrelevance gate                  did not run. This says nothing about tool appetite."
            );
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "error: the suite contains NO irrelevance case — the one limit that cannot              be broken is unmeasured."
        );
        return ExitCode::FAILURE;
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

#[cfg(test)]
mod compare_identity {
    use super::*;

    /// TWO RUNS OF DIFFERENT WEIGHTS MUST NOT BE PAIRED.
    ///
    /// MEASURED BY WALKING INTO IT. A CUDA run on a rented RTX 3090 was compared
    /// against the checked-in Metal baseline and the comparator answered happily
    /// — the two model files differ: 2 497 280 256 bytes against 2 497 281 120,
    /// both Q4_K_M, both called "qwen3-4b", 864 bytes and one declared context
    /// length apart (40960 against 16954). The report has carried
    /// `model_fingerprint` from the start so exactly this could be caught, and
    /// the comparator read `cases` and nothing else.
    ///
    /// A sign test over paired cases answers "did this change help". That
    /// question needs one model held still; across two it measures the weights
    /// as much as the diff, and prints a verdict either way.
    ///
    /// IT REFUSES RATHER THAN WARNS, and the exit code is what the test pins: a
    /// warning above a verdict is a warning people scroll past, and the verdict
    /// below it is wrong rather than approximate.
    #[test]
    fn a_comparison_across_two_models_is_refused() {
        let dir = std::env::temp_dir().join(format!("tacet-cmp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        // TWENTY-FOUR CASES, NOT TWO. The pairing floor added alongside this
        // guard refuses a verdict drawn from a handful of shared names, and a
        // two-case fixture is exactly what it exists to stop — so the fixture
        // has to be the size of a thing somebody would really compare.
        let write = |name: &str, fp: &str| {
            let path = dir.join(name);
            let cases: Vec<String> = (0..24)
                .map(|i| format!(r#"{{"name":"c{i}","passed":{}}}"#, i % 3 != 0))
                .collect();
            let body = format!(
                r#"{{"identity":{{"engine":"candle","model_fingerprint":"{fp}"}},
                    "cases":[{}]}}"#,
                cases.join(",")
            );
            std::fs::write(&path, body).expect("write");
            path.to_string_lossy().into_owned()
        };
        let one = write("one.json", "aaaa0000");
        let two = write("two.json", "bbbb1111");
        let same = write("same.json", "aaaa0000");

        assert_eq!(
            eval_compare(&one, &two),
            ExitCode::FAILURE,
            "two different fingerprints must not be paired"
        );
        // NOT VACUOUS: the identical pair must still compare, or the guard would
        // be "refuse everything" and pass the assertion above for free.
        assert_eq!(
            eval_compare(&one, &same),
            ExitCode::SUCCESS,
            "the same weights must still compare"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Writes a report of `n` cases with the given shape key (`cases` or
    /// `outcomes`) and body, for the guards below.
    fn report(dir: &std::path::Path, name: &str, body: String) -> String {
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write");
        path.to_string_lossy().into_owned()
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tacet-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// A ROUTING REPORT AND A MODEL REPORT MEASURE DIFFERENT THINGS.
    ///
    /// `outcomes` means only the router ran and a case passes when the expected
    /// tool REACHED the prompt; `cases` means a model ran and a case passes when
    /// it CALLED the tool. They share case names, so nothing stopped them
    /// pairing — and on this machine the routing report against the model
    /// baseline paired on 155 names and printed `A REAL LOSS at 95%`,
    /// p = 0.0000, exit 0. The router measured, the model reported.
    #[test]
    fn a_routing_report_and_a_model_report_are_not_paired() {
        let dir = scratch("cmp-shape");
        let entries: Vec<String> = (0..24)
            .map(|i| format!(r#"{{"case":"c{i}","rank":1}}"#))
            .collect();
        let routing = report(
            &dir,
            "routing.json",
            format!(r#"{{"outcomes":[{}]}}"#, entries.join(",")),
        );
        let entries: Vec<String> = (0..24)
            .map(|i| format!(r#"{{"name":"c{i}","passed":true}}"#))
            .collect();
        let model = report(
            &dir,
            "model.json",
            format!(r#"{{"cases":[{}]}}"#, entries.join(",")),
        );
        assert_eq!(
            eval_compare(&routing, &model),
            ExitCode::FAILURE,
            "a routing report must not pair against a model report"
        );
        // NOT VACUOUS in either direction: two of each still compare.
        assert_eq!(eval_compare(&routing, &routing), ExitCode::SUCCESS);
        assert_eq!(eval_compare(&model, &model), ExitCode::SUCCESS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SIX SHARED NAMES OUT OF 256 IS NOT A PAIRING.
    ///
    /// The only refusal was "no shared names at all", so the logic baseline
    /// against the model baseline — the default pair the nightly job forms —
    /// discarded 250 of 256 cases, kept the six whose names collide, and printed
    /// a verdict from them, while the workflow told its reader the command
    /// "will refuse to pair".
    #[test]
    fn a_verdict_is_refused_when_most_of_the_suite_was_discarded() {
        let dir = scratch("cmp-floor");
        let big: Vec<String> = (0..80)
            .map(|i| format!(r#"{{"name":"c{i}","passed":true}}"#))
            .collect();
        let small: Vec<String> = (0..6)
            .map(|i| format!(r#"{{"name":"c{i}","passed":false}}"#))
            .collect();
        let a = report(
            &dir,
            "big.json",
            format!(r#"{{"cases":[{}]}}"#, big.join(",")),
        );
        let b = report(
            &dir,
            "small.json",
            format!(r#"{{"cases":[{}]}}"#, small.join(",")),
        );
        assert_eq!(
            eval_compare(&a, &b),
            ExitCode::FAILURE,
            "six pairs out of eighty is a verdict about whichever names collided"
        );
        // NOT VACUOUS: a real pairing of the same suite still compares.
        assert_eq!(eval_compare(&a, &a), ExitCode::SUCCESS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A REPORT THIS COMMAND CANNOT READ IS NOT A REPORT OF ALL-FAILURES.
    ///
    /// A case with neither `passed` nor `rank` used to fall through to `false`,
    /// so any JSON with a `cases` array printed
    /// `0/50 · fixed 0 · broke 0 · NOT DISTINGUISHABLE` — every line
    /// arithmetically correct about data that was never read.
    #[test]
    fn a_report_with_no_results_in_it_is_refused() {
        let dir = scratch("cmp-unread");
        let entries: Vec<String> = (0..24).map(|i| format!(r#"{{"name":"c{i}"}}"#)).collect();
        let blank = report(
            &dir,
            "blank.json",
            format!(r#"{{"cases":[{}]}}"#, entries.join(",")),
        );
        assert_eq!(eval_compare(&blank, &blank), ExitCode::FAILURE);
        // NOT VACUOUS: `rank`-only (the routing shape) and `passed`-only both
        // remain readable.
        let ranked: Vec<String> = (0..24)
            .map(|i| format!(r#"{{"case":"c{i}","rank":2}}"#))
            .collect();
        let r = report(
            &dir,
            "ranked.json",
            format!(r#"{{"outcomes":[{}]}}"#, ranked.join(",")),
        );
        assert_eq!(eval_compare(&r, &r), ExitCode::SUCCESS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE GUARD'S OWN INPUT MUST NOT ABORT IT. `&fa[..16]` panics when the
    /// sixteenth byte falls inside a multi-byte character — exit 101 from the
    /// code whose entire job is to refuse cleanly. The old fixture used eight
    /// ASCII bytes, so the boundary was never reached.
    #[test]
    fn a_fingerprint_that_is_not_ascii_is_refused_not_aborted() {
        let dir = scratch("cmp-utf8");
        let cases: Vec<String> = (0..24)
            .map(|i| format!(r#"{{"name":"c{i}","passed":true}}"#))
            .collect();
        let one = report(
            &dir,
            "one.json",
            format!(
                r#"{{"identity":{{"engine":"candle","model_fingerprint":"{}é"}},"cases":[{}]}}"#,
                "a".repeat(15),
                cases.join(",")
            ),
        );
        let two = report(
            &dir,
            "two.json",
            format!(
                r#"{{"identity":{{"engine":"candle","model_fingerprint":"{}"}},"cases":[{}]}}"#,
                "b".repeat(32),
                cases.join(",")
            ),
        );
        assert_eq!(eval_compare(&one, &two), ExitCode::FAILURE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A report with no `identity` block still compares. The routing and
    /// fake-engine reports do not carry one, and they are the two this
    /// comparator is used on most.
    #[test]
    fn a_report_without_an_identity_still_compares() {
        let dir = std::env::temp_dir().join(format!("tacet-cmp2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("plain.json");
        let cases: Vec<String> = (0..24)
            .map(|i| format!(r#"{{"name":"c{i}","passed":true}}"#))
            .collect();
        std::fs::write(&path, format!(r#"{{"cases":[{}]}}"#, cases.join(","))).expect("write");
        let p = path.to_string_lossy().into_owned();
        assert_eq!(eval_compare(&p, &p), ExitCode::SUCCESS);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// THE TWO RUNS MUST HAVE HAD THE SAME TOOLS.
///
/// MEASURED BY WALKING INTO IT, on the same evening as the weights guard above.
/// A CUDA run on a rented Linux box was compared against the checked-in Metal
/// baseline — same weights this time, so the fingerprint guard was satisfied —
/// and the verdict read −6.0 points with twenty-one cases broken. Nineteen of
/// the twenty-one were `calendar-*`, `run_code-*` and `write_code-*`: tools that
/// are DISCOVERED rather than compiled in. The calendar bridge is macOS-only,
/// and on that box `bwrap` could not cut the network, so the sandbox tools left
/// the catalog exactly as they are designed to. With those cases set aside the
/// same two files read +3.0 — the sign of the answer came from the host.
#[cfg(test)]
mod compare_catalog {
    use super::*;

    /// A report with the given catalog and one case per (name, tool) pair.
    fn run(catalog: Option<&[&str]>, cases: &[(&str, &str)]) -> Run {
        Run {
            cases: cases
                .iter()
                .map(|(name, tool)| Case {
                    name: (*name).to_string(),
                    passed: true,
                    tools: vec![(*tool).to_string()],
                })
                .collect(),
            shape: Shape::Model,
            catalog: catalog.map(|c| c.iter().map(|s| (*s).to_string()).collect()),
        }
    }

    #[test]
    fn a_case_needing_a_tool_the_other_side_lacked_is_set_aside() {
        let before = run(
            Some(&["calculate", "calendar"]),
            &[("sums", "calculate"), ("diary", "calendar")],
        );
        let after = run(
            Some(&["calculate"]),
            &[("sums", "calculate"), ("diary", "calendar")],
        );
        assert_eq!(
            incomparable_cases(&before, &after),
            vec!["diary".to_string()],
            "the calendar case is a difference between the machines, not the builds"
        );
    }

    /// NOT VACUOUS. If the rule were "exclude everything when the catalogs
    /// differ" the assertion above would pass for free and the comparator would
    /// have nothing left to measure.
    #[test]
    fn the_cases_both_hosts_could_run_are_kept() {
        let before = run(
            Some(&["calculate", "calendar"]),
            &[("sums", "calculate"), ("diary", "calendar")],
        );
        let after = run(
            Some(&["calculate"]),
            &[("sums", "calculate"), ("diary", "calendar")],
        );
        assert!(
            !incomparable_cases(&before, &after).contains(&"sums".to_string()),
            "a case that only needed a tool both sides had is still comparable"
        );
    }

    /// THE TOOL A CASE ACTUALLY CALLED COUNTS, not only the one it expected. A
    /// model that reaches for a tool the other host did not have was handed a
    /// different problem, and pairing the two answers measures that instead of
    /// the change.
    #[test]
    fn a_case_that_called_an_absent_tool_is_also_set_aside() {
        let mut before = run(Some(&["calculate", "run_code"]), &[("sums", "calculate")]);
        before.cases[0].tools.push("run_code".to_string());
        let after = run(Some(&["calculate"]), &[("sums", "calculate")]);
        assert_eq!(
            incomparable_cases(&before, &after),
            vec!["sums".to_string()]
        );
    }

    /// Two runs on the same host exclude nothing — the ordinary case, and the
    /// one every existing comparison is.
    #[test]
    fn matching_catalogs_exclude_nothing() {
        let c = Some(&["calculate", "calendar"][..]);
        let before = run(c, &[("sums", "calculate"), ("diary", "calendar")]);
        let after = run(c, &[("sums", "calculate"), ("diary", "calendar")]);
        assert!(incomparable_cases(&before, &after).is_empty());
    }

    /// A report written before `catalog` was recorded, and the routing report,
    /// which has no catalog at all: both must compare exactly as they did.
    #[test]
    fn a_report_without_a_catalog_excludes_nothing() {
        let before = run(None, &[("sums", "calculate"), ("diary", "calendar")]);
        let after = run(Some(&["calculate"]), &[("sums", "calculate")]);
        assert!(incomparable_cases(&before, &after).is_empty());
    }

    /// A case only one side ran is already reported as unpaired. Naming it here
    /// too would say the same thing in two vocabularies and inflate the count
    /// the warning prints.
    #[test]
    fn an_unpaired_case_is_not_counted_as_incomparable() {
        let before = run(
            Some(&["calculate", "calendar"]),
            &[("sums", "calculate"), ("diary", "calendar")],
        );
        let after = run(Some(&["calculate"]), &[("sums", "calculate")]);
        assert!(incomparable_cases(&before, &after).is_empty());
    }
}
