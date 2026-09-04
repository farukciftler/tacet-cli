//! DOES BATCHING GIVE THE MULTIPLIER? — a measurement, not an argument.
//!
//! WHY THIS EXISTS. Running the eval suite on a rented RTX 3090 took an hour,
//! and the first idea for making it faster — run five cases at once — was
//! measured and wins nothing: two copies of the suite side by side run at ~28
//! tok/s each where one alone runs at ~55, because at batch 1 a decode step
//! reads every weight in the model to produce ONE token, and one stream already
//! saturates the memory bus. Two streams read the weights twice for two tokens.
//!
//! BATCHING IS THE OTHER SHAPE OF THE SAME IDEA, and it is not the same trade:
//! one forward pass over a `[b, 1]` input reads the weights ONCE and produces
//! `b` tokens. The bytes moved barely change, so the throughput should scale
//! with `b` until the card runs out of arithmetic instead of bandwidth. That is
//! the "multiplier" — and where it stops is a property of the card, not
//! something to be reasoned out from a datasheet.
//!
//! WHAT IT MEASURES, deliberately narrowly: the decode loop and nothing else.
//! No tokenizer, no sampler, no grammar mask — the input tokens are a fixed
//! arbitrary pattern, and the logits are dropped. A number that included the
//! sampler would not answer the question the engine has to decide.
//!
//! MEASURED 5 SEP 2026, RTX 3090 (24 GB, CUDA 12.8), qwen3-4b Q4_K_M, 256 tokens
//! of context, 64 timed decode steps. Two runs on two different weight files
//! agreed to three significant figures, so the shape is the card's and not
//! noise:
//!
//!     batch   decode tok/s   per stream   vs batch 1
//!         1          124.0        124.0       1.00x
//!         2          196.5         98.3       1.58x
//!         4          253.2         63.3       2.04x
//!         8          264.4         33.1       2.13x
//!        16          437.4         27.3       3.53x
//!        32          503.9         15.7       4.06x
//!
//! FOUR TIMES THE TOKENS FOR THE SAME CARD — against 1.0x for the other way of
//! going wide, which is to run several copies of the program (measured in
//! `run_selection`: two streams give ~28 tok/s each where one gives ~55). The
//! difference is the whole point: `b` streams in ONE forward read the weights
//! once, `b` processes read them `b` times.
//!
//! IT IS NOT LINEAR, and the plateau at 8 is reproducible rather than a bad
//! sample. Read it as a floor on what a batched engine could give here, not a
//! promise: this loop has no sampler, no detokenizer and no grammar mask in it,
//! and a real engine pays those `b` times per step on the CPU.
//!
//! WHAT STOPS US USING IT TODAY, precisely, because "candle cannot batch" would
//! be wrong: the quantized CUDA matmul takes `[b, m, k]` and folds `b * m` into
//! its row count, and `quantized_qwen3`'s attention reads `b` from the input and
//! keeps a batched KV cache. What fails is the PREFILL — `ModelWeights::forward`
//! ends with `h.narrow(1, l - 1, 1)` to keep the last position, which for `b > 1`
//! is one strided row per sequence, and the CUDA quantized matmul then refuses
//! it with "dmmv only supports contiguous tensors". One `.contiguous()` inside
//! candle closes that. The work that is genuinely ours is an engine API that
//! decodes several sequences together, with one sampler and one grammar mask per
//! sequence, and prompts of different lengths padded and masked.
//!
//! RUN IT:
//!   cargo run --release --example batch_decode --features candle,cuda
//!   cargo run --release --example batch_decode --features candle,metal
//! `TACET_MODEL` overrides the weights; otherwise `~/models/qwen3-4b`.

use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::models::{quantized_gemma3, quantized_qwen2, quantized_qwen3};

/// The batch widths tried. 1 is the shape the engine runs today and the
/// denominator of every ratio below.
const WIDTHS: [usize; 6] = [1, 2, 4, 8, 16, 32];
/// Tokens of context established before the timed part, so the decode steps
/// carry a realistic KV cache rather than an empty one.
const PREFILL: usize = 256;
/// Timed decode steps per width. Enough that the load is steady and short
/// enough that the whole sweep is a couple of minutes.
const STEPS: usize = 64;

enum Model {
    Qwen2(quantized_qwen2::ModelWeights),
    Qwen3(quantized_qwen3::ModelWeights),
    Gemma3(quantized_gemma3::ModelWeights),
}

impl Model {
    fn forward(&mut self, input: &Tensor, position: usize) -> candle_core::Result<Tensor> {
        match self {
            Model::Qwen2(m) => m.forward(input, position),
            Model::Qwen3(m) => m.forward(input, position),
            Model::Gemma3(m) => m.forward(input, position),
        }
    }

