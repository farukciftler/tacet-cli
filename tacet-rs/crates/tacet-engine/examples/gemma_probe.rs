//! Gemma3 load probe — NOT TEMPORARY, a permanent measurement tool.
//!
//! WHY IT EXISTS: with multi-architecture loading, Gemma3 was producing broken
//! output even though it loaded with the right module and the right template.
//! Separating "is it the template, the context length, or the load itself"
//! required a point that runs on the SHORTEST possible prompt; the measurement
//! example runs with the full catalog and cannot separate that.
//!
//! Run:
//!   TACET_MODEL=~/models/gemma3-4b/model.gguf \
//!   TACET_TOKENIZER=~/models/gemma3-4b/tokenizer.json \
//!     cargo run -p tacet-engine --features metal --release --example gemma_probe

use tacet_engine::{EngineProvider, Prompt, SamplingSetting, wait};

fn main() {
    // The variable names are THE SAME as on the production path (`MODEL_VAR` /
    // `TOKENIZER_VAR` in `tacet-cli`); if the probe reads an environment other
    // than the one production reads, what it diagnoses is not production.
    let model = std::env::var("TACET_MODEL").expect("TACET_MODEL");
    let tokenizer = std::env::var("TACET_TOKENIZER").expect("TACET_TOKENIZER");

    let device = match std::env::var("TACET_DEVICE").as_deref() {
        Ok("cpu") => tacet_engine::candle_engine::Device::Cpu,
        _ => tacet_engine::candle_engine::Device::Metal,
    };
    let setting = tacet_engine::ModelSetting::new(&model, &tokenizer).with_device(device);
    let engine = tacet_engine::CandleEngine::load(&setting).expect("load");

    println!("architecture : {}", engine.architecture().name());
    println!("template     : {:?}", engine.template());
    println!("stop tokens  : {:?}", engine.stop_tokens());

    // Tokenization: Gemma MUST START with <bos>. Without it the model runs
    // outside the distribution and falls into repetition — the value has to be
    // seen by eye.
    let sample = "<start_of_turn>user\nHello<end_of_turn>\n<start_of_turn>model\n";
    let encoded = engine.tokenizer().encode(sample, true).expect("encode");
    println!(
        "\nfirst 8 tokens : {:?}",
        &encoded.get_ids()[..8.min(encoded.len())]
    );
    println!(
        "first 8 texts  : {:?}",
        &encoded.get_tokens()[..8.min(encoded.len())]
    );

    // The SHORTEST prompt: no tool description, no history. If it is broken here
    // too, the problem is not context length but the load itself.
    let prompt = Prompt::new("You are a helpful assistant.", "Hello, how are you?");
    let generation =
        wait(engine.generate(&prompt, None, SamplingSetting::default())).expect("generation");
    println!("\n== SHORT PROMPT ==");
    println!("stop    : {:?}", generation.stop);
    println!("text    : {:?}", generation.text);

    // PROMPT LENGTH SWEEP — makes the sliding window threshold visible.
    //
    // Gemma3's `attention.sliding_window` value is 1024. candle's
    // `quantized_gemma3::forward` DOES NOT build the mask at all on the decode
    // step (`seq_len == 1`); that is, the local (sliding-window) layers see ALL
    // the tokens in the cache. While the prompt is shorter than the window there
    // is no difference; past it, generation falls into repetition. This sweep
    // measures exactly that threshold.
    println!("\n== PROMPT LENGTH SWEEP ==");
    // THE FILLER MUST VARY. The first attempt repeated a fixed sentence and the
    // measurement was INVALID: repetitive context encourages the model towards
    // repetition anyway, i.e. "it breaks on a long prompt" could not be told
    // apart from "it breaks on a repetitive prompt". Every sentence carries a
    // different number and different words.
    for repeat in [1usize, 40, 120, 300] {
        let filler: String = (0..repeat)
            .map(|i| {
                format!(
                    "In item {i} let us note: the number {} multiplied by {} gives a different \
                     result, and this is examined in the context of {}. ",
                    i * 7 + 3,
                    i % 9 + 2,
                    [
                        "history",
                        "geography",
                        "music",
                        "architecture",
                        "medicine",
                        "law"
                    ][i % 6]
                )
            })
            .collect();
        let prompt = Prompt::new(format!("You are a helpful assistant. {filler}"), "Hello!");
        let n = engine
            .tokenizer()
            .encode(prompt.text_with_template(engine.template()), true)
            .map(|e| e.len())
            .unwrap_or(0);
        let g =
            wait(engine.generate(&prompt, None, SamplingSetting::default())).expect("generation");
        let short: String = g.text.chars().take(90).collect();
        println!("  prompt {n:>5} tokens -> {:?} {short:?}", g.stop);
    }
}
