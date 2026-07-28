//! The tool SELECTION measurement — "did the model call the right tool".
//!
//! WHY A SEPARATE SET: the set in `case.rs` measures Tacet's LOGIC and
//! therefore runs with `FakeEngine` — there the model's choice is pinned by the
//! script. What is measured here is the exact opposite: NO script, the FULL
//! catalog, and one single question — "which tool did the model call when it saw
//! the user's sentence". This measurement is only meaningful with a REAL engine;
//! run with FakeEngine, what it measures is its own script.
//!
//! TWO NUMBERS ARE REPORTED SEPARATELY — HIT RATE and IRRELEVANCE. Every change
//! that makes the tools get called more aggressively raises the hit rate and, in
//! the same move, starts calling tools for a greeting. A single "success rate"
//! hides that trade-off: 6 irrelevance cases get lost among 23 tool cases and a
//! regression looks like a couple of percent of wobble. Separate numbers make
//! the trade-off VISIBLE.
//!
//! THE CATALOG IS THE SAME AS IN PRODUCTION (see
//! `tacet_tools::catalog::production_catalog`). A selection measured with a
//! shortened catalog is not the selection the application makes: if the model
//! chooses among 10 tools, the measurement must see 10 tools too.
//!
//! NO NETWORK. `web_search`/`web_fetch` ARE DRIED OUT: the name, the description
//! and the schema are preserved EXACTLY (those are what determine the
//! selection), only the body returns a fixed string instead of opening a socket.
//! That way the question "was the web tool selected" is answered independently
//! of whether the user's search server happens to be up.
//!
//! LANGUAGE: the case messages are English. Turkish phrasings measured earlier
//! were carried over one to one, so the intents are the same ones; if
//! multilingual selection is to be measured, a separate set is written — a
//! mixed-language set makes a single hit-rate number unreadable.

use crate::case::FIXED_EPOCH;
use crate::env::Env;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tacet_engine::{
    EngineProvider, MAX_TURNS, Prompt, SYSTEM_INSTRUCTIONS, SamplingSetting, Turn, wait,
};
use tacet_grammar::CallConstraint;
use tacet_kernel::{
    ArgSchema, Tool, ToolCatalog, ToolContext, ToolFuture, ToolOutcome, TraceCollector, boxed,
};
use tacet_tools::executor::ToolExecutor;
use tacet_tools::memory::SharedMemory;
use tacet_tools::router::Router;

/// The names of the tools that open the network — they are dried out in this set.
const TO_DRY: &[&str] = &["web_search", "web_fetch"];

/// THE TOOLS BOUND TO THE DISCOVERY GATE — THEY MAY NOT BE IN THE CATALOG.
///
/// `run_code` and `write_code` are only in the catalog when the machine has an
/// interpreter AND a MEASURED network shield (see `RunCodeTool::discover`):
/// `sandbox-exec` on macOS, `bwrap` on Linux; there is no equivalent on Windows.
/// So the absence of these two tools IS NOT A REGRESSION, it is a fact of the
/// platform — and the whole reason this list exists is to be able to tell the
/// two apart: a tool that is NOT in this list dropping out of the catalog is
/// still a regression and fails the test.
///
/// `calendar` JOINED THE LIST WHEN LINUX CI SAID SO, and it is worth writing
/// down which of the two claims turned out to be wrong. The tool reaches the
/// user's calendar through `/usr/bin/osascript`, so it is macOS-only by
/// construction (`#[cfg(target_os = "macos")]` in `calendar.rs`) — but the
/// comment above says the list is for tools bound to a DISCOVERY gate, and
/// `calendar` is bound to a COMPILE-TIME one. The distinction does not matter to
/// the test, which asks a single question: is this name absent for a reason the
/// platform can explain, or is it a typo in a case? Both reasons are the
/// platform. What DID matter is that nobody could answer it before the tool ran
/// on something other than a Mac — this repository's own eval was red on Linux
/// for a tool that was working exactly as designed.
const DISCOVERY_BOUND: &[&str] = &["run_code", "write_code", "calendar"];

// ---------------------------------------------------------------------------
// The dry tool
// ---------------------------------------------------------------------------

/// Neutralizes a tool while PRESERVING ITS IDENTITY.
///
/// What determines the selection is the tool's name, description and schema;
/// not its body. This wrapper passes all three through unchanged and only
/// replaces `run`. The reason for wrapping the real tool instead of WRITING a
/// fake one: when the description changes in production the measurement
/// automatically measures that description, and nobody can forget to keep two
/// texts in sync.
struct DryTool(Arc<dyn Tool>);

