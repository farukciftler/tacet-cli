//! THE ACCEPTANCE TEST for the tokenizer read out of a GGUF file.
//!
//! WHAT IS MEASURED: the tokenizer `tacet_engine::tokenizer_from_gguf` rebuilds
//! from `model.gguf` is encoded against the REAL `tokenizer.json` lying next to
//! the same weights, and the two TOKEN ID SEQUENCES must be identical, element
//! for element.
//!
//! WHY THAT AND NOT "it loads": a wrong tokenizer raises no error. Encoding a
//! space one token differently, or splitting `ğ` at its byte boundary, produces
//! ids the model has never seen in that arrangement — and the model answers
//! anyway, with plausible-looking nonsense. Nothing in the stack points at the
//! tokenizer. So the only honest check is byte-identical output on inputs picked
//! for the places byte-level BPE usually breaks: Turkish letters, emoji, code,
//! the ChatML fences, long space runs and the empty string.
//!
//! WHEN THE FIXTURE IS MISSING the test RETURNS EARLY and prints why. It is
//! deliberately not `#[ignore]`: an ignored test is silent, and "no weights on
//! this machine" and "nobody ever ran this" then look exactly alike.
//!
//! Run:
//!   cargo test -p tacet-engine --features candle --test gguf_tokenizer -- --nocapture

#![cfg(feature = "candle")]

use std::path::PathBuf;

/// The fixture pair. `TACET_MODEL` / `TACET_TOKENIZER` — THE SAME variable names
/// the production path uses (see `tacet-cli`), so the test is fed by whatever the
/// user already set up. With no variables it falls back to the conventional
/// `~/models/qwen3-4b` layout.
fn fixture() -> Option<(PathBuf, PathBuf)> {
    let model = std::env::var_os("TACET_MODEL").map(PathBuf::from);
    let tokenizer = std::env::var_os("TACET_TOKENIZER").map(PathBuf::from);
    let (model, tokenizer) = match (model, tokenizer) {
        (Some(m), Some(t)) => (m, t),
        _ => {
            let home = PathBuf::from(std::env::var_os("HOME")?);
            let dir = home.join("models").join("qwen3-4b");
            (dir.join("model.gguf"), dir.join("tokenizer.json"))
        }
    };
    (model.is_file() && tokenizer.is_file()).then_some((model, tokenizer))
}

/// The strings the two tokenizers are compared on.
///
/// Every entry is here for a REASON, not for volume:
/// * Turkish letters (ı ğ ş ö ç İ) are multi-byte and byte-level BPE splits them
///   into byte pieces — the first place a missing `ByteLevel` shows up.
/// * emoji are 4-byte code points, and one of them carries a variation selector.
/// * the ChatML fences must come out as SINGLE ids; if the added-token table is
///   wrong they fall apart into text and the model stops recognising its own
///   frame markers.
/// * long space runs are exactly what the `\s+(?!\S)` branch of the split regex
///   is for; a near-miss regex differs here and nowhere else.
/// * leading/trailing space decides `add_prefix_space`.
/// * the empty string is the classic panic/off-by-one case.
const CASES: &[&str] = &[
    "",
    " ",
    "Merhaba dünya",
    "İstanbul'da yağmur yağıyor, şemsiyeni unutma.",
    "ığüşöçİĞÜŞÖÇ",
    "Çiğdem'in ödevi bitti mi?",
    "  başında iki boşluk",
    "sonunda iki boşluk  ",
    "                    yirmi boşluk",
    "satır\nsonu\r\nve\ttab",
    "🇹🇷 bayrak ve 👨‍👩‍👧‍👦 aile, ☺️ yüz",
    "<|im_start|>user",
    "<|im_start|>system\nYou are Tacet.<|im_end|>\n",
    "<|im_start|>assistant\n<tool_call>{\"name\":\"calculate\"}</tool_call><|im_end|>",
    "<think>düşünüyorum</think>",
    "fn main() { println!(\"{}\", 1 + 2); }",
    "let x: Vec<HashMap<String, u32>> = Vec::new();",
    "{\"expression\": \"12*(3+4)\", \"digits\": 2}",
    "SELECT * FROM users WHERE id = 42 AND name LIKE '%çiftçi%';",
    "0123456789 3.14159 1_000_000 0xFF",
    "don't can't we're I'll they've",
    "aynı satırda ASCII and Türkçe mixed 混合 текст",
    "----====>>>> !!! ??? ... ,,, ;;;",
    "a",
    "\u{fffd}",
    "Tacet — bir tire, “tırnak”, ve ‘tek tırnak’ …",
];

