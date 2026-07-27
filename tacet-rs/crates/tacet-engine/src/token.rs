//! TokenCounter — rough token estimation + the context budget gate.
//!
//! WHY AN ESTIMATE: a real count requires the tokenizer (hence the `tokenizers`
//! dependency and a loaded model file). The truncation decision MUST be
//! reachable without weights, on FakeEngine and in CI — otherwise the budget
//! logic could only be tested with a real model, i.e. in practice never tested
//! at all. When a real count is at hand it can be swapped in via `with_measure`;
//! the policy stays the same.
//!
//! THE SAFE DIRECTION: the estimate deliberately runs HIGH. Under-estimating and
//! blowing the budget means the model gets the prompt cut off in the middle (a
//! silent, undiagnosable failure); over-estimating and dropping one extra turn
//! only costs a little context.

use crate::prompt::{Prompt, Turn};

/// The context window of the on-device model. A constant of the architecture
/// (see the 4096 channel).
pub const CONTEXT_BUDGET: usize = 4096;

/// The share reserved for generation; the prompt must fit in what is left.
///
/// If the prompt could fill the whole budget the model would have no room to
/// write an answer — a completely full prompt produces a zero-token answer.
///
/// 512 -> 1024 CAME FROM MEASUREMENT (write_code, 26 Jul 2026): Qwen3-8B is a
/// thinking model — the `<think>` block is not printed to screen but it BURNS
/// TOKENS — and in a "write a python script" turn the thinking plus the full
/// script DID NOT FIT in 512; generation was cut with `Length` every time and
/// the tool was never called. The price is the prompt cap dropping to 3072, i.e.
/// the history getting truncated a little earlier in long chats — a far cheaper
/// loss than silent half-finished JSON.
///
/// THIS CONSTANT IS A MINIMUM, NOT A CAP: the real generation cap is derived
/// from the length of the prompt (see `generation_cap`).
/// `SamplingSetting::default()` uses it as the default only for call sites that
/// DO NOT HAVE a prompt to compute the cap from (tests, direct engine calls).
/// Both paths stay inside the 4096 window, so Candle #3705's 8192 KV cache
/// region is never approached.
pub const GENERATION_SHARE: usize = 1024;

/// The average BYTES per token — 5/2 = 2.5; since it is fractional, numerator
/// and denominator are kept apart.
///
/// MEASURED ON A REAL MODEL, and the previous value (3) TURNED OUT WRONG. With
/// the Qwen2.5 tokenizer, Turkish prose averages **2.71 bytes/token**. That is,
/// the old estimate dividing bytes by 3 was doing the exact OPPOSITE of the
/// "safe direction" argument: it landed ~10% BELOW the truth.
///
/// The concrete failure: truncating a 200-turn history, the estimate said 3561
/// and decided it "fit" the cap of 3584, and `validate()` returned success; the
/// REAL token count of that same prompt was 3937. Together with the generation
/// share (3937 + 512) the 4096 window WAS BLOWN — exactly the silent cut-off
/// this file exists to prevent.
///
/// 2.5 was chosen, not 2.71: the measurement is the average of one kind of text
/// and token density varies with content (code and English are sparser, dense
/// Turkish inflection is denser). The ~8% between them is a deliberate safety
/// margin. An estimate running HIGH is cheap (a little lost context); running
/// LOW is expensive (an undiagnosable cut-off).
const BYTE_NUMERATOR: usize = 2;
const BYTE_DENOMINATOR: usize = 5;

pub struct TokenCounter {
    /// The total window.
    pub budget: usize,
    /// The share reserved for generation.
    pub generation_share: usize,
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self { budget: CONTEXT_BUDGET, generation_share: GENERATION_SHARE }
    }
}

/// What truncation did — the call site logs this, the tests assert on it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TruncationReport {
    /// How many old turns were dropped from the front.
    pub dropped_turns: usize,
    /// Was the guide sacrificed.
    pub guide_dropped: bool,
    /// Was the question itself truncated (last resort).
    pub question_truncated: bool,
    /// The estimated token count after truncation.
    pub final_estimate: usize,
}

impl TruncationReport {
    pub fn changed(&self) -> bool {
        self.dropped_turns > 0 || self.guide_dropped || self.question_truncated
    }
}

impl TokenCounter {
    pub fn new(budget: usize, generation_share: usize) -> Self {
        Self { budget, generation_share }
    }

    /// The cap the prompt has to fit into.
    pub fn prompt_cap(&self) -> usize {
        self.budget.saturating_sub(self.generation_share)
    }

