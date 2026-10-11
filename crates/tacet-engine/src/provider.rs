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
    /// The CALLER'S token cap was hit — `SamplingSetting::max_tokens` ran out.
    /// The output may be HALF-FINISHED.
    ///
    /// SEPARATE FROM `CallTooLong` since the eval started reporting why a step
    /// could not be measured. Both leave half-finished output and both used to
    /// be this variant, so "cut off" named two opposite defects with one word:
    /// this one says the budget the caller granted was not enough for what the
    /// model wanted to say, which is a windowing decision; the other says the
    /// model got stuck INSIDE a call, which is a grammar or a prompt problem.
    /// A report that cannot tell them apart cannot point at either.
    Length,
    /// A tool call armed the grammar and then ran past the call's own budget
    /// (`TOOL_CALL_CAP`), counted from where the call started rather than from
    /// the start of generation.
    ///
    /// The largest legitimate call measured in this repository is 1523 tokens,
    /// so this is not a near miss on a normal call — it is a runaway inside one.
    CallTooLong,
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
    /// Is the output complete — every ending except the two caps and a cancel
    /// is complete.
    pub fn is_complete(self) -> bool {
        !matches!(
            self,
            StopReason::Length | StopReason::CallTooLong | StopReason::Cancelled
        )
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
    /// What the model produced, before the thinking split. Kept because the
    /// split made here cannot see the catalog, and one call shape — the glued
    /// `write_filepath: "a.md"` — is a call only by a catalog name; see
    /// `split_against`.
    pub raw: String,
    /// Whether the executor may run a call it finds in `text`. False when the
    /// split kept a thinking block on a GUESS — a call start inside it that does
    /// not parse — and the two readings of the text name different calls: the
    /// one the loops would run may be one the model wrote while reasoning and
    /// turned down (audit H5; see `thinking::split`). The executing loops treat
    /// such a turn as an answer. Set by the constructor without the catalog
    /// (any guess blocks) and refined by `split_against`.
    pub executable: bool,
    pub token_count: usize,
    pub stop: StopReason,
}

impl Generation {
    pub fn new(text: impl Into<String>, token_count: usize, stop: StopReason) -> Self {
        // The split happens IN THE CONSTRUCTOR so no call site can skip this
        // step. Left to each engine to remember, the thinking would silently leak
        // to the screen the moment a new engine was added.
        //
        // THE PRICE OF "EVERY OUTPUT" IS THAT CALLS PASS THROUGH HERE TOO, and
        // `extract` used to edit them (audit H5): the text the executor parses
        // is `self.text`, so whatever this line changes inside a call's
        // arguments is what runs. `extract` now leaves everything from the
        // first call start onward byte-for-byte alone; the tests below hold
        // that on the constructor itself, since it is the only door.
        let raw = text.into();
        let split = crate::thinking::split(&raw, &[]);
        Self {
            text: split.visible,
            thinking: split.thinking,
            executable: split.other_reading.is_none(),
            raw,
            token_count,
            stop,
        }
    }

