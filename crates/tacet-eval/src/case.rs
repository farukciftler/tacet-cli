//! Eval cases — Tacet's behavioural contract, WRITTEN DOWN.
//!
//! WHY THERE IS A SCRIPT FIELD: an eval case has to separate two questions —
//! "did the model pick the right tool" and "did Tacet EXECUTE that choice
//! correctly". The second is logic and has to come out THE SAME on every run,
//! independently of the model. The `script` field stands in for the model and
//! isolates the second question; when run with a real engine (see
//! `SingleEngine`) the script is ignored and the first question is measured. The
//! same case list feeds both measurements.
//!
//! WHERE THE EVIDENCE IS COLLECTED FROM: not only from the model's last
//! sentence — also from tool outputs, chip texts and `source_ref`s. The lesson
//! learned on the Swift side: the model writing "200" in its answer is not
//! evidence that the TOOL COMPUTED 200; the model may have invented the number
//! itself. That is why the evidence pool also contains what the tool said.
//!
//! ONE RULE ABOUT THE SCRIPT TEXT, learned the hard way while adding cases:
//! **keep every scripted line inside U+0000..U+0FFF.** `FakeEngine` takes a code
//! point to BE a token id and publishes only the first `FAKE_VOCAB` = 0x1000 of
//! them (tacet-engine/src/fake.rs), and the grammar constraint runs on the fake
//! engine too — deliberately, so eval measures genuinely masked generation. An
//! em dash (U+2014 = 8212) is therefore not in the vocabulary and the case dies
//! with "engine error: constraint rejected the token: 8212", which reads like an
//! engine bug and is a fixture limit. Three cases in this file were written with
//! em dashes and failed exactly that way. Plain ASCII punctuation in scripts.

use serde::Serialize;

/// WHAT A CASE HOLDS RESPONSIBLE when it fails.
///
/// WHY THE SUITE NEEDED THIS: with a real engine the set printed ONE number —
/// 69.2% — over two questions that move for different reasons and are fixed in
/// different files. `document-schema-violation` failing is a defect in this
/// repository; `read-document-absent` failing is the model declining to call.
/// Averaged together, a drop tells you nothing about where to look, and the
/// selection set had already learned this lesson and written it down: separate
/// numbers make the trade-off visible.
///
/// THE ONE CLAIM THIS BUYS, and it is the reason to bother: **with a real engine
/// the LOGIC line should still read 100%.** Anything less is a bug in Tacet that
/// the fake engine cannot see, and until now it was hidden inside an average.
///
/// THE HONEST LIMIT: under a real engine a `Logic` case can still fail for a
/// model reason — the model picks another tool and the evidence never appears.
/// The split narrows where to look, it does not decide. The gate for `Logic`
/// stays the `FakeEngine` run, where the choice is pinned by the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Measures {
    /// Tacet's own logic: gates, channel refs, retry flags, what gets written.
    Logic,
    /// The model's behaviour: whether it calls at all, and what it then says.
    Behaviour,
}

/// A single evaluation case.
#[derive(Debug, Clone, Serialize)]
pub struct EvalCase {
    pub name: String,
    /// The user's message.
    pub input: String,
    /// The tool expected to be called. `None` = NO tool must be called
    /// (greeting, small talk). Leaving it empty does not mean "I don't care",
    /// it means "I want no tool" — tool appetite is the most frequent
    /// regression.
    ///
    /// WHEN NEITHER IS RIGHT, see `tool_claim_waived`. Two states could not
    /// express the case that found the gap: for "what is sin(45)?" both calling
    /// the tool (and being told it cannot) and declining outright are correct
    /// behaviour, and the claim worth making is about the SENTENCE.
    pub expected_tool: Option<String>,
    /// This case makes NO claim about which tool ran.
    ///
    /// THE THIRD STATE, and it was added by a case that failed for the wrong
    /// reason. `calc-unsupported` asks "what is sin(45)?" — the tool's own
    /// description names a closed set that does not include `sin`, so calling it
    /// and getting a refusal is correct, and answering "I cannot compute that"
    /// is equally correct. What must not happen is a NUMBER. Written with
    /// `expected_tool: None` the case additionally asserted "no tool at all" and
    /// scored the legitimate call as tool appetite — a claim nobody meant to
    /// make, produced by a field that had no way to stay silent.
    ///
    /// IT IS NOT A DEFAULT AND MUST NOT BECOME ONE. Every case that CAN name its
    /// tool still must: waiving the claim to make a red case green is how a suite
    /// stops measuring. It is for cases whose subject is genuinely something else.
    #[serde(skip)]
    pub tool_claim_waived: bool,
    /// The fragments that MUST BE PRESENT in the evidence pool (all of them).
    pub expected_evidence: Vec<String>,
    /// The fragments that MUST NOT BE PRESENT in the evidence pool (none of
    /// them) — detecting invention and silent fallbacks.
    pub forbidden: Vec<String>,
    /// EVERY NUMBER IN THE ANSWER MUST HAVE COME FROM A TOOL.
    ///
    /// WHY IT EXISTS: `expected_evidence` proves a tool produced a value; it
    /// does NOT prove the answer stayed inside what the tools produced. The
    /// failure that motivated it, from a real session: `web_search` was selected
    /// correctly, 38 results came back, and the answer said "the water
    /// temperature will be 230°C". Tool selection was perfect, the sentence was
    /// nonsense, and every measurement in this suite scored it as a pass —
    /// because the suite only ever asked WHICH TOOL, never WHAT WAS SAID.
    ///
    /// THE CLAIM IS DELIBERATELY WEAK: each digit run of two or more characters
    /// in the answer must appear SOMEWHERE in the tool-sourced half of the pool.
    /// It does not check units, arithmetic or meaning, and a number that happens
    /// to be a substring of a longer one passes. That is the right direction to
    /// err: a false alarm here would make a correct answer look like a
    /// regression, while a missed invention only costs what the suite already
    /// did not catch.
    ///
    /// IT IS A NO-OP UNDER `FakeEngine` — the script's answer is written by hand
    /// and is grounded by construction. The claim only bites with a REAL engine,
    /// which is exactly where the 230°C came from.
    pub grounded: bool,
    /// Which of the two questions this case answers — see `Measures`.
    pub measures: Measures,
    /// How many tool calls this turn may take in total. `None` = no claim.
    ///
    /// WHY LATENCY NEEDED A CLAIM OF ITS OWN: `gate-tainted-session` satisfied
    /// every claim in this file with `["read_document", "read_document",
    /// "read_document"]`. The executor's duplicate guard means the second and
    /// third never RAN — they came back as `duplicate_call` — but the user still
    /// waited through three generations to reach one answer. Every other claim
    /// here is about what the system PRODUCED, so a turn that produced the right
    /// thing three times over read exactly like one that did it once.
    ///
    /// THE TOTAL, NOT THE EXPECTED TOOL'S SHARE. The repetition that cost the
    /// most was on a tool the case does not even name: `gate-tainted-session`
    /// expects `send_out` and burned its turn on `read_document`. A cap that
    /// only counted the expected tool would have watched that happen.
    ///
    /// SO THE NUMBER IS "how many calls does this turn NEED": one for a single
    /// tool, two for a read-then-write chain. It is a budget, not a preference.
    pub max_calls: Option<usize>,
    /// The FakeEngine script: the turns produce these outputs in order.
    #[serde(skip)]
    pub script: Vec<String>,
    /// This case runs with the grammar constraint SWITCHED OFF.
    ///
    /// WHY SUCH A FLAG WAS NEEDED: `ToolExecutor` builds the defence in two
    /// layers — the grammar makes an invalid call UNPRODUCIBLE, and the schema
    /// gate (GATE 2) validates it anyway. The executor's own comment says so
    /// explicitly: "the grammar already forces it, but the grammar can be
    /// disabled; the gate being two-layered is deliberate". A case measuring
    /// the lower layer gets stuck AT GENERATION TIME when the upper layer is on,
    /// and never reaches the gate it wants to measure. The flag makes the case
    /// itself answer "which layer am I measuring"; instead of switching the
    /// constraint off silently it makes it a written decision.
    #[serde(skip)]
    pub unconstrained: bool,
    /// This case runs with the approval gate set to APPROVE.
    ///
    /// WHY IT HAD TO EXIST: the runner builds its executor with the default
    /// `SilentDeny`, so until this flag every gate case in the file measured the
    /// SAME arm — the closed one. A gate hard-wired to refuse everything, or one
    /// whose open path had been deleted, passed the entire suite. `gate-clean-
    /// session` does not cover it either: a clean session never reaches the gate
    /// at all, it is not asked. The only way to measure "the gate can OPEN" is a
    /// case that taints the session AND supplies a gate that says yes.
    ///
    /// NOT A DEFAULT, and the direction matters: the deny arm is the safe one,
    /// so a case that forgets to say `.approved()` gets the strict behaviour.
    #[serde(skip)]
    pub gate_opens: bool,
    /// Cancel the turn just BEFORE the pass with this index (0 = before the
    /// first generation).
    ///
    /// WHY A NUMBER AND NOT A FLAG: the guarantee has two halves and they need
    /// different indices. `Some(0)` measures "a cancelled turn writes NOTHING";
    /// `Some(1)` measures "the work already done stays done and the write that
    /// had not happened yet does not happen" — the read's `source_ref` is still
    /// in the pool while `file_created` must not be.
    ///
    /// GATE 4 (`executor.rs`, `ExecutionReason::Cancelled`) was the only gate in
    /// the executor with no case anywhere in this file, and the reason was
    /// mechanical rather than deliberate: `run_case` takes `active_turn()` and
    /// never calls `cancel()`, so the arm was UNREACHABLE from eval.
    #[serde(skip)]
    pub cancel_before: Option<usize>,
    /// Fragments that must never appear IN THE PROMPT, on any pass.
    ///
    /// THIS IS THE ARCHITECTURE'S HEADLINE CLAIM AND IT WAS NOT MEASURED. "Bulk
    /// data never reaches the model" is what the bypass channel exists for, and
    /// `forbidden` CANNOT express it: `forbidden` is checked against the evidence
    /// pool, and that pool contains `outcome.raw_output` (see `runner`), into
    /// which `read_document` deliberately puts the WHOLE file — precisely because
    /// that half does not go to the model. So the two need different pools, and
    /// this is the second one: everything `Prompt::text()` produced, pass by
    /// pass.
    ///
    /// WHAT IT CATCHES THAT NOTHING ELSE DOES, measured: raising
    /// `read_document`'s `MODEL_CAP` from 1500 to `usize::MAX` leaves every other
    /// case in this suite green. `channel-preview-cap` goes red.
    ///
    /// THE HONEST LIMIT: the pool is the PLAIN prompt text. A real engine may
    /// wrap the same pieces in a chat template (`Prompt::text_with_template`);
    /// the fences differ, the content does not, so a fragment absent here is
    /// absent there too.
    #[serde(skip)]
    pub never_shown: Vec<String>,
}

