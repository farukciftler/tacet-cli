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
    // TACET_TOKENIZER IS NOW OPTIONAL. Unset, the tokenizer inside the GGUF is
    // used (see `gguf_tokenizer`) — and that is the point of measuring it here:
    // the path a single downloaded `.gguf` takes has to be RUN, not just
    // compiled. Set, the user's file wins, exactly as `resolve_tokenizer` says.
    let tokenizer = std::env::var("TACET_TOKENIZER").ok();

    // The device is chosen FROM THE ENVIRONMENT; the default is the CPU. When
    // `metal` is asked for, the crate's `metal` feature must be on as well —
    // otherwise it errors out openly, it does NOT silently FALL BACK to the CPU
    // (see `Device::Metal`).
    let device = match std::env::var("TACET_DEVICE").as_deref() {
        Ok("metal") => tacet_engine::candle_engine::Device::Metal,
        _ => tacet_engine::candle_engine::Device::Cpu,
    };
    let setting = match &tokenizer {
        Some(path) => tacet_engine::ModelSetting::new(&model, path),
        None => tacet_engine::ModelSetting::from_gguf(&model),
    }
    .with_device(device);
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
    // WHICH tokenizer was used. Two sources that cannot be told apart from the
    // output is exactly how a measurement ends up describing a run nobody
    // intended.
    println!("tokenizer     : {}", engine.tokenizer_source().name());

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

    // --- 6. THE CONTEXT SWEEP — what a bigger window actually costs -----------
    //
    // GATED BY AN ENVIRONMENT VARIABLE, not on by default: each step prefills a
    // prompt of that many tokens and at 32768 that alone is tens of seconds. The
    // rest of this example is a quick sanity run and must stay quick.
    if std::env::var("TACET_CONTEXT_SWEEP").is_ok() {
        context_sweep(&engine);
    }

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

// ---------------------------------------------------------------------------
// The context sweep
// ---------------------------------------------------------------------------

/// The process's resident set size in MEGABYTES.
///
/// WHY `ps` AND NOT A CRATE: reading RSS needs either a platform crate (libc,
/// sysinfo) or the OS's own tool. This project does not add a dependency for a
/// measurement, and `ps -o rss=` is in POSIX. It reports KILOBYTES on macOS.
///
/// RSS IS NOT PEAK AND DOES NOT SHRINK back to the OS when the KV cache is
/// dropped, so the sweep prints it after every step and reads the numbers as a
/// high-water mark, not as "what this step alone costs".
fn rss_mb() -> f64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<f64>()
            .map(|kb| kb / 1024.0)
            .unwrap_or(f64::NAN),
        Err(_) => f64::NAN,
    }
}

/// Builds a prompt whose TEMPLATED form tokenizes to roughly `target` tokens.
///
/// The filler is grown by doubling and then trimmed back sentence by sentence;
/// the number reported is the REAL tokenized length, never the target. Reporting
/// the target would make the table a table of intentions.
fn prompt_of_size(engine: &tacet_engine::CandleEngine, target: usize) -> (Prompt, usize) {
    // The same Turkish-density filler the truncation section uses, so the sweep
    // measures the kind of text this assistant actually carries.
    let sentence = "turn: a fairly long sentence written to fill the context window of an \
                    assistant running on the device. ";
    let count = |p: &Prompt| {
        engine
            .tokenizer()
            .encode(p.text_with_template(engine.template()), true)
            .map(|e| e.get_ids().len())
            .unwrap_or(0)
    };
    let build = |turns: usize| {
        Prompt::new(
            tacet_engine::SYSTEM_INSTRUCTIONS,
            "Summarise what was said above in one sentence.",
        )
        .with_history((0..turns).map(|i| Turn::user(format!("{i} {sentence}"))))
    };

    let mut turns = 8usize;
    while count(&build(turns)) < target && turns < 1 << 20 {
        turns *= 2;
    }
    // Walk back down: overshooting by a doubling would put a "4096" row 3000
    // tokens above its label.
    let mut step = turns / 2;
    while step > 0 {
        if count(&build(turns - step)) >= target {
            turns -= step;
        }
        step /= 2;
    }
    let prompt = build(turns);
    let real = count(&prompt);
    (prompt, real)
}

/// Measures prefill and decode SEPARATELY at increasing context lengths.
///
/// THE METHOD: two runs from the same prompt, `max_tokens = 1` and
/// `max_tokens = 1 + STEPS`. Greedy sampling is deterministic, so the longer run
/// reproduces the shorter one's token exactly and the difference in wall time is
/// pure decode. `prefill = t_one - (1 / decode)`.
///
/// WHY NOT THE 32/64 DIFFERENCE the section above uses: that one needs BOTH runs
/// to hit the cap, and on a 32k-token junk prompt the model often stops after a
/// few tokens. Anchoring on `max_tokens = 1` cannot stop early — one token is
/// always produced — and the longer run's own `token_count` is used, so an early
/// stop only shortens the baseline, it does not corrupt the arithmetic.
fn context_sweep(engine: &tacet_engine::CandleEngine) {
    const STEPS: usize = 24;

    println!("\n== CONTEXT SWEEP ==");
    println!(
        "model declares : {:?} tokens",
        engine.context_length()
    );
    println!("baseline RSS   : {:.2} GB (weights + rope table, before any KV cache)", rss_mb() / 1024.0);
    println!(
        "\n{:>8}  {:>8}  {:>10}  {:>12}  {:>10}  {:>9}",
        "target", "real", "prefill s", "prefill tok/s", "decode t/s", "RSS GB"
    );

    let sizes: Vec<usize> = match std::env::var("TACET_CONTEXT_SIZES") {
        Ok(list) => list.split(',').filter_map(|s| s.trim().parse().ok()).collect(),
        Err(_) => vec![4096, 8192, 16384, 32768],
    };

    for target in sizes {
        // The prompt has to leave room for the tokens we are about to generate;
        // otherwise the "4096" row would really be running at 4096 + 25.
        let (prompt, real) = prompt_of_size(engine, target.saturating_sub(STEPS + 8));
        let run = |n: usize| -> (f64, Generation) {
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

        let (t_one, g_one) = run(1);
        let (t_many, g_many) = run(1 + STEPS);
        let extra = g_many.token_count.saturating_sub(g_one.token_count);
        let decode = if extra > 0 && t_many > t_one {
            extra as f64 / (t_many - t_one)
        } else {
            f64::NAN
        };
        // One decode step is inside `t_one`; subtract it to leave the prefill.
        let prefill = t_one - 1.0 / decode;
        println!(
            "{target:>8}  {real:>8}  {prefill:>10.2}  {:>12.1}  {decode:>10.1}  {:>9.2}",
            real as f64 / prefill,
            rss_mb() / 1024.0
        );
    }
}
