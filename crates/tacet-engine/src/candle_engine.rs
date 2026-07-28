//! CandleEngine — pure-Rust GGUF inference. Behind the `candle` feature.
//!
//! USER DECISION: NO llama.cpp FFI. Inference runs on Candle, in pure Rust.
//!
//! THIS FILE IS ABSENT FROM THE DEFAULT BUILD. `cargo build -p tacet-engine`
//! never pulls the candle tree; all eval and CI run on `FakeEngine`. This file
//! only compiles with `--features candle`.
//!
//! NO NETWORK: the model and the tokenizer are loaded from LOCAL paths. The
//! `hf-hub` extension was deliberately left off — this crate never downloads
//! under any condition.

use crate::constraint::Constrainer;
use crate::error::{EngineError, EngineResult};
use crate::prompt::{Prompt, Template};
use crate::provider::{
    EngineProvider, Generation, GenerationFuture, SamplingSetting, StopReason, boxed_generation,
};

// `candle_core::Device` is imported under an alias because our own `Device`
// (cpu/metal choice) is the type the call sites see; letting the foreign type
// take the plain name would make the public API read like candle's.
use candle_core::{Device as CandleDevice, Tensor, quantized::gguf_file};
use candle_transformers::generation::{LogitsProcessor, Sampling};
// THE ARCHITECTURE MODULE IS NO LONGER FIXED — see `Architecture`.
//
// An earlier round pinned a single module (`quantized_qwen2`). The reason still
// holds: Qwen2.5 attention layers carry q/k/v BIAS tensors that DO NOT EXIST in
// llama, and llama's `forward_attn` DOES NOT ADD the bias. What changed is that
// THREE different architectures are now supported; which one is loaded is READ
// from the GGUF metadata, not assumed.
use candle_transformers::models::{quantized_gemma3, quantized_qwen2, quantized_qwen3};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokenizers::Tokenizer;

/// How many recent tokens the repeat penalty looks at.
///
/// Looking at the whole history also suppresses natural repetition (Turkish
/// suffixes, conjunctions, proper nouns) and damages the text; too short a
/// window falls outside the loop.
const REPEAT_WINDOW: usize = 96;

/// Loop detector: if a sequence of this length repeats this many times within the
/// last tokens, generation is cut.
///
/// WHY THE PENALTY IS NOT ENOUGH: `apply_repeat_penalty` penalises individual
/// TOKENS. When the model loops on a PATTERN like "It was developed in year X.",
/// the tokens inside the pattern leave and re-enter the window every round and
/// the penalty may not be enough to break the loop. This detector sees THE
/// PATTERN ITSELF.
const LOOP_SEQUENCE_LENGTH: usize = 12;
const LOOP_THRESHOLD: usize = 3;

/// The padding character the tokenizer inserts for half-decoded bytes (U+FFFD).
/// In the stream, TRAILING padding is dropped (see `run_loop`).
const REPLACEMENT: char = '\u{FFFD}';

/// The index of the highest logit — for diagnostics: "what would the model have
/// wanted without the mask".
///
/// `LogitsProcessor` cannot be reused (it has state, the seed advances), so the
/// argmax is done by hand. Only called when `TACET_TRACE_DUMP` is on.
fn largest_index(logits: &[f32]) -> usize {
    logits
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Does the same `LOOP_SEQUENCE_LENGTH`-long slice occur `LOOP_THRESHOLD` times
/// back to back at the end of the produced sequence?
///
/// It looks only at THE END: a legitimate repetition in the middle of the text (a
/// list, a table row) must not cut generation; what has to be cut is generation
/// being STUCK right now.
fn is_looping(produced: &[u32]) -> bool {
    let needed = LOOP_SEQUENCE_LENGTH * LOOP_THRESHOLD;
    if produced.len() < needed {
        return false;
    }
    let last = &produced[produced.len() - LOOP_SEQUENCE_LENGTH..];
    (1..LOOP_THRESHOLD).all(|k| {
        let start = produced.len() - LOOP_SEQUENCE_LENGTH * (k + 1);
        &produced[start..start + LOOP_SEQUENCE_LENGTH] == last
    })
}

/// The device inference will run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Cpu,
    /// Apple GPU. Returns an error when candle-core's `metal` feature is off —
    /// it DOES NOT silently FALL BACK to the CPU: the user deliberately asked for
    /// the GPU, and presenting an engine running 10x slower as "working" would be
    /// wrong.
    Metal,
}