#[test]
fn the_tokenizer_inside_the_gguf_is_identical_to_the_real_tokenizer_json() {
    let Some((model, tokenizer_json)) = fixture() else {
        // NOT `#[ignore]`: it prints why, so "no fixture" cannot be mistaken for
        // "measured and passed".
        println!(
            "SKIPPED: the fixture pair was not found. Set TACET_MODEL and TACET_TOKENIZER, \
             or place model.gguf + tokenizer.json under ~/models/qwen3-4b."
        );
        return;
    };
    println!("gguf      : {}", model.display());
    println!("reference : {}", tokenizer_json.display());

    // The cheap check must ALSO agree — this is the function discovery calls, and
    // it saying "no" for a file we can in fact rebuild would be just as wrong.
    // Its COST is printed: discovery runs it once per package it finds, so
    // "cheap" is a claim that has to carry a number.
    let clock = std::time::Instant::now();
    let has = tacet_engine::gguf_has_tokenizer(&model);
    println!(
        "gguf_has_tokenizer : {has} in {:.0} ms (file: {:.1} GB)",
        clock.elapsed().as_secs_f64() * 1000.0,
        std::fs::metadata(&model).map(|m| m.len()).unwrap_or(0) as f64 / 1e9
    );
    assert!(
        has,
        "gguf_has_tokenizer said no for a file the loader can handle"
    );

    let reference = tokenizers::Tokenizer::from_file(&tokenizer_json)
        .expect("the reference tokenizer.json would not load");
    let rebuilt = tacet_engine::tokenizer_from_gguf(&model)
        .expect("the tokenizer inside the gguf would not build");

    assert_eq!(
        rebuilt.get_vocab_size(true),
        reference.get_vocab_size(true),
        "vocabulary sizes differ"
    );

    let mut matched = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    for case in CASES {
        // `true` = parse special tokens — the same call `CandleEngine::tokenize`
        // makes. Measuring it any other way would measure a path production never
        // takes.
        let expected = reference.encode(*case, true).expect("reference encode");
        let actual = rebuilt.encode(*case, true).expect("rebuilt encode");
        if expected.get_ids() == actual.get_ids() {
            matched += 1;
        } else {
            mismatches.push(format!(
                "  {case:?}\n    tokenizer.json: {:?}\n    gguf          : {:?}",
                expected.get_ids(),
                actual.get_ids()
            ));
        }
    }
    println!("compared strings : {}", CASES.len());
    println!("identical id sequences : {matched}");
    assert!(
        mismatches.is_empty(),
        "{} of {} strings tokenized differently:\n{}",
        mismatches.len(),
        CASES.len(),
        mismatches.join("\n")
    );

    // --- Decoding too -------------------------------------------------------
    //
    // Encoding being identical is not enough on its own: `build_vocab` feeds the
    // GRAMMAR MASK from `decode`, and a wrong decoder would show up as "valid
    // JSON gets rejected" rather than as broken text.
    let mut decode_matched = 0usize;
    for case in CASES {
        let ids = reference
            .encode(*case, true)
            .expect("encode")
            .get_ids()
            .to_vec();
        let expected = reference.decode(&ids, false).expect("reference decode");
        let actual = rebuilt.decode(&ids, false).expect("rebuilt decode");
        assert_eq!(expected, actual, "decoding differs for {case:?}");
        decode_matched += 1;
    }
    println!("identical decodings : {decode_matched}");

    // --- The surface forms the mask is built from ---------------------------
    //
    // A whole-vocabulary sweep, because the mask asks every id the same question
    // and one wrong entry is enough to close off a valid branch of the grammar.
    let size = reference.get_vocab_size(true) as u32;
    let mut surface_matched = 0usize;
    for id in 0..size {
        let expected = reference.decode(&[id], false).unwrap_or_default();
        let actual = rebuilt.decode(&[id], false).unwrap_or_default();
        assert_eq!(expected, actual, "the surface form of id {id} differs");
        surface_matched += 1;
    }
    println!("identical surface forms : {surface_matched} / {size}");

    // The stop-token lookup rests on `is_special_token`; if that answered
    // differently, generation would only stop at the token cap.
    for name in ["<|im_end|>", "<|endoftext|>"] {
        assert_eq!(
            rebuilt.token_to_id(name),
            reference.token_to_id(name),
            "the id of {name} differs"
        );
        assert!(
            rebuilt.get_added_vocabulary().is_special_token(name),
            "{name} is not marked special — generation would never stop"
        );
    }

    // --- THE ONE PLACE THE TWO ARE ALLOWED TO DIFFER, measured not assumed ---
    //
    // The `special` flag is DERIVED from GGUF's `token_type`, and GGUF carries
    // less information than tokenizer.json does: it marks the FIM/repo markers
    // CONTROL, while tokenizer.json calls them ordinary added tokens. This does
    // NOT touch encoding (`AddedVocabulary` splits its match trie by
    // `normalized`, not by `special`) — the id comparison above already proves
    // that. It only changes `decode(skip_special_tokens = true)`.
    //
    // The difference is PRINTED AND BOUNDED rather than swept up: it must stay
    // one-directional (the gguf may only be MORE conservative) and must never
    // reach a token Tacet actually writes.
    let reference_added = reference.get_added_vocabulary();
    let rebuilt_added = rebuilt.get_added_vocabulary();
    let mut only_gguf: Vec<String> = Vec::new();
    let mut only_json: Vec<String> = Vec::new();
    for id in 0..size {
        let Some(name) = reference.id_to_token(id) else {
            continue;
        };
        match (
            rebuilt_added.is_special_token(&name),
            reference_added.is_special_token(&name),
        ) {
            (true, false) => only_gguf.push(name),
            (false, true) => only_json.push(name),
            _ => {}
        }
    }
    println!("special only in gguf : {only_gguf:?}");
    println!("special only in tokenizer.json : {only_json:?}");
    assert!(
        only_json.is_empty(),
        "the gguf misses a token tokenizer.json calls special — a stop token could be lost: \
         {only_json:?}"
    );
    for name in &only_gguf {
        assert!(
            !["<|im_start|>", "<|im_end|>", "<tool_call>", "</tool_call>"].contains(&name.as_str()),
            "a fence Tacet writes itself is classified differently: {name}"
        );
    }
}

