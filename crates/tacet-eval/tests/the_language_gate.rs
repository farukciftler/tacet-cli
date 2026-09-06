//! THE LANGUAGE GATE PROVES THE NEGATIVE, and it used to demand the positive.
//!
//! It asked for evidence FOR the language and failed the answer when it found
//! none. Measured on the shipped baseline, that failed four answers written in
//! exactly the language asked for — three Turkish and one English — which is
//! four of the nine answer-quality failures being the instrument rather than the
//! model.
//!
//! Turkish's proof letters exclude `ö ü` because German writes them too; English
//! has no proof letters at all. So `81'in karekökü 9'dur.` carried nothing the
//! gate would accept, and neither did a fluent English joke that happened to use
//! none of the 23 English function words.
//!
//! Every fixture below is a VERBATIM answer from
//! `crates/tacet-eval/baselines/qwen3-4b-both.json`.

use tacet_eval::tool_selection::{Language, SelectionStep, check_answer_quality};

fn asked(lang: Language) -> SelectionStep {
    SelectionStep::new("m", Some("calculate")).with_language(lang)
}

#[test]
fn a_turkish_answer_is_not_failed_for_being_turkish() {
    for answer in [
        // Suffixes attach with an apostrophe; splitting there used to hand the
        // token `in` to the ENGLISH word list.
        "480'in yüzde 18'i 86.4'tür.",
        "1000 eksi 375, yani 1000 - 375 = 625'tir.",
        "81'in karekökü 9'dur.",
    ] {
        assert!(
            check_answer_quality(&asked(Language::Turkish), answer, &[], &[]),
            "correct Turkish failed the Turkish gate: {answer:?}"
        );
    }
}

#[test]
fn an_english_answer_with_none_of_the_listed_words_is_not_failed() {
    let joke = "Why don't scientists trust atoms?\n\nBecause they make up everything! 😄";
    assert!(
        check_answer_quality(&asked(Language::English), joke, &[], &[]),
        "fluent English failed the English gate for using none of its 23 words"
    );
}

/// AND THE HALF THAT MUST NOT REGRESS. Abstaining when there is no evidence is
/// not the same as accepting everything: an answer that shows evidence for
/// ANOTHER language still fails.
#[test]
fn an_answer_in_the_wrong_language_still_fails() {
    // Verbatim from the baseline: Turkish was asked, English was written, and
    // this is a genuine failure that must stay one.
    let english = "Pleasure! 😊 Let me know if you need anything else—happy to help!";
    assert!(!check_answer_quality(
        &asked(Language::Turkish),
        english,
        &[],
        &[]
    ));

    assert!(!check_answer_quality(
        &asked(Language::English),
        "Bunun için sonuç 1000 çıkıyor ve dosya kaydedildi.",
        &[],
        &[]
    ));
    assert!(!check_answer_quality(
        &asked(Language::Turkish),
        "The result is 1000 and the file is on the desktop.",
        &[],
        &[]
    ));
}

/// The competition has to separate the two languages that share `ö ü`, which is
/// why those letters were dropped from both proof sets in the first place.
#[test]
fn german_and_turkish_are_still_told_apart() {
    let german = "Die Datei ist nicht da, und die Uhr zeigt zwölf.";
    assert!(check_answer_quality(
        &asked(Language::German),
        german,
        &[],
        &[]
    ));
    assert!(!check_answer_quality(
        &asked(Language::Turkish),
        german,
        &[],
        &[]
    ));

    let turkish = "Dosya burada değil ve saat on iki'yi gösteriyor.";
    assert!(check_answer_quality(
        &asked(Language::Turkish),
        turkish,
        &[],
        &[]
    ));
    assert!(!check_answer_quality(
        &asked(Language::German),
        turkish,
        &[],
        &[]
    ));
}
