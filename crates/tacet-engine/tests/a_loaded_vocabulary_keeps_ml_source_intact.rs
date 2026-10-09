//! AUDIT L37's collateral, measured in review on 9 Oct 2026: with the named
//! marker list applied whatever model was loaded, `tok.pad_token = "<pad>"`
//! and `bos="<s>"` did not reach a Qwen model verbatim in a `read_document`
//! result — and a model that writes the file back writes the damage back.
//!
//! ITS OWN BINARY, ONE TEST: declaring a vocabulary is process-wide.

use tacet_engine::{Prompt, Template, Turn};
use tacet_kernel::defang::{Vocabulary, register_vocabulary};

fn read_document_result(text: &str) -> String {
    Prompt::new("sys", "")
        .with_history(vec![Turn::user("q"), Turn::tool(text)])
        .text_with_template(Template::ChatML)
}

#[test]
fn ml_source_reaches_a_model_whose_vocabulary_lacks_its_tokens() {
    let source = "tok.pad_token = \"<pad>\"\ntok.add_special_tokens({\"bos_token\": \"<s>\", \
                  \"eos_token\": \"</s>\", \"unk_token\": \"<unk>\", \"mask_token\": \"<mask>\"})\n\
                  gemma = [\"<bos>\", \"<eos>\"]";
    assert!(
        !read_document_result(source).contains(source),
        "with no vocabulary the whole named list applies"
    );

    // What `prompt::register_vocab_markers` declares for a Qwen3 tokenizer.
    register_vocabulary(Vocabulary::new(
        ["<|im_start|>", "<|im_end|>", "<|endoftext|>"],
        ["<think>", "</think>", "<tool_call>", "</tool_call>"],
    ));
    let text = read_document_result(&format!("{source}\n<think>x</think>"));
    assert!(text.contains(source), "ML source was altered:\n{text}");
    assert!(
        !text.contains("<think>"),
        "a token Qwen holds passed raw:\n{text}"
    );
}
