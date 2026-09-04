//! THE ACCEPTANCE TEST for `build_vocab` — the one function that decides what
//! alphabet the grammar speaks.
//!
//! WHAT IS MEASURED: `build_vocab` turns every token id into the text that token
//! contributes to the answer, and that `Vec<String>` is the ONLY thing the
//! grammar ever sees (`CandleEngine::vocab` -> `CallConstraint::new` at
//! chat.rs:403 and main.rs:886). Its doc comment made two claims and marked both
//! UNVERIFIED. This file measures them on real weights:
//!
//!   1. THE MARKERS ARE RESOLVED — `Ġ` (byte-level BPE) and `▁` (sentencepiece)
//!      leave the raw form and arrive as a real space. TRUE, and asserted in the
//!      direction that can fail: every raw form starting with the family's marker
//!      must have a surface starting with U+0020, and the number of such ids is
//!      checked so an all-empty vocabulary cannot pass vacuously.
//!   2. SPECIAL TOKENS ARE EMPTY, so `TokenMask` keeps them closed. This was
//!      FALSE — with `decode(id, false)` not one id of 151669 was empty and
//!      `<|im_end|>` went into the trie as ten ordinary characters. The engine
//!      now decodes with `skip_special_tokens = true`; this file pins the
//!      corrected property, and `tacet-cli/tests/mask_alphabet.rs` drives the
//!      leak the old vocabulary allowed.
//!
//! AND THE THIRD THING, which nobody had claimed either way: whether the
//! per-token surfaces ADD UP to the text `run_loop` delivers. They do, except on
//! text that tokenizes through a byte fragment — that divergence is measured
//! here and bounded, not swept up.
//!
//! WHEN THE FIXTURE IS MISSING the test RETURNS EARLY and prints why —
//! deliberately not `#[ignore]`, for the reason written at
//! tests/gguf_tokenizer.rs:16-18.
//!
//! Run:
//!   TACET_MODEL=~/models/qwen3-4b/model.gguf \
//!   TACET_TOKENIZER=~/models/qwen3-4b/tokenizer.json \
//!     cargo test -p tacet-engine --features candle --test vocab_alphabet -- --nocapture
//!
//! MEASURED ON THIS MACHINE (macOS arm64, 4 Sep 2026), for the record:
//!
//!   file                     source  ids     Ġ/▁ raw  empty  U+FFFD
//!   qwen3-4b/model.gguf      gguf    151669  53021    20     1457
//!   qwen3-4b/tokenizer.json  file    151669  53021    14     1457
//!   qwen2.5-3b (q4_k_m)      gguf    151665  53021    20     1457
//!   qwen3-8b/model.gguf      gguf    151669  53021    20     1457
//!   gemma3-4b/tokenizer.json file    262145  137541   9      134
//!
//! gemma3-4b's GGUF is the REFUSAL CASE: it carries a sentencepiece vocabulary
//! and `tokenizer_from_gguf` errors rather than guessing, so only its File source
//! is measurable. The test prints the refusal instead of skipping past it.
//!
//! COST, so "it runs in seconds" carries a number: 2.3 s wall for the qwen3-4b
//! pair (both sources, ~240 ms of it inside `build_vocab` itself) once the
//! candle + tokenizers + onig tree is built. THAT tree is the expensive part and
//! it is paid once, not by this file.

#![cfg(feature = "candle")]

use std::path::PathBuf;
use std::time::Instant;

use tacet_engine::candle_engine::build_vocab;

/// The fixture. `TACET_MODEL` / `TACET_TOKENIZER` — THE SAME variable names the
/// production path uses, so the test is fed by whatever the user already set up.
///
/// THE ONE DIFFERENCE from tests/gguf_tokenizer.rs's `fixture`: the
/// `tokenizer.json` is OPTIONAL here. `resolve_tokenizer` (candle_engine.rs) can
/// pick either source and the alphabet has to be measured on BOTH, so a model
/// with no `tokenizer.json` next to it is still worth running — and a model whose
/// GGUF cannot be rebuilt (gemma3) is worth running for the other half.
fn fixture() -> Option<(PathBuf, Option<PathBuf>)> {
    let model = std::env::var_os("TACET_MODEL").map(PathBuf::from);
    let tokenizer = std::env::var_os("TACET_TOKENIZER").map(PathBuf::from);
    let (model, tokenizer) = match model {
        Some(m) => (m, tokenizer),
        None => {
            let home = PathBuf::from(std::env::var_os("HOME")?);
            let dir = home.join("models").join("qwen3-4b");
            (dir.join("model.gguf"), Some(dir.join("tokenizer.json")))
        }
    };
    model
        .is_file()
        .then_some((model, tokenizer.filter(|t| t.is_file())))
}

