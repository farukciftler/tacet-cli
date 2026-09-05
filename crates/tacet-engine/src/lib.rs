//! tacet-engine — the ABSTRACTION of the model runner.
//!
//! This crate does not run a model; it defines what "running a model" means: how
//! the prompt is built (`Prompt`), what gets sacrificed when the context budget
//! is exceeded (`TokenCounter`), how generation is forced into a grammar
//! (`Constrainer`) and how the thing doing all this stays replaceable
//! (`EngineProvider`).
//!
//! USER DECISION — PURE RUST: inference is done with the Candle / mistral.rs
//! family, NO llama.cpp FFI. The reason is the same as Tacet's identity: the
//! build, cross-compilation and security surface of a C++ dependency is on its
//! own bigger than the rest of the project.
//!
//! THE DEFAULT BUILD DOES NOT PULL CANDLE. Real inference sits behind the
//! `candle` feature; `FakeEngine` compiles by default and all eval/CI run on it.
//! Reversed, testing the logic layer would depend on a weight file and a
//! minutes-long build.
//!
//! NO NETWORK: this crate makes no network call at all; the model file is
//! supplied from outside.

pub mod constraint;
pub mod error;
pub mod executor;
pub mod fake;
// NOT BEHIND THE `candle` FEATURE — deliberately. The GGUF metadata reader is
// plain `std::io`, and the DISCOVERY side (which package is loadable at all) runs
// in the default build, where candle is absent. Only the function that produces a
// `tokenizers::Tokenizer` is gated inside the module.
pub mod gguf_tokenizer;
pub mod prompt;
pub mod provider;
pub mod session;
pub mod thinking;
pub mod token;

#[cfg(feature = "candle")]
pub mod candle_engine;

pub use constraint::{Constrainer, ConstraintSession, FreeConstraint};
pub use error::{EngineError, EngineResult};
pub use executor::wait;
pub use fake::{FakeEngine, FakeStep};
#[cfg(feature = "candle")]
pub use gguf_tokenizer::tokenizer_from_gguf;
pub use gguf_tokenizer::{
    gguf_context_length, gguf_has_tokenizer, gguf_kv_bytes_per_token, is_context_length_key,
};
pub use prompt::{GUIDE_LIMIT, Prompt, Role, Template, Turn};
pub use provider::{
    EngineIdentity, EngineProvider, Generation, GenerationFuture, SamplingSetting, StopReason,
    boxed_generation,
};
pub use session::{FINAL_PASS_INSTRUCTION, MAX_TURNS, SYSTEM_INSTRUCTIONS};
pub use thinking::extract as extract_thinking;
pub use token::{
    CONTEXT_BUDGET, Device, GENERATION_SHARE, KV_CACHE_BUDGET_BYTES, TokenCounter,
    TruncationReport, context_budget, kv_cache_budget,
};

