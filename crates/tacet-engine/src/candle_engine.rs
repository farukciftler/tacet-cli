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
    EngineIdentity, EngineProvider, Generation, GenerationFuture, SamplingSetting, StopReason,
    boxed_generation,
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

/// The index of the highest logit.
///
/// TWO CALLERS NOW, and the second is not a diagnostic. It was written for
/// `TACET_TRACE_DUMP` ("what would the model have wanted without the mask"),
/// because `LogitsProcessor` cannot be reused — it has state and the seed
/// advances. It is now also THE GREEDY SAMPLER on the constrained path: see the
/// note at the call site for the `-inf` bit pattern candle's Metal argmax
/// handed back as a token id.
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

pub use crate::token::Device;

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

/// Where the tokenizer of a loaded engine actually came from.
///
/// PUBLIC AND RECORDED because the two sources are indistinguishable from the
/// output: a tokenizer built from the wrong place does not error, it produces
/// text that looks like a broken model. The user has to be able to see which one
/// was used before spending an hour blaming the weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerSource {
    /// A `tokenizer.json` the user put there (or discovery found).
    File,
    /// The tokenizer carried inside the GGUF's own metadata.
    Gguf,
}

impl TokenizerSource {
    pub fn name(self) -> &'static str {
        match self {
            TokenizerSource::File => "tokenizer.json",
            TokenizerSource::Gguf => "gguf metadata",
        }
    }
}

/// Model loading settings.
#[derive(Debug, Clone)]
pub struct ModelSetting {
    /// The GGUF weight file (local).
    pub model_path: PathBuf,
    /// `tokenizer.json` (local). `None` means "use the tokenizer inside the
    /// GGUF" — see `ModelSetting::from_gguf`.
    pub tokenizer_path: Option<PathBuf>,
    pub device: Device,
    /// The token ids that stop generation. Left empty, common names are looked up
    /// in the tokenizer's vocabulary.
    pub stop_tokens: Vec<u32>,
}

impl ModelSetting {
    /// Weights + an EXPLICIT `tokenizer.json`.
    ///
    /// THE FILE GIVEN HERE WINS over the one inside the GGUF, and if it is
    /// missing the load FAILS rather than quietly falling back: the user named a
    /// path, and silently using something else would turn a typo into an
    /// unexplainable difference in output.
    pub fn new(model_path: impl Into<PathBuf>, tokenizer_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            tokenizer_path: Some(tokenizer_path.into()),
            device: Device::default(),
            stop_tokens: Vec::new(),
        }
    }

    /// Weights ALONE — the tokenizer is read out of the GGUF's own metadata.
    ///
    /// This is what makes a single downloaded `.gguf` a complete package: the
    /// vocabulary, the merges and the special tokens are already in the file (see
    /// `gguf_tokenizer`), and demanding a separate `tokenizer.json` next to it
    /// was us not reading data we already had.
    pub fn from_gguf(model_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            tokenizer_path: None,
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
    /// WHERE the tokenizer came from — see `TokenizerSource`.
    tokenizer_source: TokenizerSource,
    /// The window the GGUF declares (`<arch>.context_length`), when it does.
    /// READ ONCE at load: this is what turns the fixed 4096 into a number derived
    /// from the model actually loaded.
    context_length: Option<usize>,
    device: CandleDevice,
    stop_tokens: Vec<u32>,
    /// Token id -> SURFACE text. The prerequisite for setting up a constraint
    /// (see `vocab`). Produced once at load time: a decode call for 32k tokens is
    /// not cheap, but it is invisible next to the gguf load and repeating it per
    /// generation would be pointless.
    vocab: Vec<String>,
    /// WHAT this engine is, recorded at load. Built once: hashing and metadata
    /// reading happen while the file is open anyway.
    identity: EngineIdentity,
}

