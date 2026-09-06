//! The eval runner — drives one case from start to finish.
//!
//! THE TURN LOOP: build the prompt -> generate -> is there a call -> execute ->
//! write the outcome into the history -> repeat. If there is no call the
//! generated text IS THE ANSWER and the loop ends.
//!
//! THE EVIDENCE POOL: the verification does not look only at the last sentence.
//! The model writing "1000" in its sentence is not evidence that the tool
//! computed 1000 — the model may have invented the number. That is why the pool
//! also carries tool outputs, chip texts and structural flags; the claim is made
//! at the level of "the SYSTEM PRODUCED this".
//!
//! THE ENGINE COMES FROM OUTSIDE: `EngineSelector` separates the two ends —
//! `FakeSelector` runs the case's script (a logic measurement, deterministic),
//! `SingleEngine` runs a real `EngineProvider` (a model measurement). The same
//! case list feeds both.

use crate::case::EvalCase;
use crate::env::{EXTERNAL_TOOL, Env};
use std::sync::Arc;
use tacet_engine::{
    EngineProvider, FINAL_PASS_INSTRUCTION, FakeEngine, MAX_TURNS, Prompt, SYSTEM_INSTRUCTIONS,
    SamplingSetting, Turn, wait,
};
use tacet_grammar::CallConstraint;
use tacet_kernel::{ToolCatalog, ToolContext, TraceCollector};
use tacet_tools::executor::{AlwaysApprove, ExecutionReason, ToolExecutor};
use tacet_tools::router::Router;

/// The authority that produces an engine for a case.
pub trait EngineSelector {
    fn engine_for(&self, case: &EvalCase) -> Arc<dyn EngineProvider>;
    fn name(&self) -> &str;
}

/// The fake engine that runs the case's script. The default.
pub struct FakeSelector;

impl EngineSelector for FakeSelector {
    fn engine_for(&self, case: &EvalCase) -> Arc<dyn EngineProvider> {
        // When the script runs out, a fixed string rather than an error: the
        // case must not have to know the number of turns in advance.
        Arc::new(FakeEngine::script(case.script.clone()).with_default("Okay."))
    }
    fn name(&self) -> &str {
        "fake"
    }
}

/// The interface for a real engine: a single provider runs all the cases.
pub struct SingleEngine(pub Arc<dyn EngineProvider>);

impl EngineSelector for SingleEngine {
    fn engine_for(&self, _case: &EvalCase) -> Arc<dyn EngineProvider> {
        Arc::clone(&self.0)
    }
    fn name(&self) -> &str {
        self.0.name()
    }
}

/// WHO A FAILURE IS ATTRIBUTED TO, decided from the FAULTS and not from the
/// case.
///
/// WHY THE CASE-LEVEL SPLIT WAS NOT ENOUGH: `EvalCase::measures` says what a
/// case is ABOUT, and with a real engine that turned out to answer the wrong
/// question. The logic line read 40.9% and every one of its thirteen failures
/// was the model's — a tool never called, a call repeated, a generation cut off
/// — so the number named a part of the codebase that was not at fault.
///
/// THE RULE, and it is deliberately one-sided: **the model is blamed first.**
/// Tacet is held responsible only when the model did its part — the expected
/// tool ran, the turn answered, nothing repeated — and the system STILL produced
/// the wrong evidence. Anything else is a downstream symptom: "evidence missing"
/// after a tool that was never called says nothing about the code that would
/// have produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Blame {
    /// The model chose wrongly, repeated itself, or said something unsourced.
    Model,
    /// The model did its part and Tacet produced the wrong thing. THESE are the
    /// failures that are bugs in this repository.
    Tacet,
}

/// The outcome of a single case.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaseOutcome {
    pub name: String,
    pub passed: bool,
    /// What this case holds responsible — carried through so the report can
    /// split the two lines. See `EvalCase::measures`.
    pub measures: crate::case::Measures,
    /// The tools that were called, in order.
    pub called: Vec<String>,
    /// Why it did not pass — empty means it passed.
    pub faults: Vec<String>,
    /// Who the failure is attributed to; `None` when it passed. See `Blame`.
    pub blame: Option<Blame>,
    /// The model's last sentence.
    pub answer: String,
}

