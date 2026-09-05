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
//! is one strided row per sequence, and the matmul refuses it.
//!
//! IT IS NOT A CUDA BUG, which decides who should fix it. The same fallback
//! happens on Metal, where the message is different and the shape is the same:
//!
//!     input tensor is not contiguous
//!     Layout { shape: [2, 1, 1024], stride: [262144, 1024, 1], start_offset: 261120 }
//!
//! One row per sequence, 256 apart. Two backends refusing the same tensor for
//! the same reason means the defect is the strided narrow, not a kernel.
//!
//! THE ONE WORD, AND IT IS FREE. Adding `.contiguous()` to that narrow in a
//! local candle-transformers 0.11 flips every width from the stepped prefill to
//! the batched one, and costs nothing. MEASURED on an M-series Metal,
//! qwen3-0.6b Q4_K_M, one quiet machine, decode tok/s:
//!
//!     build            prefill    b=1    b=2    b=8    b=32
//!     stock candle     stepped   170.5  203.2  210.8  205.1
//!     patched          batched   170.5  201.1  208.5  204.7
//!
//! THAT TABLE REPLACES ONE THAT SAID THE PATCH COST 17%, and the retraction is
//! the more useful half. The earlier reading — 167 tok/s patched against 199
//! stock — was an artefact of THIS FILE. A stepped prefill runs the `[b, 1]`
//! forward 256 times before the clock starts, so every kernel for that shape is
//! already built; a batched prefill runs `[b, PREFILL]` instead and left the
//! `[b, 1]` kernels to be compiled inside a 64-step timed loop. The fix is the
//! warm-up below, and the check that it was never the KV cache is in candle:
//! `ConcatKvCache::append` calls `contiguous` and `cat` on EVERY append, so both
//! paths leave the same tensor behind.
//!
//! Two process notes, because they cost more than the measurement did. The
//! stale-binary trap: an edit that failed to compile left `cargo build | grep`
//! silent and the previous binary in place, so three runs measured a build that
//! did not match the source — hence the `contiguous()?;` grep in front of the
//! patched runs. And two of these runs overlapped on one laptop, which is worth
//! more than 30% of the throughput; they are serialised now.
//!
//! WHAT IS ACTUALLY IN THE WAY of a batched engine is not this patch. It is that
//! `ModelWeights::forward(input, offset)` takes no attention mask —
//! `causal_mask(b, tgt, offset, sw)`'s fourth argument is a SLIDING WINDOW, not
//! padding, and `forward` always passes `None`. Sequences of different lengths
//! therefore cannot share a batch: pad tokens enter the KV cache and every real
//! token attends to them. Real prompts are never the same length, so a batched
//! engine needs that upstream before it needs anything from us. What remains
//! ours after it lands is one sampler and one grammar mask per sequence.
//!
//! RUN IT:
//!   cargo run --release --example batch_decode --features candle,cuda
//!   cargo run --release --example batch_decode --features candle,metal
//! `TACET_MODEL` overrides the weights (a FILE, not a directory); otherwise
//! `~/models/qwen3-4b`. `TACET_BATCH_PREFILL=0` forces the stepped prefill on a
//! build where the batched one works, which is what tells the two costs apart.

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

        // PREFILL: THE BATCHED FORWARD IS TRIED FIRST AND THE RESULT IS
        // REPORTED, because which path ran changes what the number below means.
        //
        // On stock candle the batched prefill fails for every `b > 1` and the
        // stepped path takes over. With the one-word fix described below it
        // succeeds, and the prefill stops being O(prompt length) forwards.
        //
        // The fix is in candle, not here: `ModelWeights::forward` ends with
        // `h.narrow(1, l - 1, 1)` to keep the last position, and for `b > 1`
        // that is one strided row per sequence, which the CUDA quantized matmul
        // refuses with "dmmv only supports contiguous tensors". Adding
        // `.contiguous()` to that narrow closes it.
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
        let batched = Tensor::ones((b, PREFILL), candle_core::DType::U32, &device)?;
        // FORCING THE STEPPED PATH ON A PATCHED BUILD is what separates the two
        // things the patch could be costing. With `.contiguous()` in candle the
        // decode loop ran ~18% slower on Metal (167 vs 199 tok/s at b=2..32,
        // two runs each) — and the suspects are the copy itself, which is on
        // every decode step, or the KV cache that a one-shot batched prefill
        // leaves behind, which the stepped path builds by 256 concatenations.
        // `TACET_BATCH_PREFILL=0` holds the build still and changes only the
        // prefill, so the two are told apart by one run instead of by argument.
        let try_batched = std::env::var("TACET_BATCH_PREFILL").as_deref() != Ok("0");
        let attempt = if try_batched {
            model.forward(&batched, 0)
        } else {
            Err(candle_core::Error::Msg("forced off".into()))
        };
        let prefill_path = match attempt {
            Ok(_) => "batched".to_string(),
            Err(e) => {
                if !try_batched && b == 2 {
                    println!("  batched prefill: off by TACET_BATCH_PREFILL=0");
                }
                // THE ERROR IS PRINTED, not swallowed. "it falls back" is the
                // observation; WHICH failure it falls back from is what decides
                // whether this is one backend's kernel or the model code, and
                // the two want fixing in different places.
                if try_batched && b == 2 {
                    println!("  batched prefill refused: {e}");
                }
                // The batched attempt may have written part of the KV cache
                // before it failed, so start the stepped path from a clean one.
                model.clear();
                for p in 0..PREFILL {
                    let _ = model.forward(&one, p)?;
                }
                "stepped".to_string()
            }
        };
        // PRINTED FOR EVERY WIDTH, not once: `b == 1` always takes the batched
        // path — a single sequence's last row is already contiguous — so
        // reporting only the first width would print "batched" on stock candle
        // and hide the very thing this line exists to show.
        println!("b={b:<3} prefill: {prefill_path}");

        let step = one;

        // WARM-UP, OUTSIDE THE TIMER, AND IT IS THE POINT OF THIS EXAMPLE'S
        // SECOND FINDING. The stepped prefill runs the `[b, 1]` forward 256
        // times before the clock starts, so every kernel for that shape is
        // already compiled and cached; a batched prefill runs `[b, PREFILL]`
        // instead and leaves the `[b, 1]` kernels to be built inside the timed
        // loop. With only 64 timed steps that is not noise — it was measured as
        // a 17% "cost of the batched prefill" and blamed on the KV cache layout,
        // which `ConcatKvCache::append` rules out: it calls `contiguous` and
        // `cat` on every append, so both paths leave the same tensor.
        //
        // `TACET_WARMUP=0` turns this off to reproduce the older number.
        let warmup: usize = std::env::var("TACET_WARMUP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8);
        for w in 0..warmup {
            let t = model.forward(&step, PREFILL + w)?;
            let _ = t.flatten_all()?.narrow(0, 0, 1)?.to_vec1::<f32>()?;
        }

        // The timed loop. `to_scalar` on one element forces the queue to drain,
        // so the wall clock covers work that actually finished rather than work
        // that was merely submitted — without it a GPU measurement times the
        // enqueue and reports a number several times too good.
        let started = std::time::Instant::now();
        let mut last = None;
        for s in 0..STEPS {
            last = Some(model.forward(&step, PREFILL + warmup + s)?);
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