impl Tool for DryTool {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn schema(&self) -> ArgSchema {
        self.0.schema()
    }
    fn taints_session(&self) -> bool {
        self.0.taints_session()
    }
    fn run<'a>(&'a self, _args: Value, _ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        // Fixed but NOT EMPTY: a tool returning "no results" pushes the model to
        // try another tool and would break the call sequence.
        boxed(async move {
            ToolOutcome::read_ok(
                "dry tool",
                "1. Sample result — the network is off in this case.",
            )
        })
    }
}

// ---------------------------------------------------------------------------
// The case shape
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Category {
    /// A tool must be called.
    Tool,
    /// NO tool must be called.
    Irrelevance,
    /// Several user messages; the history is preserved.
    MultiTurn,
}

impl Category {
    pub fn name(&self) -> &'static str {
        match self {
            Category::Tool => "tool",
            Category::Irrelevance => "irrelevance",
            Category::MultiTurn => "multi-turn",
        }
    }
}

/// A single user message and the tool expected from it.
#[derive(Debug, Clone, Serialize)]
pub struct SelectionStep {
    pub message: String,
    /// `None` = no tool must be called for this message.
    pub expected: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectionCase {
    pub name: String,
    pub category: Category,
    pub steps: Vec<SelectionStep>,
}

impl SelectionCase {
    /// A single-message tool case.
    fn tool(name: &str, message: &str, expected: &str) -> Self {
        Self {
            name: name.into(),
            category: Category::Tool,
            steps: vec![SelectionStep {
                message: message.into(),
                expected: Some(expected.into()),
            }],
        }
    }

    /// A case where a tool must NOT be called.
    fn chat(name: &str, message: &str) -> Self {
        Self {
            name: name.into(),
            category: Category::Irrelevance,
            steps: vec![SelectionStep {
                message: message.into(),
                expected: None,
            }],
        }
    }

