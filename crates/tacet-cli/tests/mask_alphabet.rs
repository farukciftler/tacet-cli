//! THE MASK, MEASURED ON A REAL VOCABULARY FOR THE FIRST TIME.
//!
//! Every existing test of `CallConstraint` runs on the fake vocabulary
//! "code point = token id" (call.rs:493, tests.rs:497). That vocabulary has one
//! character per token, so it can measure the automaton and NOTHING about the
//! two things that only exist in a real tokenizer: tokens that carry several
//! characters and cross a grammar boundary, and tokens that carry no visible
//! character at all. `build_vocab`'s doc comment said so itself — "on the fake
//! engine a code point is a token, so this conversion cannot be measured there".
//!
//! WHAT IS MEASURED:
//!   1. The NATURAL tokenization of a valid call survives mask+advance, token by
//!      token, on the model's own boundaries. mask.rs:98-118 argues about the
//!      merged token `"})`; it had been reasoned about, never run.
//!   2. THE LEAK THAT WAS REAL. `build_vocab` used to decode with
//!      `skip_special_tokens = false`, so `<|im_start|>` reached the trie as
//!      twelve ordinary characters and the mask OPENED it inside a JSON string
//!      value — while `run_loop` deletes it again from the text it delivers. This
//!      file rebuilds that old vocabulary by hand, drives the leak, shows what it
//!      costs, and then shows the same token CLOSED on the vocabulary
//!      `build_vocab` produces today.
//!
//! WHY IT LIVES IN tacet-cli: this is the one crate that already depends on
//! tacet-engine AND tacet-grammar AND tacet-tools, so it needs no manifest line.
//! A dev-dependency from tacet-grammar onto a candle-featured tacet-engine would
//! invert the direction constraint.rs:3-10 and call.rs:10-13 both spell out as
//! deliberate, and would drag the candle tree into every
//! `cargo test -p tacet-grammar`.
//!
//! WHY NO TYPE FROM `tokenizers` IS NAMED BELOW: `tokenizers` is a dependency of
//! tacet-engine, not of tacet-cli, so it cannot be named here — and adding it as
//! a dev-dependency is not an option, because Cargo dev-dependencies cannot be
//! optional and it would then be built for every default `cargo test -p
//! tacet-cli` too. So the fixture hands out only `Vec<String>`, `Vec<u32>` and
//! two boxed closures, and the tokenizer's own type stays inside `fixture()`
//! where inference covers it. For the same reason only the GGUF source is
//! reachable here (`Tokenizer::from_file` would have to be named); BOTH sources
//! are measured in tacet-engine/tests/vocab_alphabet.rs.
//!
//! Run:
//!   TACET_MODEL=~/models/qwen3-4b/model.gguf \
//!     cargo test -p tacet-cli --features candle --test mask_alphabet -- --nocapture

#![cfg(feature = "candle")]

use std::path::PathBuf;
use std::sync::Arc;

use tacet_engine::candle_engine::build_vocab;
use tacet_engine::{Constrainer, ConstraintSession};
use tacet_grammar::{CallConstraint, TokenMask};
use tacet_kernel::{
    ArgSchema, Field, Tool, ToolCatalog, ToolContext, ToolFuture, ToolOutcome, boxed,
};

/// `text -> token ids`, boxed. The box is what keeps the tokenizer's own type
/// out of every signature in this file (see the module header).
type Encode = Box<dyn Fn(&str) -> Vec<u32>>;

/// `(ids, skip_special_tokens) -> text`, boxed for the same reason.
type Decode = Box<dyn Fn(&[u32], bool) -> String>;

/// Everything the tests need, with the tokenizer's own type left inside.
struct Fixture {
    /// What `build_vocab` produces today: `decode(id, skip_special_tokens = true)`.
    new: Vec<String>,
    /// What it produced before this was measured: `decode(id, false)`. Kept
    /// because the defect is only visible as a DIFFERENCE — asserting the new
    /// behaviour alone would pass on a build that never had the bug.
    old: Vec<String>,
    /// The added-special ids, ascending.
    special: Vec<usize>,
    /// `id_to_token` for every id, so a test can name a token without holding the
    /// tokenizer.
    names: Vec<String>,
    encode: Encode,
    /// `(ids, skip_special_tokens)`. With `true` this is exactly
    /// `CandleEngine::decode` (candle_engine.rs:572-576) — which is private and
    /// needs loaded weights, so no test can call it — and therefore exactly the
    /// text `run_loop` hands back at :938.
    decode: Decode,
}