/// The default device is decided AT COMPILE TIME.
///
/// With the `metal` feature on the default is Metal; that is measured (Qwen2.5-3B
/// q4_k_m, same prompt): decode 31 -> 73 tokens/s, prefill 113 -> 775 tokens/s.
/// Explicitly compiling the feature in and still running on the CPU is nobody's
/// intention; on the contrary, it invites the question "why is this slow".
///
/// With the feature off Metal cannot be chosen anyway (opening the device errors
/// out), so the default is the CPU. Making this distinction with `cfg` is
/// deliberate: tried at runtime with a silent fallback to the CPU, there would be
/// nowhere at all showing why the GPU was not used.
impl Default for Device {
    fn default() -> Self {
        #[cfg(feature = "metal")]
        {
            Device::Metal
        }
        #[cfg(not(feature = "metal"))]
        {
            Device::Cpu
        }
    }
}

/// The supported GGUF architectures.
///
/// WHY IT IS READ AND NOT ASSUMED: the weight file's name ("model.gguf") says
/// nothing about the architecture and the folder name is whatever the user
/// wanted. The only reliable source is the `general.architecture` key in the GGUF
/// metadata; the converter that produced the file is obliged to write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    Qwen2,
    Qwen3,
    Gemma3,
}

impl Architecture {
    /// Turns the name in the GGUF metadata into an architecture.
    ///
    /// AN UNKNOWN ARCHITECTURE ERRORS OUT; it DOES NOT FALL BACK to the nearest
    /// module. Loading the wrong module gives one of two outcomes: either a
    /// metadata key is not found (noisy, harmless) or the keys happen to overlap
    /// and the model SILENTLY PRODUCES GARBAGE. The second looks to the user like
    /// "the model is stupid" and takes hours to diagnose; to make that impossible
    /// from the start the match has to be EXACT.
    pub fn resolve(name: &str) -> EngineResult<Self> {
        match name {
            "qwen2" => Ok(Architecture::Qwen2),
            "qwen3" => Ok(Architecture::Qwen3),
            "gemma3" => Ok(Architecture::Gemma3),
            other => Err(EngineError::Inference(format!(
                "unsupported GGUF architecture: '{other}' \
                 (supported: qwen2, qwen3, gemma3)"
            ))),
        }
    }

    /// The chat template the model was trained on. TIED to the architecture, not
    /// selectable: the wrong template makes the role boundaries invisible and the
    /// model stops emitting a stop token and falls into rambling.
    pub fn template(self) -> Template {
        match self {
            Architecture::Qwen2 | Architecture::Qwen3 => Template::ChatML,
            Architecture::Gemma3 => Template::Gemma,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Architecture::Qwen2 => "qwen2",
            Architecture::Qwen3 => "qwen3",
            Architecture::Gemma3 => "gemma3",
        }
    }
}

/// The loaded weights — the only place that branches on the architecture.
///
/// The `ModelWeights` types of the three modules SHARE NO COMMON TRAIT (candle
/// exposes them as independent concrete types), so bridging them needs a
/// hand-written enum. `Box<dyn ...>` was not an option: even if we defined the
/// trait ourselves, the types are foreign — the `forward` signatures match, but
/// implementing the trait for them would be as much code as this enum.
enum ArchitectureModel {
    Qwen2(quantized_qwen2::ModelWeights),
    Qwen3(quantized_qwen3::ModelWeights),
    Gemma3(quantized_gemma3::ModelWeights),
}

