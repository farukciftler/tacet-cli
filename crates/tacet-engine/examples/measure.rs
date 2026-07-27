//! A real-model measurement run — compiled with `--features candle`.
//!
//! WHY AN EXAMPLE AND NOT A TEST: this run needs a 2 GB weight file and takes
//! minutes. Put inside `cargo test` it would either go red in CI for lack of a
//! model or sit there as an `ignore`d test producing an illusion of green. As an
//! example, running it is a DELIBERATE act.
//!
//! Run:
//!   TACET_MODEL=... TACET_TOKENIZER=... \
//!     cargo run -p tacet-engine --features candle --release --example measure

use std::time::Instant;
use tacet_engine::prompt::Template;
use tacet_engine::{
    EngineProvider, Generation, Prompt, SamplingSetting, StopReason, TokenCounter, Turn, wait,
};

fn main() {
    // The variable names are THE SAME as on the production path (`MODEL_VAR` /
    // `TOKENIZER_VAR` in `tacet-cli`). If the measurement runs with an
    // environment other than the one the user set up, what it measures is not
    // production.
    let Ok(model) = std::env::var("TACET_MODEL") else {
        eprintln!("TACET_MODEL must be set");
        std::process::exit(2);
    };
    let Ok(tokenizer) = std::env::var("TACET_TOKENIZER") else {
        eprintln!("TACET_TOKENIZER must be set");
        std::process::exit(2);
    };

    // The device is chosen FROM THE ENVIRONMENT; the default is the CPU. When
    // `metal` is asked for, the crate's `metal` feature must be on as well —
    // otherwise it errors out openly, it does NOT silently FALL BACK to the CPU
    // (see `Device::Metal`).
    let device = match std::env::var("TACET_DEVICE").as_deref() {
        Ok("metal") => tacet_engine::candle_engine::Device::Metal,
        _ => tacet_engine::candle_engine::Device::Cpu,
    };
    let setting = tacet_engine::ModelSetting::new(&model, &tokenizer).with_device(device);
    println!("device requested : {device:?}");

    // --- 1. Load time ---
    let t = Instant::now();
    let engine = match tacet_engine::CandleEngine::load(&setting) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("load failed: {e}");
            std::process::exit(1);
        }
    };
    let load = t.elapsed();
    println!("== LOAD ==");
    println!("duration      : {:.2} s", load.as_secs_f64());
    println!("template      : {:?}", engine.template());

    // --- 2. Vocabulary: `build_vocab` measured with the real tokenizer ---
    // The COST of `build_vocab` is measured separately: `decode`ing 32k tokens
    // one by one is invisible inside the load but may not be cheap.
    let t = Instant::now();
    let _ = tacet_engine::candle_engine::build_vocab(engine.tokenizer());
    let build_time = t.elapsed();

    let t = Instant::now();
    let vocab = engine
        .vocab()
        .expect("the candle engine declares a vocabulary");
    let vocab_time = t.elapsed();
    println!("\n== build_vocab COST ==");
    println!(
        "build         : {:.1} ms",
        build_time.as_secs_f64() * 1000.0
    );
    let empty = vocab.iter().filter(|s| s.is_empty()).count();
    println!("\n== VOCABULARY (build_vocab) ==");
    println!("size          : {}", vocab.len());
    println!(
        "cloning       : {:.1} ms",
        vocab_time.as_secs_f64() * 1000.0
    );
    println!("empty texts   : {empty}  (special/control tokens)");
    // These samples are the critical part: the grammar works character by
    // character, so if `Ġ` or `▁` shows up here the mask was set up wrongly from
    // the start.
    for id in [15u32, 264, 5867, 314, 341, 8, 9, 314] {
        if let Some(s) = vocab.get(id as usize) {
            println!("  id {id:<6} -> {s:?}");
        }
    }
    let marked = vocab
        .iter()
        .filter(|s| s.contains('Ġ') || s.contains('▁'))
        .count();
    println!("BPE marked    : {marked}  (must be 0 — decode should resolve the markers)");

    // Do the merged tokens really exist (the risk noted in STATUS.md)?
    println!("\n== MERGED TOKENS (candidates that cross the grammar boundary) ==");
    for candidate in [
        "\"})", "\"}", "})", "\":", "\": \"", "\",", "\":\"", "({", "()",
    ] {
        let found: Vec<usize> = vocab
            .iter()
            .enumerate()
            .filter(|(_, s)| s.as_str() == candidate)
            .map(|(i, _)| i)
            .collect();
        println!("  {candidate:<8?} -> {found:?}");
    }

    // --- 3. Was a stop token really found ---
    println!("\n== STOP TOKENS ==");
    println!("{:?}", engine.stop_tokens());

    // --- 4. Generation speed ---
    // The performance prompt asks for a LONG answer: a short answer stops early
    // on a stop token and the difference method (below) cannot be applied.
    let prompt = Prompt::new(
        tacet_engine::SYSTEM_INSTRUCTIONS,
        "Describe a city trip in full detail, at length.",
    );
    println!("\n== PROMPT, in the engine's template (first 320 chars) ==");
    // The template is taken FROM THE ENGINE, not hard-coded: with
    // multi-architecture loading (Qwen -> ChatML, Gemma -> Gemma) a fixed ChatML
    // would feed Gemma THE WRONG fences and the measurement would measure a broken
    // run rather than real generation.
    let wire = prompt.text_with_template(engine.template());
    println!("{}", &wire[..wire.len().min(320)]);
    let prompt_size = engine
        .tokenizer()
        .encode(wire.as_str(), true)
        .map(|e| e.get_ids().len())
        .unwrap_or(0);

    // PREFILL AND DECODE ARE MEASURED SEPARATELY. A single "tokens per second"
    // number misleads: on short answers almost all the time goes into processing
    // the prompt once and the engine looks far slower than it is. Generating N and
    // 2N tokens from the same prompt and taking THE DIFFERENCE cancels out the
    // prefill; what is left is the pure decode speed.
    let measure = |n: usize| -> (f64, Generation) {
        let t = Instant::now();
        let g = wait(engine.generate(
            &prompt,
            None,
            SamplingSetting {
                max_tokens: n,
                ..Default::default()
            },
        ))
        .expect("generation");
        (t.elapsed().as_secs_f64(), g)
    };

    // WARM-UP RUN — NOT PART of the measurement. GGUF weights are lazily paged in
    // from disk; the first generation alone pays the 2 GB page-in cost. Without a
    // warm-up that cost is charged to "prefill" and produced a nonsense result:
    // prefill looked SLOWER per token than decode, whereas prefill is processed in
    // a batch and is always faster.
    let _ = measure(8);

    let (t1, g1) = measure(32);
    let (t2, g2) = measure(64);
    // The difference is only meaningful if BOTH runs hit the cap. Greedy
    // generation is reproducible, so both write the same text; if one stops early
    // on a stop token the difference becomes "0 tokens / negative time" and the
    // measurement silently talks nonsense. Hence the condition is checked openly.
    let hit_the_cap = g1.stop == StopReason::Length && g2.stop == StopReason::Length;
    let decode = if hit_the_cap && g2.token_count > g1.token_count {
        (g2.token_count - g1.token_count) as f64 / (t2 - t1)
    } else {
        f64::NAN
    };
    let prefill = t1 - g1.token_count as f64 / decode;
    if !hit_the_cap {
        println!(
            "(note: generation finished before the cap — the difference method could not be \
             applied; only the end-to-end speed is meaningful)"
        );
    }

    println!("\n== PERFORMANCE ==");
    println!("prompt tokens     : {}", prompt_size);
    println!(
        "prefill           : {prefill:.2} s  ({:.1} tokens/s)",
        prompt_size as f64 / prefill
    );
    println!("decode            : {decode:.2} tokens/s");
    println!("32 tokens total   : {t1:.2} s");
    println!("64 tokens total   : {t2:.2} s");

    // --- 5. Context budget truncation with the REAL tokenizer ---
    //
    // The estimate (`TokenCounter`) rests on a fixed ratio per character; here we
    // ACTUALLY tokenize the truncated prompt and check whether it stays under the
    // cap. If the estimate comes out LOW the model gets the prompt cut off in the
    // middle — a silent, undiagnosable failure.
    println!("\n== CONTEXT BUDGET TRUNCATION ==");
    let counter = TokenCounter::default();
    // THE FILLER SENTENCE STAYS TURKISH ON PURPOSE. `TokenCounter`'s
    // bytes-per-token constant was measured on Turkish prose (2.71 bytes/token);
    // filling this history with English would make the run measure a sparser
    // text than the constant was derived from, and the check would pass for the
    // wrong reason.
    let mut long = Prompt::new(
        tacet_engine::SYSTEM_INSTRUCTIONS,
        "Summarise: what is the state?",
    )
    .with_history((0..200).map(|i| {
        Turn::user(format!(
            "turn {i}: a fairly long sentence written to fill the context window of an \
                 assistant running on the device."
        ))
    }));
    println!(
        "estimate before truncation : {}",
        counter.prompt_estimate(&long)
    );
    let report = counter.truncate(&mut long);
    println!("dropped turns              : {}", report.dropped_turns);
    println!("guide dropped              : {}", report.guide_dropped);
    println!("question truncated         : {}", report.question_truncated);
    println!("estimate after truncation  : {}", report.final_estimate);
    println!("prompt cap                 : {}", counter.prompt_cap());
    println!(
        "validate()                 : {:?}",
        counter.validate(&long).is_ok()
    );

    // The REAL token count of the two templates — the estimate is made over the
    // plain format while generation sends ChatML. The difference shows up here.
    let real = |t: Template| -> usize {
        engine
            .tokenizer()
            .encode(long.text_with_template(t), true)
            .map(|e| e.get_ids().len())
            .unwrap_or(0)
    };
    let plain_real = real(Template::Plain);
    let chatml_real = real(Template::ChatML);
    println!("REAL tokens (plain)   : {plain_real}");
    println!("REAL tokens (ChatML)  : {chatml_real}");
    println!(
        "did ChatML fit the cap: {}",
        chatml_real <= counter.prompt_cap()
    );

    let (duration, generation) = measure(160);
    println!("\n== GENERATION ==");
    println!("tokens        : {}", generation.token_count);
    println!("duration      : {duration:.2} s");
    println!(
        "end-to-end    : {:.2} tokens/s",
        generation.token_count as f64 / duration
    );
    println!("stop          : {:?}", generation.stop);
    println!("--- TEXT ---\n{}", generation.text);
}