/// The strings the surfaces are added up over.
///
/// The list is the one from tests/gguf_tokenizer.rs plus the two shapes THIS
/// test is about: a complete tool call, and a call whose argument carries code
/// with newlines. Those are the exact language `CallSession::advance`
/// reconstructs character by character, so if the surfaces do not add up there,
/// the constraint is masking against a text nobody will ever write.
const CASES: &[&str] = &[
    "",
    " ",
    "Merhaba dünya",
    "İstanbul'da yağmur yağıyor, şemsiyeni unutma.",
    "ığüşöçİĞÜŞÖÇ",
    "  başında iki boşluk",
    "                    yirmi boşluk",
    "satır\nsonu\r\nve\ttab",
    "🇹🇷 bayrak ve 👨‍👩‍👧‍👦 aile, ☺️ yüz",
    "<|im_start|>assistant\n<tool_call>{\"name\":\"calculate\"}</tool_call><|im_end|>",
    "create_document({\"format\":\"excel\",\"file_name\":\"report.xlsx\"})",
    "write_code({\"code\":\"for i in range(10):\\n    print(i)\"})",
    "{\"expression\": \"12*(3+4)\", \"digits\": 2}",
    "aynı satırda ASCII and Türkçe mixed 混合 текст",
];

/// The characters the CALL GRAMMAR branches on. A surface that carries one of
/// these can open or close a JSON structure; a surface that does not is, as far
/// as the automaton is concerned, just text.
const STRUCTURAL: &str = "{}[]\":,";