impl ArchitectureModel {
    fn forward(&mut self, input: &Tensor, position: usize) -> candle_core::Result<Tensor> {
        match self {
            ArchitectureModel::Qwen2(m) => m.forward(input, position),
            ArchitectureModel::Qwen3(m) => m.forward(input, position),
            ArchitectureModel::Gemma3(m) => m.forward(input, position),
        }
    }

    /// Resets the KV cache between generations.
    ///
    /// THE BODY IS EMPTY FOR GEMMA3 AND THAT IS NOT AN OMISSION. candle's
    /// `quantized_gemma3` module deliberately DOES NOT OFFER `clear_kv_cache`,
    /// because it does not need it: before concatenating the cache the attention
    /// layer checks `if index_pos == 0 { (k, v) }`, i.e. on the prefill step it
    /// DOES NOT USE the old cache and overwrites it. Since our generation loop
    /// always starts at `position = 0`, Gemma3 clears itself. Qwen2/Qwen3 do an
    /// unconditional `Tensor::cat` — for those the cleanup is MANDATORY.
    ///
    /// This distinction was READ FROM THE SOURCE, not assumed; picking the wrong
    /// side would give an insidious corruption (the first answer fine, later ones
    /// progressively broken).
    fn clear_cache(&mut self) {
        match self {
            ArchitectureModel::Qwen2(m) => m.clear_kv_cache(),
            ArchitectureModel::Qwen3(m) => m.clear_kv_cache(),
            ArchitectureModel::Gemma3(_) => {}
        }
    }
}

/// Model loading settings.
#[derive(Debug, Clone)]
pub struct ModelSetting {
    /// The GGUF weight file (local).
    pub model_path: PathBuf,
    /// `tokenizer.json` (local).
    pub tokenizer_path: PathBuf,
    pub device: Device,
    /// The token ids that stop generation. Left empty, common names are looked up
    /// in the tokenizer's vocabulary.
    pub stop_tokens: Vec<u32>,
}

impl ModelSetting {
    pub fn new(model_path: impl Into<PathBuf>, tokenizer_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            tokenizer_path: tokenizer_path.into(),
            device: Device::default(),
            stop_tokens: Vec::new(),
        }
    }

    pub fn with_device(mut self, device: Device) -> Self {
        self.device = device;
        self
    }
}

pub struct CandleEngine {
    /// `forward` wants `&mut self`, whereas `EngineProvider::generate` takes
    /// `&self`. The mutex both closes that gap and enforces the right thing: the
    /// KV cache is kept INSIDE the model, and two generations running at once
    /// would corrupt each other's cache. The lock is not a concurrency
    /// concession, it is a correctness requirement.
    model: Mutex<ArchitectureModel>,
    /// The architecture READ from the GGUF metadata. The source of the template
    /// and of the diagnostic output; the user must be able to see which module
    /// was loaded.
    architecture: Architecture,
    tokenizer: Tokenizer,
    device: CandleDevice,
    stop_tokens: Vec<u32>,
    /// Token id -> SURFACE text. The prerequisite for setting up a constraint
    /// (see `vocab`). Produced once at load time: a decode call for 32k tokens is
    /// not cheap, but it is invisible next to the gguf load and repeating it per
    /// generation would be pointless.
    vocab: Vec<String>,
}

