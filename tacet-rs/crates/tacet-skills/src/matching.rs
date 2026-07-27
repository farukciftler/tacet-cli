//! The ONE rule of trigger matching.
//!
//! This is the shared text rule used by both the skill and the memory layer.
//! It was not copied into two places because the rule is not a NUMBER, it is a
//! BEHAVIOUR: if the copies drift apart the "same rule" claim silently becomes
//! a lie and nobody notices — memory notes start matching differently from
//! skills. That is why `tacet-memory` depends on `tacet-skills` for this
//! module; the direction is not arbitrary, the rule was born in the skill layer
//! (that is where it was measured).

/// The length limit below which matching at a TERM START is not enough;
/// anything shorter demands a WHOLE TERM.
///
/// MEASURED FAILURE: the trigger "ara" was catching "**ara**lik" (Turkish for
/// December), so every question about December pulled in the search skill and
/// ate ~700 characters of the 4096 budget. The same trap exists for
/// "bul" -> "bulut/bulmaca".
///
/// Why only for SHORT triggers: Turkish suffixes attach at the END, so for a
/// long trigger prefix matching is REQUIRED ("line" -> "lines",
/// "carp" -> "carparsak"). A short root, on the other hand, can also sit at the
/// start of unrelated words.
///
/// Threshold 4: three-letter roots (ara/bul/oku/dok) demand a whole term, 4+
/// keeps prefix matching. Making it 5 would break "carp".
///
/// The asymmetry is deliberate: injecting the WRONG skill misleads the model
/// and eats ~700 characters of the budget; MISSING a skill only loses the extra
/// guide — the tool is still in the session. Missing is cheap, mismatching is
/// expensive.
pub const WHOLE_TERM_LIMIT: usize = 4;

/// Turkish-aware lowercasing.
///
/// `str::to_lowercase` applies the Unicode default: 'I' -> 'i' and
/// 'İ' -> "i\u{307}" (i + combining dot). Both are WRONG in Turkish and break
/// matching: "Istanbul" must match the trigger "istanbul", while "İSTANBUL"
/// must not scatter into two code points and stop matching at all. We map
/// those two letters by hand and leave the rest to Unicode.
pub fn lowercase(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            'I' => output.push('ı'),
            'İ' => output.push('i'),
            _ => {
                for lower in c.to_lowercase() {
                    output.push(lower);
                }
            }
        }
    }
    output
}

/// Does `text` (which MUST already be lowercased) contain `trigger` while
/// respecting term boundaries.
///
/// A raw substring search kept finding short triggers INSIDE frequent words
/// ("betik" inside "alfabetik" pulled in the code skill); once the wrong guide
/// is injected the model forces that skill's tool and produces a call that does
/// not match the schema. Hence the term-START condition.
pub fn contains(text: &str, trigger: &str) -> bool {
    let Some(first) = trigger.chars().next() else {
        return false;
    };
    // In scripts that do not use spaces (CJK, Korean) there is no such thing as
    // a term boundary; triggers there fall back to a raw substring, otherwise
    // they would NEVER match.
    if (first as u32) >= 0x0590 {
        return text.contains(trigger);
    }

    // char slices: the input may be Turkish and multi-byte. Walking by byte
    // index risks landing in the middle of a character.
    let t: Vec<char> = text.chars().collect();
    let g: Vec<char> = trigger.chars().collect();
    if g.is_empty() || g.len() > t.len() {
        return false;
    }

    // The shortness rule applies only to SINGLE-TERM roots; a trigger that
    // contains a space ("how many rows") is already a phrase and carries its
    // own specificity.
    let needs_whole_term = g.len() < WHOLE_TERM_LIMIT && !g.contains(&' ');

    for i in 0..=(t.len() - g.len()) {
        if t[i..i + g.len()] != g[..] {
            continue;
        }
        let at_start = i == 0 || !t[i - 1].is_alphanumeric();
        if !at_start {
            continue;
        }
        if !needs_whole_term {
            return true;
        }
        let end = i + g.len();
        if end == t.len() || !t[end].is_alphanumeric() {
            return true;
        }
    }
    false
}

/// The SUM OF LENGTHS of the matching triggers — NOT the count.
///
/// That way a specific phrase beats a generic word: in the sentence
/// "show this as a table" read-document's "as a table" (10) beats
/// create-document's "table" (5). Had the count been used both would score 1
/// and randomness would decide the order ON A TIE — two different guides for
/// the same sentence.
pub fn score(lowered_text: &str, triggers: &[String]) -> usize {
    triggers
        .iter()
        .filter(|t| contains(lowered_text, t))
        .map(|t| t.chars().count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turkish_lowercasing_maps_the_i_letters_correctly() {
        assert_eq!(lowercase("ISTANBUL"), "ıstanbul");
        assert_eq!(lowercase("İstanbul"), "istanbul");
        assert_eq!(lowercase("TABLE As"), "table as");
    }

    #[test]
    fn a_short_root_demands_a_whole_term() {
        // MEASURED REGRESSION (kept verbatim): the trigger "ara" was catching
        // "aralik". The Turkish data stays because it is the record of the bug.
        assert!(!contains("what is in the calendar", "cal"));
        assert!(contains("notlarimda ara", "ara"));
        assert!(!contains("bulut nerede", "bul"));
        assert!(contains("sunu bul", "bul"));
        // Same rule with an English trigger: "pdf" must not match "pdfs".
        assert!(!contains("pdfs everywhere", "pdf"));
        assert!(contains("give me a pdf", "pdf"));
    }

    #[test]
    fn a_long_root_keeps_prefix_matching() {
        // Turkish suffixes attach at the END; prefix matching is mandatory for
        // a long root. Data kept as the measured record.
        assert!(contains("change its lines", "line"));
        assert!(contains("what if we multiplied", "multipl"));
        // But a root sitting in the MIDDLE of a word must not match.
        assert!(!contains("alfabetik siralama", "betik"));
    }

    #[test]
    fn a_phrase_trigger_keeps_its_space_requirement() {
        assert!(contains("show this as a table", "as a table"));
        assert!(!contains("show this table", "as a table"));
    }

    #[test]
    fn score_is_a_length_sum_not_a_count() {
        let specific = vec!["as a table".to_string()];
        let generic = vec!["table".to_string(), "show".to_string()];
        let t = lowercase("Show this as a table");
        // Had the count been used the generic set (2) would beat the specific
        // one (1); by length the specific one wins.
        assert_eq!(score(&t, &specific), 10);
        assert_eq!(score(&t, &generic), 5 + 4);
    }

    #[test]
    fn a_non_matching_trigger_scores_zero() {
        assert_eq!(score("hello how are you", &["excel".to_string()]), 0);
        assert!(!contains("hello", ""));
    }
}
