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

use crate::prompt::{Prompt, Template, Turn};

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
/// It stays as the floor because a model that declares nothing, or whose KV
/// cache the memory budget can barely afford, must not end up worse off than
/// before. It is NOT lifted over a model that declares LESS (audit L21): that
/// number is the model's position table, and see `context_budget`.
pub const CONTEXT_BUDGET: usize = 4096;

/// The device inference will run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Cpu,
    /// Apple GPU. Returns an error when candle-core's `metal` feature is off —
    /// it DOES NOT silently FALL BACK to the CPU: the user deliberately asked for
    /// the GPU, and presenting an engine running 10x slower as "working" would be
    /// wrong.
    Metal,
    /// NVIDIA GPU via CUDA. Returns an error when candle-core's `cuda` feature is off —
    /// it DOES NOT silently FALL BACK to the CPU: the user deliberately asked for
    /// the GPU, and presenting an engine running 10x slower as "working" would be
    /// wrong.
    Cuda,
}

#[cfg(all(feature = "metal", feature = "cuda"))]
compile_error!(
    "`metal` and `cuda` are mutually exclusive: the default device is decided at \
     compile time and there is only one default. Pick the backend of the machine \
     you are building for."
);

// `derivable_impls` IS SUPPRESSED, NOT OBEYED — and the reason is that clippy
// only ever sees one third of this function. With neither `metal` nor `cuda`
// on, the body really is a bare `Device::Cpu` and the lint is right about the
// text it can see; `#[derive(Default)] + #[default] Cpu` would compile and
// would then pin EVERY build to the CPU, including the two that were built for
// a GPU. That is exactly the silent downgrade this enum's own doc comments
// refuse ("it DOES NOT silently FALL BACK to the CPU"). CI runs clippy without
// features, so this was a hard error on the default lint set — see the
// measurement note: a lint whose premise is a feature flag has to be answered
// where the flag is, not by deleting the branches it cannot see.
#[allow(clippy::derivable_impls)]
impl Default for Device {
    fn default() -> Self {
        #[cfg(feature = "metal")]
        {
            Device::Metal
        }
        #[cfg(feature = "cuda")]
        {
            Device::Cuda
        }
        #[cfg(not(any(feature = "metal", feature = "cuda")))]
        {
            Device::Cpu
        }
    }
}

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

/// How much memory the KV cache may take for a given device.
pub fn kv_cache_budget(device: Device) -> usize {
    match device {
        Device::Cpu | Device::Metal => 2_500_000_000,
        Device::Cuda => 8_000_000_000,
    }
}