    /// A multi-turn case: a sequence of `(message, expected tool)`.
    fn chain(name: &str, steps: &[(&str, &str)]) -> Self {
        Self {
            name: name.into(),
            category: Category::MultiTurn,
            steps: steps
                .iter()
                .map(|(m, e)| SelectionStep {
                    message: (*m).into(),
                    expected: Some((*e).into()),
                })
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// The case set
// ---------------------------------------------------------------------------

/// The tool selection case set.
///
/// AT LEAST TWO CASES FOR EVERY TOOL IN THE CATALOG, and the same intent in
/// different phrasings. Measuring with a single phrasing misleads: "what time is
/// it" may work while "what is the date today" does not, and a single-case set
/// reports that as "the time tool is fine". Phrasing variety is the only tool
/// that asks whether the description really covers the intent.
/// The TURKISH selection set — a SEPARATE list, exactly as the module doc
/// promises: mixing languages into one list would make the single hit-rate
/// number unreadable. The intents mirror the English set's core (arithmetic,
/// clock, dates in natural Turkish, documents, files, web, memory, smalltalk
/// that must select NOTHING), so the two reports are comparable side by side.
/// Run with `tacet eval --tool-selection --turkish`.
pub fn turkish_selection_cases() -> Vec<SelectionCase> {
    vec![
        // --- calculate ---
        SelectionCase::tool("tr-hesap-carpma", "125 çarpı 8 kaç eder?", "calculate"),
        SelectionCase::tool("tr-hesap-yuzde", "480'in yüzde 18'i ne kadar?", "calculate"),
        SelectionCase::tool(
            "tr-hesap-toplama",
            "347 ile 268'i toplar mısın?",
            "calculate",
        ),
        // --- time ---
        SelectionCase::tool("tr-saat", "Saat kaç şu an?", "time"),
        SelectionCase::tool("tr-tarih", "Bugün ayın kaçı?", "time"),
        SelectionCase::tool("tr-gun-farki", "Yılbaşına kaç gün kaldı?", "time"),
        SelectionCase::tool("tr-hafta-gunu", "Bugün günlerden ne?", "time"),
        SelectionCase::tool("tr-dogal-tarih", "Önümüzdeki salıya kaç gün var?", "time"),
        // --- documents ---
        SelectionCase::tool(
            "tr-belge-olustur",
            "Alışveriş listemi bir excel tablosu yap",
            "create_document",
        ),
        SelectionCase::tool(
            "tr-belge-oku",
            "notlar.md dosyasında ne yazıyor, özetler misin?",
            "read_document",
        ),
        SelectionCase::tool(
            "tr-belge-duzenle",
            "Az önceki tabloya bir satır daha ekle",
            "edit_document",
        ),
        // --- files ---
        SelectionCase::tool(
            "tr-dosya-ara",
            "Bütçeyle ilgili notu hangi dosyaya yazmıştım?",
            "find_file",
        ),
        // --- code ---
        SelectionCase::tool(
            "tr-kod-calistir",
            "1'den 100'e kadar asal sayıları listeler misin?",
            "run_code",
        ),
        SelectionCase::tool(
            "tr-kod-dosya",
            "Bana fibonacci hesaplayan bir python betiği yaz ve kaydet",
            "write_code",
        ),
        // --- web ---
        SelectionCase::tool(
            "tr-hava",
            "İstanbul'da yarın hava nasıl olacak?",
            "web_search",
        ),
        SelectionCase::tool("tr-haber", "Dolar kuru şu an ne durumda?", "web_search"),
        // --- memory ---
        SelectionCase::tool(
            "tr-hatirla",
            "Kardeşimin doğum günü 3 mayıs, bunu unutma",
            "remember",
        ),
        SelectionCase::tool("tr-unut", "Kahve sevdiğimi unut artık", "remember"),
        // --- irrelevance: NOTHING must be selected ---
        SelectionCase::chat("tr-selam", "Selam, nasılsın?"),
        SelectionCase::chat("tr-tesekkur", "Çok teşekkürler, harikaydı!"),
        SelectionCase::chat("tr-sohbet", "Bugün biraz yorgunum ya"),
        SelectionCase::chat("tr-fikir", "Sence sabah sporu mu akşam sporu mu daha iyi?"),
    ]
}

pub fn selection_cases() -> Vec<SelectionCase> {
    vec![
        // --- calendar (macOS bridge; the tool is in the production catalog there) ---
        SelectionCase::tool(
            "calendar-day",
            "What is on my calendar tomorrow?",
            "calendar",
        ),
        SelectionCase::tool(
            "calendar-remind",
            "Remind me to call the dentist tomorrow at 9",
            "calendar",
        ),
        // --- calculate ---
        SelectionCase::tool("calculate-multiply", "What is 125 times 8?", "calculate"),
        SelectionCase::tool("calculate-add", "Could you add 347 and 268?", "calculate"),
        SelectionCase::tool(
            "calculate-percent",
            "How much is 250 lira with a 20 percent discount?",
            "calculate",
        ),
        // --- time --- (four separate phrasings: this tool was missed twice in
        // manual measurement)
        SelectionCase::tool("time-clock", "What time is it?", "time"),
        SelectionCase::tool(
            "time-day-of-month",
            "What day of the month is it today?",
            "time",
        ),
        SelectionCase::tool("time-todays-date", "What is today's date?", "time"),
        SelectionCase::tool(
            "time-diff",
            "How many days are left until new year?",
            "time",
        ),
        // --- read_document ---
        SelectionCase::tool(
            "read_document-content",
            "What does the file report.md say?",
            "read_document",
        ),
        SelectionCase::tool(
            "read_document-summary",
            "Could you summarize the file budget-2026.md?",
            "read_document",
        ),
        // --- create_document ---
        SelectionCase::tool(
            "create_document-excel",
            "Turn the weekly meal list into an excel file.",
            "create_document",
        ),
        SelectionCase::tool(
            "create_document-markdown",
            "Create a short markdown file for me for the meeting notes.",
            "create_document",
        ),
        // --- edit_document --- (calling read_document first is ALSO CORRECT:
        // that is what the description says. The claim is "was edit_document
        // called by the end of the loop".)
        SelectionCase::tool(
            "edit_document-row",
            "Add the row 'Thursday | Chickpeas' to the file report.md.",
            "edit_document",
        ),
        SelectionCase::tool(
            "edit_document-title",
            "Change the title of the file budget-2026.md to 'New Budget'.",
            "edit_document",
        ),
        // --- find_file ---
        SelectionCase::tool(
            "find_file-name",
            "Find the file about the budget.",
            "find_file",
        ),
        SelectionCase::tool(
            "find_file-content",
            "Which of my files mentions 'Lentils'?",
            "find_file",
        ),
        // --- web_search ---
        SelectionCase::tool(
            "web_search-weather",
            "What is the weather like in Istanbul?",
            "web_search",
        ),
        SelectionCase::tool(
            "web_search-current",
            "How much is the dollar today?",
            "web_search",
        ),
        // --- web_fetch ---
        SelectionCase::tool(
            "web_fetch-page",
            "Read the content of the page https://example.com/blog.",
            "web_fetch",
        ),
        SelectionCase::tool(
            "web_fetch-address",
            "Get me the detail of the article at this address: https://example.com/article",
            "web_fetch",
        ),
        // --- remember ---
        SelectionCase::tool(
            "remember-save",
            "Remember this: I drink my coffee without milk.",
            "remember",
        ),
        SelectionCase::tool(
            "remember-list",
            "List the notes you keep about me.",
            "remember",
        ),
        // --- run_code ---
        SelectionCase::tool(
            "run_code-primes",
            "List the prime numbers from 1 to 30.",
            "run_code",
        ),
        SelectionCase::tool(
            "run_code-fibonacci",
            "Produce the first 15 terms of the Fibonacci sequence.",
            "run_code",
        ),
        // --- write_code --- the distinction: the user wants the code AS A FILE
        // (run_code only reads the result, it leaves no file).
        SelectionCase::tool(
            "write_code-script",
            "Write me a python script that finds prime numbers, and save it as a file.",
            "write_code",
        ),
        SelectionCase::tool(
            "write_code-converter",
            "Write a script that converts temperature data from Celsius to Fahrenheit and put it in my folder.",
            "write_code",
        ),
        // --- git --- the distinction from `find_file`: the question is about the
        // REPOSITORY (what did I change, what was committed), not about a file
        // whose name or content is being looked for.
        //
        // WHAT THIS MEASURES AND WHAT IT DOES NOT: the eval working directory
        // (`Env::setup`) is a temp folder and NOT a git repository, so the tool
        // answers "no_git_repository" here. That is enough for a SELECTION
        // measurement — the score is decided by which tool the model called — but
        // it is not enough to measure the ANSWER. A `git init` fixture in
        // `env.rs` would close that gap.
        SelectionCase::tool(
            "git-status",
            "Which files have I changed in this git repository?",
            "git",
        ),
        SelectionCase::tool(
            "git-commit-message",
            "Summarize my git changes and write me a commit message.",
            "git",
        ),
        // --- IRRELEVANCE --- no tool must be called
        SelectionCase::chat("chat-greeting", "Hello"),
        SelectionCase::chat("chat-thanks", "Thank you very much."),
        SelectionCase::chat("chat-who-are-you", "Who are you?"),
        SelectionCase::chat("chat-how-are-you", "How are you today?"),
        SelectionCase::chat("chat-privacy", "Are you sending my data to the cloud?"),
        SelectionCase::chat("chat-mood", "I'm a bit tired today, feeling low."),
        // --- MULTI-TURN --- the history is preserved, the document is the one
        // "in play"
        SelectionCase::chain(
            "chain-document",
            &[
                (
                    "Turn the weekly meal list into an excel file.",
                    "create_document",
                ),
                ("Show it as a table.", "read_document"),
                ("Change Tuesday from Rice to Beans.", "edit_document"),
            ],
        ),
        SelectionCase::chain(
            "chain-calc-document",
            &[
                ("What is 125 times 8?", "calculate"),
                ("Write this result into a markdown file.", "create_document"),
            ],
        ),
        SelectionCase::chain(
            "chain-read-edit",
            &[
                ("What does the file report.md say?", "read_document"),
                ("Add a Thursday row with Chickpeas.", "edit_document"),
            ],
        ),
    ]
}

// ---------------------------------------------------------------------------
// The outcome shape
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct StepOutcome {
    pub message: String,
    pub expected: Option<String>,
    /// The tools called in this step, in order.
    pub called: Vec<String>,
    pub passed: bool,
    pub answer: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectionOutcome {
    pub name: String,
    pub category: Category,
    pub passed: bool,
    pub steps: Vec<StepOutcome>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectionReport {
    pub engine: String,
    pub cases: Vec<SelectionOutcome>,
    /// The hit rate of the tool + multi-turn cases.
    pub tool_passed: usize,
    pub tool_total: usize,
    /// Irrelevance SEPARATELY: a drop here invalidates a rise in the hit rate.
    pub irrelevance_passed: usize,
    pub irrelevance_total: usize,
    /// The hit rate PER STEP in multi-turn cases — a per-case number hides the
    /// rest of a chain that broke on the first step.
    pub step_passed: usize,
    pub step_total: usize,
}

impl SelectionReport {
    fn new(engine: &str, cases: Vec<SelectionOutcome>) -> Self {
        let mut r = Self {
            engine: engine.into(),
            tool_passed: 0,
            tool_total: 0,
            irrelevance_passed: 0,
            irrelevance_total: 0,
            step_passed: 0,
            step_total: 0,
            cases,
        };
        for c in &r.cases {
            match c.category {
                Category::Irrelevance => {
                    r.irrelevance_total += 1;
                    r.irrelevance_passed += c.passed as usize;
                }
                _ => {
                    r.tool_total += 1;
                    r.tool_passed += c.passed as usize;
                }
            }
            for s in &c.steps {
                r.step_total += 1;
                r.step_passed += s.passed as usize;
            }
        }
        r
    }

    pub fn tool_rate(&self) -> f64 {
        ratio(self.tool_passed, self.tool_total)
    }

    pub fn irrelevance_rate(&self) -> f64 {
        ratio(self.irrelevance_passed, self.irrelevance_total)
    }

    pub fn table(&self) -> String {
        let width = self
            .cases
            .iter()
            .map(|c| c.name.chars().count())
            .max()
            .unwrap_or(4)
            .max(4);
        let mut s = String::new();
        s.push_str(&format!(
            "engine: {}  (tool selection set)\n\n",
            self.engine
        ));
        s.push_str(&format!(
            "{:<width$}  {:<6}  {}\n",
            "CASE", "STATE", "EXPECTED -> CALLED"
        ));
        s.push_str(&format!("{}\n", "-".repeat(width + 50)));
        for c in &self.cases {
            let state = if c.passed { "pass" } else { "FAIL" };
            let detail: Vec<String> = c
                .steps
                .iter()
                .map(|s| {
                    let e = s.expected.clone().unwrap_or_else(|| "-".into());
                    let called = if s.called.is_empty() {
                        "-".to_string()
                    } else {
                        s.called.join("+")
                    };
                    format!("{e}->{called}")
                })
                .collect();
            s.push_str(&format!(
                "{:<width$}  {state:<6}  {}\n",
                c.name,
                detail.join(" | ")
            ));
        }
        s.push_str(&format!(
            "\nTOOL HIT RATE   {}/{}  ({:.1}%)\n",
            self.tool_passed,
            self.tool_total,
            self.tool_rate() * 100.0
        ));
        s.push_str(&format!(
            "IRRELEVANCE     {}/{}  ({:.1}%)   <- MUST NOT drop\n",
            self.irrelevance_passed,
            self.irrelevance_total,
            self.irrelevance_rate() * 100.0
        ));
        s.push_str(&format!(
            "PER STEP        {}/{}  ({:.1}%)\n",
            self.step_passed,
            self.step_total,
            ratio(self.step_passed, self.step_total) * 100.0
        ));
        s
    }

    pub fn json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| {
            format!("{{\"error\":\"the report could not be serialized: {e}\"}}")
        })
    }
}

fn ratio(passed: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        passed as f64 / total as f64
    }
}

// ---------------------------------------------------------------------------
// The runner
// ---------------------------------------------------------------------------

/// Gives the production catalog in its dried-out form.
fn selection_catalog(env: &Env, memory: &SharedMemory) -> ToolCatalog {
    let (full, _, _) =
        tacet_tools::catalog::production_catalog(&env.store, memory, Some(FIXED_EPOCH));
    let mut c = ToolCatalog::new();
    for tool in full.tools() {
        if TO_DRY.contains(&tool.name()) {
            c.add(Arc::new(DryTool(Arc::clone(tool))));
        } else {
            c.add(Arc::clone(tool));
        }
    }
    announce_missing_tools(&c);
    c
}

/// Announces ONCE the tools that could not pass the discovery gate.
///
/// The warning DOES NOT CHANGE the runner's behaviour, it makes it visible: the
/// cases expecting these tools will lose on this machine and the score will
/// drop. Left silent, every report taken outside macOS would be read as "the hit
/// rate fell" and a regression that does not exist would be hunted. That the
/// loss comes FROM THE PLATFORM is written here, next to the report.
fn announce_missing_tools(catalog: &ToolCatalog) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let missing: Vec<&str> = DISCOVERY_BOUND
            .iter()
            .copied()
            .filter(|n| catalog.find(n).is_none())
            .collect();
        if !missing.is_empty() {
            eprintln!(
                "WARNING: {} is not in the catalog on this machine; the cases expecting these \
                 tools will count as FAILED. This is not a regression, it is a fact of the \
                 platform. Reason: {}",
                missing.join(", "),
                tacet_tools::run_code::RunCodeTool::diagnose()
            );
        }
    });
}