impl EvalCase {
    pub fn new(name: &str, input: &str) -> Self {
        Self {
            name: name.into(),
            input: input.into(),
            expected_tool: None,
            tool_claim_waived: false,
            expected_evidence: Vec::new(),
            forbidden: Vec::new(),
            grounded: false,
            // LOGIC IS THE DEFAULT, and that is the strict direction: a case
            // nobody classified is held to the line that must read 100%.
            measures: Measures::Logic,
            max_calls: None,
            script: Vec::new(),
            unconstrained: false,
            gate_opens: false,
            cancel_before: None,
            never_shown: Vec::new(),
        }
    }

    /// Every number in the answer must have come from a tool — see `grounded`.
    pub fn grounded(mut self) -> Self {
        self.grounded = true;
        self
    }

    /// This case makes NO claim about which tool ran — see `tool_claim_waived`.
    pub fn any_tool(mut self) -> Self {
        self.tool_claim_waived = true;
        self
    }

    /// This case holds the MODEL responsible, not Tacet — see `Measures`.
    pub fn behaviour(mut self) -> Self {
        self.measures = Measures::Behaviour;
        self
    }

    /// This turn needs exactly one tool call. See `max_calls`.
    pub fn once(mut self) -> Self {
        self.max_calls = Some(1);
        self
    }

    /// This turn needs at most `n` tool calls. See `max_calls`.
    pub fn calls_at_most(mut self, n: usize) -> Self {
        self.max_calls = Some(n);
        self
    }

    /// Switches the constraint off — only for cases measuring EXECUTOR gates.
    pub fn unconstrained(mut self) -> Self {
        self.unconstrained = true;
        self
    }

    pub fn tool(mut self, name: &str) -> Self {
        self.expected_tool = Some(name.into());
        self
    }