    /// Re-splits the thinking with the catalog's tool names, so a glued call
    /// is protected from the split the way `name(` is (audit H5).
    ///
    /// EVERY CALLER THAT HANDS `text` TO THE EXECUTOR CALLS THIS FIRST. The
    /// constructor cannot: engines do not hold the catalog, and a constrained
    /// engine is not the only producer — the last pass runs unconstrained and
    /// the executor's glued recovery still reads it. Without the names the
    /// review measured `write_filepath: "a.md", content: "Qwen emits <think>
    /// first"` reaching the executor as `Qwen emits  first`, and a `</think>`
    /// inside one glued argument leaving `delete_filepath: "victim.md"` at a
    /// line start as a second call. `the_executing_loops_split_against_the_catalog`
    /// in `tacet-cli` holds the callers to it, and to `executable`.
    ///
    /// `parse` is the executor's own parser, and it decides `executable`: when
    /// the split had to guess, the call is runnable only if `parse` finds the
    /// SAME call in both readings, so a real call after a block with a stray
    /// `{"` in it still runs, and a call that exists only while the block is
    /// kept does not. The parser comes in as a closure because this crate does
    /// not depend on `tacet-tools`, and the comparison has to be the
    /// executor's: a parser of our own would agree with itself on shapes the
    /// executor reads differently, such as the trailing comma it repairs.
    pub fn split_against<T: PartialEq>(
        &mut self,
        tool_names: &[&str],
        parse: impl Fn(&str) -> Option<T>,
    ) {
        let split = crate::thinking::split(&self.raw, tool_names);
        self.executable = split
            .other_reading
            .as_deref()
            .is_none_or(|other| parse(other) == parse(&split.visible));
        self.text = split.visible;
        self.thinking = split.thinking;
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

/// WHO produced a number.
///
/// WHY IT EXISTS: `name()` returns the literal string "candle" for every model,
/// every quantization and every device, and that string was the only identity a
/// report carried. Four different GGUFs therefore produced four
/// indistinguishable reports — and since `TACET_MODEL` is consulted BEFORE the
/// `--model` flag, a leftover environment variable could make an entire
/// comparison matrix measure one model without anything on screen saying so.
/// A measurement whose subject is unrecorded is not a measurement.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct EngineIdentity {
    /// "candle", "fake".
    pub engine: String,
    /// Empty for engines with no weights.
    pub model_path: String,
    /// A CHEAP FINGERPRINT, NOT A FULL DIGEST: sha256 over the file's size, its
    /// first mebibyte and its last mebibyte. It answers "is this the same file
    /// as the last cell of the matrix" in milliseconds; it is not a guarantee
    /// against a crafted file, and the name says so rather than implying more.
    /// A full digest over 2.3 GB on every eval load would cost more than the
    /// eval.
    pub model_fingerprint: String,
    pub model_bytes: u64,
    /// The dominant tensor type in the GGUF ("Q4K", "Q8_0", …) — the thing a
    /// quantization sweep varies, read from the file rather than from its name.
    pub quant: String,
    /// "qwen3", "gemma3", …
    pub architecture: String,
    /// "metal", "cpu".
    pub device: String,
}

/// The architectures this project has ACTUALLY MEASURED ITSELF ON.
///
/// Not the ones it can load — `Architecture::resolve` accepts four, and loading
/// is the easy half. Every number this repository publishes (the selection
/// score, the baselines under `crates/tacet-eval/baselines/`, the timings in the
/// README) was produced on a Qwen model. Nothing here has ever been run against
/// gemma3 or a llama-family GGUF as a measured subject.
///
/// WHY THE DISTINCTION IS WORTH A CONSTANT: on 7 Sep 2026 the same binary, the
/// same six messages and the same minute gave 0 of 6 unreadable answers on
/// qwen3-4b and 4 of 6 on gemma3-4b — walls of `</tool.`, `Do not be.` repeated
/// eight times, a digit stuck in a loop where the date should have been. The
/// person who found that had `model = gemma3-4b` in their config and no reason
/// to suspect it, because nothing in the program had ever mentioned that the
/// number on the page and the model in the config were different things.
///
/// Loading an unmeasured architecture stays ALLOWED — somebody holding those
/// weights is entitled to try them. It just stops being silent.
pub const MEASURED_ARCHITECTURES: [&str; 2] = ["qwen2", "qwen3"];

impl EngineIdentity {
    /// The architecture in use, when it is one nothing here has been measured on.
    ///
    /// `None` also for an engine with no weights: the fake engine makes no claim
    /// about a model, so there is nothing to warn about.
    pub fn unmeasured_architecture(&self) -> Option<&str> {
        if self.architecture.is_empty() {
            return None;
        }
        (!MEASURED_ARCHITECTURES.contains(&self.architecture.as_str()))
            .then_some(self.architecture.as_str())
    }

    /// One line for a human: what ran.
    pub fn line(&self) -> String {
        if self.model_path.is_empty() {
            return self.engine.clone();
        }
        format!(
            "{} · {} · {} · {} · {}",
            self.engine, self.model_path, self.quant, self.device, self.model_fingerprint
        )
    }
}

pub trait EngineProvider: Send + Sync {
    /// Diagnostics/log name ("fake", "candle").
    fn name(&self) -> &str;

    /// WHAT produced the numbers — model file, quantization, device.
    ///
    /// Defaulted so an engine without weights (the fake one, a test double) is
    /// not forced to invent fields; it still reports its own name, which is the
    /// truthful answer for it.
    fn identity(&self) -> EngineIdentity {
        EngineIdentity {
            engine: self.name().to_string(),
            ..Default::default()
        }
    }

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