/// `TACET_MODEL`, falling back to `~/models/qwen3-4b/model.gguf` — the same
/// convention as tests/gguf_tokenizer.rs and tacet-engine/tests/vocab_alphabet.rs.
///
/// `TACET_TOKENIZER` is deliberately NOT read here (see the module header: the
/// File source cannot be opened without naming a `tokenizers` type). A GGUF whose
/// tokenizer cannot be rebuilt — gemma3-4b's is sentencepiece — returns `None`
/// and the tests print why.
fn fixture() -> Option<Fixture> {
    let model = match std::env::var_os("TACET_MODEL") {
        Some(m) => PathBuf::from(m),
        None => PathBuf::from(std::env::var_os("HOME")?)
            .join("models")
            .join("qwen3-4b")
            .join("model.gguf"),
    };
    if !model.is_file() {
        return None;
    }
    let tokenizer = match tacet_engine::tokenizer_from_gguf(&model) {
        Ok(t) => t,
        Err(e) => {
            println!(
                "SKIPPED: {} carries no rebuildable tokenizer: {e}",
                model.display()
            );
            return None;
        }
    };
    println!("model : {}", model.display());

    let size = tokenizer.get_vocab_size(true) as u32;
    let new = build_vocab(&tokenizer);
    let old: Vec<String> = (0..size)
        .map(|id| tokenizer.decode(&[id], false).unwrap_or_default())
        .collect();
    let names: Vec<String> = (0..size)
        .map(|id| tokenizer.id_to_token(id).unwrap_or_default())
        .collect();
    let added = tokenizer.get_added_vocabulary();
    let special: Vec<usize> = (0..size as usize)
        .filter(|i| added.is_special_token(&names[*i]))
        .collect();

    let tokenizer = Arc::new(tokenizer);
    let for_encode = Arc::clone(&tokenizer);
    let for_decode = Arc::clone(&tokenizer);
    Some(Fixture {
        new,
        old,
        special,
        names,
        // `false` = do not add the template's special tokens. What is being
        // tokenized is a CONTINUATION in the middle of an answer, not a fresh
        // prompt; a BOS spliced in here would measure a sequence generation
        // never produces.
        encode: Box::new(move |s: &str| {
            for_encode
                .encode(s, false)
                .expect("encode")
                .get_ids()
                .to_vec()
        }),
        decode: Box::new(move |ids: &[u32], skip: bool| {
            for_decode.decode(ids, skip).expect("decode")
        }),
    })
}

/// NOT `#[ignore]`: an ignored test is silent, and "no weights on this machine"
/// and "nobody ever ran this" then look exactly alike
/// (tests/gguf_tokenizer.rs:16-18).
fn skip_notice() {
    println!(
        "SKIPPED: no usable model. Set TACET_MODEL to a gpt2-vocabulary .gguf, or place \
         model.gguf under ~/models/qwen3-4b."
    );
}

/// The same schema shape call.rs's own tests use — one required enum plus one
/// required free string. Reused deliberately: this file measures the ALPHABET,
/// and a novel catalog would put a second variable into the measurement.
struct DocumentTool;

impl Tool for DocumentTool {
    fn name(&self) -> &str {
        "create_document"
    }
    fn description(&self) -> &str {
        "Produces a document."
    }
    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new("format", ArgSchema::choice(["excel", "markdown"])).required(),
            Field::new("file_name", ArgSchema::text()).required(),
        ])
    }
    fn run<'a>(&'a self, _args: serde_json::Value, _ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move { ToolOutcome::read_ok("ok", "ok") })
    }
}

fn catalog() -> ToolCatalog {
    let mut c = ToolCatalog::new();
    c.add(Arc::new(DocumentTool));
    c
}