impl CandleEngine {
    /// Loads the weights and the tokenizer from LOCAL files.
    pub fn load(setting: &ModelSetting) -> EngineResult<Self> {
        let device = match setting.device {
            Device::Cpu => CandleDevice::Cpu,
            Device::Metal => CandleDevice::new_metal(0)
                .map_err(|e| EngineError::Inference(format!("could not open metal device: {e}")))?,
        };

        let mut file = std::fs::File::open(&setting.model_path)
            .map_err(|_| EngineError::ModelNotLoaded(setting.model_path.clone()))?;
        let content = gguf_file::Content::read(&mut file)
            .map_err(|_| EngineError::ModelNotLoaded(setting.model_path.clone()))?;

        // THE ARCHITECTURE IS READ BEFORE THE WEIGHTS. The metadata is already in
        // memory; starting a 2.5 GB load with the wrong module and only erroring
        // at the end is pointless.
        let architecture = read_architecture(&content)?;

        // `from_gguf` HAS THE SAME SIGNATURE IN ALL THREE MODULES (ct, reader,
        // device) — verified from the source, not assumed.
        let model = match architecture {
            Architecture::Qwen2 => {
                quantized_qwen2::ModelWeights::from_gguf(content, &mut file, &device)
                    .map(ArchitectureModel::Qwen2)
            }
            Architecture::Qwen3 => {
                quantized_qwen3::ModelWeights::from_gguf(content, &mut file, &device)
                    .map(ArchitectureModel::Qwen3)
            }
            Architecture::Gemma3 => {
                quantized_gemma3::ModelWeights::from_gguf(content, &mut file, &device)
                    .map(ArchitectureModel::Gemma3)
            }
        }
        .map_err(|e| {
            EngineError::Inference(format!(
                "could not decode gguf ({}): {e}",
                architecture.name()
            ))
        })?;

        let tokenizer = Tokenizer::from_file(&setting.tokenizer_path)
            .map_err(|e| EngineError::Tokenization(e.to_string()))?;

        let stop_tokens = if setting.stop_tokens.is_empty() {
            find_stop_tokens(&tokenizer)
        } else {
            setting.stop_tokens.clone()
        };

        let vocab = build_vocab(&tokenizer);

        Ok(Self {
            model: Mutex::new(model),
            architecture,
            tokenizer,
            device,
            stop_tokens,
            vocab,
        })
    }

    /// The architecture of the loaded GGUF — for diagnostics and shell output.
    pub fn architecture(&self) -> Architecture {
        self.architecture
    }

    /// Verifies the files exist BEFORE loading — the gguf load takes a long time
    /// and learning about a missing file at the end of it is a pointless wait.
    pub fn files_exist(setting: &ModelSetting) -> EngineResult<()> {
        for path in [&setting.model_path, &setting.tokenizer_path] {
            if !Path::new(path).is_file() {
                return Err(EngineError::ModelNotLoaded(path.clone()));
            }
        }
        Ok(())
    }

    /// The token ids that stop generation.
    ///
    /// PUBLIC because being empty is a SILENT failure: generation would then only
    /// stop at the token cap and look like "the model talks too much". The call
    /// site must be able to print this and check it by eye.
    pub fn stop_tokens(&self) -> &[u32] {
        &self.stop_tokens
    }

    /// The loaded tokenizer — for measurement and probing.
    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    fn tokenize(&self, text: &str) -> EngineResult<Vec<u32>> {
        self.tokenizer
            .encode(text, true)
            .map(|e| e.get_ids().to_vec())
            .map_err(|e| EngineError::Tokenization(e.to_string()))
    }

    fn decode(&self, tokens: &[u32]) -> EngineResult<String> {
        self.tokenizer
            .decode(tokens, true)
            .map_err(|e| EngineError::Tokenization(e.to_string()))
    }