    pub fn evidence(mut self, parts: &[&str]) -> Self {
        self.expected_evidence = parts.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn forbidden(mut self, parts: &[&str]) -> Self {
        self.forbidden = parts.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn script(mut self, steps: &[&str]) -> Self {
        self.script = steps.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Runs with a gate that APPROVES — see `gate_opens`.
    pub fn approved(mut self) -> Self {
        self.gate_opens = true;
        self
    }

    /// Cancels the turn before pass `n` — see `cancel_before`.
    pub fn cancelled_before(mut self, n: usize) -> Self {
        self.cancel_before = Some(n);
        self
    }

    /// These fragments must never reach the PROMPT — see `never_shown`.
    pub fn never_shown(mut self, parts: &[&str]) -> Self {
        self.never_shown = parts.iter().map(|s| s.to_string()).collect();
        self
    }
}

/// The small table the runner writes into the working directory. It has to come
/// back to the model as piped markdown — the heart of the chain (see
/// `read_document`).
pub const TABLE_FILE: &str = "report.md";
/// A file that exceeds `TEXT_STORE_THRESHOLD` (1500 bytes): it triggers the
/// bypass channel.
pub const LONG_FILE: &str = "long.md";
/// The target of the `find_file` cases — there is something to search for both
/// in its name and inside it.
pub const BUDGET_FILE: &str = "budget-2026.md";
/// A file that exists and has NO TEXT IN IT.
///
/// WHY A FIXTURE FOR NOTHING: an empty file is not a malfunction, and
/// `read_document` says so with its own word (`document_empty`) rather than the
/// generic `tool_failed`. The difference matters to the model — told the tool
/// failed it retries or reports a broken tool; told the file is empty it can say
/// so. Nothing pinned that distinction before `document-empty-file`.
pub const EMPTY_FILE: &str = "empty.md";
/// A real .zip, packed by production's own writer — see `env`.
///
/// It holds MORE than `archive`'s `LIST_ROWS_TO_MODEL` (20) entries on purpose:
/// under the threshold the listing goes to the model whole and the bypass
/// channel is never used, so a fixture with five entries could not measure the
/// claim the tool exists to make.
pub const ARCHIVE_FILE: &str = "backup.zip";
/// The number of entries in `ARCHIVE_FILE`.
pub const ARCHIVE_ENTRIES: usize = 24;
/// A fixed "now" — 2026-07-20T00:00:00Z, a Monday. An eval tied to the real
/// clock cannot be deterministic.
pub const FIXED_EPOCH: i64 = 1_784_505_600;

/// All the cases.
///
/// The order is deliberate: first tool APPETITE (small talk), then correct
/// selection, then channel and gate behaviour. Even if the run is cut short,
/// the most informative part has been measured.
pub fn all() -> Vec<EvalCase> {
    let mut v = Vec::new();
    v.extend(chat());
    v.extend(calc());
    v.extend(time());
    v.extend(document());
    v.extend(channel());
    v.extend(gate());
    v.extend(grounding());
    v.extend(edit());
    v.extend(search());
    v.extend(recall());
    v.extend(loop_guard());
    v.extend(cancellation());
    v.extend(archive());
    v.extend(chain());
    v
}

/// The situations where no tool must be called. A small model calls search even
/// for a greeting; this category measures that appetite.
fn chat() -> Vec<EvalCase> {
    vec![
        EvalCase::new("chat-greeting", "Hello")
            .script(&["Hello! How can I help?"])
            .evidence(&["Hello"])
            .behaviour(),
        EvalCase::new("chat-thanks", "Thank you very much")
            .script(&["You're welcome."])
            .evidence(&["welcome"])
            .behaviour(),
        // On-device identity: the model must not think it is a cloud assistant.
        EvalCase::new("chat-on-device", "Are you sending my data to the cloud?")
            .script(&["No, everything stays on your device."])
            .evidence(&["on your device"])
            .forbidden(&["to our servers"])
            .behaviour(),
        // --- Breadth carried over from the tacet-cli tree (v0.1.22). These are
        // the irrelevance cases spec-eval-cases.md §4.1 asked for; they arrived
        // there while this branch was adding claim machinery, and both halves
        // are kept. They do NOT yet carry `.once()`/`.grounded()` — that is
        // follow-up, not a silent omission.
        EvalCase::new("chat-identity", "Who created you?")
            .script(&["I am Tacet, an on-device AI assistant."])
            .evidence(&["Tacet"])
            .forbidden(&["ChatGPT", "OpenAI", "Anthropic"])
            .behaviour(),
        EvalCase::new("chat-capabilities", "What can you do?")
            .script(&[
                "I can help with document editing, file search, calculations, and local notes.",
            ])
            .evidence(&["document", "calculations"])
            .behaviour(),
        EvalCase::new("chat-farewell", "Goodbye, see you later!")
            .script(&["Goodbye! Have a great day."])
            .evidence(&["Goodbye"])
            .behaviour(),
        EvalCase::new(
            "chat-opinion",
            "Which is better, morning or evening workout?",
        )
        .script(&["Both have benefits depending on your schedule and energy levels."])
        .evidence(&["benefits"])
        .behaviour(),
        EvalCase::new("chat-continuation", "Tell me more about that")
            .script(&["Sure, here are additional details."])
            .evidence(&["details"])
            .behaviour(),
        EvalCase::new("chat-shorter", "Can you explain it more briefly?")
            .script(&["In short: focus on consistency."])
            .evidence(&["In short"])
            .behaviour(),
        EvalCase::new("chat-general-knowledge", "What is the capital of France?")
            .script(&["The capital of France is Paris."])
            .evidence(&["Paris"])
            .forbidden(&["web_search"])
            .behaviour(),
        // --- THE HARDEST APPETITE CASES: every one of these messages CONTAINS A
        // SKILL TRIGGER and still wants no tool at all.
        //
        // WHY THIS SHAPE AND NOT MORE SMALL TALK: the appetite cases above are
        // easy — "Hello" and "Goodbye" carry nothing that looks like work. The
        // regression that actually happens is the opposite one: a message that
        // reads like a job, is not one, and pulls a tool in anyway. Each message
        // below was checked against `SkillStore::default_set()` on this machine
        // and matches at least one package skill's triggers, so under a real
        // engine they arrive with a guide attached telling the model to call.
        // That is the pressure the easy cases do not apply.
        //
        // A METHOD IS NOT A CALCULATION. Fires calc ("calculate", "percent");
        // there is no number in the message to compute.
        EvalCase::new(
            "chat-explain-not-compute",
            "How do I calculate a percentage discount?",
        )
        .script(&["Multiply the price by the discount rate, then subtract that from the price."])
        .evidence(&["subtract"])
        .behaviour(),
        // "time complexity" is not a time. The forbidden list is the vocabulary
        // of the `time` tool's own output, so a call would be visible here even
        // if the sentence read plausibly.
        EvalCase::new(
            "chat-time-complexity",
            "What is the time complexity of quicksort?",
        )
        .script(&["On average it is O(n log n), and O(n squared) in the worst case."])
        .evidence(&["log n"])
        .forbidden(&["date=", "weekday=", "time="])
        .behaviour(),
        // "read" as an IDIOM. `read_document({"path":"mind"})` is the exact
        // failure this case exists for — a path invented out of a figure of
        // speech.
        EvalCase::new("chat-read-idiom", "Can you read my mind?")
            .script(&["No, I can only read files you point me at."])
            .evidence(&["read files"])
            .forbidden(&["document_empty", "tool_failed"])
            .behaviour(),
        // "Remind me" fires the calendar guide (9 characters) ahead of
        // create-document's "markdown" (8) — measured on this machine with
        // `SkillStore::default_set().matching(...)`. The message wants NEITHER
        // tool: it is a question about syntax.
        EvalCase::new(
            "chat-remember-idiom",
            "Remind me how a markdown table is written",
        )
        .script(&[
            "Rows are lines and cells are separated by pipes, with a --- row under the header.",
        ])
        .evidence(&["pipes"])
        .forbidden(&["note saved", "file_created"])
        .behaviour(),
        // A CAPABILITY THE CATALOG DOES NOT HAVE. The honest answer is a
        // refusal; the failure is repurposing `send_out` (which does exist, and
        // does send things out) as a mail client.
        EvalCase::new("chat-no-such-capability", "Send an email to my accountant")
            .script(&["I cannot send email; there is no mail tool here."])
            .evidence(&["cannot send email"])
            .forbidden(&["sent_ok"])
            .behaviour(),
    ]
}

fn calc() -> Vec<EvalCase> {
    vec![
        EvalCase::new("calc-multiply", "What is 125 times 8?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"125*8"})"#, "125 x 8 = 1000."])
            // 1000 must be said by the TOOL; the number in the model's sentence
            // is not evidence.
            .evidence(&["1000"])
            .grounded()
            .once(),
        EvalCase::new(
            "calc-percent",
            "How much is 250 lira with a 20 percent discount?",
        )
        .tool("calculate")
        .script(&[r#"calculate({"expression":"250-250*20%"})"#, "200 lira."])
        .evidence(&["200"])
        .grounded()
        .once(),
        // AN EXPRESSION THE TOOL DOES NOT SUPPORT — and this case asserts the
        // HARM, not the mechanism.
        //
        // IT USED TO BE `calc-invalid`, and it demanded that `calculate` be
        // called and come back failed. Measured against a real model (qwen3-4b,
        // 30 Jul 2026) it failed with "the expected tool was not called" — and
        // the model was arguably right: the tool's own description names a CLOSED
        // set ("the four operations, parentheses, percent (%) and power (^)"),
        // `sin` is not in it, and declining a call that cannot work is not a
        // defect. The suite was holding the model to a mechanism while the thing
        // worth protecting sat in the last line of the same description:
        //
        //     never write a result you did not get back from this tool
        //
        // So: calling and failing is fine, and saying "I cannot compute that" is
        // fine. Producing a NUMBER is not, and `grounded` is what says so.
        EvalCase::new("calc-unsupported", "What is sin(45)?")
            .script(&["I cannot compute trigonometric functions."])
            .forbidden(&["0.707", "0.851", "0.85090"])
            .grounded()
            .any_tool()
            .behaviour(),
        EvalCase::new("calc-add", "Could you add 347 and 268?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"347+268"})"#, "347 + 268 = 615."])
            .evidence(&["615"]),
        EvalCase::new("calc-divide", "What is 144 divided by 12?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"144/12"})"#, "144 / 12 = 12."])
            .evidence(&["12"]),
        EvalCase::new("calc-float", "What is 15.5 times 4.2?")
            .tool("calculate")
            .script(&[
                r#"calculate({"expression":"15.5*4.2"})"#,
                "15.5 x 4.2 = 65.1.",
            ])
            .evidence(&["65.1"]),
        EvalCase::new("calc-complex", "Calculate (45 + 55) * 12 / 4")
            .tool("calculate")
            .script(&[
                r#"calculate({"expression":"(45+55)*12/4"})"#,
                "Result is 300.",
            ])
            .evidence(&["300"]),
        EvalCase::new("calc-syntax-error", "What is 12 ++ * 5?")
            .tool("calculate")
            .script(&[
                r#"calculate({"expression":"12++*5"})"#,
                "Syntax error in expression.",
            ])
            .evidence(&["tool_failed"]),
        EvalCase::new("calc-zero-division", "What is 100 divided by 0?")
            .tool("calculate")
            .script(&[
                r#"calculate({"expression":"100/0"})"#,
                "Division by zero is undefined.",
            ])
            .evidence(&["tool_failed"]),
        EvalCase::new("calc-large-number", "What is 999999 times 999999?")
            .tool("calculate")
            .script(&[
                r#"calculate({"expression":"999999*999999"})"#,
                "999998000001.",
            ])
            .evidence(&["999998000001"]),
    ]
}

fn time() -> Vec<EvalCase> {
    vec![
        EvalCase::new("time-date", "What is today's date?")
            .tool("time")
            .script(&[r#"time({"kind":"date"})"#, "Today is 20 July 2026."])
            .evidence(&["date=2026-07-20"])
            .grounded()
            .once(),
        EvalCase::new("time-weekday", "What day of the week is it today?")
            .tool("time")
            .script(&[r#"time({"kind":"weekday"})"#, "Monday."])
            .evidence(&["weekday=Monday"])
            .once(),
        // Calendar arithmetic is done IN THE TOOL; the model must not count for
        // itself.
        EvalCase::new("time-diff", "How many days until 2 December 2026?")
            .tool("time")
            .script(&[
                r#"time({"kind":"diff","target":"2026-12-02"})"#,
                "135 days.",
            ])
            .evidence(&["days=135", "to=2026-12-02"])
            .grounded()
            .once(),
        // WHEN THE TIME CANNOT BE RESOLVED IT MUST FAIL. Falling back to today
        // silently shows the model "0 days" and the model takes that for the
        // answer.
        EvalCase::new("time-unresolvable", "How many days until whatsit day?")
            .tool("time")
            .script(&[
                r#"time({"kind":"diff","target":"whatsit day"})"#,
                "I could not understand the date, could you clarify?",
            ])
            .evidence(&["unparsable_date"])
            .forbidden(&["days=0"])
            .once(),
        EvalCase::new("time-clock", "What time is it right now?")
            .tool("time")
            .script(&[r#"time({"kind":"clock"})"#, "It is 14:30."])
            .evidence(&["time=00:00"]),
        EvalCase::new("time-year-end", "How many days until the end of the year?")
            .tool("time")
            .script(&[
                r#"time({"kind":"diff","target":"2026-12-31"})"#,
                "164 days left.",
            ])
            .evidence(&["to=2026-12-31"]),
        EvalCase::new("time-past-date", "How many days since 1 January 2026?")
            .tool("time")
            .script(&[
                r#"time({"kind":"diff","target":"2026-01-01"})"#,
                "200 days ago.",
            ])
            .evidence(&["2026-01-01"]),
        EvalCase::new("time-timezone", "What is the UTC time?")
            .tool("time")
            .script(&[
                r#"time({"kind":"clock"})"#,
                "The current UTC time is 12:00.",
            ])
            .evidence(&["time=00:00"]),
    ]
}

fn document() -> Vec<EvalCase> {
    vec![
        // THE TABLE MARKDOWN CHAIN: the text going to the model must be PIPED;
        // had a pipe-less summary come back, the model could not rebuild the
        // table and would say "the table was shown" while skipping the content.
        EvalCase::new("read-document-table", "What is in the file report.md?")
            .tool("read_document")
            .script(&[
                r#"read_document({"path":"report.md"})"#,
                "The file has a weekly meal table.",
            ])
            .evidence(&["| Day | Meal |", "| --- |", "| Monday | Lentils |"])
            .once(),
        EvalCase::new("create-document-excel", "Make an excel file for the weekly meal list")
            .tool("create_document")
            .script(&[
                r#"create_document({"format":"excel","file_name":"meals","content":"| Day | Meal |\n| --- | --- |\n| Monday | Lentils |"})"#,
                "I created the excel file.",
            ])
            // THE FORMAT AND THE EXTENSION, NOT THE STEM. The case used to demand
            // `meals.xlsx` and a real model produced `weekly_meal_list.xlsx` —
            // a correct file for a request that never named one. That single
            // over-specification was the ONLY failure the blame rule attributed
            // to Tacet in a 33-case run, and it was not a defect in Tacet: the
            // user's message says "make an excel file for the weekly meal list"
            // and says nothing about what to call it. A claim must not require
            // what the request never asked for.
            .evidence(&["file_created (excel)", ".xlsx"])
            .once(),
        // THE REQUEST NAMES THE FORMAT, because the claim does. It used to say
        // "Create a short note file" while demanding `file_created (markdown)`,
        // and a real model produced a plain text file — a correct answer to a
        // request that never said markdown. The excel case beside it has always
        // named its format ("Make an excel file …"); this one did not, and the
        // difference was scored as a defect.
        //
        // IT ALSO EXPOSED A LIMIT OF THE BLAME RULE, worth knowing about: the
        // expected tool WAS called, the turn answered and nothing repeated, so
        // the failure was attributed to Tacet — the rule cannot see that the
        // wrong ARGUMENT was chosen. Argument choice is the model's, and the
        // attribution has no way to say so.
        EvalCase::new("create-document-markdown", "Create a short markdown note file")
            .tool("create_document")
            .script(&[
                r#"create_document({"format":"markdown","file_name":"note","content":"Hello"})"#,
                "The note file is ready.",
            ])
            .evidence(&["file_created (markdown)", ".md"])
            .once(),
        // A FILE THAT DOES NOT EXIST — the harm, again, not the mechanism.
        //
        // The predecessor (`read-document-missing`) required the call and the
        // failure. A model that answers "I do not see that file" has behaved
        // correctly and was scored as a regression. What must never happen is a
        // SUMMARY of a file nobody read: that is invention with a citation. The
        // forbidden list is therefore the vocabulary of a fabricated summary —
        // the fixture words that exist elsewhere in this directory and could only
        // reach the answer by being made up for a file that is not there.
        EvalCase::new("read-document-absent", "Summarize the file missing.md")
            .script(&["I could not find that file."])
            .forbidden(&["Lentils", "Rent:", "18000"])
            .grounded()
            .any_tool()
            .behaviour(),
        // A schema violation: the tool must NOT RUN AT ALL.
        EvalCase::new("document-schema-violation", "Create a file")
            .tool("create_document")
            .script(&[
                r#"create_document({"format":"excel"})"#,
                "Could you tell me the file name?",
            ])
            .evidence(&["tool_failed"])
            .forbidden(&["file_created"])
            // UNCONSTRAINED: this case measures not the grammar but THE SCHEMA
            // GATE (GATE 2). With the constraint on, the model cannot skip the
            // required `file_name` field and close the object — `}` is masked,
            // generation stops right at the start and the gate we want to
            // measure is never reached. That the grammar blocks the same
            // violation AT GENERATION TIME is proven separately by a unit test
            // (tacet-grammar/src/call.rs).
            .unconstrained(),
        // AN EMPTY FILE IS NOT A MALFUNCTION, and `read_document` has a separate
        // word for it (`document_empty`, read_document.rs). The forbidden entry
        // is the whole point: told `tool_failed` the model says "the tool is
        // broken" or retries; told the file is empty it can answer. Nothing
        // pinned the distinction, so collapsing the empty branch into the error
        // branch would have been invisible.
        EvalCase::new("document-empty-file", "What is in the file empty.md?")
            .tool("read_document")
            .script(&[
                r#"read_document({"path":"empty.md"})"#,
                "That file has no content.",
            ])
            .evidence(&["document_empty"])
            .forbidden(&["tool_failed"])
            .once(),
    ]
}

/// The 4096 token bypass channel — the heart of the architecture.
fn channel() -> Vec<EvalCase> {
    vec![
        // A large document does NOT go to the model in full: a short preview +
        // a source_ref come back.
        EvalCase::new("channel-source-ref", "What is in the file long.md?")
            .tool("read_document")
            .script(&[r#"read_document({"path":"long.md"})"#, "There is a long list."])
            .evidence(&["source_ref=document#1"])
            .once(),
        // THE CHAIN: device data lands in a file WITHOUT PASSING THROUGH the
        // model. The model carries only the reference; the bulk data never
        // appears in the prompt on any turn.
        EvalCase::new("channel-chain", "Dump the contents of long.md into a markdown file")
            .tool("create_document")
            .script(&[
                r#"read_document({"path":"long.md"})"#,
                r#"create_document({"format":"markdown","file_name":"dump","source_ref":"document#1"})"#,
                "I created the file.",
            ])
            .evidence(&["source_ref=document#1", "file_created (markdown)", "dump.md"])
            .calls_at_most(2),
        // An unresolvable reference: the file must NOT be written at all.
        EvalCase::new("channel-unknown-ref", "Dump the data in the store into a file")
            .tool("create_document")
            .script(&[
                r#"create_document({"format":"markdown","file_name":"ghost","source_ref":"document#99"})"#,
                "I could not find the source data.",
            ])
            .evidence(&["unknown_data_ref"])
            .forbidden(&["file_created"]),
        // THE CLAIM THE WHOLE ARCHITECTURE RESTS ON, AND IT WAS NEVER MEASURED.
        //
        // `channel-chain` above proves a `source_ref` was minted and a file was
        // written. It does NOT prove the bulk data stayed out of the model's
        // window — `forbidden` cannot say that, because the pool it reads
        // contains `outcome.raw_output` and `read_document` puts the whole file
        // there deliberately (see `EvalCase::never_shown`). This case reads the
        // SECOND pool: every prompt, on every pass.
        //
        // WHY THESE TWO FRAGMENTS. Measured here today: long.md is 6704 bytes,
        // `read_document`'s MODEL_CAP is 1500, and the line-boundary cut lands at
        // byte 1487 — so the model sees lines 1 to 45 and nothing after. Lines
        // 120 and 199 are on the far side of that cut by a wide margin, so this
        // case does not go red for a one-character change to the fixture.
        EvalCase::new(
            "channel-bulk-never-in-prompt",
            "Dump the contents of long.md into a markdown file",
        )
        .tool("create_document")
        .script(&[
            r#"read_document({"path":"long.md"})"#,
            r#"create_document({"format":"markdown","file_name":"dump","source_ref":"document#1"})"#,
            "I created the file.",
        ])
        .evidence(&["source_ref=document#1", "file_created (markdown)"])
        .never_shown(&["line 120:", "line 199:"])
        .calls_at_most(2),
        // THE PREVIEW CAP, ON ITS OWN. One tool, one claim: what came back was a
        // preview plus a reference, not the file.
        //
        // THE INVARIANT WAS BROKEN ON PURPOSE TO SEE THIS FAIL. Measured on this
        // machine (4 Sep 2026): with `read_document`'s MODEL_CAP set to
        // `usize::MAX` the suite went 75/78 — and the only three failures were
        // this case, `channel-bulk-never-in-prompt` and `channel-edit-by-ref`,
        // each reporting `bulk data reached the model: "line 199:" was in the
        // prompt`, all three blamed on Tacet. Every one of the 51 cases that
        // existed before these three stayed GREEN, including `channel-source-ref`
        // and `grounding-long-list`, which read the same file. That is the gap
        // measured rather than argued.
        EvalCase::new("channel-preview-cap", "What is in the file long.md?")
            .tool("read_document")
            .script(&[
                r#"read_document({"path":"long.md"})"#,
                "It is a long list of filler lines.",
            ])
            .evidence(&["source_ref=document#1"])
            .never_shown(&["line 199:"])
            .once(),
        // THE OTHER HALF OF THE CHANNEL. `create_document` had a source_ref case;
        // `edit_document` did not, and its source_ref branch is a separate code
        // path with a separate failure mode (an unresolvable ref that fell back
        // to an empty body EMPTIED the user's document — the costliest error
        // class the Swift side ever produced).
        EvalCase::new(
            "channel-edit-by-ref",
            "Copy the contents of long.md into report.md",
        )
        .tool("edit_document")
        .script(&[
            r#"read_document({"path":"long.md"})"#,
            r#"edit_document({"path":"report.md","source_ref":"document#1"})"#,
            "I copied it across.",
        ])
        // `report (edited)` AND NOT `report.md`, and the difference is a claim:
        // `edit_document` writes a NEW version beside the original rather than
        // overwriting it (`edited_name`, edit_document.rs). Asserting the source
        // name would have been satisfied by the `read_document` call in the
        // neighbouring case and would have said nothing about where the bytes
        // landed.
        .evidence(&["file_edited", "report (edited)"])
        // Two dozen kilobytes of file moved from one path to another and the
        // model never saw the far end of it.
        .never_shown(&["line 199:"])
        .calls_at_most(2),
        // A DANGLING REF MUST LEAVE THE FILE ALONE. `report.md` exists and has
        // content; a fallback to an empty body would report `file_edited` over a
        // document it had just emptied.
        EvalCase::new(
            "channel-edit-unknown-ref",
            "Replace report.md with the data in the store",
        )
        .tool("edit_document")
        .script(&[
            r#"edit_document({"path":"report.md","source_ref":"document#99"})"#,
            "I could not find the source data.",
        ])
        .evidence(&["unknown_data_ref"])
        .forbidden(&["file_edited"])
        .once(),
    ]
}

/// Tainted session / the approval gate. There is no real external tool in this
/// turn; the gate's MECHANISM is measured with `FakeExternalTool`.
fn gate() -> Vec<EvalCase> {
    vec![
        // A clean session: the gate is not asked, the call goes through.
        // Approval must be rare so that it gets read.
        EvalCase::new("gate-clean-session", "Send this note to the server: meeting at 14:00")
            .tool("send_out")
            .script(&[
                r#"send_out({"body":"meeting at 14:00"})"#,
                "Sent.",
            ])
            .evidence(&["sent_ok"])
            .once(),
        // A TAINTED SESSION: sending out AFTER a personal document has been read
        // hits the gate deterministically.
        EvalCase::new("gate-tainted-session", "Read report.md and send it to the server")
            .tool("send_out")
            .script(&[
                r#"read_document({"path":"report.md"})"#,
                r#"send_out({"body":"| Monday | Lentils |"})"#,
                "I did not send it.",
            ])
            .evidence(&["permission_denied"])
            .forbidden(&["sent_ok"])
            .calls_at_most(2),
        // NO RETRY AFTER A SIDE EFFECT: once the file has been written, sending
        // the same prompt a second time creates a second file.
        EvalCase::new("gate-no-retry", "Create a report file")
            .tool("create_document")
            .script(&[
                r#"create_document({"format":"markdown","file_name":"report-output","content":"body"})"#,
                "Created.",
            ])
            .evidence(&["file_created", "retryable=false"])
            .once(),
        // THE GATE CAN OPEN, AND UNTIL THIS CASE NOTHING SAID SO.
        //
        // Every gate case above runs against the runner's default `SilentDeny`,
        // so the whole suite measured one arm. A gate hard-wired to refuse — or
        // one whose approving branch had been deleted — passed all 51 cases.
        // `gate-clean-session` is not the missing half: a clean session never
        // reaches the gate, it is not asked at all. Only a TAINTED session with a
        // gate that says yes exercises the path where data actually leaves.
        //
        // `.approved()` is what supplies that gate — see `EvalCase::gate_opens`.
        EvalCase::new(
            "gate-tainted-approved",
            "Read report.md and send it to the server",
        )
        .approved()
        .tool("send_out")
        .script(&[
            r#"read_document({"path":"report.md"})"#,
            r#"send_out({"body":"| Monday | Lentils |"})"#,
            "Sent.",
        ])
        .evidence(&["sent_ok"])
        .forbidden(&["permission_denied"])
        .calls_at_most(2),
        // AN INSISTENCE LOOP HAS TO END, AND THE TURN STILL HAS TO ANSWER.
        //
        // WHAT THIS DOES NOT MEASURE, stated so nobody reads more into it: the
        // executor's DENIAL CACHE is not what refuses the second send here — the
        // runner's gate is `SilentDeny`, so the second call is denied by the gate
        // itself and deleting the cache would leave this case green. The cache
        // has its own tests, which count how many times the gate was asked
        // (`a_source_denied_once_is_never_asked_again` and
        // `a_case_variant_does_not_reopen_a_denied_source`, tacet-tools).
        //
        // WHAT IT DOES MEASURE is the turn-level consequence nothing else covers:
        // two refusals in a row do not eat the budget, the turn still reaches a
        // sentence, and nothing was sent. It sits exactly on `MAX_TURNS` = 4
        // (three calls plus the answering pass), so it goes red the day the turn
        // budget drops — which is the point, and the fault message names the
        // budget so it does not read as a model regression.
        EvalCase::new(
            "gate-denied-stays-denied",
            "Read report.md and send it to the server, then send it again",
        )
        .tool("send_out")
        .script(&[
            r#"read_document({"path":"report.md"})"#,
            r#"send_out({"body":"| Monday | Lentils |"})"#,
            r#"send_out({"body":"the meal table from report.md"})"#,
            "I did not send it.",
        ])
        .evidence(&["permission_denied"])
        .forbidden(&["sent_ok"])
        .calls_at_most(3),
    ]
}

/// GROUNDING — the answer may not carry a number no tool produced.
///
/// WHAT THESE ARE FOR: every case in this file above measures Tacet's LOGIC and
/// is therefore fully determined by its script. These are the same, with one
/// difference — they are written so that the interesting claim is about the
/// SENTENCE, and the sentence is the one thing the script hands over to the
/// model when the suite runs on a real engine.
///
/// SO THEY PASS TRIVIALLY ON `FakeEngine` AND THAT IS NOT A DEFECT: the scripts
/// below are grounded by construction, and the group's job here is to keep the
/// claim itself from rotting (the checker is tested in `runner`). With a real
/// engine the same six lines become the only place in the suite that can fail
/// because of what was SAID rather than which tool was picked.
///
/// THE SHAPE TO COPY: pick an input where the tool returns numbers, and leave
/// the answer free to invent more. A number-free answer (`grounding-long-list`)
/// is still worth a case — that is the one a model spoils by announcing a count
/// it never saw.
fn grounding() -> Vec<EvalCase> {
    vec![
        // Three figures in the file, three in the answer, and a fourth would be
        // an invention.
        EvalCase::new(
            "grounding-budget-figures",
            "What are the figures in budget-2026.md?",
        )
        .tool("read_document")
        .script(&[
            r#"read_document({"path":"budget-2026.md"})"#,
            "Rent 18000 TL, kitchen 9000 TL, transport 2500 TL.",
        ])
        .evidence(&["18000", "9000", "2500"])
        .grounded()
        .behaviour()
        .once(),
        // A result the model cannot plausibly hold: if the sentence and the tool
        // disagree, the sentence is the one that reached the user.
        EvalCase::new("grounding-calc-large", "What is 1234 times 7?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"1234*7"})"#, "1234 x 7 = 8638."])
            .evidence(&["8638"])
            .grounded()
            .behaviour()
            .once(),
        // Calendar arithmetic: the count comes from the tool, and a model that
        // recounts it for itself lands one or two off.
        EvalCase::new("grounding-countdown", "How many days until 1 August 2026?")
            .tool("time")
            .script(&[r#"time({"kind":"diff","target":"2026-08-01"})"#, "12 days."])
            .evidence(&["days=12", "to=2026-08-01"])
            .grounded()
            .behaviour()
            .once(),
        // The bypass channel hands over a PREVIEW. Any count in the answer is
        // therefore a number the model did not see — including the right one.
        EvalCase::new("grounding-long-list", "What is in long.md?")
            .tool("read_document")
            .script(&[
                r#"read_document({"path":"long.md"})"#,
                "It is a long list of filler lines.",
            ])
            .evidence(&["source_ref=document#1"])
            .grounded()
            .behaviour()
            .once(),
        // The table has three rows in the preview and five in the file; "the
        // table has N rows" is the sentence this case exists to catch.
        EvalCase::new("grounding-table-rows", "What does report.md say?")
            .tool("read_document")
            .script(&[
                r#"read_document({"path":"report.md"})"#,
                "A weekly meal table: Monday lentils, Tuesday rice, Wednesday pasta.",
            ])
            .evidence(&["| Monday | Lentils |"])
            .grounded()
            .behaviour()
            .once(),
    ]
}

/// `edit_document` and `find_file` — two tools that had NO logic case at all.
///
/// THE GAP THIS CLOSES: counted against the catalog, seven tools carried zero
/// cases in this file. `create_document` was the one that DID have them, and the
/// last pass over it found three defects (a destination that could not be named,
/// a title written twice, a path the model could not read back). There is no
/// reason to expect the untested ones to be cleaner; these are the two that can
/// be claimed without a network, a repository or a platform gate.
fn edit() -> Vec<EvalCase> {
    vec![
        // THE READ-THEN-EDIT CHAIN: the tool's own description says to read
        // first and pass the FULL new content. Two calls, and the file must
        // really change.
        EvalCase::new("edit-document-row", "Add the row 'Thursday | Beans' to report.md")
            .tool("edit_document")
            .script(&[
                r#"read_document({"path":"report.md"})"#,
                r#"edit_document({"path":"report.md","new_content":"| Day | Meal |\n| --- | --- |\n| Monday | Lentils |\n| Thursday | Beans |"})"#,
                "I added the row.",
            ])
            .evidence(&["file_edited", "report.md"])
            .calls_at_most(2),
        // A FILE THAT IS NOT THERE MUST NOT BE CREATED BY AN EDIT. `edit` means
        // "change what exists"; silently writing a new file would turn a typo in
        // a name into a second, divergent document.
        EvalCase::new("edit-document-missing", "Change the title of nowhere.md")
            .tool("edit_document")
            .script(&[
                r##"edit_document({"path":"nowhere.md","new_content":"# New"})"##,
                "I could not find that file.",
            ])
            .evidence(&["tool_failed"])
            .forbidden(&["file_created"])
            .once(),
        // FIND BY NAME. The fixture exists for exactly this (`BUDGET_FILE`).
        EvalCase::new("find-file-by-name", "Which file is about the budget?")
            .tool("find_file")
            .script(&[r#"find_file({"pattern":"budget"})"#, "budget-2026.md."])
            .evidence(&["budget-2026.md"])
            .once(),
        // FIND BY CONTENT — a different code path, and the one that reads files
        // rather than listing them.
        EvalCase::new("find-file-by-content", "Which file mentions the rent?")
            .tool("find_file")
            .script(&[
                r#"find_file({"pattern":"Rent","search_content":true})"#,
                "budget-2026.md mentions it.",
            ])
            .evidence(&["budget-2026.md"])
            .once(),
        // NOTHING MATCHES: the tool must say so and the model must not name a
        // file. An invented filename is the failure that sends the next turn
        // reading a document that does not exist.
        EvalCase::new("find-file-no-match", "Which file is about submarines?")
            .tool("find_file")
            .script(&[
                r#"find_file({"pattern":"submarine"})"#,
                "I could not find such a file.",
            ])
            .forbidden(&["budget-2026.md", "report.md"])
            .once(),
    ]
}

/// `web_search` against the DRIED tool — the network is off, the description
/// and schema are production's (see `env::DryWebSearch`).
fn search() -> Vec<EvalCase> {
    vec![
        // The number in the answer comes from the search result or from nowhere.
        // This is the case shaped exactly like the session that produced
        // "the water temperature will be 230°C".
        EvalCase::new("search-weather", "What is the weather in Istanbul today?")
            .tool("web_search")
            .script(&[
                r#"web_search({"query":"Istanbul weather today"})"#,
                "Clear, 24 degrees.",
            ])
            .evidence(&["24"])
            .grounded()
            .behaviour()
            .once(),
        // A SECOND NUMBER OUT OF THE SAME RESULT, and that is deliberate. One
        // grounded web case can pass by luck: if the checker only ever sees the
        // one figure the fixture leads with, a grounding rule that matched
        // "the first number in the pool" would look correct. This case takes the
        // humidity (54) instead of the temperature (24) out of the same fixed
        // result, so the two together say the claim is about the POOL and not
        // about a position in it.
        EvalCase::new(
            "search-humidity",
            "What is the humidity in Istanbul right now?",
        )
        .tool("web_search")
        .script(&[
            r#"web_search({"query":"Istanbul humidity today"})"#,
            "Humidity is 54%.",
        ])
        .evidence(&["54"])
        .grounded()
        .behaviour()
        .once(),
    ]
}

/// `remember` — the tool that exists because the model would otherwise answer
/// "I will remember that" while calling nothing, which tells the user something
/// untrue.
fn recall() -> Vec<EvalCase> {
    vec![
        EvalCase::new("remember-fact", "Remember that my sister's name is Ayse")
            .tool("remember")
            .script(&[
                // KEYWORDS ARE PASSED, and finding out that they must be was the
                // first thing this case did. The schema marks `keywords`
                // OPTIONAL, so the grammar happily produces a call without them
                // — and `MemoryStore::add` then refuses it with `NoKeys`,
                // because recall is by keyword and a note with none is
                // unfindable. A model that follows the schema to the letter
                // therefore gets a guaranteed failure.
                //
                // SINCE FIXED, and `remember-derived-keywords` below is what
                // pins the fix: `MemoryTool::save` now derives keys from the
                // text when none arrive, rather than the field being marked
                // required (the schema is one object for three actions and
                // `list` has nothing to key). This case keeps taking the
                // intended path — keywords given by the model always win.
                r#"remember({"action":"save","text":"the user's sister is called Ayse","keywords":"sister, family"})"#,
                "Noted.",
            ])
            // `note saved`, NOT the text: the tool's `to_model` is the
            // structural fact and the chip TRUNCATES the note, so asserting the
            // name would be asserting the truncation limit. What this claim is
            // worth is exactly what the tool guarantees — a save happened, and
            // the model cannot say "I'll remember that" without one.
            //
            // THE ROUND TRIP IS NOT MEASURED HERE — `remember-round-trip` below
            // is where it lives now. The gap this comment used to describe was
            // real and stayed open for as long as it was only written down.
            .evidence(&["note saved"])
            .once(),
        // THE SCHEMA SAYS `keywords` IS OPTIONAL AND THE STORE USED TO REFUSE A
        // NOTE WITHOUT ONE.
        //
        // A model that followed the schema to the letter got a guaranteed
        // failure and the user was told their note could not be saved for a
        // field they were never asked for. `MemoryTool::save` closed it by
        // deriving keys from the note's own words — and NOTHING PINNED THAT.
        // Deleting the derivation would restore the original bug and this suite
        // would have stayed green. That is what this case is: the call the
        // grammar actually produces, with no `keywords` at all.
        EvalCase::new("remember-derived-keywords", "Remember that I am a vegetarian")
            .tool("remember")
            .script(&[
                r#"remember({"action":"save","text":"The user is a vegetarian."})"#,
                "Noted.",
            ])
            .evidence(&["note saved"])
            .forbidden(&["tool_failed"])
            .once(),
        // THE ROUND TRIP — save, then read back — AND IT READS BACK THROUGH THE
        // CHANNEL.
        //
        // TWO CLAIMS IN ONE TURN, and neither could be made before. First, a
        // note saved in this turn is visible to `list` in the same turn ("1
        // notes stored"): a save that reported success and stored nothing was
        // indistinguishable from a working one. Second — and this is the reason
        // the case is worth its length — notes are PERSONAL DATA and `list`
        // returns a COUNT plus a `source_ref`, never the notes themselves. The
        // `never_shown` fragment is the store body's own line prefix, which
        // exists nowhere else: it can only appear in the prompt if `list`
        // stopped using the channel and dumped the notes into the window.
        //
        // MEASURED, NOT ASSUMED: `MemoryTool::list` was edited to append the
        // note body to its `to_model` and this case went red with
        // `bulk data reached the model: "- [fact]" was in the prompt`. Nothing
        // else in the suite moved.
        EvalCase::new(
            "remember-round-trip",
            "Remember my sister is called Ayse, then tell me what you know about me",
        )
        .tool("remember")
        .script(&[
            r#"remember({"action":"save","text":"the user's sister is called Ayse","keywords":"sister, family"})"#,
            r#"remember({"action":"list"})"#,
            "I have one note about you.",
        ])
        .evidence(&["note saved", "1 notes stored", "source_ref=memory#1"])
        .never_shown(&["- [fact]"])
        .calls_at_most(2),
        // A FORGET THAT MATCHED NOTHING MUST NOT REPORT SUCCESS. "Done, I
        // forgot it" over a note that is still there is the worst answer this
        // tool can give: the user stops asking and the note stays.
        EvalCase::new("remember-forget-no-match", "Forget what I told you about submarines")
            .tool("remember")
            .script(&[
                r#"remember({"action":"forget","text":"submarine"})"#,
                "There was no such note.",
            ])
            .evidence(&["no matching note"])
            .forbidden(&["note removed", "notes forgotten"])
            .once(),
        // HOSTILE INPUT: an empty phrase would match EVERY note. In a delete
        // tool that is not a degenerate case, it is a wipe, and `MemoryTool::
        // forget` refuses it outright. The message is the one a real user types.
        EvalCase::new("remember-forget-everything", "Forget everything about me")
            .tool("remember")
            .script(&[
                r#"remember({"action":"forget","text":""})"#,
                "Tell me which note to forget and I will remove it.",
            ])
            .evidence(&["tool_failed"])
            .forbidden(&["note removed", "notes forgotten"])
            .once(),
    ]
}

/// THE LOOP BREAKERS — the two executor gates that end a turn the model cannot
/// end for itself.
///
/// WHY THEY NEEDED A GROUP: `duplicate_call` (GATE 5) and `unknown tool`
/// (GATE 1) appeared in no `expected_evidence` anywhere in this file. Both
/// mechanisms could have been deleted with the suite staying green, and both are
/// what stand between a wrong first move and a turn that never answers.
fn loop_guard() -> Vec<EvalCase> {
    vec![
        // THE SAME CALL TWICE. The executor refuses to RUN it a second time and
        // the runner turns that into "you must answer now"; the two together are
        // the loop breaker. The claim is not only that the repeat was refused —
        // it is that the turn STILL PRODUCED AN ANSWER, which is the half the
        // user actually experiences.
        EvalCase::new("loop-duplicate-call", "What is 125 times 8?")
            .tool("calculate")
            .script(&[
                r#"calculate({"expression":"125*8"})"#,
                r#"calculate({"expression":"125*8"})"#,
                "125 x 8 = 1000.",
            ])
            // `reason=RepeatedCall` is the STRUCTURAL flag, `duplicate_call` the
            // sentence the model is shown. Asserting both is deliberate: the flag
            // alone would pass if the model were told nothing, the sentence alone
            // would pass if the tool had actually run again.
            .evidence(&["duplicate_call", "reason=RepeatedCall", "1000"])
            .calls_at_most(2),
        // GATE 1 — A NAME THAT IS NOT IN THE CATALOG. The model must be told the
        // call failed and must not then invent the answer it was after; a made-up
        // timetable is the failure, not the refusal.
        EvalCase::new("loop-unknown-tool", "Look up the ferry timetable")
            // NO TOOL CLAIM: the point is what happens to a call for a tool that
            // does not exist, and "which tool ran" has no answer — none did.
            .any_tool()
            // UNCONSTRAINED for the same reason `document-schema-violation` is:
            // this case measures the EXECUTOR's lower layer, and the grammar
            // cannot emit a name outside the catalog, so with the constraint on
            // the gate would never be reached. (Under `FakeEngine` the constraint
            // is absent anyway — `FakeEngine` publishes no vocabulary — so the
            // flag is what makes the case still measure the gate when a real
            // engine runs it.)
            .unconstrained()
            .script(&[
                r#"ferry_times({"line":"1"})"#,
                "I do not have a tool for ferry timetables.",
            ])
            .evidence(&["tool_failed", "reason=UnknownTool"])
            .forbidden(&["18:30"])
            .grounded(),
    ]
}

/// GATE 4 — CANCELLATION. The only executor gate with no case anywhere in this
/// file, and the reason was mechanical: `run_case` took `active_turn()` and
/// never called `cancel()`, so the arm was unreachable from eval. See
/// `EvalCase::cancel_before`.
fn cancellation() -> Vec<EvalCase> {
    vec![
        // THE GUARANTEE IS THAT A CANCELLED TURN WRITES NOTHING. `create_document`
        // is the right tool to point at it: it has a side effect, so if the gate
        // is asked one instruction too late the file exists and `file_created`
        // shows up in the pool.
        //
        // `chip_world_changed=false` is the INDEPENDENT half. The executor's own
        // `world_changed` could say false while a tool wrote anyway; the trace
        // collector watched the same turn from the other side.
        EvalCase::new("cancel-before-write", "Create a report file")
            .tool("create_document")
            .cancelled_before(0)
            .script(&[
                r#"create_document({"format":"markdown","file_name":"report-output","content":"body"})"#,
                "I stopped before writing anything.",
            ])
            .evidence(&[
                "cancelled: the user stopped this turn",
                "reason=Cancelled",
                "chip_world_changed=false",
            ])
            .forbidden(&["file_created"])
            .once(),
        // CANCELLED MID-CHAIN: the read already happened and its result stays in
        // the pool; the write had not, and must not. A cancellation that rolled
        // the whole turn back would lose work the user already had, and one that
        // arrived too late would write the file — this case fails on both sides.
        EvalCase::new(
            "cancel-mid-chain",
            "Dump the contents of long.md into a markdown file",
        )
        .tool("create_document")
        .cancelled_before(1)
        .script(&[
            r#"read_document({"path":"long.md"})"#,
            r#"create_document({"format":"markdown","file_name":"dump","source_ref":"document#1"})"#,
            "I stopped before writing the file.",
        ])
        .evidence(&["source_ref=document#1", "reason=Cancelled"])
        .forbidden(&["file_created"])
        .calls_at_most(2),
    ]
}

/// `archive` and `checksum` — two tools that landed with selection cases and no
/// LOGIC case at all.
///
/// WHY THEY QUALIFY WHERE `run_code` DOES NOT: both are pure local computation
/// with no discovery gate, no interpreter and no network, so a case for them
/// measures this code rather than the host. See the catalog note in `env`.
fn archive() -> Vec<EvalCase> {
    vec![
        // BULK GOES TO THE STORE — the same rule `read_document` follows, on a
        // tool that reached the catalog after the rule was written. Two dozen
        // entry names would be two dozen lines of the model's window for data it
        // does not need; what comes back is a count and a reference.
        //
        // `never_shown` is the claim, not `evidence`: a listing that dumped every
        // path into the window would still satisfy "source_ref=archive#1" if the
        // reference were minted and the table sent anyway.
        EvalCase::new("archive-listing-by-ref", "What is inside backup.zip?")
            .tool("archive")
            .script(&[
                r#"archive({"path":"backup.zip","action":"list"})"#,
                "It holds 24 note files.",
            ])
            .evidence(&["24 entries in backup.zip", "source_ref=archive#1"])
            .never_shown(&["notes/entry-01.txt", "notes/entry-24.txt"])
            .once(),
        // EXTRACT REPORTS COUNTS AND A PATH, NEVER CONTENT. Unpacking 24 files
        // is not a reason to put 24 files in the prompt; if the user wants one
        // read, that is a `read_document` call on a path the model now has.
        // `retryable=false` is the second claim: files were written, so the same
        // turn must not be replayed.
        //
        // MEASURED, NOT ASSUMED: `ArchiveTool::extract` was edited to append the
        // decoded entry bodies to its `to_model` and this case went red with
        // `bulk data reached the model: "entry 24 of the fixture archive" was in
        // the prompt`. Nothing else in the suite moved.
        EvalCase::new("archive-extract-counts-not-content", "Unpack backup.zip")
            .tool("archive")
            .script(&[
                r#"archive({"path":"backup.zip","action":"extract"})"#,
                "I unpacked 24 files.",
            ])
            .evidence(&["extracted 24 files", "retryable=false"])
            .never_shown(&["entry 24 of the fixture archive"])
            .once(),
        // THE ONE TOOL IN THE CATALOG WHOSE RIGHT ANSWER CAN BE WRITTEN DOWN IN
        // ADVANCE.
        //
        // MEASURED, NOT COPIED FROM THE CODE: the digest below is what
        // `shasum -a 256` printed for the 66 bytes of `BUDGET_CONTENT` on this
        // machine (macOS arm64, 4 Sep 2026). It is an INDEPENDENT implementation,
        // which is the whole value of the case — the workspace's hand-written
        // SHA-256 is checked against something that did not come from this
        // repository. A defect in the hasher that its own unit tests shared would
        // fail here.
        EvalCase::new("checksum-digest", "What is the SHA-256 of budget-2026.md?")
            .tool("checksum")
            .script(&[
                r#"checksum({"path":"budget-2026.md"})"#,
                "I computed the fingerprint of that file.",
            ])
            .evidence(&[
                "sha256=415066d23b6fe858eff1a3be8e4940b49e7da48b67ac56165dd2d0ff3fdd8c88",
                "bytes=66",
            ])
            .once(),
        // A MISMATCH IS A NORMAL ANSWER, NOT AN ERROR — the tool's own
        // description says so, and the failure to guard against is the opposite
        // one: reporting a match for a digest that does not match.
        EvalCase::new(
            "checksum-mismatch",
            "Does budget-2026.md match the checksum the publisher gave?",
        )
        .tool("checksum")
        .script(&[
            r#"checksum({"path":"budget-2026.md","expected":"0000000000000000000000000000000000000000000000000000000000000000"})"#,
            "No, the file does not match that checksum.",
        ])
        .evidence(&["digest_mismatch"])
        // `digest_match` is not a substring of `digest_mismatch` (they diverge at
        // the ninth character), so this forbids the success word and nothing else.
        .forbidden(&["digest_match", "tool_failed"])
        .once(),
        // HOSTILE INPUT, AND IT IS THE ATTACK A `starts_with` COMPARISON WALKS
        // INTO: a correct PREFIX of the correct digest. Eight characters of the
        // real answer must be refused as a malformed digest, not accepted as a
        // match — the shape is the check.
        EvalCase::new(
            "checksum-truncated-digest",
            "Check budget-2026.md against 415066d2",
        )
        .tool("checksum")
        .script(&[
            r#"checksum({"path":"budget-2026.md","expected":"415066d2"})"#,
            "That is not a full SHA-256; it needs all 64 characters.",
        ])
        .evidence(&["tool_failed"])
        .forbidden(&["digest_match", "digest_mismatch"])
        .once(),
    ]
}

/// THE TURN-BUDGET BOUNDARY. One case, and it exists to be the case that breaks.
fn chain() -> Vec<EvalCase> {
    vec![
        // THREE TOOL CALLS AND AN ANSWER — exactly `MAX_TURNS` (4). Nothing else
        // in this file runs to the ceiling, so lowering the turn budget from 4 to
        // 3 would break real three-step work with the whole suite still green.
        //
        // THE THIRD CALL PASSES `content`, NOT A `source_ref`, and that is not an
        // oversight. `budget-2026.md` is 66 bytes, far under `read_document`'s
        // 1500-byte store threshold, so no reference is ever minted for it — a
        // `source_ref` here would resolve to nothing and the case would fail for
        // a reason that has nothing to do with the turn budget. Small data
        // travels through the model; that is what the threshold is for.
        EvalCase::new(
            "chain-find-read-create",
            "Find the file about the budget and turn it into an excel file",
        )
        .tool("create_document")
        .script(&[
            r#"find_file({"pattern":"budget"})"#,
            r#"read_document({"path":"budget-2026.md"})"#,
            r#"create_document({"format":"excel","file_name":"budget-2026","content":"| Item | Amount |\n| --- | --- |\n| Rent | 18000 |\n| Kitchen | 9000 |\n| Transport | 2500 |"})"#,
            "I made the excel file.",
        ])
        .evidence(&["budget-2026.md", "file_created (excel)", ".xlsx"])
        .calls_at_most(3),
    ]
}