/// The names `find_stop_tokens` (candle_engine.rs:1084-1098) collects. A leak
/// demonstrated on one of these would be indistinguishable from the SEPARATE
/// stop-token mask at candle_engine.rs:788-794, which already closes them for the
/// whole argument region — so the leak token is picked from outside this list.
const STOP_NAMES: &[&str] = &[
    "</s>",
    "<|im_end|>",
    "<|eot_id|>",
    "<|end_of_text|>",
    "<end_of_turn>",
    "<|endoftext|>",
];

/// An added-special token that is NOT a stop token, chosen AT RUNTIME rather
/// than by literal id.
///
/// WHY NOT A LITERAL: `<think>` and `<|fim_prefix|>` are the obvious candidates
/// and both are wrong. Measured on qwen3-4b: `<think>` (151667) is an added token
/// but `is_special_token` is FALSE for it, so `decode(skip = true)` keeps it and
/// there is no asymmetry to show; `<|fim_prefix|>` is special in the
/// GGUF-rebuilt tokenizer and NOT special in `tokenizer.json`, so the test would
/// pass or fail depending on which source the fixture resolved to. On qwen3-4b
/// this picks id 151644, `<|im_start|>` — the lowest special id that is not a
/// stop token, and a fence Tacet writes into its own prompts, which makes it the
/// one a confused model is most likely to reach for mid-argument.
fn leak_token(fx: &Fixture) -> Option<(u32, String)> {
    fx.special
        .iter()
        .map(|i| (*i as u32, fx.names[*i].clone()))
        .find(|(_, name)| !STOP_NAMES.contains(&name.as_str()))
}

/// Steps a session over `ids`, asserting the mask leaves each one OPEN before it
/// is advanced. That is the invariant call.rs:292 and mask.rs:108-112 are
/// written around — "masking and advancing must not drift apart" — and until now
/// it had only ever been exercised one character at a time.
fn drive(session: &mut Box<dyn ConstraintSession>, ids: &[u32], vocab: &[String]) {
    for id in ids {
        let mut logits = vec![0.0f32; vocab.len()];
        session.mask(&mut logits);
        assert!(
            logits[*id as usize].is_finite(),
            "the mask closed token {id} ({:?}), which is part of the model's OWN tokenization of \
             a valid call — the constraint would be making its own accepted output impossible to \
             produce",
            vocab[*id as usize]
        );
        session.advance(*id).unwrap_or_else(|e| {
            panic!(
                "advance refused token {id} ({:?}): {e}",
                vocab[*id as usize]
            )
        });
    }
}