#[cfg(feature = "candle")]
pub use candle_engine::{Architecture, CandleEngine, ModelSetting, TokenizerSource};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tacet_kernel::{
        ArgSchema, Field, Tool, ToolCatalog, ToolContext, ToolFuture, ToolOutcome, boxed,
    };

    // --- Test helpers ---

    struct FakeTool;

    impl Tool for FakeTool {
        fn name(&self) -> &str {
            "calendar_read"
        }
        fn description(&self) -> &str {
            "Reads calendar events."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![Field::new("day", ArgSchema::text()).required()])
        }
        fn run<'a>(
            &'a self,
            _args: serde_json::Value,
            _ctx: &'a mut ToolContext,
        ) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::read_ok("ok", "ok") })
        }
    }

    fn catalog() -> ToolCatalog {
        let mut c = ToolCatalog::new();
        c.add(Arc::new(FakeTool));
        c
    }

    /// A toy constraint that forbids a given character and reaches "accept" after
    /// 5 tokens. The real grammar is tacet-grammar's job; the only duty here is to
    /// prove that the mask/advance/is_done triangle IS ACTUALLY DRIVEN by the
    /// engine.
    struct ToyConstraint {
        forbidden: char,
    }

    struct ToySession {
        forbidden: char,
        counter: usize,
    }

    impl Constrainer for ToyConstraint {
        fn session(&self) -> Box<dyn ConstraintSession> {
            Box::new(ToySession {
                forbidden: self.forbidden,
                counter: 0,
            })
        }
        fn name(&self) -> &str {
            "toy"
        }
    }

    impl ConstraintSession for ToySession {
        fn mask(&self, logits: &mut [f32]) {
            let i = self.forbidden as usize;
            if i < logits.len() {
                logits[i] = f32::NEG_INFINITY;
            }
        }
        fn advance(&mut self, _token: u32) -> Result<(), tacet_kernel::ConstraintError> {
            self.counter += 1;
            Ok(())
        }
        fn is_done(&self) -> bool {
            self.counter >= 5
        }
    }

    // --- Prompt assembly ---

    #[test]
    fn prompt_pieces_are_laid_out_in_a_fixed_order() {
        let prompt = Prompt::new("You are Tacet.", "what is on tomorrow?")
            .with_tools(&catalog())
            .with_guide("Call calendar_read first.")
            .with_history([Turn::user("hello"), Turn::assistant("hi")]);

        let m = prompt.text();
        let at = |needle: &str| {
            m.find(needle)
                .unwrap_or_else(|| panic!("missing: {needle}"))
        };

        assert!(at("<system>") < at("<tools>"));
        assert!(at("<tools>") < at("<history>"));
        assert!(at("<history>") < at("<guidance>"));
        // The question is LAST: in a small model the last block carries the most
        // weight.
        assert!(at("<guidance>") < at("what is on tomorrow?"));
        assert!(m.trim_end().ends_with("Assistant:"));
    }

    #[test]
    fn static_first_prompt_ordering_preserves_prefix_when_memory_notes_differ() {
        use crate::prompt::Template;
        let prompt1 = Prompt::new("SYSTEM INSTRUCTIONS\ndir_block: /path/to/dir", "question 1")
            .with_tools(&catalog())
            .with_memory("user prefers dark mode");

        let prompt2 = Prompt::new("SYSTEM INSTRUCTIONS\ndir_block: /path/to/dir", "question 2")
            .with_tools(&catalog())
            .with_memory("user prefers light mode");

        let text1 = prompt1.text_with_template(Template::ChatML);
        let text2 = prompt2.text_with_template(Template::ChatML);

        let system_prefix = "<|im_start|>system\nSYSTEM INSTRUCTIONS\ndir_block: /path/to/dir";
        assert!(text1.starts_with(system_prefix));
        assert!(text2.starts_with(system_prefix));

        let common_prefix_len = text1
            .bytes()
            .zip(text2.bytes())
            .take_while(|(b1, b2)| b1 == b2)
            .count();
        let tools_end = text1.find("</tools>").unwrap() + "</tools>".len();
        assert!(common_prefix_len >= tools_end);
    }

    #[test]
    fn the_chatml_template_separates_roles_with_fences() {
        use crate::prompt::Template;
        let prompt = Prompt::new("You are Tacet.", "what is on tomorrow?")
            .with_tools(&catalog())
            .with_history([
                Turn::user("hello"),
                Turn::assistant("hi"),
                Turn::tool("result: 3"),
            ]);
        let m = prompt.text_with_template(Template::ChatML);

        // The tool description is INSIDE the SYSTEM turn: as a separate turn it
        // could be taken for an "old turn" and truncated under budget pressure.
        let system_end = m.find("<|im_end|>").unwrap();
        assert!(m.find("calendar_read").unwrap() < system_end);

        assert!(m.contains("<|im_start|>system\n"));
        assert!(m.contains("<|im_start|>user\nhello<|im_end|>"));
        assert!(m.contains("<|im_start|>assistant\nhi<|im_end|>"));
        // TOOL OUTPUT IS IN THE `user` ROLE and wrapped in `<tool_response>`.
        // Verified against Qwen3's official template in the GGUF metadata: the
        // fence written in the `tool` role branch is `<|im_start|>user`. It used
        // to emit `<|im_start|>tool` here; the model saw a role name it had never
        // seen in training, took the tool result for context-free text and called
        // the tool again.
        assert!(
            m.contains("<|im_start|>user\n<tool_response>\nresult: 3\n</tool_response><|im_end|>"),
            "\n{m}"
        );
        assert!(!m.contains("<|im_start|>tool"), "\n{m}");
        // The generation anchor is last, OPEN and EMPTY.
        //
        // Nothing must be written after the anchor. In particular, DO NOT
        // PRE-FILL `<think></think>`: it was tried, it turns thinking off but
        // breaks the tool call format (Qwen3-4B measurement 7/10 -> 2/10). The
        // reasoning is in the anchor comment in `prompt.rs`. This test locks that
        // reversal in.
        assert!(m.ends_with("<|im_start|>assistant\n"), "\n{m}");
    }

    /// GEMMA HAS NO SYSTEM ROLE. The easiest mistake is to copy ChatML, change
    /// the tags and write `<start_of_turn>system`; that string DOES NOT OCCUR in
    /// Gemma's training data and loses its instruction-binding force. This test
    /// forbids exactly that.
    #[test]
    fn the_gemma_template_produces_no_system_role() {
        use crate::prompt::Template;
        let prompt = Prompt::new("You are Tacet.", "what is on tomorrow?").with_tools(&catalog());
        let m = prompt.text_with_template(Template::Gemma);

        assert!(!m.contains("system"), "Gemma has no system role:\n{m}");
        assert!(!m.contains("<|im_start|>"), "ChatML fences leaked:\n{m}");
        // The system instructions and the tool description are inside THE FIRST
        // USER TURN.
        assert!(m.starts_with("<start_of_turn>user\nYou are Tacet."));
        assert!(m.contains("calendar_read"));
        // With no history there is a single user turn: two consecutive `user`
        // turns would break Gemma's alternating template.
        assert_eq!(m.matches("<start_of_turn>user").count(), 1, "\n{m}");
        // The generation anchor is `model`, NOT `assistant`.
        assert!(m.ends_with("<start_of_turn>model\n"), "\n{m}");
    }

    #[test]
    fn the_gemma_template_alternates_roles_in_the_history() {
        use crate::prompt::Template;
        let prompt = Prompt::new("You are Tacet.", "what is on tomorrow?").with_history([
            Turn::user("hello"),
            Turn::assistant("hi"),
            Turn::tool("result: 3"),
        ]);
        let m = prompt.text_with_template(Template::Gemma);

        // Since the system block is the head of the first `user` turn, "hello" is
        // ITS continuation; opening a separate user turn would give two
        // consecutive `user` turns.
        assert!(
            m.starts_with("<start_of_turn>user\nYou are Tacet.\n\nhello<end_of_turn>"),
            "\n{m}"
        );
        // The assistant is named `model`.
        assert!(m.contains("<start_of_turn>model\nhi<end_of_turn>"));
        // TOOL OUTPUT is written on the `user` side (inventing a third role name
        // would be a fence the model has never seen) and marked with
        // `<tool_response>`. THE QUESTION CONTINUES IN THE SAME TURN: since both
        // the tool result and the question are `user`, opening separate turns
        // would break Gemma's alternating template.
        assert!(
            m.contains(
                "<start_of_turn>user\n<tool_response>\nresult: 3\n</tool_response>\n\nwhat is on tomorrow?<end_of_turn>"
            ),
            "\n{m}"
        );
        // THE REAL INVARIANT: nowhere may the same role turn appear twice in a
        // row. The Gemma template has to alternate; two `user` turns side by side
        // push the model outside the training distribution.
        let roles: Vec<&str> = m
            .match_indices("<start_of_turn>")
            .map(|(i, c)| {
                if m[i + c.len()..].starts_with("user") {
                    "user"
                } else {
                    "model"
                }
            })
            .collect();
        assert!(
            roles.windows(2).all(|p| p[0] != p[1]),
            "consecutive identical roles: {roles:?}\n{m}"
        );
        assert!(!m.contains("assistant") && !m.contains("tool\n"), "\n{m}");
    }

    /// The guide must sit IMMEDIATELY BEFORE the question (see the SKILL DECISION
    /// in prompt.rs). This was verified for ChatML; the Gemma branch is separate
    /// code and needs its own check — it is easy for it to drift silently.
    #[test]
    fn gemma_keeps_the_guide_in_front_of_the_question() {
        use crate::prompt::Template;
        for with_history in [false, true] {
            let mut prompt = Prompt::new("s", "the real question here").with_guide("GUIDE TEXT");
            if with_history {
                prompt = prompt.with_history([Turn::user("earlier")]);
            }
            let m = prompt.text_with_template(Template::Gemma);
            let g = m.find("GUIDE TEXT").expect("no guide");
            let q = m.find("the real question here").expect("no question");
            assert!(
                g < q,
                "guide is not in front of the question (with_history={with_history}):\n{m}"
            );
            // Both are inside THE SAME turn: no role boundary may come between.
            assert!(
                !m[g..q].contains("<start_of_turn>"),
                "a role boundary got in between:\n{m}"
            );
        }
    }

    /// THE TOOL LOOP'S SECOND TURN STILL GETS ITS GUIDANCE — on every template.
    ///
    /// The shape being measured is the one production builds from turn two
    /// onward: the question has MOVED INTO THE HISTORY (in front of the tool
    /// call) and `question` is left empty, deliberately, so the model does not
    /// read its own request as unanswered. The guide used to be written only
    /// inside the question turn, so it vanished at exactly that point — the turn
    /// where the model is recovering from a tool result and needs the guidance
    /// most. `Plain` kept it, which is worse than a plain bug: eval runs on
    /// FakeEngine, FakeEngine is `Plain`, so the one template no model sees was
    /// the one template the measurement covered.
    #[test]
    fn the_guide_survives_an_empty_question_on_every_template() {
        use crate::prompt::Template;
        let prompt = Prompt::new("s", "").with_guide("GUIDE TEXT").with_history([
            Turn::user("the real question here"),
            Turn::assistant("calculate({\"expression\":\"12*8\"})"),
            Turn::tool("96"),
        ]);
        for template in [Template::Plain, Template::ChatML, Template::Gemma] {
            let m = prompt.text_with_template(template);
            assert!(
                m.contains("GUIDE TEXT"),
                "the guide was dropped on {template:?}:\n{m}"
            );
            // AND IT IS THE LAST BLOCK, right before the generation anchor —
            // being present but buried above the tool result would give away the
            // weight advantage the guide is placed for.
            let g = m.find("GUIDE TEXT").expect("no guide");
            let r = m.find("96").expect("no tool result");
            assert!(
                r < g,
                "the guide is above the tool result on {template:?}:\n{m}"
            );
        }
    }

    /// The fix for the test above must not buy its way out with a second
    /// consecutive `user` fence — a sequence absent from the training
    /// distribution, and the reason `chatml_text` merges tool results in the
    /// first place.
    #[test]
    fn the_guide_does_not_open_a_second_consecutive_user_turn() {
        use crate::prompt::Template;
        let prompt = Prompt::new("s", "").with_guide("GUIDE TEXT").with_history([
            Turn::user("q"),
            Turn::assistant("call"),
            Turn::tool("96"),
        ]);

        let chatml = prompt.text_with_template(Template::ChatML);
        assert!(
            !chatml.contains("<|im_end|>\n<|im_start|>user\n<guidance>"),
            "the guidance opened its own user turn after the tool result:\n{chatml}"
        );
        let gemma = prompt.text_with_template(Template::Gemma);
        assert!(
            !gemma.contains("<end_of_turn>\n<start_of_turn>user\n<guidance>"),
            "the guidance opened its own user turn after the tool result:\n{gemma}"
        );
    }

    #[test]
    fn the_plain_template_never_uses_chatml_fences() {
        use crate::prompt::Template;
        let prompt = Prompt::new("s", "q").with_tools(&catalog());
        let m = prompt.text_with_template(Template::Plain);
        assert!(!m.contains("<|im_start|>"));
        // The default `text()` is the plain format: the budget measurement and the
        // CLI output rest on it.
        assert_eq!(m, prompt.text());
    }

    #[test]
    fn the_fake_engine_declares_the_plain_template() {
        use crate::prompt::Template;
        // The template is a property OF THE ENGINE; since a mismatch produces
        // silent breakage, keeping the default plain is deliberate.
        assert_eq!(
            FakeEngine::script(Vec::<FakeStep>::new()).template(),
            Template::Plain
        );
    }

    #[test]
    fn the_system_instructions_contain_no_placeholder_tool_name() {
        // MEASURED ON A REAL MODEL: a 3B model copies the placeholder call in the
        // prompt verbatim. If a fake signature such as `tool_name(` or `name(`
        // appears in the instructions, that regression comes back.
        assert!(!SYSTEM_INSTRUCTIONS.contains("tool_name("));
        assert!(!SYSTEM_INSTRUCTIONS.contains(" name("));
        // The example has to be given over a REAL tool — and it MUST be given.
        // Removing the example fixes Gemma's copying bug but breaks Qwen3-4B:
        // measured, with no example it never wrote the tool NAME at all and
        // produced bare JSON directly (even ```json blocks), i.e. the call became
        // unparseable. A verbal description ("write the name first, a call
        // without a name is invalid") was not enough on its own. Between the two
        // the primary model wins; Gemma's copying remains a known and reported
        // limitation.
        assert!(SYSTEM_INSTRUCTIONS.contains("calculate({"));
    }

    #[test]
    fn the_tool_description_is_derived_only_from_the_catalog() {
        let m = Prompt::new("s", "q").with_tools(&catalog()).text();
        assert!(m.contains("calendar_read"));
        assert!(m.contains("Reads calendar events."));
        // THE SIGNATURE IS IN SHORT FORM: the model should not have to invent a
        // signature, but the full JSON Schema is not printed either. The arguments
        // are already forced by the grammar, and a schema kept in two places will
        // eventually drift — plus in a 4096 window the full schema alone was
        // eating a large part of the budget.
        assert!(m.contains("calendar_read(day: text)"), "\n{m}");
        assert!(
            !m.contains("\"required\""),
            "the full JSON Schema leaked:\n{m}"
        );
        assert!(
            !m.contains("additionalProperties"),
            "the full JSON Schema leaked:\n{m}"
        );
    }

    /// The required/optional distinction MUST BE VISIBLE in the signature: the
    /// model has no other place to learn which field it may omit.
    #[test]
    fn the_short_signature_marks_required_plain_and_optional_with_a_question_mark() {
        use tacet_kernel::{ArgSchema, Field};
        let schema = ArgSchema::object(vec![
            Field::new("expression", ArgSchema::text()).required(),
            Field::new("digits", ArgSchema::integer()),
        ]);
        assert_eq!(
            schema.short_signature(),
            "expression: text, digits?: integer"
        );
    }

    #[test]
    fn an_empty_catalog_produces_no_tool_block() {
        let m = Prompt::new("s", "q").with_tools(&ToolCatalog::new()).text();
        assert!(!m.contains("<tools>"));
    }

    #[test]
    fn the_guide_is_cut_from_the_front_and_stays_within_the_limit() {
        // The core (the concrete call example) sits at the FRONT of the file; if
        // the cut were from the end, exactly that part would be thrown away.
        let long = format!("CORE: calendar_read(day)\n{}", "filler ".repeat(400));
        let prompt = Prompt::new("s", "q").with_guide(&long);
        let g = prompt.guide.as_deref().unwrap();
        assert!(g.starts_with("CORE: calendar_read(day)"));
        assert_eq!(g.chars().count(), GUIDE_LIMIT);
    }

    #[test]
    fn an_empty_guide_opens_no_block() {
        let prompt = Prompt::new("s", "q").with_guide("   \n  ");
        assert!(prompt.guide.is_none());
        assert!(!prompt.text().contains("<guidance>"));
    }

    // --- Budget truncation policy ---

    fn full_prompt(turn_count: usize, turn_length: usize) -> Prompt {
        Prompt::new("SYSTEM INSTRUCTIONS", "the real question here")
            .with_tools(&catalog())
            .with_guide("GUIDE TEXT")
            .with_history(
                (0..turn_count).map(|i| Turn::user(format!("t{i} {}", "x".repeat(turn_length)))),
            )
    }

    #[test]
    fn old_turns_are_dropped_first_when_the_budget_is_exceeded() {
        let counter = TokenCounter::new(1024, 256);
        let mut prompt = full_prompt(40, 200);
        let report = counter.truncate(&mut prompt);

        assert!(
            report.dropped_turns > 0,
            "old turns should have been dropped"
        );
        // The cheap thing was sacrificed first: the guide and the question were
        // never reached.
        assert!(!report.guide_dropped);
        assert!(!report.question_truncated);
        assert!(counter.validate(&prompt).is_ok());
    }

    /// THE GENERATION CAP IS DERIVED FROM THE PROMPT: with a short prompt the
    /// empty part of the window goes to generation; with a full prompt the cap
    /// drops to the minimum share.
    ///
    /// This test exists because of a measured failure: with the cap a fixed
    /// `generation_share`, an 11-tool catalog (~2300-token prompt) left ~770
    /// tokens unused and a thinking model (Qwen3-8B) hit the cap and could never
    /// call the tool. The claim is two-sided — the cap must not waste room BUT
    /// must not fall below the minimum either.
    #[test]
    fn the_generation_cap_is_derived_from_the_prompt_length() {
        let counter = TokenCounter::new(4096, 1024);

        // Short prompt: the cap must be LARGER than the minimum (empty room goes
        // to generation).
        let short = Prompt::new("instructions", "hello");
        let short_cap = counter.generation_cap(&short);
        assert!(
            short_cap > 1024,
            "the empty window was not given to generation: {short_cap}"
        );
        // And it must NOT EXCEED the window.
        assert!(short_cap + counter.prompt_estimate(&short) <= 4096);

        // A truncated, full prompt: the cap must not fall BELOW the minimum.
        let mut full = full_prompt(40, 200);
        counter.truncate(&mut full);
        let full_cap = counter.generation_cap(&full);
        assert!(
            full_cap >= 1024,
            "the minimum share was not preserved: {full_cap}"
        );
        assert!(full_cap + counter.prompt_estimate(&full) <= 4096 + 1024);
        // The full prompt's cap must be smaller than the short prompt's cap.
        assert!(full_cap < short_cap);
    }

    #[test]
    fn old_turns_drop_from_the_front_and_recent_ones_stay() {
        let counter = TokenCounter::new(1024, 256);
        let mut prompt = full_prompt(40, 200);
        counter.truncate(&mut prompt);
        let remaining: Vec<_> = prompt.history.iter().map(|t| t.text.clone()).collect();
        assert!(!remaining.is_empty());
        // The oldest (t0) is gone, the newest (t39) stayed.
        assert!(!remaining[0].starts_with("t0 "));
        assert!(remaining.last().unwrap().starts_with("t39 "));
    }

    #[test]
    fn the_guide_is_sacrificed_once_the_turns_run_out() {
        // NO history: the first stage of truncation is empty, so the turn goes
        // straight to the second stage (the guide).
        let counter = TokenCounter::new(400, 100);
        let mut prompt = Prompt::new("SYSTEM", "question")
            .with_tools(&catalog())
            .with_guide("G".repeat(700));
        assert!(counter.prompt_estimate(&prompt) > counter.prompt_cap());

        let report = counter.truncate(&mut prompt);
        assert!(report.guide_dropped);
        assert!(prompt.guide.is_none());
        // Sacrificing the guide was enough: the question was never reached.
        assert!(!report.question_truncated);
        assert!(counter.validate(&prompt).is_ok());
    }

    #[test]
    fn the_system_instructions_and_tool_description_are_never_truncated() {
        // The budget is deliberately impossibly small: the policy must still not
        // touch these two, and must return an error.
        let counter = TokenCounter::new(40, 0);
        let system = "SYSTEM INSTRUCTIONS IN FULL";
        let mut prompt = full_prompt(10, 100);
        prompt.system = system.into();
        counter.truncate(&mut prompt);

        assert_eq!(prompt.system, system);
        assert!(prompt.tools.contains("calendar_read"));
        assert!(matches!(
            counter.validate(&prompt),
            Err(EngineError::BudgetExceeded { .. })
        ));
    }

    #[test]
    fn the_last_resort_truncates_the_question_preserving_its_end() {
        let counter = TokenCounter::new(300, 0);
        let mut prompt = Prompt::new("S", format!("{} THE REAL REQUEST", "preamble ".repeat(300)));
        let report = counter.truncate(&mut prompt);

        assert!(report.question_truncated);
        // The user's real request is at the END of the sentence.
        assert!(prompt.question.ends_with("THE REAL REQUEST"));
    }

    #[test]
    fn a_fitting_prompt_passes_through_untouched() {
        let counter = TokenCounter::default();
        let mut prompt = full_prompt(2, 10);
        let before = prompt.text();
        let report = counter.truncate(&mut prompt);

        assert!(!report.changed());
        assert_eq!(prompt.text(), before);
    }

    #[test]
    fn the_estimate_does_not_undercount_turkish_text() {
        // Multi-byte letters are counted by byte; counting characters would
        // systematically UNDER-estimate Turkish.
        let ascii = "igusocIGUSOC";
        let multi_byte = "ığüşöçİĞÜŞÖÇ";
        assert!(TokenCounter::estimate(multi_byte) > TokenCounter::estimate(ascii));
        assert_eq!(TokenCounter::estimate(""), 0);
    }

    /// MEASUREMENT-BASED REGRESSION: the estimate must not fall BELOW the real
    /// token count.
    ///
    /// Measured with the Qwen2.5 tokenizer on Turkish prose: 2.71 bytes/token on
    /// average. Dividing bytes by 3 (the old form) left the estimate 10% below the
    /// truth, and truncation would say "it fits" and blow the 4096 window. This
    /// test pins that ratio: at least 1/2.71 tokens must be counted per byte.
    #[test]
    fn the_estimate_does_not_fall_below_the_measured_turkish_density() {
        let text = "A fairly long sentence written to fill the context window of an \
                    assistant running on the device.";
        // The measured worst case: 2.71 bytes/token.
        let realistic_upper_bound = (text.len() as f64 / 2.71).ceil() as usize;
        assert!(
            TokenCounter::estimate(text) >= realistic_upper_bound,
            "estimate {} < measured truth {}: truncation would blow the window",
            TokenCounter::estimate(text),
            realistic_upper_bound
        );
    }

    // --- FakeEngine and the constraint flow ---

    #[test]
    fn the_fake_engine_returns_the_script_in_order_and_records_the_prompt() {
        let engine = FakeEngine::script(["first", "second"]);
        let prompt = Prompt::new("S", "question one");
        let g = wait(engine.generate(&prompt, None, SamplingSetting::default())).unwrap();
        assert_eq!(g.text, "first");
        assert_eq!(g.stop, StopReason::Token);

        let prompt2 = Prompt::new("S", "question two");
        let g2 = wait(engine.generate(&prompt2, None, SamplingSetting::default())).unwrap();
        assert_eq!(g2.text, "second");

        assert_eq!(engine.call_count(), 2);
        assert!(engine.seen_prompts()[0].contains("question one"));
        assert!(engine.last_prompt().unwrap().contains("question two"));
    }

    #[test]
    fn an_exhausted_script_errors_unless_a_default_is_given() {
        let engine = FakeEngine::script(["only"]);
        let prompt = Prompt::new("S", "q");
        wait(engine.generate(&prompt, None, SamplingSetting::default())).unwrap();
        assert!(matches!(
            wait(engine.generate(&prompt, None, SamplingSetting::default())),
            Err(EngineError::ScriptExhausted { call: 2 })
        ));

        let engine2 = FakeEngine::script(["only"]).with_default("done");
        wait(engine2.generate(&prompt, None, SamplingSetting::default())).unwrap();
        let g = wait(engine2.generate(&prompt, None, SamplingSetting::default())).unwrap();
        assert_eq!(g.text, "done");
    }

    #[test]
    fn the_constraint_is_driven_on_every_token() {
        let engine = FakeEngine::script(["abcdefgh"]);
        let constraint = ToyConstraint { forbidden: 'z' };
        let prompt = Prompt::new("S", "q");
        let g =
            wait(engine.generate(&prompt, Some(&constraint), SamplingSetting::default())).unwrap();

        // The constraint enters the accepting state on the 5th token; generation
        // stops THERE.
        assert_eq!(g.text, "abcde");
        assert_eq!(g.stop, StopReason::ConstraintDone);
        assert_eq!(engine.constraint_steps(), 5);
        assert_eq!(engine.constraint_names(), vec!["toy"]);
    }

    #[test]
    fn generation_does_not_pass_silently_when_the_mask_is_pierced() {
        // The script tries to produce a forbidden token: if masking really is
        // applied this must be an error, not a silent acceptance.
        let engine = FakeEngine::script(["zebra"]);
        let constraint = ToyConstraint { forbidden: 'z' };
        let prompt = Prompt::new("S", "q");
        assert!(matches!(
            wait(engine.generate(&prompt, Some(&constraint), SamplingSetting::default())),
            Err(EngineError::ConstraintViolation(_))
        ));
    }

    #[test]
    fn the_free_constraint_blocks_nothing() {
        let engine = FakeEngine::script(["zzz"]);
        let prompt = Prompt::new("S", "q");
        let g = wait(engine.generate(&prompt, Some(&FreeConstraint), SamplingSetting::default()))
            .unwrap();
        assert_eq!(g.text, "zzz");
        assert_eq!(engine.constraint_steps(), 3);
    }

    #[test]
    fn output_is_marked_half_finished_when_the_token_cap_is_hit() {
        let engine = FakeEngine::script(["a long output"]);
        let setting = SamplingSetting {
            max_tokens: 4,
            ..Default::default()
        };
        let g = wait(engine.generate(&Prompt::new("S", "q"), None, setting)).unwrap();

        assert_eq!(g.text, "a lo");
        assert_eq!(g.stop, StopReason::Length);
        // So the call site does not try to parse half a JSON document.
        assert!(!g.stop.is_complete());
    }

    #[test]
    fn the_engine_can_be_carried_as_dyn() {
        // `EngineProvider` being dyn-compatible is a condition of the contract:
        // the engine is chosen at runtime (fake or candle).
        let engine: Arc<dyn EngineProvider> = Arc::new(FakeEngine::script(["ok"]));
        assert_eq!(engine.name(), "fake");
        let g = wait(engine.generate(&Prompt::new("S", "q"), None, SamplingSetting::default()))
            .unwrap();
        assert_eq!(g.text, "ok");
    }

    #[test]
    fn a_truncated_prompt_fits_the_engine() {
        // End to end: truncate -> validate -> generate.
        let counter = TokenCounter::new(1024, 256);
        let mut prompt = full_prompt(60, 150);
        counter.truncate(&mut prompt);
        counter.validate(&prompt).unwrap();

        let engine = FakeEngine::script(["answer"]);
        wait(engine.generate(&prompt, None, SamplingSetting::default())).unwrap();
        let seen = engine.last_prompt().unwrap();
        assert!(TokenCounter::estimate(&seen) <= counter.prompt_cap());
        assert!(seen.contains("SYSTEM INSTRUCTIONS"));
        assert!(seen.contains("the real question here"));
    }
}