/// The whole measurement, for ONE tokenizer source. It is a function rather than
/// two tests because the GGUF and the `tokenizer.json` are two DIFFERENT
/// vocabularies of the same model (they disagree on which tokens are special —
/// 20 vs 14 on qwen3-4b) and a claim that holds for one is not a claim about the
/// other.
fn measure(label: &str, tokenizer: &tokenizers::Tokenizer) {
    println!("\n──────── SOURCE: {label} ────────");

    // The COST carries a number rather than the word "cheap": this runs once per
    // model load, next to a 2 GB weight read.
    let clock = Instant::now();
    let vocab = build_vocab(tokenizer);
    let build_ms = clock.elapsed().as_secs_f64() * 1000.0;
    println!("build_vocab      : {} ids in {build_ms:.0} ms", vocab.len());
    assert_eq!(
        vocab.len(),
        tokenizer.get_vocab_size(true),
        "build_vocab must cover the ADDED vocabulary too — the mask indexes by token id"
    );

    // --- 1. THE MARKERS ARE RESOLVED ---------------------------------------
    //
    // WHICH marker is a property of the vocabulary family, and it is DETECTED
    // rather than assumed: `Ġ` means nothing in a sentencepiece vocabulary and
    // `▁` means nothing in a byte-level BPE one. Asserting both against both
    // would fail on gemma3 for a reason that is not a defect — measured: gemma3
    // holds exactly ONE id whose raw form starts with `Ġ` (245237), and its text
    // really is that character.
    let raw_of = |id: u32| tokenizer.id_to_token(id).unwrap_or_default();
    let leading = |marker: char| {
        (0..vocab.len() as u32)
            .filter(|id| raw_of(*id).starts_with(marker))
            .count()
    };
    let byte_level = leading('\u{0120}');
    let sentencepiece = leading('\u{2581}');
    println!("raw forms leading Ġ / ▁ : {byte_level} / {sentencepiece}");
    let marker = if byte_level >= sentencepiece {
        '\u{0120}'
    } else {
        '\u{2581}'
    };

    let mut resolved = 0usize;
    let mut unresolved: Vec<(u32, String, String)> = Vec::new();
    for id in 0..vocab.len() as u32 {
        if raw_of(id).starts_with(marker) {
            if vocab[id as usize].starts_with(' ') {
                resolved += 1;
            } else {
                unresolved.push((id, raw_of(id), vocab[id as usize].clone()));
            }
        }
    }
    println!("marker {marker:?} resolved   : {resolved}");
    assert!(
        unresolved.is_empty(),
        "a raw form starts with {marker:?} but its surface does not start with a space — the \
         mask would be built on a character the model never writes: {unresolved:?}"
    );
    // NON-VACUITY. Asserting only "no surface starts with a marker" would pass on
    // an all-empty vocabulary, which is exactly the failure this file exists to
    // catch. Measured: 53021 (Qwen, both sources), 137541 (gemma3).
    assert!(
        resolved > 1000,
        "only {resolved} ids carry the {marker:?} marker — this vocabulary is not the byte-level \
         or sentencepiece family the claim is about, so the check proved nothing"
    );

    // A marker CHARACTER may still appear in a surface, and that is not a leak:
    // measured on both Qwen sources, ids 144242 (raw `Äł`) and 148848 (raw `ÄĬ`)
    // decode to the single characters U+0120 and U+010A, which is their real
    // text. gemma3 has the same two, 245237 and 247723. What may NOT happen is a
    // marker riding along inside a longer piece.
    let carriers: Vec<(u32, String)> = (0..vocab.len() as u32)
        .filter(|id| {
            vocab[*id as usize]
                .chars()
                .any(|c| c == '\u{0120}' || c == '\u{2581}' || c == '\u{010A}')
        })
        .map(|id| (id, vocab[id as usize].clone()))
        .collect();
    println!("surfaces holding a marker char : {carriers:?}");
    for (id, surface) in &carriers {
        assert_eq!(
            surface.chars().count(),
            1,
            "id {id} surface {surface:?} carries a marker inside a longer piece — that is an \
             unresolved marker, not a literal character"
        );
    }

    // --- 2. THE CLAIM THAT WAS FALSE ---------------------------------------
    //
    // With `decode(id, false)` this count was ZERO on every file measured, so
    // `TokenMask::empty_tokens()` was empty and "special tokens are kept closed"
    // described a mechanism that had never once fired. With
    // `skip_special_tokens = true` the empty set is EXACTLY the added-special
    // set — asserted in BOTH directions, because "every special is empty" alone
    // would still let an ordinary word vanish from the grammar's alphabet, and
    // "every empty is special" alone would still leave a special token loose in
    // the middle of a JSON string.
    let added = tokenizer.get_added_vocabulary();
    let mut special_ids: Vec<u32> = Vec::new();
    let mut empty_ids: Vec<u32> = Vec::new();
    for id in 0..vocab.len() as u32 {
        if added.is_special_token(&raw_of(id)) {
            special_ids.push(id);
        }
        if vocab[id as usize].is_empty() {
            empty_ids.push(id);
        }
    }
    println!(
        "special ids ({}) : {:?}",
        special_ids.len(),
        special_ids.iter().map(|id| raw_of(*id)).collect::<Vec<_>>()
    );
    println!("empty surfaces  : {}", empty_ids.len());
    assert_eq!(
        empty_ids, special_ids,
        "the empty surfaces and the added-special tokens must be the SAME set"
    );
    assert!(
        !special_ids.is_empty(),
        "no added-special token at all — either this vocabulary has none (then the claim is \
         untestable here) or `is_special_token` answered wrongly and the stop token is lost too"
    );

    // --- 3. THE SURFACES MUST ADD UP TO WHAT `run_loop` DELIVERS ------------
    //
    // `CandleEngine::decode` is PRIVATE and needs loaded weights, so it cannot be
    // called from here; `tokenizer.decode(&ids, true)` reproduces it exactly
    // (candle_engine.rs:572-576), and that is the text `run_loop` returns at
    // :938. The concatenation on the other side is what `CallSession::advance`
    // reconstructs, character by character, out of `vocab`.
    //
    // THE SPLIT IS THE POINT. On text that tokenizes through a byte fragment the
    // two CANNOT agree — a fragment has no text of its own — so the halves are
    // measured separately and the divergence is asserted to EXIST rather than
    // being exempted away. Measured: 2 of the 14 cases fragment on the Qwen
    // files (the emoji ZWJ sequence and the CJK/Cyrillic mix), 0 on gemma3.
    let mut clean = 0usize;
    let mut fragmented = 0usize;
    for case in CASES {
        let ids = tokenizer
            .encode(*case, true)
            .expect("encode")
            .get_ids()
            .to_vec();
        let concat: String = ids.iter().map(|i| vocab[*i as usize].as_str()).collect();
        let delivered = tokenizer.decode(&ids, true).expect("decode");
        let has_fragment = ids.iter().any(|i| vocab[*i as usize].contains('\u{FFFD}'));
        if has_fragment {
            fragmented += 1;
            assert_ne!(
                concat, delivered,
                "case {case:?} tokenizes through a byte fragment yet the two agreed — if that \
                 ever becomes true this split is measuring nothing"
            );
            println!(
                "  FRAGMENTED {case:?}\n    grammar sees : {concat:?}\n    delivered    : \
                 {delivered:?}"
            );
        } else {
            clean += 1;
            assert_eq!(
                concat, delivered,
                "the grammar's character stream and the delivered text disagree on {case:?} — \
                 the mask is being built against a text the model will not write"
            );
        }
    }
    println!("cases: {clean} additive, {fragmented} fragmented");
    assert!(
        clean >= 10,
        "only {clean} of {} cases were fragment-free; the additivity claim was barely exercised",
        CASES.len()
    );

    // --- 4. THE BYTE-FRAGMENT BOUND ----------------------------------------
    //
    // The divergence above is tolerable only because a fragment cannot open a
    // JSON structure. Measured on both Qwen sources: 1457 surfaces contain
    // U+FFFD, NOT ONE of them contains `{}[]":,` and exactly ONE contains a
    // parenthesis — id 94825, surface " (" + U+FFFD, which `CallConstraint`
    // already handles as a paren token (its remainder fails the grammar, so
    // `prefix_mask` closes it). gemma3: 134 U+FFFD surfaces, none of either kind.
    //
    // The parenthesis case is asserted as "at most one, and here it is" with the
    // id PRINTED rather than pinned, so a vocabulary that grows a second one
    // fails loudly and says which.
    let replacement: Vec<u32> = (0..vocab.len() as u32)
        .filter(|id| vocab[*id as usize].contains('\u{FFFD}'))
        .collect();
    let json_carrying: Vec<(u32, String)> = replacement
        .iter()
        .filter(|id| vocab[**id as usize].chars().any(|c| STRUCTURAL.contains(c)))
        .map(|id| (*id, vocab[*id as usize].clone()))
        .collect();
    let paren_carrying: Vec<(u32, String)> = replacement
        .iter()
        .filter(|id| vocab[**id as usize].chars().any(|c| c == '(' || c == ')'))
        .map(|id| (*id, vocab[*id as usize].clone()))
        .collect();
    println!("U+FFFD surfaces : {}", replacement.len());
    println!("  carrying {STRUCTURAL} : {json_carrying:?}");
    println!("  carrying ( or )    : {paren_carrying:?}");
    assert!(
        json_carrying.is_empty(),
        "a byte fragment carries a JSON structural character — the mask can open a structure the \
         delivered text does not contain: {json_carrying:?}"
    );
    assert!(
        paren_carrying.len() <= 1,
        "more than one byte fragment carries a parenthesis; each is a candidate for closing a \
         call the answer never wrote: {paren_carrying:?}"
    );
}