/// The window to actually use.
///
/// `declared` is what the model says it can take, `kv_bytes_per_token` what one
/// token of context costs it. Either being `None` means the file did not say,
/// and the answer is then the floor — a guessed window puts positions past the
/// model's rope table and produces plausible-looking nonsense, which is worse
/// than a small window. A missing or zero COST leaves the memory side at the
/// floor, but still under what the model declares.
///
/// THE MODEL'S OWN NUMBER IS A CEILING THE FLOOR MAY NOT LIFT (audit L21). The
/// floor used to win over both: a model declaring 2048 was handed 4096, on the
/// argument that every path here was measured at 4096. But a declared window is
/// the length of the model's position table, and positions past it do not
/// error — they produce exactly the plausible nonsense the paragraph above
/// refuses to guess into. The floor now protects against the MEMORY budget
/// coming out small, which is what it was for; it no longer overrides the model.
///
/// The caller hands in the window the ENGINE can run, not merely the one the
/// file states: candle's llama module sizes its rope table at a fixed 4096
/// whatever the file says (see `CandleEngine::context_length`).
pub fn context_budget(
    declared: Option<usize>,
    kv_bytes_per_token: Option<usize>,
    device: Device,
) -> usize {
    // Zero is "the file said something impossible", which has the same answer
    // as saying nothing — a zero window would refuse every prompt.
    let Some(declared) = declared.filter(|d| *d > 0) else {
        return CONTEXT_BUDGET;
    };
    let affordable = match kv_bytes_per_token {
        Some(per_token) if per_token > 0 => {
            (kv_cache_budget(device) / per_token).max(CONTEXT_BUDGET)
        }
        _ => CONTEXT_BUDGET,
    };
    declared.min(affordable)
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
    /// The part truncation may not touch — system block, tool descriptions,
    /// note, memory — is over the prompt cap ON ITS OWN. The question was left
    /// whole and the prompt does not fit; the turn cannot run (see `fit`).
    pub fixed_part_too_large: bool,
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
    /// derived from the REAL length of the prompt. A prompt `fit` accepted is
    /// inside `prompt_cap()`, so its cap is `generation_share` or more.
    ///
    /// THE CAP NEVER REACHES PAST THE WINDOW (audit L21, second pass). It used to
    /// end in `.max(generation_share)`, which is a no-op for a fitted prompt and
    /// a lie for any other: when the fixed part alone was over the prompt cap,
    /// the answer was 1024 however little room was left. Measured through
    /// `context_budget` + `truncate` + this: a 2048-token window with a
    /// 1300-token system came to 2346 positions, and the shell's own ~2586-token
    /// prompt to 3632; a 4096 window (SmolLM2 after the rope-table cut) with a
    /// 3300-token system came to 4346 — past candle llama's 4096 table, where
    /// every forward fails. A prompt that does not leave the minimum is refused
    /// by `fit`; it is not handed a cap the window does not have.
    ///
    /// The estimate is deliberately biased HIGH (see `BYTE_NUMERATOR`), so the
    /// cap comes out slightly SMALLER than the room actually left; losing a few
    /// tokens is the right trade against overflowing the window.
    pub fn generation_cap(&self, prompt: &Prompt) -> usize {
        self.budget.saturating_sub(self.prompt_estimate(prompt))
    }

    /// Rough token estimate.
    ///
    /// UTF-8 BYTE length is used, not characters: Turkish letters (ç, ğ, ş, ı, ö,
    /// ü) are multi-byte and tokenizers split at exactly those points too. If we
    /// counted characters we would systematically UNDER-estimate Turkish text.
    ///
    /// A DIGIT IS A WHOLE TOKEN (audit L22). The Qwen pre-tokenizer splits on
    /// `\p{N}` one character at a time (see `gguf_tokenizer::split_regex`), so
    /// every digit is its own token — 1 per byte, where the 2/5 ratio counts 0.4.
    /// Measured by the audit: 4000 digits estimated at 1600 against a real count
    /// of at least 4000, i.e. a table of numbers or a hex dump came in at less
    /// than half and truncation waved it through.
    ///
    /// `\p{N}` IS NOT ASCII. Pricing only `0-9` left the Arabic-Indic digits
    /// (U+0660..U+0669, two bytes each) at 0.8 a digit — 1000 of them estimated
    /// at 800 — so a number table in that script stayed on the unsafe side. Any
    /// `char::is_numeric` (Nd, Nl, No: the same class as `\p{N}`) is a token.
    ///
    /// A NUMERIC CHARACTER COSTS THE LARGER OF ONE TOKEN AND ITS BYTES AT THE
    /// RATIO, never the smaller: a three-byte full-width digit was 1.2 under the
    /// byte ratio, and a byte-level BPE may well split a rare one further, so
    /// "one each" would have LOWERED its estimate. Taking the maximum means this
    /// change can only raise the estimate of any text — the safe direction holds.
    ///
    /// Counted in fifths of a token so the rounding happens once, at the end.
    pub fn estimate(text: &str) -> usize {
        let fifths: usize = text
            .chars()
            .map(|c| {
                let at_ratio = c.len_utf8() * BYTE_NUMERATOR;
                if c.is_numeric() {
                    at_ratio.max(BYTE_DENOMINATOR)
                } else {
                    at_ratio
                }
            })
            .sum();
        fifths.div_ceil(BYTE_DENOMINATOR)
    }

    /// The estimate for the whole prompt, IN THE LARGEST WIRE FORMAT.
    ///
    /// WHY NOT `prompt.text()` (audit L22): that is the Plain template, and no
    /// real engine is fed Plain. Candle tokenizes `text_with_template` in ChatML
    /// or Gemma, and their fences (`<|im_start|>system\n…<|im_end|>\n` per turn)
    /// are text Plain does not carry — on a long history the gap is a few tokens
    /// per turn, every turn, on the side that blows the window.
    ///
    /// THE COUNTER DOES NOT KNOW THE ENGINE, so it measures every template and
    /// keeps the largest. Whatever format the engine really speaks, the estimate
    /// is at least the estimate of that format — the same "run high" bargain as
    /// the byte ratio, paid once per template rather than guessed.
    pub fn prompt_estimate(&self, prompt: &Prompt) -> usize {
        every_template()
            .into_iter()
            .map(|template| Self::estimate(&prompt.text_with_template(template)))
            .max()
            .unwrap_or(0)
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
        //
        // THE CUT IS MEASURED, NOT CONVERTED (audit L22). It used to turn the
        // excess tokens into bytes at the prose ratio (2/5), which stopped being
        // the price of every byte the day digits started costing one token each:
        // a question of digits had 2.5x the bytes it needed cut. Measured on the
        // default counter (cap 3072): 3500 digits kept 2345 bytes, 5000 kept 95,
        // and 6000 kept NOTHING — the user's pasted table or hex dump was deleted
        // outright when about 3000 digits would have fit. So the longest tail
        // that fits is searched for against `prompt_estimate` itself, whatever
        // the text is made of. The estimate only grows with the tail, so a
        // binary search is exact; ~20 estimates for a megabyte question.
        if self.prompt_estimate(prompt) > cap {
            let original = std::mem::take(&mut prompt.question);
            // AN EMPTY QUESTION THAT STILL DOES NOT FIT means the fixed part is
            // the problem, and cutting the question cannot solve it. This branch
            // used to cut anyway, all the way to nothing — so on a 2048-token
            // window every turn reached the model with the user's question
            // deleted, over the window regardless (audit L21, second pass). The
            // question is put back whole and the report says why; `fit` turns
            // that into a refusal the user can read.
            if self.prompt_estimate(prompt) > cap {
                prompt.question = original;
                report.fixed_part_too_large = true;
                report.final_estimate = self.prompt_estimate(prompt);
                return report;
            }
            let (mut fits, mut too_long) = (0, original.len());
            while too_long - fits > 1 {
                let middle = fits + (too_long - fits) / 2;
                prompt.question = from_last_bytes(&original, middle);
                if self.prompt_estimate(prompt) <= cap {
                    fits = middle;
                } else {
                    too_long = middle;
                }
            }
            prompt.question = from_last_bytes(&original, fits);
            report.question_truncated = prompt.question.len() < original.len();
        }

        report.final_estimate = self.prompt_estimate(prompt);
        report
    }

    /// `truncate`, then REFUSE what still does not fit — the call the shell
    /// makes.
    ///
    /// ONE CALL, NOT TWO, because two is what the shell had and it made one of
    /// them: it truncated and never validated, so a prompt whose fixed part was
    /// over the cap went to the model with its question cut to nothing and a
    /// generation cap reaching past the window (audit L21, second pass). After
    /// `truncate` the only prompt left over the cap is one whose system block,
    /// tools, note and memory are over it alone; no amount of history or
    /// question removal helps it, and the error says how far over it is.
    pub fn fit(&self, prompt: &mut Prompt) -> crate::error::EngineResult<TruncationReport> {
        let report = self.truncate(prompt);
        self.validate(prompt)?;
        Ok(report)
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

/// Every wire format a prompt can be rendered in.
///
/// The `match` is there so a new `Template` variant does not compile until
/// someone has looked at this list: a format left out here would be the one
/// format the budget never measured.
fn every_template() -> [Template; 3] {
    let _exhaustive = |template: Template| match template {
        Template::Plain | Template::ChatML | Template::Gemma => (),
    };
    [Template::Plain, Template::ChatML, Template::Gemma]
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

        assert_eq!(
            context_budget(Some(262_144), Some(qwen3_4b), Device::Cpu),
            16_954
        );
        assert_eq!(
            context_budget(Some(40_960), Some(qwen3_8b), Device::Cpu),
            16_954
        );
        assert_eq!(
            context_budget(Some(131_072), Some(gemma3_4b), Device::Cpu),
            17_951
        );

        // On CUDA, the 8 GB budget affords 54,253 tokens for qwen3-4b.
        assert_eq!(
            context_budget(Some(262_144), Some(qwen3_4b), Device::Cuda),
            54_253
        );
    }

    /// A model whose OWN declared window is smaller than what the memory budget
    /// would allow keeps its own number — exceeding it puts positions past the
    /// rope table.
    #[test]
    fn the_models_own_limit_wins_when_it_is_the_smaller_one() {
        let small_cache = 36 * 2 * (128 + 128) * 2;
        assert_eq!(
            context_budget(Some(32_768), Some(small_cache), Device::Cpu),
            32_768
        );
    }

    /// THE FLOOR HOLDS AGAINST THE MEMORY BUDGET. Nothing that declares at
    /// least 4096 may come out of this worse off than the fixed 4096 it
    /// replaced, whatever the cache costs or whether the file says.
    #[test]
    fn nothing_ends_up_below_the_old_fixed_window() {
        assert_eq!(
            context_budget(Some(8192), Some(usize::MAX / 2), Device::Cpu),
            CONTEXT_BUDGET
        );
        assert_eq!(
            context_budget(None, Some(1_000), Device::Cpu),
            CONTEXT_BUDGET
        );
        assert_eq!(
            context_budget(Some(262_144), None, Device::Cpu),
            CONTEXT_BUDGET
        );
        assert_eq!(context_budget(None, None, Device::Cpu), CONTEXT_BUDGET);
        // A cache so large that the budget affords nothing.
        assert_eq!(
            context_budget(Some(262_144), Some(usize::MAX), Device::Cpu),
            CONTEXT_BUDGET
        );
    }

    /// Division by zero is a crash, and a zero here means the file said
    /// something impossible rather than nothing.
    #[test]
    fn a_zero_cost_per_token_does_not_divide() {
        assert_eq!(
            context_budget(Some(262_144), Some(0), Device::Cpu),
            CONTEXT_BUDGET
        );
        assert_eq!(
            context_budget(Some(0), Some(1_000), Device::Cpu),
            CONTEXT_BUDGET
        );
    }

    /// AUDIT L22, the digits: the audit's own case, 4000 digits estimated at
    /// 1600. Each one is a token in the Qwen split, so the estimate may not come
    /// in under one per digit — and mixed text must still run at least as high
    /// as the byte ratio alone, or the change bought digits by selling prose.
    #[test]
    fn a_digit_is_counted_as_a_whole_token() {
        let digits = "7".repeat(4000);
        assert!(
            TokenCounter::estimate(&digits) >= 4000,
            "4000 digits estimated at {} tokens",
            TokenCounter::estimate(&digits)
        );
        let table = "| 2026-10-09 | 31.415 | 0xFF00AA |\n".repeat(50);
        let byte_ratio = (table.len() * BYTE_NUMERATOR).div_ceil(BYTE_DENOMINATOR);
        let digit_count = table.bytes().filter(u8::is_ascii_digit).count();
        assert!(TokenCounter::estimate(&table) >= byte_ratio);
        assert!(TokenCounter::estimate(&table) >= digit_count);
        // Prose without digits is untouched.
        let prose = "Merhaba dünya, bugün hava güzel.";
        assert_eq!(
            TokenCounter::estimate(prose),
            (prose.len() * BYTE_NUMERATOR).div_ceil(BYTE_DENOMINATOR)
        );
        // `\p{N}` is not ASCII: Arabic-Indic three (U+0663) is two bytes, so the
        // ratio alone priced 1000 of them at 800.
        let arabic = "\u{0663}".repeat(1000);
        assert!(
            TokenCounter::estimate(&arabic) >= 1000,
            "1000 Arabic-Indic digits estimated at {} tokens",
            TokenCounter::estimate(&arabic)
        );
        // A wider digit keeps the byte price when that is the larger one — the
        // whole-token rule may raise an estimate, never lower it. Full-width
        // three (U+FF13) and Devanagari three (U+0969) are three bytes each.
        for wide in ["\u{FF13}", "\u{0969}"] {
            let text = wide.repeat(1000);
            let byte_ratio = (text.len() * BYTE_NUMERATOR).div_ceil(BYTE_DENOMINATOR);
            assert!(
                TokenCounter::estimate(&text) >= byte_ratio,
                "1000 x {wide} estimated at {} under the byte ratio's {byte_ratio}",
                TokenCounter::estimate(&text)
            );
        }
    }

    /// AUDIT L22, the cut: pricing digits right must not make the truncator
    /// wipe them. A 6000-digit question on the default counter has to come out
    /// as a tail that FILLS the cap — not an empty string, which is what the
    /// byte-ratio cut produced (it dropped 2.5x the bytes the excess called for).
    #[test]
    fn a_digit_heavy_question_is_cut_to_what_fits_not_to_nothing() {
        let counter = TokenCounter::default();
        let cap = counter.prompt_cap();
        for count in [3500, 6000, 20_000] {
            let digits = "1234567890".repeat(count / 10);
            let mut prompt = Prompt::new("sys", digits.as_str());
            let report = counter.truncate(&mut prompt);
            assert!(report.question_truncated, "{count} digits were not cut");
            assert!(
                report.final_estimate <= cap,
                "{count} digits: {} over the cap of {cap}",
                report.final_estimate
            );
            assert!(
                report.final_estimate + 64 >= cap,
                "{count} digits: cut to {} bytes, estimate {} against a cap of {cap}",
                prompt.question.len(),
                report.final_estimate
            );
            // THE END is what survives.
            assert!(digits.ends_with(&prompt.question));
            assert!(counter.validate(&prompt).is_ok());
        }
        // Multi-byte text is still cut on a character boundary.
        let mut prompt = Prompt::new("sys", "ğ١".repeat(5000).as_str());
        let report = counter.truncate(&mut prompt);
        assert!(report.question_truncated && !prompt.question.is_empty());
        assert!(report.final_estimate <= cap);
    }

    /// AUDIT L22, the template: the budget must cover the format the engine
    /// is really fed. Plain is what `prompt.text()` renders and no real engine
    /// speaks it; ChatML wraps every history turn in fences Plain does not
    /// write. Whatever template an engine declares, the estimate is at least
    /// that template's estimate.
    #[test]
    fn the_estimate_covers_every_wire_format() {
        let mut history = Vec::new();
        for i in 0..40 {
            history.push(Turn::user(format!("question {i}")));
            history.push(Turn::assistant(format!("answer {i}")));
        }
        let prompt = Prompt::new("You are Tacet.", "and now?").with_history(history);
        let counter = TokenCounter::default();
        let estimate = counter.prompt_estimate(&prompt);
        for template in every_template() {
            let rendered = TokenCounter::estimate(&prompt.text_with_template(template));
            assert!(
                estimate >= rendered,
                "the budget says {estimate} but the {template:?} prompt is {rendered}"
            );
        }
        // And the fences are not a rounding error: on this history the formats
        // really differ, which is what makes measuring the wrong one matter.
        let plain = TokenCounter::estimate(&prompt.text());
        assert!(estimate > plain, "{estimate} vs plain {plain}");
    }

    /// AUDIT L21: the floor is not a licence to run past the model.
    ///
    /// The first assertion in `nothing_ends_up_below_the_old_fixed_window` used
    /// to read the other way — `Some(2048)` came back as 4096 — and that
    /// expectation WAS the bug: positions 2048..4096 are past that model's
    /// position table, and nothing errors there. A model that declares less than
    /// the floor gets what it declares, in every combination of the memory
    /// inputs.
    #[test]
    fn a_model_declaring_less_than_the_floor_keeps_its_own_window() {
        assert_eq!(context_budget(Some(2048), Some(1_000), Device::Cpu), 2048);
        assert_eq!(context_budget(Some(2048), None, Device::Cpu), 2048);
        assert_eq!(context_budget(Some(2048), Some(0), Device::Cpu), 2048);
        assert_eq!(
            context_budget(Some(2048), Some(usize::MAX), Device::Cuda),
            2048
        );
    }

    /// AUDIT L21, SECOND PASS: the window is the window for the WHOLE turn, the
    /// prompt and what is generated after it.
    ///
    /// The three cases are the review's probe, through the same three calls the
    /// shell makes: a 2048-token model (TinyLlama) under a 1300-token system and
    /// under the shell's measured ~2586-token prompt, and a 4096-token one
    /// (SmolLM2 after the rope-table cut) under 3300. In each the fixed part is
    /// over the prompt cap on its own. Before: the question was cut to "" and
    /// `generation_cap` still answered 1024, landing at 2346, 3632 and 4346
    /// positions. After: the question is untouched, the cap stays inside the
    /// window, and `fit` refuses the turn.
    #[test]
    fn a_fixed_part_over_the_cap_is_refused_not_run_past_the_window() {
        const QUESTION: &str = "what is in this folder?";
        for (declared, system_tokens) in [(2048, 1300), (2048, 2586), (4096, 3300)] {
            let window = context_budget(Some(declared), Some(1_000), Device::Cpu);
            assert_eq!(window, declared, "the window is not the model's");
            let counter = TokenCounter::new(window, GENERATION_SHARE);
            // "word " is five bytes, two tokens at the prose ratio.
            let system = "word ".repeat(system_tokens / 2);
            assert_eq!(TokenCounter::estimate(&system), system_tokens);
            let mut prompt = Prompt::new(&system, QUESTION);
            let report = counter.truncate(&mut prompt);
            assert_eq!(
                prompt.question, QUESTION,
                "{declared}/{system_tokens}: the user's question was cut to {:?}",
                prompt.question
            );
            assert!(report.fixed_part_too_large && !report.question_truncated);
            assert_eq!(report.final_estimate, counter.prompt_estimate(&prompt));
            // The cap may add nothing past the window. A prompt already over
            // it (2586 in 2048) gets no generation at all, not a share on top.
            let estimate = counter.prompt_estimate(&prompt);
            let cap = counter.generation_cap(&prompt);
            assert!(
                cap <= window.saturating_sub(estimate),
                "{declared}/{system_tokens}: a {estimate}-token prompt with a \
                 {cap}-token generation cap is {} positions in a {window}-token window",
                estimate + cap
            );
            match counter.fit(&mut prompt) {
                Err(crate::error::EngineError::BudgetExceeded { measured, budget }) => {
                    assert!(measured > budget && budget == counter.prompt_cap());
                }
                other => panic!("{declared}/{system_tokens}: the turn was not refused: {other:?}"),
            }
            assert_eq!(prompt.question, QUESTION, "fit deleted the question");
        }
    }

    /// The other side of the refusal: a fixed part that FITS still runs, with
    /// the question cut from the front as before and at least the minimum share
    /// left for the answer. The refusal is for prompts the window cannot hold,
    /// not for every prompt that needed cutting.
    #[test]
    fn a_fixed_part_that_fits_is_still_fitted_and_run() {
        let counter = TokenCounter::new(2048, GENERATION_SHARE);
        let system = "word ".repeat(400); // 800 of a 1024 cap
        let question = format!("{} THE REAL REQUEST", "preamble ".repeat(400));
        let mut prompt = Prompt::new(&system, question.as_str());
        let report = counter
            .fit(&mut prompt)
            .expect("a fitting fixed part was refused");
        assert!(report.question_truncated && !report.fixed_part_too_large);
        assert!(prompt.question.ends_with("THE REAL REQUEST"));
        let cap = counter.generation_cap(&prompt);
        assert!(cap >= GENERATION_SHARE, "only {cap} left for the answer");
        assert!(counter.prompt_estimate(&prompt) + cap <= 2048);
    }
}