/// Architecture resolution — under the `candle` feature.
///
/// NEEDS NO WEIGHTS: what is measured here is the decision from a metadata string
/// to the correct module choice. A real GGUF load reads 2.5 GB and cannot run in
/// CI; yet the risk of "silently loading the wrong module for an unknown
/// architecture" lives in exactly this pure function.
#[cfg(all(test, feature = "candle"))]
mod architecture_tests {
    use crate::candle_engine::Architecture;
    use crate::prompt::Template;

    #[test]
    fn known_architectures_go_to_the_right_module_and_template() {
        assert_eq!(Architecture::resolve("qwen2").unwrap(), Architecture::Qwen2);
        assert_eq!(Architecture::resolve("qwen3").unwrap(), Architecture::Qwen3);
        assert_eq!(
            Architecture::resolve("gemma3").unwrap(),
            Architecture::Gemma3
        );

        // The template is TIED to the architecture: the Qwen family uses ChatML,
        // Gemma its own format. A mismatch is silent breakage (the model does not
        // recognise the fences).
        assert_eq!(Architecture::Qwen2.template(), Template::ChatML);
        assert_eq!(Architecture::Qwen3.template(), Template::ChatML);
        assert_eq!(Architecture::Gemma3.template(), Template::Gemma);

        // `llama` IS THE ONE ENTRY WHOSE TEMPLATE THIS TABLE CANNOT SETTLE. It
        // names a family, and the family disagrees: SmolLM2 and TinyLlama speak
        // ChatML, Llama-3 does not. ChatML is what this table answers, and the
        // loader refuses a llama GGUF whose tokenizer has no `<|im_start|>`
        // rather than trusting the answer.
        assert_eq!(Architecture::resolve("llama").unwrap(), Architecture::Llama);
        assert_eq!(Architecture::Llama.template(), Template::ChatML);
    }

    /// THE MOST IMPORTANT TEST. Falling back to the "nearest" module for an
    /// unknown architecture means the model silently produces garbage; the error
    /// message must list what is supported so the user knows what to do.
    #[test]
    fn an_unknown_architecture_errors_instead_of_falling_back() {
        for name in ["gemma2", "phi3", "qwen", "llama2", "", "QWEN3"] {
            let result = Architecture::resolve(name);
            assert!(result.is_err(), "'{name}' was accepted silently");
            let message = result.unwrap_err().to_string();
            assert!(
                message.contains("unsupported GGUF architecture"),
                "{message}"
            );
            assert!(message.contains("qwen2, qwen3, gemma3, llama"), "{message}");
        }
    }
}