    /// The actual generation loop. Separate and SYNCHRONOUS: inference is bound
    /// to the CPU/GPU and there is no I/O to await inside it — wrapping it in an
    /// `async` body only satisfies the contract.
    fn run_loop(
        &self,
        prompt: &Prompt,
        constraint: Option<&dyn Constrainer>,
        setting: SamplingSetting,
        listener: Option<&(dyn Fn(&str) + Send + Sync)>,
    ) -> EngineResult<Generation> {
        // THE ENGINE declares the template (see `EngineProvider::template`):
        // Qwen2.5 was trained with ChatML, and fed plain text it loses the roles
        // and falls into rambling.
        //
        // `true` = PARSE special tokens. The ChatML fences (`<|im_start|>`) have
        // to pass as SINGLE tokens, not as text; otherwise the model does not
        // recognise its own frame markers and never emits a stop token
        // (generation runs to the cap).
        let input = self.tokenize(&prompt.text_with_template(self.template()))?;
        if input.is_empty() {
            return Err(EngineError::Tokenization(
                "the prompt tokenized to nothing".into(),
            ));
        }

        // Temperature 0 -> ArgMax (greedy). That is the default: eval being
        // reproducible comes before sampling variety.
        let sampling = if setting.temperature <= f32::EPSILON {
            Sampling::ArgMax
        } else if setting.top_p >= 1.0 {
            Sampling::All {
                temperature: setting.temperature as f64,
            }
        } else {
            Sampling::TopP {
                p: setting.top_p as f64,
                temperature: setting.temperature as f64,
            }
        };
        let mut sampler = LogitsProcessor::from_sampling(setting.seed, sampling);

        let mut session = constraint.map(|c| c.session());
        let mut model = self.model.lock().expect("model lock");

        // THE KV CACHE IS RESET ON EVERY GENERATION. The cache lives INSIDE the
        // model and persists between generations, whereas `position` below starts
        // at 0. Uncleared, the second generation would write OVER the first one's
        // cache rows and attention would see a mixture of that turn's prompt and
        // the leftovers of the previous turn. The symptom is insidious: the first
        // answer is fine, later ones get progressively broken.
        model.clear_cache();

        let mut produced: Vec<u32> = Vec::with_capacity(setting.max_tokens);
        // Diagnostics (env-gated, read once — polling an environment variable at
        // every step of the hot loop would slow down the measurement itself).
        // The read goes through a single place (`tacet_kernel::env`) — it MUST read
        // the same variable as the CLI's trace dump flag; a diagnostic that opens
        // in two halves is useless.
        let dump = tacet_kernel::env_var("TACET_TRACE_DUMP").is_some();
        // How many tokens were produced in a STRUCTURAL region (repeat penalty
        // skipped).
        let mut structural_count = 0usize;
        // How many times the mask overrode the model's first preference.
        let mut mask_interventions = 0usize;
        // The byte length of the text already emitted to the listener — so the
        // next step can send only the addition.
        let mut written = String::new();
        let mut stop = StopReason::Length;
        // The prompt is processed in one pass (prefill); on later steps a single
        // token is fed and `position` says where the KV cache is.
        let mut position = 0usize;
        let mut next: Vec<u32> = input;

        for _ in 0..setting.max_tokens {
            // CANCELLATION IS POLLED PER TOKEN. There is no finer place: a
            // forward pass cannot be split, and ~30-80 ms per token already means
            // an instant stop on a human scale. The poll is at the very top,
            // because producing one more token in a cancelled turn means wasted
            // time and a word leaking to the screen.
            if let Some(flag) = setting.cancel
                && flag.load(std::sync::atomic::Ordering::Relaxed)
            {
                stop = StopReason::Cancelled;
                break;
            }
            let input_t = Tensor::new(next.as_slice(), &self.device)
                .and_then(|t| t.unsqueeze(0))
                .map_err(|e| EngineError::Inference(e.to_string()))?;

            let logits = model
                .forward(&input_t, position)
                .map_err(|e| EngineError::Inference(e.to_string()))?;
            // forward returns [batch, vocab]; we reduce it to a single row.
            let logits = logits
                .squeeze(0)
                .and_then(|t| t.to_dtype(candle_core::DType::F32))
                .map_err(|e| EngineError::Inference(e.to_string()))?;

            position += next.len();

            // THE REPEAT PENALTY — BEFORE THE CONSTRAINT, before the sampler.
            //
            // WHY IT IS NEEDED: the default sampling is greedy (temperature 0),
            // because eval being reproducible takes priority over variety. But
            // greedy picks the most likely token at every step, and if the model
            // latches onto a sentence it writes that sentence forever. That was
            // exactly the bug seen in the field: "The first version of C++ was
            // developed in 19997." dozens of times, until generation hit the cap.
            //
            // The penalty looks at the last `REPEAT_WINDOW` tokens, not all of
            // them: penalising the entire history would also suppress natural
            // repetition (conjunctions, proper nouns, Turkish suffixes) and damage
            // the text.
            //
            // THE ORDER MATTERS: penalty first, THEN the constraint mask. The
            // penalty only SHIFTS logits; the mask sets invalid tokens to
            // -infinity. In the other order the penalty could turn the mask's
            // -infinity into a finite number and pierce the grammar. The mask
            // always has the last word.
            // ...BUT NOT IN A STRUCTURAL REGION. Inside tool arguments (code,
            // JSON) repetition is NATURAL: indentation, newlines, a second
            // occurrence of the same identifier. The measured failure
            // (write_code/Qwen3-8B): under greedy, the penalty broke the second
            // spelling of `asal_sayi_kontrol` and turned every attempt into a
            // syntax error. The loop detector is already skipped while a
            // constraint is present; the penalty falls silent for the same reason
            // and ONLY inside the arguments — it stays in free text, so the prose
            // loop protection is not lost.
            let structural = session.as_ref().is_some_and(|s| s.is_structural());
            if structural {
                structural_count += 1;
            }
            let logits = if setting.repeat_penalty > 1.0 && !produced.is_empty() && !structural {
                let start = produced.len().saturating_sub(REPEAT_WINDOW);
                candle_transformers::utils::apply_repeat_penalty(
                    &logits,
                    setting.repeat_penalty,
                    &produced[start..],
                )
                .map_err(|e| EngineError::Inference(e.to_string()))?
            } else {
                logits
            };

            // THE CONSTRAINT IS APPLIED ON THE RAW LOGITS, before entering the
            // sampler.
            //
            // Candle's `LogitsProcessor::sample_f` callback looks tempting but
            // DOES NOT FIT HERE, for two reasons: (1) the callback works on the
            // probabilities AFTER softmax, whereas the mask speaks the language of
            // "-infinity logits"; (2) on the `Sampling::ArgMax` path the callback
            // is NEVER called — that is, in exactly our default greedy mode the
            // constraint would be silently disabled. Applying the mask here makes
            // the constraint binding whichever sampling is chosen.
            let token = if let Some(s) = session.as_mut() {
                let mut raw: Vec<f32> = logits
                    .to_vec1()
                    .map_err(|e| EngineError::Inference(e.to_string()))?;
                // DIAGNOSTIC: the steps where the mask CHANGED the model's choice.
                //
                // Without this measurement the question "is the model writing bad
                // code, or is the mask forbidding the right token" comes down to
                // guessing — and the guess was wrong once (the repeat penalty was
                // suspected; with a counter it turned out the penalty was never
                // applied in that region at all). While it is off the cost is one
                // env read.
                let unmasked_best = dump.then(|| largest_index(&raw));
                s.mask(&mut raw);
                // If everything was forbidden the sampler would make a meaningless
                // choice (a NaN probability distribution); this is a grammar bug
                // and must not be glossed over.
                if !raw.iter().any(|v| v.is_finite()) {
                    return Err(EngineError::Inference(
                        "the constraint forbade every token".into(),
                    ));
                }
                let masked = Tensor::new(raw.as_slice(), &self.device)
                    .map_err(|e| EngineError::Inference(e.to_string()))?;
                let chosen = sampler
                    .sample(&masked)
                    .map_err(|e| EngineError::Inference(e.to_string()))?;
                if let Some(wanted) = unmasked_best
                    && wanted != chosen as usize
                {
                    mask_interventions += 1;
                    eprintln!(
                        "(mask intervention @{}: model wanted {:?}, got {:?})",
                        produced.len(),
                        self.vocab.get(wanted).map(String::as_str).unwrap_or("?"),
                        self.vocab
                            .get(chosen as usize)
                            .map(String::as_str)
                            .unwrap_or("?"),
                    );
                }
                chosen
            } else {
                sampler
                    .sample(&logits)
                    .map_err(|e| EngineError::Inference(e.to_string()))?
            };

            if self.stop_tokens.contains(&token) {
                stop = StopReason::Token;
                break;
            }

            if let Some(s) = session.as_mut() {
                s.advance(token)?;
            }
            produced.push(token);

            // CUT stuck generation. Giving the user a half-finished but readable
            // answer beats giving them a wall repeating the same sentence 40
            // times. SKIPPED while a constraint is active: a valid JSON call may
            // by nature contain repeated strings, and cutting the call in the
            // middle would make it unparseable.
            if session.is_none() && is_looping(&produced) {
                produced.truncate(produced.len() - LOOP_SEQUENCE_LENGTH * (LOOP_THRESHOLD - 1));
                stop = StopReason::Loop;
                break;
            }
            next = vec![token];

            // STREAMING: at every step, decode the accumulated text and hand the
            // NEW addition to the listener. Decoding the whole vector every time
            // looks O(n^2), but n is a few hundred tokens and the cost is
            // negligible next to a `forward` pass; the gain is emitting BPE's
            // multi-byte pieces (Turkish 'ş', 'ı') at the correct boundary instead
            // of cutting them wrongly out of a single token. It works with the
            // constraint on too: the text the listener sees is the text really
            // produced, not a hallucination.
            //
            // A HALF-DECODED CHARACTER IS NOT EMITTED. When the first byte of a
            // multi-byte character (an emoji, some Turkish letters) is produced,
            // the tokenizer decodes it padded with THE REPLACEMENT CHARACTER
            // (U+FFFD); on the next step that same position becomes the real
            // character. Computing the difference by byte length first spilled a
            // broken character onto the screen and then the diff drifted — the
            // "...olabilirim? [broken]" line the user saw was this. DROPPING the
            // trailing padding both prevents the broken character and keeps
            // `written` always a PREFIX of `full`. A replacement character in the
            // MIDDLE of the text is left alone: that one really was produced.
            if let Some(f) = listener
                && let Ok(full) = self.decode(&produced)
            {
                let full = full.trim_end_matches(REPLACEMENT);
                if full.len() > written.len() && full.starts_with(written.as_str()) {
                    f(&full[written.len()..]);
                    written = full.to_string();
                }
            }

            // The constraint reached an accepting state: the grammar is complete
            // and stopping here is SAFE. Continuing would be allowing the model to
            // append rambling after valid JSON.
            if session.as_ref().is_some_and(|s| s.is_done()) {
                stop = StopReason::ConstraintDone;
                break;
            }
        }

        if dump {
            eprintln!(
                "(engine: {} tokens, {structural_count} structural/unpenalised, {mask_interventions} mask interventions)",
                produced.len()
            );
        }
        let text = self.decode(&produced)?;
        Ok(Generation::new(text, produced.len(), stop))
    }
}

