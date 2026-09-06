//! MEASURED AGAINST SOMEONE ELSE'S BENCHMARK.
//!
//! Every number on this project's README is this project grading itself on cases
//! it wrote. That is a real limitation and it is the first thing a reviewer who
//! knows the field will notice: the "irrelevance gate" is the Berkeley Function
//! Calling Leaderboard's relevance/irrelevance detection under another name, and
//! nothing here had ever been run against it.
//!
//! This runs BFCL's `irrelevance` category — 240 cases — through the real
//! engine, the real prompt, the real router and the real grammar, with BFCL's
//! OWN function definitions rather than this project's catalog. The correct
//! behaviour on every case is to call nothing.
//!
//!     cargo run --release -p tacet-eval --features metal --example \
//!         bfcl_irrelevance -- ~/models/qwen3-4b/model.gguf /tmp/bfcl_irr.json
//!
//! Get the data (it is not vendored — it is someone else's benchmark and it
//! moves):
//!
//!     curl -sLO https://raw.githubusercontent.com/ShishirPatil/gorilla/main/\
//!         berkeley-function-call-leaderboard/bfcl_eval/data/BFCL_v4_irrelevance.json
//!
//! WHAT THIS IS NOT. It is not a leaderboard submission and the number is not
//! comparable to the published board: BFCL scores through its own harness, its
//! own prompt and its own parser, and every one of those is part of what is
//! being measured there. What this answers is narrower and still worth having —
//! *given the same questions and the same functions, how often does THIS stack
//! invent a call*. Three translations stand between the two, and each is stated
//! where it happens: BFCL's `dict`/`float` type names, tool names carrying
//! characters this call format cannot express, and one turn rather than a
//! conversation.

use serde_json::Value;
use std::sync::Arc;
use tacet_eval::tool_selection::{CatalogFor, run_selection_case_in};
use tacet_eval::{Category, SelectionCase, env::Env};
use tacet_kernel::{ArgSchema, Tool, ToolCatalog, ToolContext, ToolFuture, ToolOutcome, boxed};
use tacet_tools::memory::SharedMemory;

/// A tool that exists to be OFFERED and never to be run.
///
/// Every case here is an irrelevance case, so a call is by definition the
/// failure being counted — there is nothing to execute. Returning an error
/// rather than a plausible result also keeps a second turn from being scored on
/// a result this harness invented.
struct Offered {
    name: String,
    description: String,
    schema: ArgSchema,
}

impl Tool for Offered {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn schema(&self) -> ArgSchema {
        self.schema.clone()
    }
    fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move { ToolOutcome::read_ok("bfcl", "this benchmark does not execute tools") })
    }
}

/// TRANSLATION ONE: BFCL's type names are not JSON Schema's.
///
/// It writes `"type": "dict"` where JSON Schema writes `"object"`, and `"float"`
/// where it writes `"number"`. `bridge::convert_schema` — the translator a real
/// MCP server's tools come through — speaks JSON Schema, so the names are
/// rewritten before it sees them and nothing else is touched. Doing it the other
/// way round, by teaching the bridge BFCL's dialect, would put benchmark-only
/// code on the path every user's remote tools take.
fn to_json_schema(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let v = if k == "type" {
                    match v.as_str() {
                        Some("dict") => Value::from("object"),
                        Some("float") => Value::from("number"),
                        Some("tuple") => Value::from("array"),
                        _ => to_json_schema(v),
                    }
                } else {
                    to_json_schema(v)
                };
                out.insert(k.clone(), v);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(to_json_schema).collect()),
        other => other.clone(),
    }
}

