#![cfg(feature = "candle")]
//! `CandleEngine::load` declares the tokenizer's vocabulary to the prompt's
//! defang through `guard_markers`, the call it needs for the sampler's mask.
//!
//! WHY THIS EXISTS: the declaration was once a line of its own in `load`, and
//! deleting it left every test green, because the tests called the library
//! function and not the loader (adversarial review, 9 Oct 2026). `load` itself
//! needs real weights and is not run here; what is held is that the call it
//! cannot do without also declares.
//!
//! ITS OWN BINARY, ONE TEST: the declaration is process-wide and narrows the
//! named-marker list for anything running beside it.

use std::str::FromStr;
use tacet_engine::candle_engine::guard_markers;
use tacet_engine::{Prompt, Template, Turn};
use tokenizers::Tokenizer;

/// Qwen3-shaped, in memory: two special markers (one a stop token), `<think>`
/// added but NOT special, an ordinary added `</div>`. No `<pad>`, no `<s>`.
const TOKENIZER: &str = r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[
 {"id":3,"content":"<|loader_test_end|>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true},
 {"id":4,"content":"<loader_test_turn>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":true},
 {"id":5,"content":"<think>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":false},
 {"id":6,"content":"</div>","single_word":false,"lstrip":false,"rstrip":false,"normalized":false,"special":false}],
 "normalizer":null,"pre_tokenizer":null,"post_processor":null,"decoder":null,
 "model":{"type":"BPE","dropout":null,"unk_token":null,"continuing_subword_prefix":null,
 "end_of_word_suffix":null,"fuse_unk":false,"byte_fallback":false,"ignore_merges":false,
 "vocab":{"a":0,"b":1,"ab":2},"merges":["a b"]}}"#;

fn tool_result(text: &str) -> String {
    Prompt::new("sys", "")
        .with_history(vec![Turn::user("q"), Turn::tool(text)])
        .text_with_template(Template::ChatML)
}

#[test]
fn the_call_the_loader_needs_declares_the_vocabulary() {
    let tk = Tokenizer::from_str(TOKENIZER).expect("tokenizer json");
    let source = "tok.pad_token = \"<pad>\"; bos = \"<s>\"";
    let novel = "<loader_test_turn>";

    // Nothing declared yet: the static guess applies, the novel marker is text.
    assert!(
        !tool_result(source).contains(source),
        "the fallback is the whole list"
    );
    assert!(tool_result(novel).contains(novel));

    // The stop token is masked out of the sampler's set but declared to the
    // prompt: `<|loader_test_end|>` in a document is the forgery.
    let masked = guard_markers(&tk, &[3]);
    assert_eq!(masked, vec![4], "the sampler's mask");

    let after = tool_result(&format!("{source} {novel} <think> </div>"));
    assert!(
        after.contains(source),
        "a token this vocabulary lacks was altered:\n{after}"
    );
    assert!(
        after.contains("< loader_test_turn>"),
        "the declared marker passed raw:\n{after}"
    );
    assert!(
        after.contains("< think>"),
        "a held token passed raw:\n{after}"
    );
    assert!(
        after.contains("</div>"),
        "an ordinary added token was altered:\n{after}"
    );
}
