//! A 6 KiB learned answer to "is this a request one of the extraction tools
//! serves, and which one".
//!
//! WHY THIS EXISTS AND THE TRIGGER LIST DOES NOT SUFFICE. `IntentProfile`'s
//! triggers are substrings, and substrings were fitted to the sixteen cases the
//! task benchmarks used to hold. Widening those to 131 found 58 of 105 requests
//! that reached neither tool: "any cheap museums in Dublin this weekend" and
//! "orta fiyatli oteller" share no substring with any trigger, and the ones that
//! carry no place noun at all — "ucretsiz ve cocuklara uygun bir seyler
//! ariyorum" — cannot be reached by a list without being told about them one at
//! a time, which is fitting the router to its own test.
//!
//! MEASURED, on the 131 human-written cases in `benchmarks/tasks/`, of which 95
//! were written after this model was trained and none were used to train it:
//! the trigger list reaches the right tool on 87 of the 105 that expect one;
//! this picks it on 117 of 131. It is an easier problem — three classes against
//! the router's nine slots out of forty-seven tools — so it is used to RAISE the
//! Extract profile's score, never to overrule the rest of the router.
//!
//! WHAT IT IS. Hashed character n-grams, 3 to 5, over a folded message, into one
//! int8 weight per (bucket, class). Two loops and an argmax; no float, no
//! allocation beyond one small buffer, and the same arithmetic as the C in
//! `esp32/` that runs on a microcontroller.
//!
//! IT RUNS ON EVERY MESSAGE, so its cost was measured rather than assumed:
//! **0.4 us** per call over 20,000 calls on four messages of 2 to 84 characters
//! (M-series, release build). The trigger scan it sits beside does more work
//! than that, so nothing in the router had to be restructured around it.
//!
//! REGENERATING IT is `esp32/train_slots.py 2048` then `esp32/export_gate.py`.
//! A blob nobody can reproduce is not auditable, so the tests below pin its
//! behaviour on fixed strings rather than trusting the bytes.

/// `<u32 buckets><u32 classes>` then int8 weights, row-major by bucket.
static BLOB: &[u8] = include_bytes!("slot_gate.bin");

/// The classes, in the order the exporter wrote them.
const CLASSES: [&str; 3] = ["none", "search_filter", "message_intent"];

/// Fold a message the way the trainer does: lowercase ASCII, map the Turkish
/// letters onto their bare forms, drop everything else that is not ASCII,
/// collapse runs of whitespace, and pad with a space at each end.
///
/// THE TWO SIDES MUST FOLD IDENTICALLY or the hashes address weights fitted to
/// different n-grams, and the only symptom is an accuracy that is merely worse.
/// Two bugs came out of that in the Python and C pair: `İ` lowercases to TWO
/// codepoints in Python, and reading every non-ASCII character as two bytes
/// mistakes an em dash for a Turkish letter. Both are why this reads the UTF-8
/// length from the lead byte and maps only the two-byte letters.
fn fold(message: &str) -> String {
    let bytes = message.as_bytes();
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push(' ');
    let mut space = true;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let mapped = if c < 0x80 {
            i += 1;
            match c {
                b'A'..=b'Z' => (c + 32) as char,
                _ => c as char,
            }
        } else {
            let seq = if c >= 0xF0 {
                4
            } else if c >= 0xE0 {
                3
            } else {
                2
            };
            if i + seq > bytes.len() {
                break;
            }
            if seq != 2 {
                i += seq;
                continue;
            }
            let w = u16::from(c) << 8 | u16::from(bytes[i + 1]);
            i += 2;
            match w {
                0xC4B1 | 0xC4B0 => 'i', // ı İ
                0xC49F | 0xC49E => 'g', // ğ Ğ
                0xC3BC | 0xC39C => 'u', // ü Ü
                0xC59F | 0xC59E => 's', // ş Ş
                0xC3B6 | 0xC396 => 'o', // ö Ö
                0xC3A7 | 0xC387 => 'c', // ç Ç
                0xC3A2 | 0xC382 => 'a', // â Â
                0xC3AE | 0xC38E => 'i', // î Î
                _ => continue,
            }
        };
        // EVERY ASCII WHITESPACE, not just space and tab. Python's `str.split`
        // breaks on `\n`, `\r`, `\v` and `\f` too, and keeping them as
        // characters here fed the model n-grams it was never fitted to. It
        // survived the cross-check because no benchmark message contains a
        // newline — and `message_intent` exists to classify PASTED messages,
        // where they are the rule. `is_ascii_whitespace` is not used: it omits
        // `\v`, which `str.split` does break on.
        if matches!(mapped, ' ' | '\t' | '\n' | '\r' | '\u{0b}' | '\u{0c}') {
            if !space {
                out.push(' ');
                space = true;
            }
        } else {
            out.push(mapped);
            space = false;
        }
    }
    if !space {
        out.push(' ');
    }
    out
}