impl CandleEngine {
    /// Loads the weights and the tokenizer from LOCAL files.
    pub fn load(setting: &ModelSetting) -> EngineResult<Self> {
        let device = match setting.device {
            Device::Cpu => CandleDevice::Cpu,
            Device::Metal => CandleDevice::new_metal(0)
                .map_err(|e| EngineError::Inference(format!("could not open metal device: {e}")))?,
            Device::Cuda => CandleDevice::new_cuda(0)
                .map_err(|e| EngineError::Inference(format!("could not open cuda device: {e}")))?,
        };

        let mut file = std::fs::File::open(&setting.model_path)
            .map_err(|_| EngineError::ModelNotLoaded(setting.model_path.clone()))?;
        let content = gguf_file::Content::read(&mut file)
            .map_err(|_| EngineError::ModelNotLoaded(setting.model_path.clone()))?;

        // THE ARCHITECTURE IS READ BEFORE THE WEIGHTS. The metadata is already in
        // memory; starting a 2.5 GB load with the wrong module and only erroring
        // at the end is pointless.
        let architecture = read_architecture(&content)?;
        // THE QUANTIZATION, read from the FILE rather than from its name. A
        // matrix cell labelled by a folder name is labelled by whatever somebody
        // typed; this is what is actually in the tensors.
        let quant = Self::dominant_quant(&content);
        if setting.device == Device::Cuda {
            Self::validate_cuda_quant(&quant, &setting.model_path)?;
        }
        // READ BEFORE THE WEIGHTS, for the same reason: the metadata is in memory
        // right now and `from_gguf` below consumes `content`.
        let context_length = read_context_length(&content);

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

        let (tokenizer, tokenizer_source) = resolve_tokenizer(setting)?;

        let stop_tokens = if setting.stop_tokens.is_empty() {
            find_stop_tokens(&tokenizer)
        } else {
            setting.stop_tokens.clone()
        };

        let vocab = build_vocab(&tokenizer);

        let identity = EngineIdentity {
            engine: "candle".into(),
            model_path: setting.model_path.display().to_string(),
            model_fingerprint: Self::file_fingerprint(&setting.model_path),
            model_bytes: std::fs::metadata(&setting.model_path)
                .map(|m| m.len())
                .unwrap_or(0),
            quant,
            architecture: architecture.name().to_string(),
            device: match setting.device {
                Device::Cpu => "cpu".into(),
                Device::Metal => "metal".into(),
                Device::Cuda => "cuda".into(),
            },
        };

        Ok(Self {
            model: Mutex::new(model),
            architecture,
            tokenizer,
            tokenizer_source,
            context_length,
            device,
            stop_tokens,
            vocab,
            identity,
        })
    }

    /// Pre-checks if the GGUF quantization format is supported on CUDA before loading weights.
    fn validate_cuda_quant(quant: &str, model_path: &Path) -> EngineResult<()> {
        let is_supported = matches!(
            quant,
            "Q4_0"
                | "Q4_1"
                | "Q5_0"
                | "Q5_1"
                | "Q8_0"
                | "Q8_1"
                | "Q2_K"
                | "Q3_K"
                | "Q4_K"
                | "Q5_K"
                | "Q6_K"
                | "Q8_K"
                | "F16"
                | "F32"
                | "BF16"
        );
        if !is_supported {
            return Err(EngineError::Inference(format!(
                "CUDA backend does not support GGUF quantization format '{quant}' in file '{}'. \
                 (Supported CUDA formats: Q4_K, Q6_K, Q8_0, Q4_0, Q5_0, Q5_K, Q2_K, Q3_K, Q8_K, F16, F32)",
                model_path.display()
            )));
        }
        Ok(())
    }