    /// The context window THE MODEL DECLARES, in tokens.
    ///
    /// WHY ON THE ENGINE: only the thing that opened the weight file knows this
    /// number; it is in the GGUF metadata (`<arch>.context_length`). The call site
    /// feeds it to `token::context_budget`, which clamps it into the range we have
    /// actually measured.
    ///
    /// `None` = "this engine does not declare a window", and the caller MUST then
    /// keep the default 4096 rather than assume something larger. Default `None`
    /// for the same reason as `vocab`: an engine that says nothing must not have a
    /// window invented for it — feeding a model positions past its rope table
    /// produces plausible-looking nonsense, not an error.
    fn context_length(&self) -> Option<usize> {
        None
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for the executor's parser that tells every text apart: two
    /// readings agree only when they are the same text.
    fn whole_text(text: &str) -> Option<String> {
        Some(text.to_string())
    }

    /// A stand-in that sees one call, from the last `web_search(` to the end —
    /// the executor's own `ToolCall::parse` reads right to left the same way.
    fn last_search(text: &str) -> Option<String> {
        text.rfind("web_search(").map(|at| text[at..].to_string())
    }

    /// The review's case at the type the loops read: a block kept on a guess
    /// holds a declined call, and `executable` must say so — from the
    /// constructor (no catalog, so any guess blocks) and from `split_against`
    /// (the readings disagree). And the gate must not cost a real call: after
    /// a block with a stray `{"` in it, both readings end in the same call.
    #[test]
    fn a_declined_call_in_a_guessed_block_is_not_executable() {
        let declined =
            r#"<think>maybe web_search({"query":"leak secret",}) no</think>No search needed."#;
        let mut g = Generation::new(declined, 9, StopReason::Token);
        assert!(
            g.text.contains("web_search"),
            "the block is kept: {}",
            g.text
        );
        assert!(!g.executable, "the constructor let a guess through");
        g.split_against(&["web_search"], last_search);
        assert!(!g.executable, "the readings disagree, yet the call may run");

        let real = r#"<think>shape is {"k": v}</think>web_search({"query":"weather"})"#;
        let mut g = Generation::new(real, 9, StopReason::Token);
        g.split_against(&["web_search"], last_search);
        assert!(g.executable, "the same call in both readings was blocked");
        assert_eq!(
            last_search(&g.text).as_deref(),
            Some(r#"web_search({"query":"weather"})"#)
        );

        let plain = Generation::new(
            r#"<think>plan</think>web_search({"query":"w"})"#,
            9,
            StopReason::Token,
        );
        assert!(plain.executable);
    }

    /// Audit H5, at the door every engine goes through (`CandleEngine` and
    /// `FakeEngine` both build their result with `Generation::new`). The two
    /// strings are the audit's own; before the fix the first came back with
    /// `Qwen emits  first` and the second as `write_file({"path":"X"})`.
    #[test]
    fn the_constructor_hands_the_executor_the_call_the_model_wrote() {
        for call in [
            r#"write_file({"path":"a.md","content":"Qwen emits <think> first"})"#,
            r#"write_file({"path":"<think>","content":"</think>X"})"#,
        ] {
            let g = Generation::new(call, 12, StopReason::ConstraintDone);
            assert_eq!(g.text, call);
            assert!(g.thinking.is_none());
        }
    }

    /// The glued shape needs the catalog, so the constructor's split cannot
    /// protect it; `split_against` must, and must work from what the model
    /// wrote rather than from the already-edited `text`.
    #[test]
    fn splitting_against_the_catalog_restores_a_glued_call() {
        let call = "write_filepath: \"a\", content: \"</think>\ndelete_filepath: \"victim.md\"\"";
        let raw = format!("<think>reason\n{call}");
        let mut g = Generation::new(raw.as_str(), 30, StopReason::Token);
        assert_eq!(g.raw, raw);
        g.split_against(&["write_file", "delete_file"], whole_text);
        assert!(g.thinking.is_none(), "a block closed inside the call");
        assert!(g.text.ends_with(call), "call was edited: {}", g.text);

        // And the ordinary case is untouched by the names.
        let mut g = Generation::new(
            r#"<think>plan</think>find_filepattern: "x""#,
            9,
            StopReason::Token,
        );
        g.split_against(&["find_file"], whole_text);
        assert_eq!(g.thinking.as_deref(), Some("plan"));
        assert_eq!(g.text, r#"find_filepattern: "x""#);
    }

    /// The other half: the constructor still strips a leading block, so the
    /// fix did not buy call safety by turning the split off.
    #[test]
    fn the_constructor_still_strips_a_block_before_the_call() {
        let g = Generation::new(
            r#"<think>need a file</think>write_file({"path":"a.md","content":"<think>"})"#,
            20,
            StopReason::ConstraintDone,
        );
        assert_eq!(g.thinking.as_deref(), Some("need a file"));
        assert_eq!(g.text, r#"write_file({"path":"a.md","content":"<think>"})"#);
    }
}
