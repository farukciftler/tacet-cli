#![cfg(feature = "candle")]
//! A model may write text and it may end its turn. It may not write a marker.
//!
//! REPORTED FROM A REAL SESSION, gemma3-4b, "ortaköy üsküdar vapur saatleri":
//!
//!     Tacet <unused6088><unused6088><unused6088>… (twelve of them)
//!     engine error: inference failed: the sampler returned 262207, which is
//!     not a token id (the vocabulary has 262145 entries)
//!
//! Two faults in one turn. The engine error is the padded output layer and is
//! fixed by narrowing the logits. The `<unused6088>` is this: gemma3 carries
//! 6242 placeholder tokens, marks them `special: false`, and so they decode to
//! their own literal text and land on the user's screen.
//!
//! THE RULE THAT WAS WRITTEN FIRST AND WAS WRONG: "mask the added tokens".
//! gemma3's added vocabulary contains `\n`, `\n\n`, `\n\n\n` and ninety-one more
//! pieces of ordinary text — that rule would have forbidden the model a newline.
//! It was caught by reading the tokenizer before shipping rather than after,
//! and this file is where that stays caught.
//!
//! TWO KINDS OF TEST, AND WHY (audit T1). Every test here used to read
//! `~/models/*/tokenizer.json` and `continue`/`return` when it was absent — so
//! on a machine without weights, which is every CI machine, the whole file
//! passed while asserting nothing, and "measured and held" looked exactly like
//! "never ran". Now:
//!
//!   * the FIXTURE tests need no weights and run wherever the file is built
//!     with `--features candle`: `cargo test -p tacet-engine --features candle
//!     --test a_marker_is_not_text` locally, and the nightly job's "guards that
//!     need no weights" step in CI. On a PR, ci.yml's `candle-compiles` job
//!     builds them (`cargo test --workspace --features candle --no-run`) but
//!     does not run them, so a PR proves they compile and only the nightly
//!     proves they pass; the rule itself is also held in the default
//!     build by `gguf_tokenizer::markers_are_found_by_flag_and_by_name`. They load
//!     `tests/fixtures/marker_tokenizer.json` — a 4 KB tokenizer whose added
//!     vocabulary copies the shapes of qwen3-4b's and gemma3-4b's (the table it
//!     is written from is `FIXTURE_ADDED` in `gguf_tokenizer.rs`) — through the
//!     real `tokenizers` loader and the real `marker_tokens`.
//!   * the REAL-FILE tests are `#[ignore]`d with the reason, and FAIL when run
//!     without the files. This departs from the "print SKIPPED" convention in
//!     `gguf_tokenizer.rs` on purpose: a passing test's output is captured, so
//!     that line is only seen under `--nocapture`, while "N ignored" is in every
//!     summary. Run them with
//!     `cargo test -p tacet-engine --features candle --test a_marker_is_not_text -- --ignored`.

use tacet_engine::candle_engine::marker_tokens;
use tokenizers::Tokenizer;

/// The in-repo fixture. Loaded through `Tokenizer::from_file` — the path
/// `CandleEngine` takes for a user-supplied `tokenizer.json`, which is the
/// source where the FIM markers arrive `special: false`.
fn fixture() -> Tokenizer {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("marker_tokenizer.json");
    Tokenizer::from_file(&path)
        .unwrap_or_else(|e| panic!("the fixture {} would not load: {e}", path.display()))
}

fn id(tk: &Tokenizer, text: &str) -> u32 {
    tk.token_to_id(text)
        .unwrap_or_else(|| panic!("the fixture has no {text:?}"))
}

/// The real tokenizers this project loads. Only the `#[ignore]`d tests call
/// this, so a missing file is a FAILURE: whoever asked for these tests asked
/// for a measurement, and a green run without one would be the silence T1 was
/// about.
fn real(name: &str) -> Tokenizer {
    let home = std::env::var_os("HOME").expect("HOME is not set");
    let path = std::path::PathBuf::from(home)
        .join("models")
        .join(name)
        .join("tokenizer.json");
    Tokenizer::from_file(&path).unwrap_or_else(|e| {
        panic!(
            "{} is needed for this measurement and could not be read: {e}",
            path.display()
        )
    })
}