    /// The tensor type most of the weights are in.
    ///
    /// A GGUF mixes types on purpose (the embedding and the output head are often
    /// kept at higher precision than the body), so "the quantization" is the MODE,
    /// not a single value — and that is exactly the thing a mixed-precision recipe
    /// would change, which is why it is read from the tensors rather than off the
    /// file name.
    ///
    /// THE MODE IS TAKEN OVER BYTES, NOT OVER TENSOR COUNT, and the difference is
    /// not cosmetic. Counting tensors, Gemma-3-4B-It reported **F32** — for
    /// Unsloth's 2.49 GB q4 file. Gemma3 carries six small F32 norm tensors per
    /// layer against seven quantized matrices, and across 34 layers the norms win
    /// the head count while holding a rounding error's worth of the file. Anybody
    /// reading that cell of a comparison matrix would conclude the model had been
    /// run unquantized, and would compare it against the others as if it had.
    ///
    /// Weighting by `elem_count × the type's own byte size` asks the question that
    /// was meant all along — what are the WEIGHTS stored as — and a handful of
    /// norm vectors can no longer outvote the body of the model.
    fn dominant_quant(content: &gguf_file::Content) -> String {
        let mut bytes: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for info in content.tensor_infos.values() {
            let elems = info.shape.elem_count();
            let block = info.ggml_dtype.block_size().max(1);
            let size = info.ggml_dtype.type_size();
            *bytes.entry(format!("{:?}", info.ggml_dtype)).or_default() += elems / block * size;
        }
        bytes
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(name, _)| name)
            .unwrap_or_else(|| "unknown".into())
    }

    /// A CHEAP FINGERPRINT of a weight file: sha256 over its size, its first
    /// mebibyte and its last mebibyte.
    ///
    /// WHY NOT THE WHOLE FILE: a full digest of 2.3 GB costs more than the eval it
    /// is meant to label, and it would be paid on every single load. What this has
    /// to answer is "is this the same file as the previous cell of the matrix",
    /// and for that the size plus both ends is decisive in practice — two quants of
    /// the same weights differ in size, and two builds of the same quant differ in
    /// their header. It is NOT a guarantee against a file crafted to collide, and
    /// the field is named `fingerprint` rather than `sha256` so nobody reads more
    /// into it than it carries.
    fn file_fingerprint(path: &Path) -> String {
        use std::io::{Read, Seek, SeekFrom};
        const EDGE: u64 = 1024 * 1024;
        let Ok(mut file) = std::fs::File::open(path) else {
            return String::new();
        };
        let Ok(size) = file.metadata().map(|m| m.len()) else {
            return String::new();
        };
        let mut hasher = tacet_kernel::Sha256::new();
        hasher.feed(&size.to_le_bytes());
        let mut head = vec![0u8; EDGE.min(size) as usize];
        if file.read_exact(&mut head).is_err() {
            return String::new();
        }
        hasher.feed(&head);
        if size > EDGE {
            let tail_len = EDGE.min(size - EDGE);
            if file.seek(SeekFrom::End(-(tail_len as i64))).is_err() {
                return String::new();
            }
            let mut tail = vec![0u8; tail_len as usize];
            if file.read_exact(&mut tail).is_err() {
                return String::new();
            }
            hasher.feed(&tail);
        }
        tacet_kernel::hash::hex(&hasher.finish())
    }

    /// The architecture of the loaded GGUF — for diagnostics and shell output.
    pub fn architecture(&self) -> Architecture {
        self.architecture
    }

    /// The context window the loaded GGUF declares, in tokens.
    ///
    /// PUBLIC for the same reason as `stop_tokens` and `tokenizer_source`: a
    /// window silently falling back to the default is invisible from the output —
    /// the session just starts forgetting turns earlier than it should. The shell
    /// prints this next to the architecture.
    pub fn context_length(&self) -> Option<usize> {
        self.context_length
    }

    /// Verifies the files exist BEFORE loading — the gguf load takes a long time
    /// and learning about a missing file at the end of it is a pointless wait.
    ///
    /// IT ASKS THE SAME QUESTION `load` WILL. When no `tokenizer.json` is given,
    /// the check that has to pass is "does the GGUF carry a tokenizer we can
    /// rebuild" — if this function and `load` disagreed, the pre-check would
    /// reject exactly the packages the loader can handle (or wave through ones it
    /// cannot), which is worse than having no pre-check at all.
    pub fn files_exist(setting: &ModelSetting) -> EngineResult<()> {
        if !Path::new(&setting.model_path).is_file() {
            return Err(EngineError::ModelNotLoaded(setting.model_path.clone()));
        }
        match &setting.tokenizer_path {
            Some(path) => {
                if !Path::new(path).is_file() {
                    return Err(EngineError::ModelNotLoaded(path.clone()));
                }
            }
            None => {
                if !crate::gguf_tokenizer::gguf_has_tokenizer(&setting.model_path) {
                    return Err(EngineError::Tokenization(format!(
                        "no tokenizer.json was given and '{}' does not carry a tokenizer we can \
                         rebuild",
                        setting.model_path.display()
                    )));
                }
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

    /// Which of the two sources the tokenizer was actually built from.
    ///
    /// PUBLIC for the same reason as `stop_tokens`: the failure mode is silent.
    /// The shell prints this next to the architecture.
    pub fn tokenizer_source(&self) -> TokenizerSource {
        self.tokenizer_source
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
        // `greedy` IS REMEMBERED BEFORE THE SAMPLER TAKES OWNERSHIP: the
        // constrained branch below decides the argmax itself (see the note
        // there) and needs to know which mode is in force.
        let greedy = sampling == Sampling::ArgMax;
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

        // A TOOL CALL IS NOT THE SIZE OF THE CONTEXT WINDOW.
        //
        // The comment further down already accepts that a model which will not
        // close its arguments runs to `max_tokens` and reports "cut off
        // halfway" — an honest failure. What was never checked is what that
        // costs: callers pass `TokenCounter::generation_cap`, which is the whole
        // remaining window, so on qwen3-4b it was 14 041 tokens. At the 15-18
        // tok/s measured on this machine that honest failure takes FIFTEEN
        // MINUTES, during which the shell shows nothing.
        //
        // THE NUMBER COMES FROM MEASUREMENT, not from taste. Across the 115-case
        // selection suite the largest legitimate constrained generation was 1523
        // tokens (`write_code-converter`, a whole python script); the next were
        // 909 and 272. 2048 clears the largest by a third and turns the runaway
        // from fifteen minutes into about two.
        //
        // ONLY WHEN CONSTRAINED. Free prose is the answer to the user and has no
        // such natural size; it keeps the caller's budget.
        const TOOL_CALL_CAP: usize = 2048;
        let setting = if constraint.is_some() {
            SamplingSetting {
                max_tokens: setting.max_tokens.min(TOOL_CALL_CAP),
                ..setting
            }
        } else {
            setting
        };
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
                // THE STOP TOKEN IS PART OF THE MASK, and leaving it out was a
                // hole straight through this project's headline claim.
                //
                // README: "Malformed JSON, a field that isn't in the schema, a
                // value out of range — none of them can be GENERATED. Not
                // validated after the fact: unrepresentable." The grammar
                // delivers that for every ORDINARY token, because `mask` walks
                // the automaton. It never covered the one token that is not
                // ordinary: end-of-turn. Nothing forbade it, so the model could
                // simply STOP in the middle of a JSON string and the text handed
                // to the parser was malformed after all.
                //
                // MEASURED, qwen3-4b, the selection suite's `write_code`
                // web-scraper case. The model wrote a correct call and ended it
                // like this:
                //
                //     ... print(result]})
                //
                // — the string holding the script was never closed, the array
                // and object were never closed, and `ToolCall::parse` returned
                // `None`. From the outside it looked like the model had refused
                // to call the tool; the model had done its part and the decoder
                // had let it stop.
                //
                // ONCE INSIDE `tool(` THE ONLY WAY OUT IS `)`. That is what
                // `is_structural` means here — it is true for the whole
                // argument region, not only where the automaton is mid-token —
                // so the stop tokens are closed for all of it. The model can
                // still finish whenever it likes: closing the JSON and writing
                // `)` leaves the structural region and hands the stop token
                // back. What it can no longer do is walk away from an open
                // brace.
                //
                // THE COST IS A HONEST FAILURE INSTEAD OF A SILENT ONE. A model
                // that will not close its arguments now runs to `max_tokens`
                // and the turn reports "generation was cut off halfway", which
                // is what actually happened, rather than delivering unparseable
                // text that reads as "no tool call".
                if structural {
                    for id in &self.stop_tokens {
                        if let Some(slot) = raw.get_mut(*id as usize) {
                            *slot = f32::NEG_INFINITY;
                        }
                    }
                }
                // If everything was forbidden the sampler would make a meaningless
                // choice (a NaN probability distribution); this is a grammar bug
                // and must not be glossed over.
                if !raw.iter().any(|v| v.is_finite()) {
                    return Err(EngineError::Inference(
                        "the constraint forbade every token".into(),
                    ));
                }
                // GREEDY IS DONE HERE, NOT ON THE GPU, and it is a correctness
                // fix rather than an optimisation.
                //
                // MEASURED, qwen3-4b on Metal, immediately after the stop token
                // joined the mask: a turn died with
                //
                //     engine error: constraint rejected the token: 4286578688
                //
                // 4286578688 is 0xFF800000 — the BIT PATTERN OF `-inf` read as
                // a `u32`. It is not a token id at all; no vocabulary has four
                // billion entries. Candle's `sample_argmax` is
                // `logits.argmax(D::Minus1)?.to_scalar::<u32>()`, and on the
                // Metal backend that reduction hands back the extremum's BITS
                // rather than its INDEX once the distribution is mostly
                // `-inf` — which is exactly the shape a grammar mask produces,
                // and which got several times more common when the stop token
                // started being masked too.
                //
                // We already hold the masked logits in `raw` (the mask is
                // applied on the CPU side), so the round trip to the GPU was
                // buying nothing but this bug. `largest_index` is the same
                // `total_cmp` argmax written out, and its result is an index
                // into a slice BY CONSTRUCTION.
                //
                // THE SAMPLED PATHS STILL GO THROUGH CANDLE, because they need
                // its seeded rng, and they do not take the argmax branch.
                let chosen = if greedy {
                    largest_index(&raw) as u32
                } else {
                    let masked = Tensor::new(raw.as_slice(), &self.device)
                        .map_err(|e| EngineError::Inference(e.to_string()))?;
                    sampler
                        .sample(&masked)
                        .map_err(|e| EngineError::Inference(e.to_string()))?
                };
                // AND A BOUND ON WHATEVER CAME BACK. The constraint reports an
                // out-of-range id as "constraint rejected the token", which
                // names the wrong layer: the grammar did not reject anything,
                // it was handed something that is not a token. Saying so here
                // is the difference between a bug report about the grammar and
                // one about the sampler.
                if chosen as usize >= self.vocab.len() {
                    return Err(EngineError::Inference(format!(
                        "the sampler returned {chosen}, which is not a token id \
                         (the vocabulary has {} entries)",
                        self.vocab.len()
                    )));
                }
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

    fn identity(&self) -> EngineIdentity {
        self.identity.clone()
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

    /// The window READ FROM the GGUF at load time — see `context_length`.
    fn context_length(&self) -> Option<usize> {
        self.context_length
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

/// Reads `<arch>.context_length` from the already-parsed GGUF metadata.
///
/// A MISSING KEY IS NOT AN ERROR, unlike `general.architecture`. The architecture
/// is mandatory in the format and loading without it is impossible; the window is
/// only an optimisation over the default 4096, and refusing to load a model
/// because its converter left the key out would turn a smaller context into no
/// context at all.
///
/// THE KEY IS NOT SPELLED OUT HERE either — the rule lives in
/// `gguf_tokenizer::is_context_length_key`, so the loader and discovery cannot
/// disagree about which key that is.
fn read_context_length(content: &gguf_file::Content) -> Option<usize> {
    content
        .metadata
        .iter()
        .find(|(key, _)| crate::gguf_tokenizer::is_context_length_key(key))
        // `to_u64` upcasts any narrower unsigned width, which is what the
        // converters actually write (u32 in all four files on this machine).
        .and_then(|(_, value)| value.to_u64().ok())
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

/// THE TOKENIZER PRIORITY, in one place.
///
/// 1. A `tokenizer.json` given explicitly. THE USER'S OWN FILE WINS — someone who
///    names a path has usually named it because the one in the weights is wrong
///    or older, and overruling that would make the override useless. If the path
///    is given but not there, this is an ERROR and NOT a fallback: a typo must
///    not turn into "it silently used a different vocabulary".
/// 2. Otherwise the tokenizer inside the GGUF (`gguf_tokenizer`).
///
/// The order lives here rather than at the call sites so `files_exist` and the
/// load path cannot drift apart.
fn resolve_tokenizer(setting: &ModelSetting) -> EngineResult<(Tokenizer, TokenizerSource)> {
    match &setting.tokenizer_path {
        Some(path) => {
            let tokenizer = Tokenizer::from_file(path)
                .map_err(|e| EngineError::Tokenization(format!("{}: {e}", path.display())))?;
            Ok((tokenizer, TokenizerSource::File))
        }
        None => {
            let tokenizer = crate::gguf_tokenizer::tokenizer_from_gguf(&setting.model_path)?;
            Ok((tokenizer, TokenizerSource::Gguf))
        }
    }
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
/// THE MARKER CLAIM IS NOW MEASURED AND IT HOLDS (tests/vocab_alphabet.rs,
/// macOS arm64, 4 Sep 2026). qwen3-4b and qwen3-8b (151669 ids) and
/// qwen2.5-3b-instruct-q4_k_m (151665), each read from BOTH sources — the
/// tokenizer rebuilt out of the GGUF and the `tokenizer.json` lying next to it:
/// 53021 raw forms begin with `Ġ` and ALL 53021 surfaces begin with a real
/// U+0020. gemma3-4b (262145 ids; its GGUF carries a sentencepiece vocabulary
/// `tokenizer_from_gguf` refuses, so only the File source is measurable): 137541
/// raw forms begin with `▁`, all 137541 surfaces begin with a space. Not one
/// marker survives into the surface. Two ids per Qwen file still CONTAIN a
/// marker character — 144242 (raw `Äł`) and 148848 (raw `ÄĬ`) — and they are not
/// leaks: their text genuinely IS the one character U+0120 / U+010A, which is
/// what `decode` correctly produced.
///
/// `skip_special_tokens` IS `true` HERE, AND IT USED TO BE `false`. THE SECOND
/// CLAIM THIS COMMENT MADE WAS FALSE UNTIL IT WAS MEASURED. It read: "an id that
/// cannot be decoded becomes an empty string; `TokenMask` treats tokens with
/// empty text as special/neutral and keeps them closed in the mask, so special
/// tokens cannot leak into the middle of the grammar." Measured with `false`:
/// ZERO ids of 151669 / 151665 / 262145 decoded to an empty string —
/// `unwrap_or_default` never fired once — so `TokenMask::empty_tokens()` was
/// EMPTY and the mechanism that whole sentence rests on had never run. What a
/// special token got instead was its literal name: `vocab[151645]` was the ten
/// ASCII characters `<|im_end|>`, which the trie offers like any other text
/// inside a free string value. `run_loop` then deletes them again — what it
/// hands back is `self.decode(&produced)`, i.e. `decode(skip_special_tokens =
/// TRUE)` — so the grammar was counting characters the answer does not contain.
/// The stop-token mask covered exactly two of them (`<|im_end|>`,
/// `<|endoftext|>`); the other 18 added-special ids of qwen3-4b were open.
/// `tacet-cli/tests/mask_alphabet.rs` drives that leak on the old vocabulary and
/// shows it closed on this one.
///
/// THE FLIP IS SAFE BECAUSE THE DIFFERENCE SET WAS MEASURED, not assumed: the
/// ids whose surface changes between `false` and `true` are EXACTLY the
/// added-special ones and no others — 20 of 151669 from the GGUF, 14 from
/// qwen3-4b's `tokenizer.json` (the GGUF additionally marks the six FIM/repo
/// markers CONTROL, see tests/gguf_tokenizer.rs), 9 of 262145 for gemma3-4b —
/// and every one of them becomes empty. `<think>` is NOT among them: it is an
/// added token but not a special one, so it keeps its text and `extract_thinking`
/// still sees it. The sweep also got cheaper, 236 ms -> 210 ms for 151669 ids.
///
/// WHAT IS STILL NOT TRUE, AND CANNOT BE FIXED HERE: decoding ONE id at a time is
/// lossy for the 1448 Qwen ids (3151 for gemma3-4b) whose raw form is a fragment
/// of a multi-byte character. Their surface is U+FFFD, so on text that tokenizes
/// through them — CJK, emoji ZWJ sequences — the grammar's character stream and
/// the delivered text DIVERGE. It is BOUNDED rather than removed: of the 1457
/// U+FFFD-carrying surfaces NOT ONE contains a JSON structural character
/// (`{}[]":,`) and exactly one, id 94825 (` (` + U+FFFD), contains a parenthesis
/// — so a byte fragment cannot open a structure the answer does not contain.
/// vocab_alphabet.rs asserts that bound rather than hoping for it.
///
/// NO CI JOB RE-RUNS ANY OF THIS, and the dates above are therefore the whole
/// provenance. Every measurement here needs a multi-gigabyte GGUF and the two
/// files that check it (`tests/vocab_alphabet.rs`,
/// `tacet-cli/tests/mask_alphabet.rs`) are `#![cfg(feature = "candle")]`, which
/// no PR job builds; the nightly's `candle-compiles` job only proves they still
/// COMPILE. Without weights they print SKIPPED and return. So this comment is a
/// dated local measurement, not a standing guarantee — re-run it (the command is
/// at the top of vocab_alphabet.rs) before trusting it against a tokenizer
/// family that is not listed above.
pub fn build_vocab(tokenizer: &Tokenizer) -> Vec<String> {
    let size = tokenizer.get_vocab_size(true);
    (0..size as u32)
        .map(|id| tokenizer.decode(&[id], true).unwrap_or_default())
        .collect()
}

#[cfg(test)]
mod cuda_quant_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn cuda_quant_precheck_accepts_supported_formats() {
        let path = Path::new("model.gguf");
        for quant in ["Q4_K", "Q6_K", "Q8_0", "Q4_0", "Q5_0", "Q5_K", "F16", "F32"] {
            assert!(CandleEngine::validate_cuda_quant(quant, path).is_ok());
        }
    }

    #[test]
    fn cuda_quant_precheck_rejects_unsupported_formats() {
        let path = Path::new("model.gguf");
        for quant in ["IQ2_XXS", "IQ3_S", "IQ1_M", "UNKNOWN"] {
            let res = CandleEngine::validate_cuda_quant(quant, path);
            assert!(res.is_err());
            let err = res.unwrap_err().to_string();
            assert!(err.contains("CUDA backend does not support GGUF quantization format"));
            assert!(err.contains(quant));
        }
    }
}

#[cfg(test)]
mod greedy_tests {
    use super::*;

    /// THE PROPERTY THE CONSTRAINED PATH NOW RESTS ON: over a vector shaped
    /// like a grammar mask — almost everything `-inf`, a handful of real
    /// values — the greedy choice is an INDEX INTO THAT VECTOR.
    ///
    /// It is written out because the thing it replaced did not have this
    /// property. `sample_argmax` in candle is
    /// `logits.argmax(D::Minus1)?.to_scalar::<u32>()`, and on Metal that came
    /// back as 4286578688 — `0xFF800000`, the bit pattern of `-inf` — which the
    /// grammar then reported as "constraint rejected the token". No vocabulary
    /// has four billion entries; it was never a token id.
    ///
    /// THE MASK SHAPE IS THE POINT. An ordinary logit vector never hits this:
    /// the bug needs a distribution that is mostly `-inf`, which is exactly and
    /// only what a tool-call grammar produces — and which got several times
    /// more common when the stop token joined the mask.
    #[test]
    fn the_greedy_choice_over_a_masked_vector_is_a_valid_index() {
        let mut logits = vec![f32::NEG_INFINITY; 4096];
        logits[3971] = -12.5;
        logits[12] = -40.0;
        let picked = largest_index(&logits);
        assert!(picked < logits.len(), "{picked} is not an index");
        assert_eq!(picked, 3971);

        // The degenerate case still answers with an index rather than a bit
        // pattern. The generation loop refuses this vector one step earlier
        // ("the constraint forbade every token"); what matters here is that the
        // fallback is 0 and not something out of range.
        let all_closed = vec![f32::NEG_INFINITY; 64];
        assert!(largest_index(&all_closed) < all_closed.len());

        // And an empty slice must not panic — `unwrap_or(0)` is load-bearing.
        assert_eq!(largest_index(&[]), 0);
    }
}
