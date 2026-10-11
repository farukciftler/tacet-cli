//! The eval must feed back what the shell feeds back.
//!
//! WHY. The shell pushes the model's generation into the history as an
//! `assistant` turn BEFORE the tool result, and its comment records the defect
//! that put it there: fed only the RESULT, the model saw a context-free line
//! with the user's question below it, took the question for unanswered, and
//! called the same tool again — up to the turn limit, without ever answering.
//! That is the `OutOfTurns` ending this suite reports and scores against the
//! model.
//!
//! The eval pushed only the result. So it measured a model with no record of
//! its own actions, counted the repeats against it, and published the number as
//! the shell's. It is also a deviation from the template the model was trained
//! on, in which a tool response follows the assistant turn that asked for it.
//!
//! This is the fourth eval/shell divergence found in one night, and they all
//! ran the same way: the eval was stricter than the program, in a direction that
//! looks like the model failing.

use std::sync::Arc;

use tacet_engine::fake::FakeEngine;
use tacet_engine::{
    Constrainer, EngineIdentity, EngineProvider, GenerationFuture, Prompt, SamplingSetting,
};
use tacet_eval::tool_selection::{SelectionCase, SelectionStep, run_selection_case};

/// `FakeEngine` with the window a real Qwen3-4B gets on this laptop.
///
/// WHY THESE TESTS NEED A WINDOW AT ALL. `FakeEngine` declares none, so the
/// eval gives it the 4096 floor, and on the floor the system block, the router's
/// nine tool descriptions and the skill guide are already over the 3072-token
/// cap before any history: the guide is sacrificed on the first pass, and on
/// the second the question, the call and its result are all dropped. These
/// tests passed on the floor only because the eval used to attach the tools
/// AFTER truncating, so its budget never saw them (audit M22) — the shell, on
/// the same floor, sends the empty conversation. What this file pins is the
/// ORDER of the feedback, which needs a window that holds it.
///
/// The window comes from where a real one does: the model declares a context
/// length and its GGUF header gives the KV bytes per token, here Qwen3-4B's
/// geometry (36 layers, 8 KV heads, 128 + 128) — 16 954 affordable on the
/// CPU/Metal budget, 16 384 declared.
struct Windowed {
    fake: Arc<FakeEngine>,
    gguf: std::path::PathBuf,
}

impl Windowed {
    fn new(fake: Arc<FakeEngine>) -> Self {
        let gguf = std::env::temp_dir().join(format!(
            "tacet-own-call-{}-{:?}.gguf",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&gguf, qwen3_4b_header()).expect("a temporary GGUF header");
        Self { fake, gguf }
    }
}

impl Drop for Windowed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.gguf);
    }
}

/// A GGUF v3 header with no tensors and only the four keys the KV-cost reader
/// needs. Not a model: nothing here loads weights.
fn qwen3_4b_header() -> Vec<u8> {
    const T_UINT32: u32 = 4;
    let keys: [(&str, u32); 4] = [
        ("qwen3.block_count", 36),
        ("qwen3.attention.head_count_kv", 8),
        ("qwen3.attention.key_length", 128),
        ("qwen3.attention.value_length", 128),
    ];
    let mut out = Vec::new();
    out.extend_from_slice(b"GGUF");
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&(keys.len() as u64).to_le_bytes());
    for (key, value) in keys {
        out.extend_from_slice(&(key.len() as u64).to_le_bytes());
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&T_UINT32.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

impl EngineProvider for Windowed {
    fn name(&self) -> &str {
        "fake-windowed"
    }
    fn identity(&self) -> EngineIdentity {
        EngineIdentity {
            engine: self.name().to_string(),
            model_path: self.gguf.to_string_lossy().into_owned(),
            ..Default::default()
        }
    }
    fn context_length(&self) -> Option<usize> {
        Some(16_384)
    }
    fn generate<'a>(
        &'a self,
        prompt: &'a Prompt,
        constraint: Option<&'a dyn Constrainer>,
        setting: SamplingSetting,
    ) -> GenerationFuture<'a> {
        self.fake.generate(prompt, constraint, setting)
    }
}

/// The fixture is only meaningful if the window really is the large one:
/// on the floor these tests would be measuring truncation, not feedback.
#[test]
fn the_fixture_window_is_a_real_models_not_the_floor() {
    let w = Windowed::new(Arc::new(FakeEngine::script(Vec::<String>::new())));
    let per_token = tacet_engine::gguf_kv_bytes_per_token(&w.gguf).expect("the header reads");
    assert_eq!(per_token, 36 * 8 * (128 + 128) * 2);
    assert_eq!(
        tacet_engine::context_budget(
            w.context_length(),
            Some(per_token),
            tacet_engine::Device::Cpu
        ),
        16_384
    );
}

#[test]
fn the_generation_that_made_the_call_is_in_the_next_prompt() {
    let engine = Arc::new(FakeEngine::script([
        r#"calculate({"expression":"125*8"})"#,
        "125 times 8 is 1000.",
    ]));
    let provider: Arc<dyn EngineProvider> = Arc::new(Windowed::new(engine.clone()));
    let case = SelectionCase {
        name: "probe".into(),
        category: tacet_eval::Category::Tool,
        steps: vec![SelectionStep::new(
            "What is 125 times 8?",
            Some("calculate"),
        )],
    };
    let outcome = run_selection_case(&case, &provider);
    assert!(outcome.passed, "the fixture must pass");

    let prompts = engine.seen_prompts();
    assert!(prompts.len() >= 2, "the loop must have made a second pass");
    let second = &prompts[1];
    assert!(
        second.contains(r#"calculate({"expression":"125*8"})"#),
        "the model's own call is missing from the prompt it gets after the tool \
         ran, so it has no record of having called anything:\n{second}"
    );
    assert!(
        second.contains("1000"),
        "the tool result must still be there too:\n{second}"
    );
}

/// The call comes BEFORE the result. The other order is not the template the
/// model was trained on, and it reads as a result that arrived first.
#[test]
fn the_call_comes_before_its_result() {
    let engine = Arc::new(FakeEngine::script([
        r#"calculate({"expression":"125*8"})"#,
        "125 times 8 is 1000.",
    ]));
    let provider: Arc<dyn EngineProvider> = Arc::new(Windowed::new(engine.clone()));
    let case = SelectionCase {
        name: "probe".into(),
        category: tacet_eval::Category::Tool,
        steps: vec![SelectionStep::new(
            "What is 125 times 8?",
            Some("calculate"),
        )],
    };
    let _ = run_selection_case(&case, &provider);
    let second = &engine.seen_prompts()[1];
    let call_at = second
        .find(r#"calculate({"expression":"125*8"})"#)
        .expect("the call");
    let result_at = second.rfind("1000").expect("the result");
    assert!(
        call_at < result_at,
        "the result appears before the call that asked for it:\n{second}"
    );
}