const ORDINARY_TEXT: &[&str] = &[
    "\n",
    "\n\n",
    "\n\n\n",
    " ",
    "the",
    "</div>",
    "</code>",
    "</h1>",
    "<think>",
    "<tool_call>",
];

// --- On the fixture: these run on every candle build -------------------------

/// THE TWO RULES THAT WERE WRITTEN AND MEASURED WRONG, kept as assertions so
/// neither comes back: "mask the added tokens" forbids `\n`, and "mask the
/// angle-bracketed added tokens" forbids `</div>`, `<think>` and `<tool_call>`.
#[test]
fn ordinary_text_is_never_forbidden_on_the_fixture() {
    let tk = fixture();
    let markers = marker_tokens(&tk, &[]);
    let mut checked = 0;
    for text in ORDINARY_TEXT {
        let Some(id) = tk.token_to_id(text) else {
            continue;
        };
        checked += 1;
        assert!(
            !markers.contains(&id),
            "{text:?} (id {id}) was masked; it is text the model may write"
        );
    }
    // The fixture carries six of the ten; a fixture that lost them would make
    // this test vacuous, which is the failure this file was rewritten for.
    assert!(
        checked >= 6,
        "only {checked} ordinary tokens are in the fixture"
    );
}

/// AUDIT L24: Qwen's FIM/repo markers arrive `special: false` from a
/// `tokenizer.json`, and the flag-only rule left all six selectable.
#[test]
fn the_fim_and_repo_markers_are_masked_although_not_special() {
    let tk = fixture();
    let markers = marker_tokens(&tk, &[]);
    let added = tk.get_added_vocabulary();
    for marker in [
        "<|fim_prefix|>",
        "<|fim_middle|>",
        "<|fim_suffix|>",
        "<|fim_pad|>",
        "<|repo_name|>",
        "<|file_sep|>",
    ] {
        assert!(
            !added.is_special_token(marker),
            "the fixture must reproduce the hole: {marker} is special in it"
        );
        assert!(
            markers.contains(&id(&tk, marker)),
            "{marker} is still selectable from a tokenizer.json source"
        );
    }
}

#[test]
fn a_placeholder_is_forbidden_on_the_fixture() {
    let tk = fixture();
    let markers = marker_tokens(&tk, &[]);
    for placeholder in ["<unused0>", "<unused6088>", "<|extra_0|>"] {
        assert!(
            markers.contains(&id(&tk, placeholder)),
            "{placeholder} is still selectable"
        );
    }
}

/// A turn marker in the model's OWN output is the other half of the
/// forged-turn hole that `a_tool_result_cannot_forge_a_turn` closes on the
/// prompt side.
#[test]
fn the_model_cannot_open_a_turn_on_the_fixture() {
    let tk = fixture();
    let markers = marker_tokens(&tk, &[]);
    for marker in ["<start_of_turn>", "<|im_start|>"] {
        assert!(
            markers.contains(&id(&tk, marker)),
            "the model can still emit {marker}"
        );
    }
}

/// Ending the turn is the one marker the model is supposed to produce — and
/// `<|im_end|>` has the fence shape, so the name rule must not reach it once
/// it is a stop token.
#[test]
fn the_stop_token_stays_available_on_the_fixture() {
    let tk = fixture();
    for stop_text in ["<end_of_turn>", "<|im_end|>", "<|endoftext|>"] {
        let stop = id(&tk, stop_text);
        assert!(
            !marker_tokens(&tk, &[stop]).contains(&stop),
            "{stop_text} was masked, so the model can never end a turn"
        );
    }
}

// --- On the real files: run with `-- --ignored` ------------------------------