#[test]
fn the_grammars_alphabet_is_the_text_the_model_delivers() {
    let Some((model, tokenizer_json)) = fixture() else {
        // NOT `#[ignore]`: it prints why, so "no weights on this machine" cannot
        // be mistaken for "measured and passed".
        println!(
            "SKIPPED: no model file. Set TACET_MODEL (and optionally TACET_TOKENIZER), or place \
             model.gguf under ~/models/qwen3-4b."
        );
        return;
    };
    println!("model : {}", model.display());

    let mut measured = 0usize;
    match tacet_engine::tokenizer_from_gguf(&model) {
        Ok(t) => {
            measure("tokenizer rebuilt from the GGUF", &t);
            measured += 1;
        }
        // NOT A SKIP. gemma3-4b lands here (sentencepiece; `tokenizer_from_gguf`
        // refuses rather than guessing) and that refusal is the one hostile input
        // this machine holds — printing it keeps it on the record while the File
        // source below still gets measured.
        Err(e) => println!("\nGGUF SOURCE REFUSED (a measured outcome, not a skip): {e}"),
    }
    match &tokenizer_json {
        Some(path) => {
            let t =
                tokenizers::Tokenizer::from_file(path).expect("the tokenizer.json would not load");
            measure(&format!("tokenizer.json ({})", path.display()), &t);
            measured += 1;
        }
        None => println!("\n(no tokenizer.json given — only the GGUF source was measured)"),
    }
    assert!(
        measured > 0,
        "neither tokenizer source could be built for {} — nothing was measured, and a green test \
         that measured nothing is the failure this file exists to prevent",
        model.display()
    );
}