/// TRANSLATION TWO: a name this call format cannot express.
///
/// BFCL has `math.sum` and `HNL.query`. This project's call format is
/// `name({...})` and its prefix automaton treats a name as an identifier run, so
/// a dot ENDS the name — `math.sum(` would arm the grammar on `math`, find no
/// such tool, and read as prose. The rewrite is the same shape `tacet-tools`
/// applies to a bridged MCP tool for the same reason. How many names needed it
/// is reported, because a benchmark that quietly rewrites a quarter of its
/// inputs is not the benchmark it says it is.
fn expressible(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(model), Some(data)) = (args.first(), args.get(1)) else {
        eprintln!(
            "usage: bfcl_irrelevance <model.gguf> <BFCL_v4_irrelevance.json> [limit]\n\
             see the header of this file for where the data comes from"
        );
        std::process::exit(2);
    };
    // A third argument caps the run. It exists for the smoke test — a harness
    // whose first full pass is also its first pass is a harness nobody has
    // debugged — and the cap is PRINTED in the summary so a partial run can
    // never be mistaken for the whole set.
    let limit: usize = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);

    let text = std::fs::read_to_string(data).expect("the BFCL file is readable");
    let entries: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .take(limit)
        .collect();
    println!(
        "BFCL irrelevance: {} cases{}",
        entries.len(),
        if limit == usize::MAX {
            String::new()
        } else {
            format!("  (CAPPED at {limit} — this is not the whole set)")
        }
    );

    let setting = tacet_engine::ModelSetting::from_gguf(model);
    tacet_engine::CandleEngine::files_exist(&setting).expect("the weights exist");
    let engine = tacet_engine::CandleEngine::load(&setting).expect("the weights load");
    println!(
        "engine: {} / {} / {}",
        engine.architecture().name(),
        engine.tokenizer_source().name(),
        model
    );
    let engine: Arc<dyn tacet_engine::EngineProvider> = Arc::new(engine);

    let mut passed = 0usize;
    let mut renamed = 0usize;
    let mut untranslatable = 0usize;
    // THE ANSWER IS KEPT FOR EVERY CASE THAT CALLED SOMETHING, because a name
    // on its own does not say whether the model produced a call or whether this
    // project's recovery layer read one out of prose. Those are opposite
    // findings and the first run of this harness produced one of each.
    let mut called: Vec<(String, Vec<String>, String)> = Vec::new();
    let started = std::time::Instant::now();

    for (i, entry) in entries.iter().enumerate() {
        let id = entry["id"].as_str().unwrap_or("?").to_string();
        // TRANSLATION THREE: BFCL nests the turns as `question[[{role, content}]]`.
        // The irrelevance category is single-turn, so the first user message IS
        // the case; a multi-turn category would need the history threaded and
        // this harness does not claim to do that.
        let message = entry["question"][0][0]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if message.is_empty() {
            continue;
        }

        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        for function in entry["function"].as_array().cloned().unwrap_or_default() {
            let raw_name = function["name"].as_str().unwrap_or("tool");
            let name = expressible(raw_name);
            if name != raw_name {
                renamed += 1;
            }
            let params = to_json_schema(&function["parameters"]);
            let Ok(converted) = tacet_mcp::bridge::convert_schema(&params) else {
                // A schema this project's grammar cannot express. Counted and
                // reported rather than silently dropped: a case offered fewer
                // functions than BFCL offered is an easier case.
                untranslatable += 1;
                continue;
            };
            tools.push(Arc::new(Offered {
                name,
                description: function["description"]
                    .as_str()
                    .unwrap_or("A tool.")
                    .to_string(),
                schema: converted.schema,
            }));
        }
        if tools.is_empty() {
            continue;
        }

        let case = SelectionCase {
            name: id.clone(),
            category: Category::Irrelevance,
            steps: vec![tacet_eval::tool_selection::SelectionStep::new(
                &message, None,
            )],
        };
        let build: CatalogFor<'_> = &|_env: &Env, _memory: &SharedMemory| {
            let mut catalog = ToolCatalog::new();
            for tool in &tools {
                catalog.add(Arc::clone(tool));
            }
            catalog
        };
        let outcome = run_selection_case_in(&case, &engine, None, false, build);
        let names: Vec<String> = outcome
            .steps
            .iter()
            .flat_map(|s| s.called.clone())
            .collect();
        if names.is_empty() {
            passed += 1;
        } else {
            let answer = outcome
                .steps
                .first()
                .map(|s| s.answer.clone())
                .unwrap_or_default();
            called.push((id, names, answer));
        }
        if (i + 1) % 20 == 0 {
            println!(
                "  {}/{}  clean {}  ({:.0?} elapsed)",
                i + 1,
                entries.len(),
                passed,
                started.elapsed()
            );
        }
    }

    let total = passed + called.len();

    // THE ARTIFACT. A number quoted on a page with nothing behind it is a number
    // the next person has to take on trust, and this one is quoted on the README.
    // Written next to the summary rather than instead of it: the file carries the
    // environment stamp and every case that called something, so the rate can be
    // recomputed rather than believed.
    let artifact = serde_json::json!({
        "benchmark": "BFCL v4 irrelevance",
        "source": "gorilla/berkeley-function-call-leaderboard/bfcl_eval/data/BFCL_v4_irrelevance.json",
        "harness": "tacet-eval/examples/bfcl_irrelevance.rs",
        "not_a_leaderboard_submission": "BFCL scores through its own harness, prompt and parser; this is the same questions and functions through this stack",
        // THE BARE FILE NAME, NEVER THE PATH. `~/models/<name>/model.gguf` is
        // where this lives on the machine that ran it, and this repository is
        // public. The rule is CONTRIBUTING's, and `cargo test -p tacet-eval
        // --test baselines` enforces it for the reports it knows about; this
        // one writes it correctly rather than relying on someone remembering to
        // scrub it afterwards.
        "model": std::path::Path::new(model)
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "entries_in_file": entries.len(),
        "cases_scored": total,
        "called_nothing": passed,
        "rate": passed as f64 / total.max(1) as f64,
        "tool_names_rewritten": renamed,
        "schemas_untranslatable": untranslatable,
        "wall_s": started.elapsed().as_secs(),
        "called": called.iter().map(|(id, names, answer)| serde_json::json!({
            "id": id, "called": names, "answer": answer,
        })).collect::<Vec<_>>(),
    });
    if let Some(path) = std::env::var_os("BFCL_JSON") {
        let text = serde_json::to_string_pretty(&artifact).expect("the report serialises");
        std::fs::write(&path, text).expect("the report is writable");
        eprintln!("wrote {}", std::path::Path::new(&path).display());
    }

    println!("\nBFCL irrelevance (v4), single turn, BFCL's own functions");
    if limit != usize::MAX {
        println!("  *** CAPPED RUN: {limit} of the file, not the whole category ***");
    }
    println!("  cases scored          {total}");
    println!(
        "  called nothing        {passed}/{total}  ({:.1}%)",
        100.0 * passed as f64 / total.max(1) as f64
    );
    println!("  tool names rewritten  {renamed}   (a dot cannot appear in a call)");
    println!("  schemas untranslatable {untranslatable}");
    println!("  wall                  {:.0?}", started.elapsed());
    if !called.is_empty() {
        println!("\n  the ones that called something:");
        for (id, names, answer) in called.iter().take(40) {
            let answer: String = answer.chars().take(110).collect();
            println!(
                "    {id:<18} {}\n                       | {}",
                names.join(", "),
                answer.replace('\n', " ")
            );
        }
        if called.len() > 40 {
            println!("    … and {} more", called.len() - 40);
        }
    }
}
