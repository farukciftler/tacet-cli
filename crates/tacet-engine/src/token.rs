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

/// The FLOOR of the context window, and the value used when the model does not
/// say what it can take.
///
/// IT USED TO BE THE WHOLE STORY, and the comment here called it "a constant of
/// the architecture". It was a constant of a DIFFERENT architecture: iOS, where
/// Apple's FoundationModels really does hand out 4096 tokens. This binary runs
/// its own weights through candle and has no such ceiling — the files on disk
/// declare 262144 (qwen3-4b), 131072 (gemma3-4b), 40960 (qwen3-8b).
///
/// Keeping it cost real context. With the tool catalog at 1559 tokens and the
/// whole prompt at 2586, about 1100 were left for the conversation, so a user
/// asking for a Python script saw "2 older turns left the window" on EVERY turn,
/// watched the model forget what it had just done, and got an explanation of a
/// tool error instead of a second attempt at the tool.
///
/// It stays as the floor because a model that declares less, or declares
/// nothing, must not end up worse off than before.
pub const CONTEXT_BUDGET: usize = 4096;

/// How much memory the KV cache may take.
///
/// The cache grows LINEARLY with the window, so the window is really a memory
/// decision. 2.5 GB on top of a 2.3 GB weight file leaves a 4-core laptop
/// usable while the model is loaded; at 32768 tokens qwen3-4b alone would want
/// 4.5 GB of cache, which is where a 16 GB machine starts swapping.
///
/// What it buys, computed from the geometry in the files themselves: qwen3-4b
/// and qwen3-8b 16954 tokens, gemma3-4b 17951, qwen2.5-3b 32768 (that one is
/// held by its own declared window, not by this budget — its two KV heads make
/// the cache cheap). Four to eight times what the fixed 4096 gave.
pub const KV_CACHE_BUDGET_BYTES: usize = 2_500_000_000;

/// The window to actually use.
///
/// `declared` is what the model says it can take, `kv_bytes_per_token` what one
/// token of context costs it. Either being `None` means the file did not say,
/// and the answer is then the floor — a guessed window puts positions past the
/// model's rope table and produces plausible-looking nonsense, which is worse
/// than a small window.
pub fn context_budget(declared: Option<usize>, kv_bytes_per_token: Option<usize>) -> usize {
    let (Some(declared), Some(per_token)) = (declared, kv_bytes_per_token) else {
        return CONTEXT_BUDGET;
    };
    if per_token == 0 {
        return CONTEXT_BUDGET;
    }
    let affordable = KV_CACHE_BUDGET_BYTES / per_token;
    // The floor wins over both: a model declaring 2048 still gets 4096, because
    // that is what every path in this crate was built and measured against.
    declared.min(affordable).max(CONTEXT_BUDGET)
}

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
        Self {
            budget: CONTEXT_BUDGET,
            generation_share: GENERATION_SHARE,
        }
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
        Self {
            budget,
            generation_share,
        }
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
        self.budget
            .saturating_sub(self.prompt_estimate(prompt))
            .max(self.generation_share)
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

#[cfg(test)]
mod budget_tests {
    use super::*;

    /// The four models on the author's disk, with the geometry read from their
    /// own files. These numbers are the reason the budget is a division rather
    /// than a constant: the same 2.5 GB buys a different window per model.
    #[test]
    fn each_model_gets_the_window_its_cache_affords() {
        // layers * kv_heads * (key + value) * 2 bytes
        let qwen3_4b = 36 * 8 * (128 + 128) * 2;
        let qwen3_8b = 36 * 8 * (128 + 128) * 2;
        let gemma3_4b = 34 * 4 * (256 + 256) * 2;

        assert_eq!(context_budget(Some(262_144), Some(qwen3_4b)), 16_954);
        assert_eq!(context_budget(Some(40_960), Some(qwen3_8b)), 16_954);
        assert_eq!(context_budget(Some(131_072), Some(gemma3_4b)), 17_951);
    }

    /// A model whose OWN declared window is smaller than what the memory budget
    /// would allow keeps its own number — exceeding it puts positions past the
    /// rope table.
    #[test]
    fn the_models_own_limit_wins_when_it_is_the_smaller_one() {
        let small_cache = 36 * 2 * (128 + 128) * 2;
        assert_eq!(context_budget(Some(32_768), Some(small_cache)), 32_768);
    }

    /// THE FLOOR HOLDS IN EVERY DIRECTION. Nothing may come out of this worse
    /// off than the fixed 4096 it replaced.
    #[test]
    fn nothing_ends_up_below_the_old_fixed_window() {
        assert_eq!(context_budget(Some(2048), Some(1_000)), CONTEXT_BUDGET);
        assert_eq!(context_budget(None, Some(1_000)), CONTEXT_BUDGET);
        assert_eq!(context_budget(Some(262_144), None), CONTEXT_BUDGET);
        assert_eq!(context_budget(None, None), CONTEXT_BUDGET);
        // A cache so large that the budget affords nothing.
        assert_eq!(
            context_budget(Some(262_144), Some(usize::MAX)),
            CONTEXT_BUDGET
        );
    }

    /// Division by zero is a crash, and a zero here means the file said
    /// something impossible rather than nothing.
    #[test]
    fn a_zero_cost_per_token_does_not_divide() {
        assert_eq!(context_budget(Some(262_144), Some(0)), CONTEXT_BUDGET);
    }
}
