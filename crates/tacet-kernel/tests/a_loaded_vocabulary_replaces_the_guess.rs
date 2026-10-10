//! The named-marker list is a guess for a process with no tokenizer loaded;
//! the first declared vocabulary replaces it with a fact.
//!
//! ITS OWN TEST BINARY because the registry is process-wide: declaring a
//! vocabulary in `defang`'s unit tests would narrow the named list under every
//! other test running beside them. One test function, for the same reason.

use tacet_kernel::defang::{Vocabulary, defang, register_vocabulary};

#[test]
fn the_first_declared_vocabulary_replaces_the_named_guess() {
    let ml_source = "tok.pad_token = \"<pad>\"; bos = \"<s>\"";

    // No vocabulary: the whole named list, cost included.
    assert_eq!(
        defang(ml_source),
        "tok.pad_token = \"< pad>\"; bos = \"< s>\""
    );
    assert_eq!(defang("<think>"), "< think>");

    // A Qwen-shaped vocabulary: holds `<think>`, declares its ChatML markers.
    assert_eq!(
        register_vocabulary(Vocabulary::new(
            ["<|im_end|>", "<kernel_it_marker>"],
            ["<think>", "</think>"],
        )),
        2
    );
    assert_eq!(
        defang(ml_source),
        ml_source,
        "a token Qwen lacks was altered"
    );
    assert_eq!(
        defang("<think>"),
        "< think>",
        "a token Qwen holds passed raw"
    );
    assert_eq!(defang("<kernel_it_marker>"), "< kernel_it_marker>");
    assert_eq!(
        defang("</MEMORY >"),
        "< /MEMORY >",
        "our fences are always ours"
    );

    // A second, Gemma-shaped one only adds: `<pad>` is its padding token.
    register_vocabulary(Vocabulary::new(["<start_of_turn>"], ["<pad>"]));
    assert_eq!(defang("\"<pad>\""), "\"< pad>\"");
    assert_eq!(
        defang("<think>"),
        "< think>",
        "a later vocabulary undid an earlier one"
    );
    assert_eq!(defang("\"<s>\""), "\"<s>\"");
}