    /// The real room left for GENERATION with this prompt.
    ///
    /// WHY NOT A FIXED `generation_share` (from measurement): the share was doing
    /// two jobs at once — the MINIMUM room truncation reserves, and the CAP on
    /// generation. Tying both to the same number left most of the window empty on
    /// short prompts: with an 11-tool catalog the prompt is ~2300 tokens, the
    /// window 4096, the cap 1024 — i.e. ~770 tokens were sitting UNUSED. The
    /// price was concrete: Qwen3-8B is a thinking model, and in a "write a python
    /// script" turn the thinking hit 1024, generation was cut in half and the
    /// tool was NEVER called.
    ///
    /// The share is now only the MINIMUM room truncation reserves; the cap is
    /// derived from the REAL length of the prompt. Since `truncate` fits the
    /// prompt into `prompt_cap()`, the result is always `generation_share` or
    /// more — i.e. this change NEVER SHRINKS the cap under any condition.
    ///
    /// The estimate is deliberately biased HIGH (see `BYTE_NUMERATOR`), so the
    /// cap comes out slightly SMALLER than the room actually left; losing a few
    /// tokens is the right trade against overflowing the window.
    pub fn generation_cap(&self, prompt: &Prompt) -> usize {
        self.budget.saturating_sub(self.prompt_estimate(prompt)).max(self.generation_share)
    }

    /// Rough token estimate.
    ///
    /// UTF-8 BYTE length is used, not characters: Turkish letters (ç, ğ, ş, ı, ö,
    /// ü) are multi-byte and tokenizers split at exactly those points too. If we
    /// counted characters we would systematically UNDER-estimate Turkish text.
    pub fn estimate(text: &str) -> usize {
        (text.len() * BYTE_NUMERATOR).div_ceil(BYTE_DENOMINATOR)
    }

    /// The estimate for the whole prompt.
    pub fn prompt_estimate(&self, prompt: &Prompt) -> usize {
        Self::estimate(&prompt.text())
    }

    /// If the budget is exceeded, shrinks the prompt BY POLICY.
    ///
    /// ORDER (cheapest to most expensive loss):
    ///   1. The OLDEST turns are dropped — that is the context whose loss is
    ///      smallest.
    ///   2. The guide is dropped — losing the guidance makes the job harder, but
    ///      the turn still runs because the tools and the question are in place.
    ///   3. Last resort: the question is truncated, PRESERVING THE END.
    ///
    /// THE SYSTEM INSTRUCTIONS AND THE TOOL DESCRIPTION ARE NEVER TRUNCATED. If
    /// the instructions are truncated the model forgets who it is and which
    /// language to answer in; if the tool description is truncated it invents a
    /// signature that does not exist. Both are far more expensive failures than
    /// losing history.
    pub fn truncate(&self, prompt: &mut Prompt) -> TruncationReport {
        let cap = self.prompt_cap();
        let mut report = TruncationReport::default();

        // 1) Drop old turns from the front.
        while self.prompt_estimate(prompt) > cap && !prompt.history.is_empty() {
            prompt.history.remove(0);
            report.dropped_turns += 1;
        }

        // 2) Sacrifice the guide.
        if self.prompt_estimate(prompt) > cap && prompt.guide.is_some() {
            prompt.guide = None;
            report.guide_dropped = true;
        }

        // 3) Last resort: truncate the question. THE END is preserved — the
        // user's actual request comes at the end of the sentence ("...now turn
        // this into a table"), the beginning sets up context.
        if self.prompt_estimate(prompt) > cap {
            let excess = self.prompt_estimate(prompt) - cap;
            let bytes_to_drop = excess * BYTE_DENOMINATOR / BYTE_NUMERATOR;
            let new = from_last_bytes(
                &prompt.question,
                prompt.question.len().saturating_sub(bytes_to_drop),
            );
            if new.len() < prompt.question.len() {
                prompt.question = new;
                report.question_truncated = true;
            }
        }

        report.final_estimate = self.prompt_estimate(prompt);
        report
    }

    /// If it still does not fit after truncation, error — the call site must not
    /// silently continue, because at that point the model sees text cut off in
    /// the middle of the prompt and produces undiagnosable nonsense.
    pub fn validate(&self, prompt: &Prompt) -> crate::error::EngineResult<()> {
        let measured = self.prompt_estimate(prompt);
        if measured > self.prompt_cap() {
            return Err(crate::error::EngineError::BudgetExceeded {
                measured,
                budget: self.prompt_cap(),
            });
        }
        Ok(())
    }

    /// The estimate for a list of turns — so call sites can decide before
    /// truncation.
    pub fn turns_estimate(turns: &[Turn]) -> usize {
        turns.iter().map(|t| Self::estimate(&t.text) + 4).sum()
    }
}

/// Returns the last `max_bytes` bytes of `text`, RESPECTING CHARACTER
/// BOUNDARIES. A raw `&s[i..]` would land in the middle of a multi-byte letter
/// and panic — in Turkish text that is not an exception but the rule.
fn from_last_bytes(text: &str, max_bytes: usize) -> String {
    if max_bytes >= text.len() {
        return text.to_string();
    }
    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}