    fn clear(&mut self) {
        match self {
            Model::Qwen2(m) => m.clear_kv_cache(),
            Model::Qwen3(m) => m.clear_kv_cache(),
            // See the note on `ArchitectureModel::clear_kv_cache`: candle's
            // gemma3 module does not offer one and does not need one.
            Model::Gemma3(_) => {}
        }
    }
}

fn model_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("TACET_MODEL") {
        return std::path::PathBuf::from(p);
    }
    let dir = format!(
        "{}/models/qwen3-4b",
        std::env::var("HOME").unwrap_or_default()
    );
    std::fs::read_dir(&dir)
        .ok()
        .and_then(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|x| x == "gguf"))
        })
        .unwrap_or_else(|| panic!("no .gguf under {dir}; set TACET_MODEL"))
}

/// CUDA, then Metal, then CPU — the same order the engine itself prefers, and
/// each `new_*` fails cleanly when the feature is not compiled in.
fn best_device() -> (Device, &'static str) {
    if let Ok(d) = Device::new_cuda(0) {
        return (d, "cuda");
    }
    if let Ok(d) = Device::new_metal(0) {
        return (d, "metal");
    }
    (Device::Cpu, "cpu")
}

fn main() -> candle_core::Result<()> {
    let path = model_path();
    let (device, device_name) = best_device();
    println!("model:  {}", path.display());
    println!("device: {device_name}");

    let mut file = std::fs::File::open(&path)?;
    let content = gguf_file::Content::read(&mut file)?;
    let architecture = content
        .metadata
        .get("general.architecture")
        .and_then(|v| v.to_string().ok())
        .cloned()
        .unwrap_or_default();
    println!("arch:   {architecture}");

    let mut model = match architecture.as_str() {
        "qwen2" => Model::Qwen2(quantized_qwen2::ModelWeights::from_gguf(
            content, &mut file, &device,
        )?),
        "qwen3" => Model::Qwen3(quantized_qwen3::ModelWeights::from_gguf(
            content, &mut file, &device,
        )?),
        "gemma3" => Model::Gemma3(quantized_gemma3::ModelWeights::from_gguf(
            content, &mut file, &device,
        )?),
        other => panic!("this measurement knows qwen2/qwen3/gemma3, not {other:?}"),
    };

    println!();
    println!("  batch   decode tok/s   per stream   vs batch 1");
    let mut baseline = 0.0f64;
    for (i, &b) in WIDTHS.iter().enumerate() {
        model.clear();

        // Prefill, ONE TOKEN AT A TIME AND NOT AS ONE `[b, PREFILL]` FORWARD,
        // which is a workaround and not a preference.
        //
        // MEASURED: the batched prefill fails on CUDA with "dmmv only supports
        // contiguous tensors" for every `b > 1`. It is not the quantized matmul
        // refusing a batch — that kernel takes `[b, m, k]` and folds `b * m`
        // into its row count. It is `ModelWeights::forward` ending with
        // `h.narrow(1, l - 1, 1)` to keep the last position: for `b = 1` that
        // slice is the tail of the buffer and contiguous, and for `b > 1` it is
        // one row out of each sequence, strided — which the CUDA quantized
        // matmul then refuses. A single `.contiguous()` inside candle would
        // close it.
        //
        // Stepping the prefill keeps `l = 1`, where the narrow is the whole
        // tensor, so the cache still ends up the same depth. It is outside the
        // timed section either way.
        let one = Tensor::ones((b, 1), candle_core::DType::U32, &device)?;
        for p in 0..PREFILL {
            let _ = model.forward(&one, p)?;
        }

        // The timed loop. `to_scalar` on one element forces the queue to drain,
        // so the wall clock covers work that actually finished rather than work
        // that was merely submitted — without it a GPU measurement times the
        // enqueue and reports a number several times too good.
        let step = one;
        let started = std::time::Instant::now();
        let mut last = None;
        for s in 0..STEPS {
            last = Some(model.forward(&step, PREFILL + s)?);
        }
        if let Some(t) = last {
            let _ = t.flatten_all()?.narrow(0, 0, 1)?.to_vec1::<f32>()?;
        }
        let elapsed = started.elapsed().as_secs_f64();

        let total = (b * STEPS) as f64 / elapsed;
        let per = total / b as f64;
        if i == 0 {
            baseline = total;
        }
        println!(
            "  {b:>5}   {total:>12.1}   {per:>10.1}   {:>8.2}x",
            total / baseline
        );
    }
    Ok(())
}
