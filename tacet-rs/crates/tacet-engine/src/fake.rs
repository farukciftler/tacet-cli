//! FakeEngine — a deterministic, script-driven engine that NEEDS NO WEIGHTS.
//!
//! WHY IT IS THE DEFAULT: the turn loop, tool routing, budget truncation and the
//! constraint flow are logic, not a model. If testing that logic depended on a
//! GGUF file it would never run in CI, and therefore in practice never be tested
//! at all. All eval and CI run on this engine; even with the `candle` feature
//! off, Tacet's logic layer can be verified end to end.
//!
//! IT RECORDS THE PROMPTS IT SEES: so a test can directly ask "was the guide
//! inside the prompt", "was the old turn dropped". Verifying the output is not
//! enough — HOW the prompt is BUILT is also this crate's promise.

use crate::error::EngineError;
use crate::prompt::Prompt;
use crate::constraint::Constrainer;
use crate::provider::{
    EngineProvider, Generation, GenerationFuture, SamplingSetting, StopReason, boxed_generation,
};
use std::sync::Mutex;

/// One step in the script.
#[derive(Debug, Clone)]
pub enum FakeStep {
    /// Generate this text.
    Generate(String),
    /// Return this error — for testing the recovery paths.
    Fail(String),
}

impl<S: Into<String>> From<S> for FakeStep {
    fn from(v: S) -> Self {
        FakeStep::Generate(v.into())
    }
}

#[derive(Default)]
struct Inner {
    steps: Vec<FakeStep>,
    next: usize,
    seen_prompts: Vec<String>,
    /// How many times the constraint session was advanced — the only proof for
    /// "was the constraint really called".
    constraint_steps: usize,
    constraint_names: Vec<String>,
}

pub struct FakeEngine {
    inner: Mutex<Inner>,
    /// When the script runs out, return this text instead of an error
    /// (None = error).
    default_text: Option<String>,
}

impl FakeEngine {
    /// Builds it with the outputs to be returned in order.
    pub fn script<S: Into<FakeStep>>(steps: impl IntoIterator<Item = S>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                steps: steps.into_iter().map(Into::into).collect(),
                ..Default::default()
            }),
            default_text: None,
        }
    }

    /// The variant that returns fixed text instead of an error when the script
    /// runs out. For tests that do not know the turn COUNT up front (loop
    /// termination tests).
    pub fn with_default(mut self, text: impl Into<String>) -> Self {
        self.default_text = Some(text.into());
        self
    }

    /// The FULL texts of the prompts sent to the engine, in order.
    pub fn seen_prompts(&self) -> Vec<String> {
        self.inner.lock().expect("fake engine lock").seen_prompts.clone()
    }

    /// The last prompt seen — what tests ask for most often.
    pub fn last_prompt(&self) -> Option<String> {
        self.inner.lock().expect("fake engine lock").seen_prompts.last().cloned()
    }

    pub fn call_count(&self) -> usize {
        self.inner.lock().expect("fake engine lock").seen_prompts.len()
    }

    /// How many tokens the constraint session was advanced by.
    pub fn constraint_steps(&self) -> usize {
        self.inner.lock().expect("fake engine lock").constraint_steps
    }

    /// The names of the constraints used, in call order.
    pub fn constraint_names(&self) -> Vec<String> {
        self.inner.lock().expect("fake engine lock").constraint_names.clone()
    }
}

/// The size of the fake vocabulary. There is no real tokenizer; a code point is
/// taken to BE a token id (see the reasoning below), so the vocabulary is a slice
/// of the Unicode code point space large enough for the tests.
const FAKE_VOCAB: usize = 0x1000;

impl EngineProvider for FakeEngine {
    fn name(&self) -> &str {
        "fake"
    }

    /// Code point = token id (the same assumption as in the generation path), so
    /// the vocabulary is the texts of the first `FAKE_VOCAB` code points.
    /// Declaring it runs the REAL constraint on the fake engine too: in eval,
    /// where the grammar is tied to the generation loop, what gets measured is
    /// genuinely masked generation, not a fake impression that "a constraint was
    /// there".
    fn vocab(&self) -> Option<Vec<String>> {
        Some(
            (0..FAKE_VOCAB)
                .map(|i| char::from_u32(i as u32).map(String::from).unwrap_or_default())
                .collect(),
        )
    }

    fn generate<'a>(
        &'a self,
        prompt: &'a Prompt,
        constraint: Option<&'a dyn Constrainer>,
        setting: SamplingSetting,
    ) -> GenerationFuture<'a> {
        boxed_generation(async move {
            let text = {
                let mut inner = self.inner.lock().expect("fake engine lock");
                inner.seen_prompts.push(prompt.text());
                let index = inner.next;
                inner.next += 1;
                match inner.steps.get(index).cloned() {
                    Some(FakeStep::Generate(m)) => m,
                    Some(FakeStep::Fail(m)) => return Err(EngineError::Inference(m)),
                    None => match &self.default_text {
                        Some(m) => m.clone(),
                        None => return Err(EngineError::ScriptExhausted { call: index + 1 }),
                    },
                }
            };

            // THE CONSTRAINT IS REALLY DRIVEN. Simply returning the scripted text
            // would make the test a liar: the constraint path would look
            // "passing" without ever running. Since there is no real tokenizer, a
            // CODE POINT is taken as a token — the absolute ids do not matter,
            // what matters is the flow of the mask/advance pair and the accepting
            // state.
            let mut produced = 0usize;
            let mut output = String::new();
            let mut stop = StopReason::Token;

            if let Some(c) = constraint {
                self.inner
                    .lock()
                    .expect("fake engine lock")
                    .constraint_names
                    .push(c.name().to_string());
                let mut session = c.session();
                let mut logits = vec![0.0f32; FAKE_VOCAB];

                for ch in text.chars() {
                    if produced >= setting.max_tokens {
                        stop = StopReason::Length;
                        break;
                    }
                    logits.fill(0.0);
                    session.mask(&mut logits);
                    let token = ch as u32;
                    // Is the mask really applied: a script trying to produce a
                    // forbidden token MUST NOT pass SILENTLY.
                    if (token as usize) < logits.len()
                        && logits[token as usize] == f32::NEG_INFINITY
                    {
                        return Err(EngineError::ConstraintViolation(token));
                    }
                    session.advance(token)?;
                    self.inner.lock().expect("fake engine lock").constraint_steps += 1;
                    output.push(ch);
                    produced += 1;
                    if session.is_done() {
                        stop = StopReason::ConstraintDone;
                        break;
                    }
                }
            } else {
                for ch in text.chars() {
                    if produced >= setting.max_tokens {
                        stop = StopReason::Length;
                        break;
                    }
                    output.push(ch);
                    produced += 1;
                }
            }

            Ok(Generation::new(output, produced, stop))
        })
    }
}