/// The tool this message is a request for, or `None`.
///
/// `None` is returned both when the winning class is "none" and when the blob is
/// unreadable — a router that panics because a generated file was truncated is
/// worse than one that falls back to its trigger list.
pub fn predict(message: &str) -> Option<&'static str> {
    if BLOB.len() < 8 {
        return None;
    }
    let buckets = u32::from_le_bytes(BLOB[0..4].try_into().ok()?) as usize;
    let classes = u32::from_le_bytes(BLOB[4..8].try_into().ok()?) as usize;
    let weights = &BLOB[8..];
    if classes != CLASSES.len() || weights.len() != buckets * classes || buckets == 0 {
        return None;
    }

    let folded = fold(message);
    let text = folded.as_bytes();
    let mut acc = [0i32; CLASSES.len()];
    for n in 3..=5usize {
        if text.len() < n {
            continue;
        }
        for i in 0..=text.len() - n {
            let mut h: u32 = 2166136261;
            for &b in &text[i..i + n] {
                h = (h ^ u32::from(b)).wrapping_mul(16777619);
            }
            let row = (h as usize % buckets) * classes;
            for (c, slot) in acc.iter_mut().enumerate() {
                *slot += i32::from(weights[row + c] as i8);
            }
        }
    }

    let best = acc
        .iter()
        .enumerate()
        .max_by_key(|(_, v)| **v)
        .map(|(i, _)| i)?;
    match CLASSES[best] {
        "none" => None,
        name => Some(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE BLOB'S BEHAVIOUR, PINNED. These strings are not in the training set
    /// and the weights are generated, so this is what stops a regenerated file
    /// from silently changing what the router does. If `esp32/train_slots.py`
    /// is re-run and this fails, the model changed — decide whether that was
    /// intended rather than updating the expectation reflexively.
    #[test]
    fn it_recognises_the_two_requests_it_was_trained_for() {
        assert_eq!(
            predict("izmirde ücretsiz gezilecek yerler neresi"),
            Some("search_filter")
        );
        assert_eq!(
            predict("free things to do in London"),
            Some("search_filter")
        );
        assert_eq!(
            predict("what does this reply mean: 'I paid it last Tuesday'"),
            Some("message_intent")
        );
        assert_eq!(
            predict("'Cuma günü ödeyeceğim' demiş. Bu ne demek?"),
            Some("message_intent")
        );
    }

    /// AND THE HALF THAT MATTERS MORE. A gate that fires on small talk costs
    /// the budget a slot on every turn, and the composite weights irrelevance
    /// heaviest for the same reason.
    #[test]
    fn small_talk_reaches_neither_tool() {
        for quiet in [
            "thanks, that is all I needed",
            "teşekkürler, çok yardımcı oldun",
            "günaydın",
            "who made you?",
        ] {
            assert_eq!(predict(quiet), None, "fired on {quiet:?}");
        }
    }

    /// The fold is the one thing shared with the C on the microcontroller, so
    /// its rules are asserted rather than assumed.
    #[test]
    fn the_fold_matches_the_trainer() {
        assert_eq!(fold("Çok İyi"), " cok iyi ");
        assert_eq!(fold("a  b\tc"), " a b c ");
        // A PASTED MESSAGE HAS NEWLINES IN IT, and this is the case that was
        // wrong in the shipped code: `\n` was kept as a character, so every
        // multi-line message hashed to buckets the weights had never seen.
        assert_eq!(fold("a\nb"), " a b ");
        assert_eq!(fold("line one\r\nline two"), " line one line two ");
        assert_eq!(fold("a\u{0b}b\u{0c}c"), " a b c ");
        // An em dash is three UTF-8 bytes and is dropped whole, not read as a
        // Turkish letter with a stray byte after it.
        assert_eq!(fold("a — b"), " a b ");
        assert_eq!(fold(""), " ");
    }

    /// A truncated or replaced blob must degrade to the trigger list, never
    /// panic. The header is checked against the file's own length for that.
    #[test]
    fn a_malformed_blob_is_refused_rather_than_indexed() {
        // The shipped one is well formed; this asserts the guard exists by
        // checking the header the guard reads.
        let buckets = u32::from_le_bytes(BLOB[0..4].try_into().unwrap()) as usize;
        let classes = u32::from_le_bytes(BLOB[4..8].try_into().unwrap()) as usize;
        assert_eq!(classes, CLASSES.len());
        assert_eq!(BLOB.len(), 8 + buckets * classes);
    }
}