impl EngineProvider for CandleEngine {
    fn name(&self) -> &str {
        "candle"
    }

    /// The template is DERIVED FROM THE LOADED ARCHITECTURE, it is not fixed.
    /// Declaring it is mandatory: fed plain text the model cannot see the role
    /// boundaries, does not emit the stop token and goes on writing "User:" to
    /// itself. Feeding ChatML to Gemma gives the same result — the fences are
    /// unfamiliar.
    fn template(&self) -> Template {
        self.architecture.template()
    }

    /// The prerequisite for setting up a constraint. Without declaring it,
    /// `CallConstraint` could never be built and THE REAL model would run
    /// unconstrained exactly where the constraint is needed most (small model,
    /// free generation) — a safeguard that works on the fake engine would vanish
    /// in production.
    fn vocab(&self) -> Option<Vec<String>> {
        Some(self.vocab.clone())
    }

    fn generate<'a>(
        &'a self,
        prompt: &'a Prompt,
        constraint: Option<&'a dyn Constrainer>,
        setting: SamplingSetting,
    ) -> GenerationFuture<'a> {
        boxed_generation(async move { self.run_loop(prompt, constraint, setting, None) })
    }

    /// Streaming generation: passes the listener to `run_loop`. It OVERRIDES the
    /// default (single-fragment) implementation because candle is the only engine
    /// that really produces token by token; on a 3B model the wait until the first
    /// token is hidden here.
    fn generate_streaming<'a>(
        &'a self,
        prompt: &'a Prompt,
        constraint: Option<&'a dyn Constrainer>,
        setting: SamplingSetting,
        listener: &'a (dyn Fn(&str) + Send + Sync),
    ) -> GenerationFuture<'a> {
        boxed_generation(async move { self.run_loop(prompt, constraint, setting, Some(listener)) })
    }
}