/// Runs all the cases. The engine must be REAL; it is not meaningful with the
/// fake engine.
pub fn run_selection(cases: &[SelectionCase], engine: &Arc<dyn EngineProvider>) -> SelectionReport {
    let outcomes = cases
        .iter()
        .map(|c| run_selection_case(c, engine))
        .collect();
    SelectionReport::new(engine.name(), outcomes)
}

/// Runs one case. In a multi-turn case the environment, the history and the
/// catalog are SHARED: for the message "show it as a table" to be resolvable,
/// the "document in play" must really have been created in the previous step.
pub fn run_selection_case(
    case: &SelectionCase,
    engine: &Arc<dyn EngineProvider>,
) -> SelectionOutcome {
    let env = match Env::setup() {
        Ok(e) => e,
        Err(e) => {
            return SelectionOutcome {
                name: case.name.clone(),
                category: case.category,
                passed: false,
                steps: vec![StepOutcome {
                    message: String::new(),
                    expected: None,
                    called: Vec::new(),
                    passed: false,
                    answer: format!("the environment could not be set up: {e}"),
                }],
            };
        }
    };
    let memory = SharedMemory::in_memory();
    let catalog = selection_catalog(&env, &memory);
    let executor = ToolExecutor::new(catalog.clone());
    let traces = Arc::new(TraceCollector::new());
    let mut ctx = ToolContext::new(
        Arc::clone(&env.store) as Arc<dyn tacet_kernel::DataStore>,
        env.dir(),
        Arc::clone(&traces) as Arc<dyn tacet_kernel::Reporter>,
    );
    // The constraint is built once OUTSIDE the loop: the trie cost is
    // proportional to the vocabulary size.
    let constraint = engine.vocab().map(|v| CallConstraint::new(&v, &catalog));
    let router = Router::new();

    let mut history: Vec<Turn> = Vec::new();
    let mut step_outcomes: Vec<StepOutcome> = Vec::new();

    for step in &case.steps {
        let ticket = executor.new_turn();
        traces.reset();
        let selected: ToolCatalog = router.select(&step.message, &catalog).into_iter().collect();

        let mut turn_tools: Vec<Turn> = Vec::new();
        let mut called: Vec<String> = Vec::new();
        let mut answer = String::new();

        for _ in 0..MAX_TURNS {
            // WHERE THE QUESTION GOES: at the END of the prompt on the first
            // turn, IN FRONT OF the tool call on later turns. The production
            // path (tacet-cli) does it this way; if eval did it differently what
            // is measured would not be the application's behaviour.
            let first = turn_tools.is_empty();
            let question = if first { step.message.as_str() } else { "" };
            let previous: Vec<Turn> = if first {
                history.clone()
            } else {
                history
                    .iter()
                    .cloned()
                    .chain(std::iter::once(Turn::user(&step.message)))
                    .chain(turn_tools.iter().cloned())
                    .collect()
            };
            let prompt = Prompt::new(SYSTEM_INSTRUCTIONS, question)
                .with_tools(&selected)
                .with_history(previous);

            let generation = match wait(
                engine.generate(
                    &prompt,
                    constraint
                        .as_ref()
                        .map(|c| c as &dyn tacet_engine::Constrainer),
                    SamplingSetting::default(),
                ),
            ) {
                Ok(g) => g,
                Err(e) => {
                    answer = format!("engine error: {e}");
                    break;
                }
            };
            if !generation.stop.is_complete() {
                answer = "generation was cut off halfway".into();
                break;
            }
            let Some(outcome) = wait(executor.execute_raw(&generation.text, ticket, &mut ctx))
            else {
                answer = generation.text.clone();
                break;
            };
            called.push(outcome.tool_name.clone());
            turn_tools.push(Turn::tool(outcome.to_model.clone()));
        }

        // THE HIT CLAIM — "was it called somewhere in the loop".
        //
        // WHY NOT THE FIRST CALL: `edit_document`'s own description says "call
        // read_document first". A claim looking at the first call would count a
        // model FOLLOWING THE INSTRUCTION as a failure. On the irrelevance side
        // the claim is absolute: no tool at all, nowhere in the loop.
        let passed = match &step.expected {
            Some(name) => called.iter().any(|c| c == name),
            None => called.is_empty(),
        };

        // The history is PERSISTENT: the next user message must see this turn.
        history.push(Turn::user(&step.message));
        history.extend(turn_tools);
        if !answer.is_empty() {
            history.push(Turn::assistant(&answer));
        }

        step_outcomes.push(StepOutcome {
            message: step.message.clone(),
            expected: step.expected.clone(),
            called,
            passed,
            answer,
        });
    }

    SelectionOutcome {
        name: case.name.clone(),
        category: case.category,
        passed: step_outcomes.iter().all(|s| s.passed),
        steps: step_outcomes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The set's COVERAGE of the catalog: if a new tool is added and no case is
    /// written for it, this test breaks. Otherwise the catalog grows, the
    /// measurement does not, and the new tool goes to production unmeasured.
    #[test]
    fn there_are_at_least_two_cases_for_every_tool() {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let catalog = selection_catalog(&env, &memory);
        let cases = selection_cases();
        for tool in catalog.tools() {
            let count = cases
                .iter()
                .flat_map(|c| c.steps.iter())
                .filter(|s| s.expected.as_deref() == Some(tool.name()))
                .count();
            assert!(
                count >= 2,
                "there are {} cases for {}, at least 2 are needed",
                count,
                tool.name()
            );
        }
    }

    #[test]
    fn there_are_at_least_five_irrelevance_cases() {
        let n = selection_cases()
            .iter()
            .filter(|c| c.category == Category::Irrelevance)
            .count();
        assert!(
            n >= 5,
            "the irrelevance case count is {n}, it must be at least 5"
        );
    }

    #[test]
    fn there_are_at_least_three_multi_turn_cases() {
        let multi: Vec<_> = selection_cases()
            .into_iter()
            .filter(|c| c.category == Category::MultiTurn)
            .collect();
        assert!(
            multi.len() >= 3,
            "the multi-turn case count is {}",
            multi.len()
        );
        assert!(
            multi.iter().all(|c| c.steps.len() >= 2),
            "a multi-turn case cannot have a single step"
        );
    }

    /// Cases saying the same intent differently: there are four phrasings for
    /// `time`.
    #[test]
    fn the_time_intent_is_measured_with_different_phrasings() {
        let messages: Vec<String> = selection_cases()
            .iter()
            .flat_map(|c| c.steps.clone())
            .filter(|s| s.expected.as_deref() == Some("time"))
            .map(|s| s.message)
            .collect();
        assert!(messages.len() >= 4, "{messages:?}");
        // "what time is it" and "what day of the month is it" ask for the SAME
        // tool without sharing the same words — the set must carry that
        // distinction.
        assert!(messages.iter().any(|m| m.contains("What time")));
        assert!(messages.iter().any(|m| m.contains("day of the month")));
    }

    #[test]
    fn the_case_names_are_unique() {
        let c = selection_cases();
        let set: HashSet<&str> = c.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(set.len(), c.len());
    }

    /// Does every expected tool REALLY exist in the catalog — a case with a typo
    /// returns "FAIL" forever and nobody understands why.
    ///
    /// PLATFORM: this test, like
    /// `the_expected_tool_does_not_drop_out_of_the_routers_budget`, DID NOT PASS
    /// OUTSIDE macOS — the audit had only flagged the other one, but the failure
    /// is THE SAME failure. Without fixing both, `cargo test --workspace` would
    /// still be red on Linux. The typo claim IS PRESERVED: the `Regression`
    /// branch still catches every misspelled name; the tools bound to the
    /// discovery gate may legitimately be absent.
    #[test]
    fn the_expected_tools_exist_in_the_catalog() {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let catalog = selection_catalog(&env, &memory);
        for c in selection_cases() {
            for s in c.steps {
                if let Some(name) = s.expected {
                    assert_ne!(
                        expectation_state(&name, &catalog),
                        Expectation::Regression,
                        "not in the catalog and its absence is not legitimate (typo?): {name}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_dried_tool_keeps_its_identity() {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let (full, _, _) =
            tacet_tools::catalog::production_catalog(&env.store, &memory, Some(FIXED_EPOCH));
        let dry = selection_catalog(&env, &memory);
        for name in TO_DRY {
            let (Some(f), Some(d)) = (full.find(name), dry.find(name)) else {
                continue;
            };
            // What determines the selection is the description: drying MUST NOT
            // change it.
            assert_eq!(
                f.description(),
                d.description(),
                "{name}'s description changed while drying"
            );
            assert_eq!(f.schema().json_schema(), d.schema().json_schema());
        }
    }

    /// An expectation's state against the catalog.
    ///
    /// IT MUST BE A SEPARATE FUNCTION: had the decision been buried directly in
    /// the test's loop, the `SkippedByPlatform` branch would NEVER RUN on macOS
    /// and the Linux fix would stay unmeasured — the very "the mechanism was
    /// built, it was never seen working" failure. As a pure function it can be
    /// measured on this machine with a synthetic catalog (see
    /// `a_missing_tool_is_told_apart_from_a_regression`).
    #[derive(Debug, PartialEq, Eq)]
    enum Expectation {
        /// The tool is in the catalog; the selection can really be measured.
        Measurable,
        /// The tool is not in the catalog but ITS ABSENCE IS LEGITIMATE (the
        /// discovery gate).
        SkippedByPlatform,
        /// The tool is not in the catalog and its absence is not legitimate — a
        /// real failure.
        Regression,
    }

    fn expectation_state(expected: &str, catalog: &ToolCatalog) -> Expectation {
        if catalog.find(expected).is_some() {
            Expectation::Measurable
        } else if DISCOVERY_BOUND.contains(&expected) {
            Expectation::SkippedByPlatform
        } else {
            Expectation::Regression
        }
    }

    /// THE SKIP RULE IS MEASURED ON THIS MACHINE — without waiting for Linux.
    ///
    /// A catalog with `run_code` dropped is SYNTHESIZED (what really happens on
    /// Linux/Windows) and all three branches run here: a tool bound to the
    /// discovery gate is skipped, a tool that is not bound counts as a
    /// REGRESSION, a tool in the catalog is measured. Otherwise whether the fix
    /// picks the right branch could only be learned on a Linux machine.
    #[test]
    fn a_missing_tool_is_told_apart_from_a_regression() {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let full = selection_catalog(&env, &memory);

        // Imitate a non-macOS machine by dropping the tools bound to the
        // discovery gate.
        let mut synthetic = ToolCatalog::new();
        for tool in full.tools() {
            if !DISCOVERY_BOUND.contains(&tool.name()) {
                synthetic.add(Arc::clone(tool));
            }
        }

        for name in DISCOVERY_BOUND {
            assert_eq!(
                expectation_state(name, &synthetic),
                Expectation::SkippedByPlatform,
                "{name}'s absence must count as a fact of the platform"
            );
        }
        assert_eq!(
            expectation_state("calculate", &synthetic),
            Expectation::Measurable
        );
        // The loss of a tool NOT BOUND to the discovery gate must not be
        // silenced.
        assert_eq!(
            expectation_state("create_document", &full),
            Expectation::Measurable
        );
        assert_eq!(
            expectation_state("no_such_tool", &full),
            Expectation::Regression
        );
    }

    /// THE ROUTER GATE — the layer BEFORE the model.
    ///
    /// There are 10 tools in the catalog and the budget is 8. If the expected
    /// tool falls outside those 8 the model NEVER SEES IT IN THE PROMPT; the
    /// case fails as "the model chose wrong", when the one choosing is not the
    /// model but the router. This test caught and fixed exactly that situation
    /// in four cases (run_code x2, create_document, web_fetch) and stops it from
    /// coming back.
    ///
    /// IT NEEDS NO MODEL: it takes seconds and runs in CI. Because a real
    /// model-based measurement takes minutes, protecting this layer separately
    /// and CHEAPLY is essential.
    ///
    /// PLATFORM: the test used to FAIL OUTSIDE macOS and that was a test bug,
    /// not a regression. `run_code`/`write_code` are never in the catalog on
    /// Linux (without bwrap) or on Windows; claiming that a tool which does not
    /// exist "dropped out of the budget" is the measurement measuring its own
    /// setup. The fix is not to SILENCE the test but to ASK THE RIGHT QUESTION:
    /// if the tool is not in the catalog, first ask whether its absence is
    /// LEGITIMATE (`DISCOVERY_BOUND`); if it is, the case is skipped and the
    /// skip IS PRINTED; if it is not, the test fails right there.
    #[test]
    fn the_expected_tool_does_not_drop_out_of_the_routers_budget() {
        let env = Env::setup().unwrap();
        let memory = SharedMemory::in_memory();
        let catalog = selection_catalog(&env, &memory);
        let router = Router::new();
        let (mut measured, mut skipped) = (0usize, 0usize);
        for c in selection_cases() {
            for s in &c.steps {
                let Some(expected) = s.expected.as_deref() else {
                    continue;
                };
                match expectation_state(expected, &catalog) {
                    Expectation::Measurable => {}
                    Expectation::SkippedByPlatform => {
                        eprintln!(
                            "{}: {expected} is not in the catalog on this platform, case skipped",
                            c.name
                        );
                        skipped += 1;
                        continue;
                    }
                    Expectation::Regression => panic!(
                        "{}: {expected} is NOT in the catalog and is not bound to the discovery \
                         gate — this is a real regression",
                        c.name
                    ),
                }
                let selection: Vec<String> = router
                    .select(&s.message, &catalog)
                    .iter()
                    .map(|x| x.name().to_string())
                    .collect();
                assert!(
                    selection.iter().any(|x| x == expected),
                    "{} / {:?}: {expected} dropped out of the budget, selection: {selection:?}",
                    c.name,
                    s.message
                );
                measured += 1;
            }
        }
        // SKIPPING MUST NOT BE AN ESCAPE HATCH: a test that skips everything
        // burns green and measures nothing. If the whole catalog has dropped,
        // that is a failure too, and it shows up here.
        assert!(
            measured > skipped,
            "most of the cases were skipped ({skipped} skipped / {measured} measured) — \
             the catalog setup may be broken"
        );
    }

    #[test]
    fn the_report_counts_the_two_rates_separately() {
        let outcomes = vec![
            SelectionOutcome {
                name: "a".into(),
                category: Category::Tool,
                passed: false,
                steps: vec![StepOutcome {
                    message: "m".into(),
                    expected: Some("calculate".into()),
                    called: vec![],
                    passed: false,
                    answer: String::new(),
                }],
            },
            SelectionOutcome {
                name: "b".into(),
                category: Category::Irrelevance,
                passed: true,
                steps: vec![StepOutcome {
                    message: "Hello".into(),
                    expected: None,
                    called: vec![],
                    passed: true,
                    answer: String::new(),
                }],
            },
        ];
        let r = SelectionReport::new("test", outcomes);
        assert_eq!((r.tool_passed, r.tool_total), (0, 1));
        assert_eq!((r.irrelevance_passed, r.irrelevance_total), (1, 1));
        // A single "success rate" would melt these two into 1/2; they must stay
        // separate.
        assert!((r.tool_rate() - 0.0).abs() < f64::EPSILON);
        assert!((r.irrelevance_rate() - 1.0).abs() < f64::EPSILON);
        assert!(r.table().contains("IRRELEVANCE"));
    }
}
