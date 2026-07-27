//! EngineProvider — the contract of the model runner.
//!
//! ASYNC SHAPE: the decision taken in `Tool` holds here too — a hand-written
//! `Pin<Box<dyn Future + Send + '_>>`. Same reason: the engine is chosen at
//! runtime (FakeEngine or CandleEngine) and carried as
//! `Arc<dyn EngineProvider>`; a trait with AFIT is not dyn-compatible, so that
//! would be impossible. `async-trait` was still NOT added; `boxed_generation`
//! does the same conversion in one line with no dependency.

use crate::constraint::Constrainer;
use crate::error::EngineResult;
use crate::prompt::{Prompt, Template};
use std::future::Future;
use std::pin::Pin;

/// How generation ended. The call site behaves accordingly: a tool call ending
/// with `Length` is HALF a JSON document and must be thrown away unparsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The model emitted a stop token — the natural end.
    Token,
    /// The constrainer reached an accepting state; the grammar is complete.
    ConstraintDone,
    /// The token cap was hit. The output may be HALF-FINISHED.
    Length,
    /// A stop string appeared.
    StopString,
    /// The user CANCELLED generation (Ctrl-C in the shell).
    ///
    /// A separate variant from `Length`: both leave half-finished output but
    /// their causes are opposite — one is "the cap ran out", the other is "the
    /// user did not want it". Treated as the same, the shell would print a
    /// "generation was cut short" warning after a cancel; that would present
    /// something the user asked for as a malfunction.
    Cancelled,
    /// Generation started repeating the same sequence and WAS CUT.
    ///
    /// A half-finished but readable answer beats a wall repeating the same
    /// sentence dozens of times. A separate variant, because it says something
    /// different from `Length`: not "the cap ran out" but "generation GOT STUCK".
    Loop,
}

impl StopReason {
    /// Is the output complete — every ending except `Length` and `Cancelled` is
    /// complete.
    pub fn is_complete(self) -> bool {
        !matches!(self, StopReason::Length | StopReason::Cancelled)
    }

    /// Did the user cut it — this is how the shell separates a warning from
    /// silence.
    pub fn is_cancelled(self) -> bool {
        matches!(self, StopReason::Cancelled)
    }
}

/// The result of one generation.
#[derive(Debug, Clone)]
pub struct Generation {
    /// The text to show the user. The thinking block IS NOT HERE.
    pub text: String,
    /// The model's inner reasoning, if any. Not printed to screen; kept for
    /// diagnostics.
    pub thinking: Option<String>,
    pub token_count: usize,
    pub stop: StopReason,
}

impl Generation {
    pub fn new(text: impl Into<String>, token_count: usize, stop: StopReason) -> Self {
        // The split happens IN THE CONSTRUCTOR so no call site can skip this
        // step. Left to each engine to remember, the thinking would silently leak
        // to the screen the moment a new engine was added.
        let (thinking, visible) = crate::thinking::extract(&text.into());
        Self {
            text: visible,
            thinking,
            token_count,
            stop,
        }
    }
}

/// Sampling settings. The default is GREEDY (temperature 0): eval being
/// reproducible comes before the sampling decision; randomness is turned on
/// deliberately.
#[derive(Debug, Clone, Copy)]
pub struct SamplingSetting {
    pub temperature: f32,
    /// Top-p (nucleus sampling). 1.0 = off.
    pub top_p: f32,
    /// The most tokens to generate.
    pub max_tokens: usize,
    /// The randomness seed — same seed + same prompt = same output.
    pub seed: u64,
    /// The cost of re-emitting recent tokens. 1.0 = off.
    ///
    /// Greedy sampling (temperature 0) is PRONE to repetition loops: at every
    /// step the most likely token is chosen, and if the model latches onto a
    /// sentence it writes that sentence forever. That was the bug seen in the
    /// field. The penalty is deterministic, so it DOES NOT BREAK eval
    /// reproducibility — that is what sets it apart from raising the temperature
    /// for variety.
    pub repeat_penalty: f32,
    /// User cancellation. Once `true`, generation stops at the NEXT token.
    ///
    /// WHY `&'static AtomicBool` AND WHY HERE:
    ///
    /// * Adding a cancel parameter to the engine interface would change every
    ///   call site; the settings struct is already the answer to "how should this
    ///   generation run", and cancellation is part of that question.
    /// * NOT `Arc`, because `SamplingSetting` is `Copy`; an `Arc` would take away
    ///   the type's copyability and break thirty-odd call sites. The `'static`
    ///   lifetime is not a constraint but the fact itself: the flag is set up by
    ///   the shell and lives for the whole process (leaked once).
    /// * `None` = generation that cannot be cancelled. Eval and tests want this:
    ///   a measurement that can be cancelled blurs the measurement itself.
    pub cancel: Option<&'static std::sync::atomic::AtomicBool>,
}