/// Reads the `general.architecture` value from the GGUF metadata.
///
/// If the key is ABSENT it errors out rather than guessing: this key is MANDATORY
/// in the GGUF specification, so without it the file is either corrupt or not
/// GGUF. Guessing the architecture for such a file would be manufacturing risk
/// where there is no problem to solve.
fn read_architecture(content: &gguf_file::Content) -> EngineResult<Architecture> {
    let value = content
        .metadata
        .get("general.architecture")
        .ok_or_else(|| {
            EngineError::Inference(
                "no 'general.architecture' in the GGUF metadata — the file is corrupt or not GGUF"
                    .into(),
            )
        })?;
    let name = value
        .to_string()
        .map_err(|e| EngineError::Inference(format!("'general.architecture' is not text: {e}")))?;
    Architecture::resolve(name)
}

/// Collects the common stop tokens from the tokenizer's vocabulary.
///
/// The GGUF family never settled on a single name (llama `</s>`, ChatML
/// `<|im_end|>`, llama-3 `<|eot_id|>`). If none is found the list stays empty and
/// generation only stops at the token cap: stopping late is preferred over
/// treating a wrong id as the end — the first cuts the output in the middle, the
/// second only wastes a few tokens.
///
/// LOOKING UP THE NAME IS NOT ENOUGH, BEING SPECIAL IS VERIFIED TOO. Measured: in
/// the Qwen2.5 vocabulary `</s>` is an ORDINARY BPE token (id 128247) — not
/// special, just a plain piece carrying those letters. Had we looked only at the
/// name, generation would be cut mid-sentence the moment the model wrote the
/// string `</s>` as text (which happens perfectly well when explaining XML/HTML).
/// `is_special_token` makes that distinction: the real stop token is the one ADDED
/// to the vocabulary afterwards and marked special (`<|im_end|>`, id 151645).
fn find_stop_tokens(tokenizer: &Tokenizer) -> Vec<u32> {
    let added = tokenizer.get_added_vocabulary();
    [
        "</s>",
        "<|im_end|>",
        "<|eot_id|>",
        "<|end_of_text|>",
        "<end_of_turn>",
        "<|endoftext|>",
    ]
    .iter()
    .filter(|name| added.is_special_token(name))
    .filter_map(|name| tokenizer.token_to_id(name))
    .collect()
}

