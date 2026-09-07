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

use tacet_engine::candle_engine::marker_tokens;
use tokenizers::Tokenizer;

/// The two tokenizers this project actually loads, when they are on the
/// machine. Skipped rather than failed when they are not: this asserts a rule
/// about real vocabularies, and inventing one would assert nothing.
fn tokenizer(name: &str) -> Option<Tokenizer> {
    let path = dirs_home()?
        .join("models")
        .join(name)
        .join("tokenizer.json");
    Tokenizer::from_file(path).ok()
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

/// THE TWO RULES THAT WERE WRITTEN AND MEASURED WRONG, kept as assertions so
/// neither comes back.
///
/// "Mask the added tokens" forbids `\n` — gemma3 keeps newlines, and ninety-one
/// other pieces of ordinary text, in its ADDED vocabulary.
///
/// "Mask the angle-bracketed added tokens" forbids `</div>` — gemma3's
/// vocabulary carries the HTML tags, and qwen3's carries `<think>` and
/// `<tool_call>`, which are added but not special.
#[test]
fn ordinary_text_is_never_forbidden() {
    for name in ["gemma3-4b", "qwen3-4b"] {
        let Some(tk) = tokenizer(name) else { continue };
        let markers = marker_tokens(&tk, &[]);
        for text in [
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
        ] {
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
fn the_masked_set_is_small_where_there_are_no_placeholders() {
    let Some(tk) = tokenizer("qwen3-4b") else {
        return;
    };
    let n = marker_tokens(&tk, &[]).len();
    assert!(
        n < 40,
        "{n} tokens masked on qwen3, which has 26 added tokens in total"
    );
}

#[test]
fn a_placeholder_is_forbidden() {
    let Some(tk) = tokenizer("gemma3-4b") else {
        return;
    };
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

/// A turn marker in the model's OWN output is the other half of the forged-turn
/// hole that `a_tool_result_cannot_forge_a_turn` closes on the prompt side.
#[test]
fn the_model_cannot_open_a_turn() {
    for (name, marker) in [
        ("gemma3-4b", "<start_of_turn>"),
        ("qwen3-4b", "<|im_start|>"),
    ] {
        let Some(tk) = tokenizer(name) else { continue };
        let Some(id) = tk.token_to_id(marker) else {
            continue;
        };
        assert!(
            marker_tokens(&tk, &[]).contains(&id),
            "{name}: the model can still emit {marker}"
        );
    }
}

/// Ending the turn is the one marker the model is supposed to produce.
#[test]
fn the_stop_token_stays_available() {
    for (name, stop_text) in [("gemma3-4b", "<end_of_turn>"), ("qwen3-4b", "<|im_end|>")] {
        let Some(tk) = tokenizer(name) else { continue };
        let Some(stop) = tk.token_to_id(stop_text) else {
            continue;
        };
        assert!(
            !marker_tokens(&tk, &[stop]).contains(&stop),
            "{name}: {stop_text} was masked, so the model can never end a turn"
        );
    }
}