#[test]
fn the_models_own_tokenization_of_a_valid_call_is_producible() {
    let Some(fx) = fixture() else {
        skip_notice();
        return;
    };
    let constraint = CallConstraint::new(&fx.new, &catalog());

    const CALL: &str = "create_document({\"format\":\"excel\",\"file_name\":\"report.xlsx\"})";
    let ids = (fx.encode)(CALL);
    // PRINTED, because the boundaries are the whole point: this is the first time
    // the mask has been asked about a token that carries several characters.
    println!(
        "{} tokens: {:?}",
        ids.len(),
        ids.iter()
            .map(|i| fx.new[*i as usize].as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        ids.len() < CALL.chars().count(),
        "this tokenizer put one character in every token; then it is the fake vocabulary again \
         and nothing new is being measured"
    );

    let mut session = constraint.session();
    drive(&mut session, &ids, &fx.new);
    assert!(
        session.is_done(),
        "the call was fully consumed but the session did not close — generation would run on \
         past a complete call"
    );
}

#[test]
fn a_special_token_leaked_into_the_argument_string_until_build_vocab_skipped_it() {
    let Some(fx) = fixture() else {
        skip_notice();
        return;
    };
    let Some((leak_id, leak_name)) = leak_token(&fx) else {
        println!("SKIPPED: this vocabulary has no non-stop added-special token to leak.");
        return;
    };
    println!("leak token : id {leak_id} {leak_name:?}");

    assert_eq!(
        fx.old[leak_id as usize], leak_name,
        "the old vocabulary is supposed to hand the grammar the special token's literal name; if \
         it does not, this test is no longer reproducing the defect it documents"
    );
    assert!(
        fx.new[leak_id as usize].is_empty(),
        "build_vocab still hands the grammar {} characters for a token the delivered text does \
         not contain",
        fx.new[leak_id as usize].chars().count()
    );

    // Both sessions are driven to the SAME place: inside the `file_name` free
    // string value, the only region of the grammar where arbitrary text is legal
    // and therefore the only region a special token can enter. At a structural
    // position `<` is not a valid continuation and the trie closes it anyway.
    const PREFIX: &str = "create_document({\"format\":\"excel\",\"file_name\":\"rep";
    const REST: &str = "ort.xlsx\"})";
    let prefix_ids = (fx.encode)(PREFIX);
    let rest_ids = (fx.encode)(REST);

    // --- THE OLD VOCABULARY: the leak, driven ------------------------------
    let old_constraint = CallConstraint::new(&fx.old, &catalog());
    let mut session = old_constraint.session();
    drive(&mut session, &prefix_ids, &fx.old);
    let mut logits = vec![0.0f32; fx.old.len()];
    session.mask(&mut logits);
    assert!(
        logits[leak_id as usize].is_finite(),
        "the leak is absent from the OLD vocabulary too — then the old vocabulary is not what was \
         measured and build_vocab's comment is describing something else"
    );
    session
        .advance(leak_id)
        .expect("the old vocabulary's grammar takes the special token as ordinary text");
    drive(&mut session, &rest_ids, &fx.old);
    assert!(session.is_done(), "the leaked call still closed cleanly");

    // WHAT THE DAMAGE ACTUALLY IS, spelled out rather than assumed. The grammar
    // counted the characters of the special token; `run_loop` hands back
    // `self.decode(&produced)` — `decode(skip_special_tokens = true)` — which
    // deletes them again.
    let all: Vec<u32> = prefix_ids
        .iter()
        .copied()
        .chain(std::iter::once(leak_id))
        .chain(rest_ids.iter().copied())
        .collect();
    let grammar_saw: String = all.iter().map(|i| fx.old[*i as usize].as_str()).collect();
    let delivered = (fx.decode)(&all, true);
    println!("grammar saw : {grammar_saw:?}");
    println!("delivered   : {delivered:?}");
    assert!(grammar_saw.contains(&leak_name));
    assert!(
        !delivered.contains(&leak_name),
        "decode(skip_special_tokens = true) was supposed to delete it — if it does not, the two \
         alphabets never differed and there was nothing to fix"
    );
    assert_ne!(grammar_saw, delivered);

    // THE STRUCTURE SURVIVES, THE VALUE DOES NOT — measured, not assumed. The
    // README's claim ("malformed JSON cannot be GENERATED") is NOT what broke
    // here: a special token's name carries no `{}[]":,`, so deleting it from
    // inside a string value leaves the call parseable. What is lost is the
    // argument the user asked for, silently.
    let parsed = tacet_tools::executor::ToolCall::parse(&delivered)
        .expect("the delivered text is still a parseable call — that is exactly the finding");
    assert_eq!(parsed.name, "create_document");
    println!("parsed args : {}", parsed.args);
    assert_eq!(
        parsed.args["file_name"], "report.xlsx",
        "the deleted token sits between `rep` and `ort.xlsx`, so the value the tool receives is \
         not the one the grammar validated"
    );

    // --- THE NEW VOCABULARY: the same token, closed ------------------------
    let new_constraint = CallConstraint::new(&fx.new, &catalog());
    let mut session = new_constraint.session();
    drive(&mut session, &prefix_ids, &fx.new);
    let mut logits = vec![0.0f32; fx.new.len()];
    session.mask(&mut logits);
    assert_eq!(
        logits[leak_id as usize],
        f32::NEG_INFINITY,
        "{leak_name} is still open inside a JSON string value — `build_vocab` must give every \
         added-special token an EMPTY surface so `TokenMask` files it under `empty_tokens` and \
         never opens it"
    );
}

#[test]
fn the_empty_tokens_of_a_real_vocabulary_are_exactly_the_special_ones() {
    let Some(fx) = fixture() else {
        skip_notice();
        return;
    };

    // THE MECHANISM, ASSERTED WHERE IT LIVES. `build_vocab`'s comment claims
    // `TokenMask` keeps special tokens closed because their text is empty; this
    // is that sentence measured on the real trie rather than on the 0x1000-entry
    // fake vocabulary of tests.rs:557. Measured on qwen3-4b's GGUF tokenizer:
    // 20 of 151669.
    let new = TokenMask::new(&fx.new);
    println!(
        "empty_tokens : {} of {}",
        new.empty_tokens().len(),
        new.vocab_size()
    );
    assert!(!fx.special.is_empty(), "no added-special token to measure");
    assert_eq!(
        new.empty_tokens(),
        fx.special.as_slice(),
        "TokenMask's neutral set must be exactly the added-special set"
    );

    // And the same measurement on the old vocabulary — which is why the claim was
    // false: the set was EMPTY, so the mechanism the comment rested on had never
    // once fired.
    let old = TokenMask::new(&fx.old);
    assert!(
        old.empty_tokens().is_empty(),
        "the old vocabulary was measured as having no empty surface at all; if it now has one, \
         the fixture changed and the numbers in build_vocab's comment need re-measuring"
    );
}

/// WHAT THE MASK ACTUALLY COSTS, ON THIS VOCABULARY, PER GENERATED TOKEN.
///
/// WHY THIS EXISTS AND WHY IT PRINTS RATHER THAN GATES. `mask.rs`'s header
/// carries a measurement — "structural positions 2-3 µs, free string body
/// ~0.95 ms" — taken on a 32k vocabulary, and concludes that even the worst case
/// is "about 3% of the budget" of a 3B model. qwen3's vocabulary is not 32k, and
/// the conclusion is an extrapolation nobody had rerun.
///
/// IT ALSO SETTLES A PROFILING MISTAKE, which is the honest reason it is here. A
/// CPU sample of a live eval showed `TokenMask::walk` above
/// `AttentionWeights::forward`, and that is NOT evidence that the mask costs more
/// than the model: `walk` runs on the CPU while the forward pass dispatches to
/// the GPU and the calling thread waits, so a CPU profile systematically
/// over-counts the mask. The only honest comparison is wall time on both, which
/// is what this measures.
///
/// NO ASSERTION ON THE TIMING. A number that gates would be a flaky test on a
/// shared laptop; a number that prints is a measurement someone can read next to
/// the claim it checks.
#[test]
fn what_one_mask_step_costs_on_a_real_vocabulary() {
    let Some(fx) = fixture() else {
        skip_notice();
        return;
    };

    let mask = TokenMask::new(&fx.new);
    let grammar = tacet_grammar::Grammar::compile(&probe_schema());

    // Three positions, chosen because the trie prunes them very differently:
    // at the start only `{` is legal, at a key only the field names are, and
    // inside a string body nearly the whole vocabulary is.
    let positions: [(&str, &str); 3] = [
        ("call start (one legal character)", ""),
        ("inside a key (a handful of branches)", "{\""),
        ("free string body (almost no pruning)", "{\"note\":\"the "),
    ];

    println!("vocabulary: {} tokens", fx.new.len());
    for (label, prefix) in positions {
        let mut state = grammar.state();
        if !prefix.is_empty() && state.advance(prefix).is_err() {
            println!("  {label:38} — prefix rejected, position not reachable");
            continue;
        }

        // Warm the caches, then time enough iterations that the clock is not the
        // thing being measured.
        let _ = mask.mask(&state);
        let rounds = 200;
        let started = std::time::Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(mask.mask(&state));
        }
        let per_step = started.elapsed().as_secs_f64() * 1000.0 / rounds as f64;
        let open = mask.mask(&state).iter().filter(|b| **b).count();
        println!("  {label:38} {per_step:>8.3} ms/step · {open} tokens open");
    }
}

/// A schema with a free-text field, which is the shape that makes the walk
/// expensive — `calendar` and `remember` both have one, and they are the slowest
/// cases in the selection suite.
fn probe_schema() -> ArgSchema {
    ArgSchema::object(vec![Field::new("note", ArgSchema::text()).required()])
}