/// Runs one case. It DOES NOT RETURN a `Result`: even if the environment cannot
/// be set up, that is a CASE outcome (a failed one), not the whole run's.
pub fn run_case(case: &EvalCase, selector: &dyn EngineSelector) -> CaseOutcome {
    let env = match Env::setup() {
        Ok(e) => e,
        Err(e) => {
            return CaseOutcome {
                name: case.name.clone(),
                passed: false,
                measures: case.measures,
                called: Vec::new(),
                faults: vec![format!("the environment could not be set up: {e}")],
                // Neither of them: the host could not give us a directory.
                blame: Some(Blame::Tacet),
                answer: String::new(),
            };
        }
    };

    let catalog = env.catalog();
    // THE GATE IS A CASE-LEVEL CHOICE, and it used to be a constant. Every gate
    // case in the suite ran against the default `SilentDeny`, so only the CLOSED
    // arm was ever exercised: a gate whose open path had been deleted passed all
    // 51 cases. `EvalCase::approved` is the case saying "measure the other arm".
    let mut executor = ToolExecutor::new(catalog.clone()).external_tool(EXTERNAL_TOOL);
    if case.gate_opens {
        executor = executor.with_gate(AlwaysApprove);
    }
    // THE TRACE COLLECTOR IS NOW HELD ON TO. It used to be created inline here
    // as `Arc::new(...)` and the handle thrown away: because `traces()` could
    // never be called, eval never observed the STATE updates made through
    // `update_chip`. If a tool set its chip to the wrong state (reporting a
    // write job as `Read`, say) eval could not catch it — the evidence pool only
    // saw the `ToolOutcome`.
    let traces = Arc::new(TraceCollector::new());
    let mut ctx = ToolContext::new(
        Arc::clone(&env.store) as Arc<dyn tacet_kernel::DataStore>,
        env.dir(),
        Arc::clone(&traces) as Arc<dyn tacet_kernel::Reporter>,
    );

    let engine = selector.engine_for(case);
    // The grammar constraint RUNS in eval too: if the production path were
    // constrained and eval unconstrained, what is measured would not be the
    // application's behaviour. The exception is the cases measuring the
    // executor's LOWER layer (see `EvalCase::unconstrained`).
    let constraint = (!case.unconstrained)
        .then(|| engine.vocab().map(|v| CallConstraint::new(&v, &catalog)))
        .flatten();
    let router = Router::new();
    let ticket = executor.active_turn();

    let mut history: Vec<Turn> = Vec::new();
    let mut called: Vec<String> = Vec::new();
    // The pool: everything produced by the SYSTEM, not by the model.
    let mut evidence = String::new();
    // THE SECOND POOL, AND IT MUST STAY SEPARATE FROM THE FIRST. Everything the
    // model was actually SHOWN — every pass's prompt, concatenated. See
    // `EvalCase::never_shown`: `forbidden` is checked against `evidence`, which
    // carries `outcome.raw_output`, and `read_document` puts the whole file
    // there ON PURPOSE because that half never reaches the model. Checking
    // `never_shown` against the same pool would fail every case by construction
    // and prove nothing.
    let mut shown = String::new();
    let mut answer = String::new();
    let mut faults: Vec<String> = Vec::new();
    // Set at every push that describes something THE MODEL did. See `Blame`:
    // the flag is raised at the fault sites rather than matched out of the
    // strings afterwards, so a reworded message cannot silently reassign blame.
    let mut model_fault = false;
    // Did the loop end because the model STOPPED CALLING and spoke?
    //
    // The only other way out is running out of turns, and that used to be
    // indistinguishable from success: the expected tool had been called (four
    // times), every `expected_evidence` fragment was in the pool, and the case
    // passed — with NO ANSWER EVER PRODUCED. A real-engine run showed
    // `calc-percent`, `time-diff` and `gate-clean-session` all passing as
    // `calculate, calculate, calculate, calculate`: the model called the tool on
    // every turn, `MAX_TURNS` ran out, and the user would have been left
    // watching four tool calls and reading nothing.
    let mut answered = false;
    // A DUPLICATE CALL ENDS THE TOOL PHASE — see the shell's loop for the
    // measurement. Set here, read by `final_turn` below.
    let mut must_answer = false;

    for turn in 0..MAX_TURNS {
        // THE LAST PASS CARRIES NO TOOLS AND NO CONSTRAINT — the shell does the
        // same thing on the same pass, and the two MUST agree. An eval that let
        // the model call on a pass where the product does not would report a hit
        // rate the user can never get, and one that forbade a call the product
        // allows would report a regression that is not there. See the rationale
        // at the shell's own loop.
        let final_turn = turn + 1 == MAX_TURNS || must_answer;
        // The tool budget is applied AGAIN on every turn: as the history grows
        // the selection must not change, the selection must derive only from the
        // user's message.
        let selected: ToolCatalog = router.select(&case.input, &catalog).into_iter().collect();
        let system = if final_turn {
            format!("{SYSTEM_INSTRUCTIONS}\n\n{FINAL_PASS_INSTRUCTION}")
        } else {
            SYSTEM_INSTRUCTIONS.to_string()
        };
        // WHERE THE QUESTION GOES, and it is the whole tool loop.
        //
        // First pass — nothing has run yet: the question sits in the `question`
        // field, i.e. at the END of the prompt, where a small model weighs it
        // most. Later passes — a tool result has arrived: the question moves
        // INTO the history, IN FRONT OF the tool result, and `question` is left
        // empty.
        //
        // THIS RUNNER DID NOT DO THAT, and it is why the logic set looked so much
        // worse than the shell. `Prompt::new(&system, &case.input)` put the
        // question after the tool result on EVERY pass — which is the exact shape
        // the shell removed, with the reason written at its own loop: "the model
        // saw the same request again, took it for unanswered and called the tool
        // again. That is where the loop came from." `tool_selection.rs` had
        // already been fixed; this file was measuring a prompt the product does
        // not build, and reporting the difference as the model's fault.
        let first_pass = history.is_empty();
        let question = if first_pass { case.input.as_str() } else { "" };
        let previous: Vec<Turn> = if first_pass {
            Vec::new()
        } else {
            std::iter::once(Turn::user(&case.input))
                .chain(history.iter().cloned())
                .collect()
        };
        let mut prompt = Prompt::new(&system, question).with_history(previous);
        if !final_turn {
            prompt = prompt.with_tools(&selected);
        }
        // COLLECTED BEFORE GENERATION, not after: what the model was shown is a
        // property of the prompt, and a claim about it must not depend on what
        // came back.
        shown.push('\n');
        shown.push_str(&prompt.text());

        // THE CANCELLATION HOOK — see `EvalCase::cancel_before`. It sits here,
        // between building the prompt and generating, because GATE 4 is asked at
        // EXECUTE time: cancelling earlier than the pass that carries the call
        // would measure nothing the executor does.
        if case.cancel_before == Some(turn) {
            executor.cancel();
        }

        let generation = match wait(
            engine.generate(
                &prompt,
                constraint
                    .as_ref()
                    .filter(|_| !final_turn)
                    .map(|c| c as &dyn tacet_engine::Constrainer),
                SamplingSetting::default(),
            ),
        ) {
            Ok(g) => g,
            Err(e) => {
                faults.push(format!("engine error: {e}"));
                model_fault = true;
                break;
            }
        };
        // Half output IS NOT PARSED: a tool call that hit the token cap is half
        // a JSON.
        if !generation.stop.is_complete() {
            faults.push("generation was cut off halfway".into());
            model_fault = true;
            break;
        }

        // THE ANSWER DOES NOT JOIN THE POOL HERE. It used to, and while `evidence`
        // was only ever asked `contains` that was harmless. `grounded` asks the
        // opposite question — "is this number in the pool WITHOUT the answer" —
        // and with the answer already inside, every number would ground itself.
        // The two halves are joined below, after the claim that needs them apart.
        // ON THE LAST PASS, WHAT THE MODEL WRITES IS THE ANSWER — the same rule
        // as the other two loops, kept in step with them deliberately.
        let last_pass = turn + 1 == MAX_TURNS;
        let executed = if last_pass {
            None
        } else {
            wait(executor.execute_raw(&generation.text, ticket, &mut ctx))
        };
        let Some(outcome) = executed else {
            answer = generation.text.clone();
            answered = true;
            break;
        };

        called.push(outcome.tool_name.clone());
        evidence.push('\n');
        evidence.push_str(&outcome.to_model);
        evidence.push('\n');
        evidence.push_str(&outcome.chip_text);
        if let Some(raw) = &outcome.raw_output {
            evidence.push('\n');
            evidence.push_str(raw);
        }
        // The structural flags are evidence too: the "no retry" claim is
        // verified from here, not from the text (see the executor, the ban on
        // text matching).
        evidence.push_str(&format!(
            "\nretryable={} world_changed={} reason={:?}",
            outcome.retryable, outcome.world_changed, outcome.reason
        ));

        // A REPEATED CALL ENDS THE TOOL PHASE. The executor already refused to
        // RUN it; what was missing was a consequence for the turn. See the
        // shell's loop for the numbers.
        if outcome.reason == ExecutionReason::RepeatedCall {
            must_answer = true;
        }

        // An approval denial and a cancellation END the turn: the model must not
        // enter an insistence loop.
        if matches!(
            outcome.reason,
            ExecutionReason::ApprovalDenied | ExecutionReason::Cancelled
        ) {
            history.push(Turn::tool(outcome.to_model.clone()));
            continue;
        }
        history.push(Turn::tool(outcome.to_model.clone()));
    }

    // THE CHIP TRACES ARE EVIDENCE TOO. The state coming from the collector is
    // an observation INDEPENDENT of what the `ToolOutcome` declared: if a tool
    // outcome says "Written" while leaving the chip at "Read" (or never updating
    // it), the difference shows up only here.
    for trace in traces.traces() {
        evidence.push('\n');
        evidence.push_str(&format!(
            "chip[{}] {} {:?}",
            trace.icon, trace.text, trace.state
        ));
    }
    // The collector's independent view of the side effects — if it contradicts
    // the executor's, one of the two is lying.
    evidence.push_str(&format!("\nchip_world_changed={}", traces.world_changed()));

    // THE TOOL-SOURCED HALF, frozen before the answer joins it. This is what
    // `grounded` measures against; `expected_evidence` and `forbidden` keep
    // seeing the union, which is what they have always seen.
    let from_tools = evidence.clone();
    evidence.push('\n');
    evidence.push_str(&answer);

    // --- The claims ---

    // A TURN THAT NEVER ANSWERS IS NOT A PASS.
    //
    // `faults.is_empty()` is what tells exhaustion apart from the two early
    // breaks above (engine error, half-generated call): those already said why
    // there is no answer, and repeating it here would only add noise. What is
    // left is the silent case — the model kept calling and the turn budget ran
    // out. Nothing else in this function can see it: the tool WAS called, the
    // evidence IS in the pool, and `grounded` finds no unsourced number because
    // there is no sentence to look at.
    if !answered && faults.is_empty() {
        faults.push(format!(
            "no answer: the model was still calling tools when the {MAX_TURNS}-turn \
             budget ran out (called: {called:?})"
        ));
        model_fault = true;
    }

    // `tool_claim_waived` SKIPS THIS BLOCK ENTIRELY, both arms of it: the case
    // says the choice is not what it is about. See `EvalCase::tool_claim_waived`.
    match &case.expected_tool {
        _ if case.tool_claim_waived => {}
        Some(name) if !called.iter().any(|c| c == name) => {
            faults.push(format!(
                "the expected tool was not called: {name} (called: {called:?})"
            ));
            model_fault = true;
        }
        None if !called.is_empty() => {
            // Tool appetite: the most frequent regression.
            faults.push(format!(
                "no tool should have been called, these were: {called:?}"
            ));
            model_fault = true;
        }
        _ => {}
    }

    for part in &case.expected_evidence {
        if !evidence.contains(part.as_str()) {
            faults.push(format!("evidence missing: {part:?}"));
        }
    }
    for part in &case.forbidden {
        if evidence.contains(part.as_str()) {
            faults.push(format!("forbidden evidence appeared: {part:?}"));
        }
    }

    // THE BYPASS CHANNEL, MEASURED. Against `shown`, never against `evidence` —
    // see `EvalCase::never_shown` for why the two pools cannot be one.
    for part in &case.never_shown {
        if shown.contains(part.as_str()) {
            faults.push(format!(
                "bulk data reached the model: {part:?} was in the prompt"
            ));
        }
    }

    // THE CALL BUDGET. See `EvalCase::max_calls` — a turn that produced the
    // right thing three times over is not the same turn as one that did it once,
    // and until this claim existed nothing here could tell them apart.
    if let Some(cap) = case.max_calls
        && called.len() > cap
    {
        faults.push(format!(
            "{} tool calls for a turn that needs at most {cap}: {called:?}",
            called.len()
        ));
        model_fault = true;
    }

    // GROUNDING: no number in the answer that no tool produced.
    //
    // ONLY WHEN A TOOL RAN. With no call there is nothing to be grounded in, and
    // asking the question anyway would fail every arithmetic-free chat answer
    // that happens to contain a year. A case that expected a tool and did not
    // get one has already failed above; this is not the place to say it twice.
    if case.grounded && !called.is_empty() {
        for number in numbers_in(&answer) {
            if !from_tools.contains(number.as_str()) {
                faults.push(format!(
                    "unsourced number in the answer: {number:?} — no tool produced it"
                ));
                model_fault = true;
            }
        }
    }

    let blame = if faults.is_empty() {
        None
    } else if model_fault {
        Some(Blame::Model)
    } else {
        // The model called what was asked, answered, and did not repeat itself —
        // and the evidence still came out wrong. That is ours.
        Some(Blame::Tacet)
    };

    CaseOutcome {
        name: case.name.clone(),
        passed: faults.is_empty(),
        measures: case.measures,
        blame,
        called,
        faults,
        answer,
    }
}