impl Default for SamplingSetting {
    fn default() -> Self {
        // 1.15: enough to break a loop, gentle enough not to damage natural
        // repetition (inflections, conjunctions, proper nouns). Higher values
        // "diversify" the text artificially and damage the meaning.
        Self {
            temperature: 0.0,
            top_p: 1.0,
            // PAIRED with `token::GENERATION_SHARE` (see the 512 -> 1024
            // measurement there): truncation fits the prompt into the window
            // outside that share; if the cap were larger than the share,
            // generation could overflow the window.
            max_tokens: crate::token::GENERATION_SHARE,
            seed: 0,
            repeat_penalty: 1.15,
            cancel: None,
        }
    }
}

pub type GenerationFuture<'a> = Pin<Box<dyn Future<Output = EngineResult<Generation>> + Send + 'a>>;

/// Turns an `async` block into a `GenerationFuture` (see `boxed` in core).
pub fn boxed_generation<'a, F>(future: F) -> GenerationFuture<'a>
where
    F: Future<Output = EngineResult<Generation>> + Send + 'a,
{
    Box::pin(future)
}

pub trait EngineProvider: Send + Sync {
    /// Diagnostics/log name ("fake", "candle").
    fn name(&self) -> &str;

    /// Which wire format this engine expects the prompt in.
    ///
    /// WHY ON THE ENGINE: the template is the format the model was TRAINED on.
    /// If it were selectable in the CLI, a wrong choice would silently give
    /// broken output — build green, answer nonsense. Once the engine declares its
    /// own format, a mismatch becomes impossible. Default `Plain`: for an engine
    /// that does not declare its format, readable tagged text is the most
    /// harmless choice.
    fn template(&self) -> Template {
        Template::Plain
    }

    /// Token id -> text table; REQUIRED in order to set up a constraint.
    ///
    /// WHY ON THE ENGINE: the mask speaks in token ids, and only the tokenizer
    /// knows those ids. If the side setting up the constraint (CLI, eval) made up
    /// its own vocabulary, the mask would mask another model's ids — i.e.
    /// silently close the WRONG tokens.
    ///
    /// `None` = this engine has no known vocabulary; the call site then generates
    /// unconstrained. Default `None`: an engine that does not declare its
    /// vocabulary should not use the constraint at all rather than break it
    /// silently.
    fn vocab(&self) -> Option<Vec<String>> {
        None
    }

    /// Turns the prompt into a generation.
    ///
    /// If `constraint` is given it is applied to the logits at EVERY step;
    /// sampling runs AFTER it, so no sampling strategy can pierce the constraint.
    fn generate<'a>(
        &'a self,
        prompt: &'a Prompt,
        constraint: Option<&'a dyn Constrainer>,
        setting: SamplingSetting,
    ) -> GenerationFuture<'a>;

    /// Drives generation in STREAMING form: every new text fragment is handed to
    /// `listener`. The returned `Generation` still carries the FULL text (sum of
    /// the fragments = `text`), so the call site can either show the stream or
    /// parse the finished output — both paths draw on the same source of truth.
    ///
    /// WHY A SEPARATE METHOD AND NOT A FLAG ON `generate`: streaming is only the
    /// UI's (the shell's) need; eval and the grammar command want the full text
    /// and must not be forced to invent a listener. The default implementation
    /// falls back to `generate` and emits the whole text in ONE fragment — an
    /// engine that does not declare streaming (FakeEngine) still works, it just
    /// does not "stream". The sole purpose of this method is that on a 3B model
    /// the user is not staring at a frozen screen for the seconds before the
    /// first token.
    fn generate_streaming<'a>(
        &'a self,
        prompt: &'a Prompt,
        constraint: Option<&'a dyn Constrainer>,
        setting: SamplingSetting,
        listener: &'a (dyn Fn(&str) + Send + Sync),
    ) -> GenerationFuture<'a> {
        boxed_generation(async move {
            let g = crate::wait(self.generate(prompt, constraint, setting));
            if let Ok(generation) = &g
                && !generation.text.is_empty()
            {
                listener(&generation.text);
            }
            g
        })
    }
}