/// The priority rule, measured: an explicitly given `tokenizer.json` WINS, and a
/// given-but-missing one is an ERROR rather than a silent fallback to the GGUF.
///
/// Needs no weights — `files_exist` is the pre-check, it does not load.
#[test]
fn an_explicit_tokenizer_path_wins_and_a_missing_one_does_not_fall_back() {
    use tacet_engine::{CandleEngine, ModelSetting};

    let Some((model, tokenizer_json)) = fixture() else {
        println!("SKIPPED: no fixture — see the other test in this file.");
        return;
    };

    // Explicit and present: accepted.
    let explicit = ModelSetting::new(&model, &tokenizer_json);
    assert_eq!(explicit.tokenizer_path.as_ref(), Some(&tokenizer_json));
    CandleEngine::files_exist(&explicit).expect("an existing pair should pass");

    // Explicit and MISSING: must fail even though the same gguf carries a
    // perfectly good tokenizer. The user named a path; using a different
    // vocabulary behind their back is the failure this guards.
    let typo = ModelSetting::new(&model, model.with_file_name("tokenizer.json.typo"));
    assert!(
        CandleEngine::files_exist(&typo).is_err(),
        "a missing explicit tokenizer.json quietly fell back to the gguf"
    );

    // Not given at all: the gguf's own tokenizer carries the package.
    let from_gguf = ModelSetting::from_gguf(&model);
    assert!(from_gguf.tokenizer_path.is_none());
    CandleEngine::files_exist(&from_gguf)
        .expect("the gguf carries a tokenizer, the pre-check should pass");
}