/// Runs all the cases.
pub fn run(cases: &[EvalCase], selector: &dyn EngineSelector) -> crate::report::EvalReport {
    let outcomes = cases.iter().map(|c| run_case(c, selector)).collect();
    crate::report::EvalReport::new(selector.name(), outcomes)
}

/// A single digit is not a number worth grounding.
///
/// WHY TWO AND NOT ONE: a lone digit is a substring of almost any pool — the
/// chip texts alone carry enough of them that the claim would be vacuous — and
/// it is also what enumerations ("1.", "2.") and ordinary prose are made of. Two
/// digits is the point where a wrong value starts being a wrong ANSWER: the
/// measured failure was "230".
const GROUNDING_MIN_DIGITS: usize = 2;

/// The maximal runs of ASCII digits in a text, deduplicated, order preserved.
///
/// DIGITS ONLY, AND DELIBERATELY SO. Separators are not part of the run, so
/// "2026-12-02" yields "2026", "12", "02" and grounds against a pool holding
/// `to=2026-12-02`; "24°C" yields "24"; "%54" yields "54". A decimal splits into
/// its two halves, which both ground against the pool's own spelling of it. Every
/// one of those is the permissive direction — see `EvalCase::grounded` for why
/// permissive is the correct direction here.
fn numbers_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut Vec<String>| {
        if run.len() >= GROUNDING_MIN_DIGITS && !out.iter().any(|n| n == run) {
            out.push(run.clone());
        }
        run.clear();
    };
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            run.push(ch);
        } else {
            flush(&mut run, &mut out);
        }
    }
    flush(&mut run, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::EvalCase;

    #[test]
    fn a_digit_run_is_taken_whole_and_separators_split_it() {
        assert_eq!(numbers_in("125 x 8 = 1000."), vec!["125", "1000"]);
        assert_eq!(numbers_in("2026-12-02"), vec!["2026", "12", "02"]);
        assert_eq!(numbers_in("24°C, %54"), vec!["24", "54"]);
        // A single digit is below the floor; so is a text with none at all.
        assert!(numbers_in("8 items").is_empty());
        assert!(numbers_in("no numbers here").is_empty());
        // Repeats are reported once — one fault per invented value, not per
        // mention.
        assert_eq!(numbers_in("230 and 230 again"), vec!["230"]);
    }

    /// THE CASE THIS WHOLE CLAIM EXISTS FOR, in miniature: the tool is called
    /// correctly, the tool's number is right, and the answer carries a number
    /// that was never produced.
    #[test]
    fn a_number_the_tool_never_produced_fails_a_grounded_case() {
        let case = EvalCase::new("grounding-probe", "What is 125 times 8?")
            .tool("calculate")
            .script(&[
                r#"calculate({"expression":"125*8"})"#,
                // 1000 is the tool's; 230 is invented, exactly as "230°C" was.
                "125 x 8 = 1000, and the water temperature will be 230 degrees.",
            ])
            .evidence(&["1000"])
            .grounded();

        let outcome = run_case(&case, &FakeSelector);
        assert!(!outcome.passed, "faults: {:?}", outcome.faults);
        assert!(
            outcome.faults.iter().any(|f| f.contains("\"230\"")),
            "the unsourced number should be named: {:?}",
            outcome.faults
        );
        // AND THE TOOL'S OWN NUMBER MUST NOT BE FLAGGED — a claim that fires on
        // correct answers is worse than no claim.
        assert!(
            !outcome.faults.iter().any(|f| f.contains("\"1000\"")),
            "1000 came from the tool: {:?}",
            outcome.faults
        );
    }

    /// The same case with an honest answer passes — the guard against a claim
    /// that fails everything.
    #[test]
    fn an_answer_that_stays_inside_the_tool_output_passes() {
        let case = EvalCase::new("grounding-clean", "What is 125 times 8?")
            .tool("calculate")
            .script(&[r#"calculate({"expression":"125*8"})"#, "125 x 8 = 1000."])
            .evidence(&["1000"])
            .grounded();

        let outcome = run_case(&case, &FakeSelector);
        assert!(outcome.passed, "faults: {:?}", outcome.faults);
    }

    /// The script both gate cases share; only the flag differs.
    fn tainted_send() -> EvalCase {
        EvalCase::new("gate-probe", "Read report.md and send it to the server")
            .tool("send_out")
            .script(&[
                r#"read_document({"path":"report.md"})"#,
                r#"send_out({"body":"| Monday | Lentils |"})"#,
                "Done.",
            ])
            .calls_at_most(2)
    }

    /// THE PROOF THAT `.approved()` IS NOT DECORATION: the same turn, the same
    /// script, opposite outcomes. Without the flag the runner's default gate
    /// (`SilentDeny`) refuses and nothing is sent; with it the data leaves.
    ///
    /// WHY IT MATTERS MORE THAN IT LOOKS: before the flag existed the suite could
    /// only ever see the refusing arm, so a gate wired shut passed everything.
    /// This test is what says the open arm is reachable at all.
    #[test]
    fn the_approval_gate_has_two_arms_and_the_flag_chooses() {
        let denied = run_case(
            &tainted_send()
                .evidence(&["permission_denied"])
                .forbidden(&["sent_ok"]),
            &FakeSelector,
        );
        assert!(denied.passed, "faults: {:?}", denied.faults);

        let approved = run_case(
            &tainted_send()
                .approved()
                .evidence(&["sent_ok"])
                .forbidden(&["permission_denied"]),
            &FakeSelector,
        );
        assert!(approved.passed, "faults: {:?}", approved.faults);

        // AND THE FLAG IS WHAT DID IT. The approved claim run WITHOUT the flag
        // must fail — otherwise the two cases above would both be passing for
        // some reason other than the gate.
        let without = run_case(
            &tainted_send()
                .evidence(&["sent_ok"])
                .forbidden(&["permission_denied"]),
            &FakeSelector,
        );
        assert!(
            !without.passed,
            "the default gate must refuse: {:?}",
            without.faults
        );
    }

    /// GATE 4, both halves: a cancelled turn writes NOTHING, and the same script
    /// uncancelled writes the file. The second half is the guard against a test
    /// that would pass on a broken `create_document`.
    #[test]
    fn a_cancelled_turn_writes_nothing() {
        let script = [
            r#"create_document({"format":"markdown","file_name":"report-output","content":"body"})"#,
            "I stopped.",
        ];
        let base = || {
            EvalCase::new("cancel-probe", "Create a report file")
                .tool("create_document")
                .script(&script)
                .once()
        };

        let cancelled = run_case(
            &base()
                .cancelled_before(0)
                .evidence(&["reason=Cancelled"])
                .forbidden(&["file_created"]),
            &FakeSelector,
        );
        assert!(cancelled.passed, "faults: {:?}", cancelled.faults);

        // WITHOUT THE CANCEL the very same claim must FAIL, and it must fail on
        // `file_created` — the fragment the guarantee is about.
        let uncancelled = run_case(
            &base()
                .evidence(&["reason=Cancelled"])
                .forbidden(&["file_created"]),
            &FakeSelector,
        );
        assert!(!uncancelled.passed);
        assert!(
            uncancelled
                .faults
                .iter()
                .any(|f| f.contains("file_created")),
            "the write should have been named: {:?}",
            uncancelled.faults
        );
    }

    /// THE BYPASS CHANNEL, AND THE CLAIM NO OTHER FIELD CAN MAKE.
    ///
    /// Both halves run the SAME turn over long.md. The fragment inside the
    /// preview (`line 3:`) MUST be found in the prompt — that is the guard
    /// against a `never_shown` that passes because it never looks. The fragment
    /// past `MODEL_CAP` (`line 199:`) must not be, and it is in the evidence pool
    /// the whole time, because `read_document` puts the entire file into
    /// `raw_output` on purpose. `forbidden` therefore CANNOT express this claim;
    /// the third assertion is the demonstration.
    #[test]
    fn bulk_data_is_kept_out_of_the_prompt_and_forbidden_cannot_say_so() {
        let base = || {
            EvalCase::new("channel-probe", "What is in the file long.md?")
                .tool("read_document")
                .script(&[r#"read_document({"path":"long.md"})"#, "It is a long list."])
                .once()
        };

        let past_the_cap = run_case(&base().never_shown(&["line 199:"]), &FakeSelector);
        assert!(past_the_cap.passed, "faults: {:?}", past_the_cap.faults);

        let inside_the_preview = run_case(&base().never_shown(&["line 3:"]), &FakeSelector);
        assert!(
            !inside_the_preview.passed,
            "a fragment the model WAS shown must be caught, or the claim never looks"
        );
        assert!(
            inside_the_preview
                .faults
                .iter()
                .any(|f| f.contains("bulk data reached the model")),
            "{:?}",
            inside_the_preview.faults
        );

        // WHY THE SECOND POOL HAD TO EXIST. The same fragment through
        // `forbidden` fails, because the evidence pool carries `raw_output` and
        // that is the whole file. Written as a passing assertion so it stands as
        // the record: `forbidden` and `never_shown` are not interchangeable.
        let through_forbidden = run_case(&base().forbidden(&["line 199:"]), &FakeSelector);
        assert!(
            !through_forbidden.passed,
            "if this ever passes, `raw_output` left the evidence pool and this whole \
             split can be reconsidered"
        );
    }

    /// Grounding is not asked of a turn with no tool call: there is nothing to
    /// be grounded in.
    #[test]
    fn a_toolless_turn_is_not_grounded() {
        let case = EvalCase::new("grounding-chat", "Hello")
            .script(&["Hello! There are 42 ways I can help."])
            .grounded();

        let outcome = run_case(&case, &FakeSelector);
        assert!(outcome.passed, "faults: {:?}", outcome.faults);
    }
}