#[test]
#[ignore = "needs ~/models/{gemma3-4b,qwen3-4b}/tokenizer.json; the fixture tests cover the rule"]
fn ordinary_text_is_never_forbidden() {
    for name in ["gemma3-4b", "qwen3-4b"] {
        let tk = real(name);
        let markers = marker_tokens(&tk, &[]);
        for text in ORDINARY_TEXT {
            let Some(id) = tk.token_to_id(text) else {
                continue;
            };
            assert!(
                !markers.contains(&id),
                "{name}: {text:?} (id {id}) was masked. It is text the model may \
                 legitimately write, and a rule that forbids it is a rule that \
                 was measured on the wrong thing."
            );
        }
    }
}

/// The set is SMALL on a model with no placeholders. A rule that masks
/// thousands of tokens on qwen3 has stopped being about markers.
#[test]
#[ignore = "needs ~/models/qwen3-4b/tokenizer.json; the fixture tests cover the rule"]
fn the_masked_set_is_small_where_there_are_no_placeholders() {
    let tk = real("qwen3-4b");
    let n = marker_tokens(&tk, &[]).len();
    assert!(
        n < 40,
        "{n} tokens masked on qwen3, which has 26 added tokens in total"
    );
}

#[test]
#[ignore = "needs ~/models/gemma3-4b/tokenizer.json; the fixture tests cover the rule"]
fn a_placeholder_is_forbidden() {
    let tk = real("gemma3-4b");
    let markers = marker_tokens(&tk, &[]);
    let id = tk
        .token_to_id("<unused6088>")
        .expect("gemma3 carries this placeholder");
    assert!(
        markers.contains(&id),
        "the exact token a real session put on the user's screen is still \
         selectable"
    );
    assert!(
        markers.len() > 6000,
        "gemma3 has 6242 placeholders; only {} were found",
        markers.len()
    );
}

/// AUDIT L24 on the real file: qwen3-4b's `tokenizer.json` is where the six
/// FIM/repo markers are `special: false`.
#[test]
#[ignore = "needs ~/models/qwen3-4b/tokenizer.json; the fixture tests cover the rule"]
fn the_fim_markers_are_masked_on_the_real_qwen_vocabulary() {
    let tk = real("qwen3-4b");
    let markers = marker_tokens(&tk, &[]);
    for marker in ["<|fim_prefix|>", "<|repo_name|>", "<|file_sep|>"] {
        let id = tk
            .token_to_id(marker)
            .unwrap_or_else(|| panic!("qwen3 carries {marker}"));
        assert!(markers.contains(&id), "qwen3: {marker} is still selectable");
    }
}

#[test]
#[ignore = "needs ~/models/{gemma3-4b,qwen3-4b}/tokenizer.json; the fixture tests cover the rule"]
fn the_model_cannot_open_a_turn() {
    for (name, marker) in [
        ("gemma3-4b", "<start_of_turn>"),
        ("qwen3-4b", "<|im_start|>"),
    ] {
        let tk = real(name);
        let id = tk
            .token_to_id(marker)
            .unwrap_or_else(|| panic!("{name} carries {marker}"));
        assert!(
            marker_tokens(&tk, &[]).contains(&id),
            "{name}: the model can still emit {marker}"
        );
    }
}

#[test]
#[ignore = "needs ~/models/{gemma3-4b,qwen3-4b}/tokenizer.json; the fixture tests cover the rule"]
fn the_stop_token_stays_available() {
    for (name, stop_text) in [("gemma3-4b", "<end_of_turn>"), ("qwen3-4b", "<|im_end|>")] {
        let tk = real(name);
        let stop = tk
            .token_to_id(stop_text)
            .unwrap_or_else(|| panic!("{name} carries {stop_text}"));
        assert!(
            !marker_tokens(&tk, &[stop]).contains(&stop),
            "{name}: {stop_text} was masked, so the model can never end a turn"
        );
    }
}