/// Turns token ids into SURFACE text (the input of the constraint mask).
///
/// WHY `decode` AND NOT `id_to_token`: `id_to_token` gives the token's RAW form,
/// and the BPE families carry invisible markers there — in GPT-2 derived
/// vocabularies a space is encoded as `Ġ`, in sentencepiece as `▁`. Since the
/// grammar works character by character, feeding the raw form would set up the
/// mask wrongly from the start: the model would produce a real space while the
/// grammar saw `Ġ`, and valid JSON would be rejected. `decode` resolves that
/// encoding and gives the text that will really be written — the mask and the
/// generated text MUST speak the same alphabet.
///
/// An id that cannot be decoded becomes an empty string; `TokenMask` already
/// treats tokens with empty text as "special/neutral" and keeps them closed in the
/// mask, so special tokens cannot leak into the middle of the grammar.
///
/// NOT VERIFIED: this path has NOT YET been run with a real GGUF + tokenizer pair
/// (see STATUS.md "Known risks"). On the fake engine a code point is a token, so
/// this conversion cannot be measured there.
pub fn build_vocab(tokenizer: &Tokenizer) -> Vec<String> {
    let size = tokenizer.get_vocab_size(true);
    (0..size as u32)
        .map(|id| tokenizer.decode(&[id], false).unwrap_or_default())
        .collect()
}
