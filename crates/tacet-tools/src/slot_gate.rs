//! A 48 KiB learned answer to "is this a request one of the extraction tools
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
//! this picks it on 107 of 131.
//!
//! THAT NUMBER WAS 117 HERE, AND 117 IS A DIFFERENT MODEL'S SCORE — the first
//! version of this gate, 2048 buckets, trained WITHOUT the other tools' work as
//! negatives. That version is the one the commit that shipped this file called
//! worse than useless: it labelled 38% of the other suites' 709 messages an
//! extraction request and cost fourteen top-three positions. The blob beside
//! this file is 16384 buckets WITH those negatives, and it scores 107 on this
//! set — ten fewer, for the reason the test set cannot show. Re-derived and
//! reproduced byte for byte on 6 Sep 2026 by
//! `esp32/train_slots.py 16384 && esp32/export_gate.py`. It is an easier problem — three classes against
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
/// PUBLIC SO IT CAN BE CROSS-CHECKED. `esp32/check.py` compares the trainer
/// against the C on the microcontroller; this one — the fold every Tacet user
/// actually runs — was compared against nothing but seven fixed strings, under
/// a test named `the_fold_matches_the_trainer` that never consulted the trainer.
/// It is exported so all three implementations can be driven over one corpus.
///
/// THE TWO SIDES MUST FOLD IDENTICALLY or the hashes address weights fitted to
/// different n-grams, and the only symptom is an accuracy that is merely worse.
/// Two bugs came out of that in the Python and C pair: `İ` lowercases to TWO
/// codepoints in Python, and reading every non-ASCII character as two bytes
/// mistakes an em dash for a Turkish letter. Both are why this reads the UTF-8
/// length from the lead byte and maps only the two-byte letters.
pub fn fold(message: &str) -> String {
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
        // EVERY ASCII WHITESPACE THE TRAINER COLLAPSES, enumerated to match it
        // exactly. `is_ascii_whitespace` is not used: it omits `\v`, and it
        // omits the four ASCII separators `\x1c`-`\x1f` (file, group, record,
        // unit) that `str.split` does break on — and that pasted EDI, CSV and
        // SMS content carries. Stopping short of them gave the same n-gram
        // COUNT with different bytes, which no length check can see and which
        // was measured flipping an argmax.
        if matches!(
            mapped,
            ' ' | '\t'
                | '\n'
                | '\r'
                | '\u{0b}'
                | '\u{0c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{1f}'
        ) {
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
    predict_with(BLOB, message)
}

/// The header of a blob, checked against the blob's own length, or `None`.
///
/// SEPARATE FROM `predict` SO IT CAN BE FED A BAD BLOB. The shipped file is
/// well formed, so a test that only ever reads it cannot tell this guard from
/// its absence: the audit of 9 Oct 2026 (T2) deleted the guard in a copy and
/// all four tests in this file still passed. The arithmetic is checked because
/// the counts come from the file — `u32::MAX * 3` wraps on a 32-bit target and
/// would let a short weight slice past the length comparison.
fn header(blob: &[u8]) -> Option<(usize, usize, &[u8])> {
    if blob.len() < 8 {
        return None;
    }
    let buckets = u32::from_le_bytes(blob[0..4].try_into().ok()?) as usize;
    let classes = u32::from_le_bytes(blob[4..8].try_into().ok()?) as usize;
    let weights = &blob[8..];
    if classes != CLASSES.len() || buckets == 0 || weights.len() != buckets.checked_mul(classes)? {
        return None;
    }
    Some((buckets, classes, weights))
}

fn predict_with(blob: &[u8], message: &str) -> Option<&'static str> {
    let (buckets, classes, weights) = header(blob)?;

    let folded = fold(message);
    let text = folded.as_bytes();

    // NO WORD IS NO EVIDENCE. The fold drops everything outside ASCII and the
    // Turkish letters but KEEPS ASCII punctuation, so "привет, как дела?"
    // folds to " , ? " and the model votes on punctuation n-grams alone. It
    // was never shown a message like that and its answer is noise: measured
    // 10 Oct 2026, that one gave acc=[-3,6,-1] and `search_filter`, a bare "?"
    // gave [-5,5,1] and `search_filter`, and "." gave [-3,-2,5] and
    // `message_intent` — each a +12 for the Extract profile on small talk.
    // An earlier fix abstained only when every sum was exactly 0, which caught
    // "спасибо" (no n-gram at all) and none of these. A message with no ASCII
    // letter left after the fold has said nothing the model can read, so it
    // abstains and the trigger list decides alone.
    //
    // A LETTER, NOT A LETTER OR DIGIT. The fold keeps digits too, so with
    // `is_ascii_alphanumeric` here a non-Latin message with a number in it
    // folded to digits and spaces and was voted on by those alone: "спасибо 2"
    // gave `search_filter`, "привет 5$" `message_intent`, 270 of "спасибо 0" to
    // "спасибо 999" fired, and two `core-zh.json` calculate requests ("…原价
    // 6980，…打八五折…" and "那把 1280 和 460 这两项加起来是多少？") gave
    // `search_filter` — a +12 for Extract on a sum the model cannot read.
    //
    // This check, with the tie-break below, is the one place the Rust departs
    // from `esp32/check.py` and the C, so both were measured over every message
    // in `benchmarks/` (10 Oct 2026): the 150 in `tasks/` (the 131 above among
    // them) predict exactly as before, so 107/131 stands. Of the 1105 in the
    // other suites the gate fired on 104 at 4f1c1a8; 32 of those hashed no
    // n-gram at all and are the tie-break's. Of the other 72 it now fires on
    // 38 — the 34 it no longer fires on are 31 of `core-ru.json` ("Привет! Как
    // дела?", "Который сейчас час?") and 3 of `core-zh.json` (one punctuation
    // only, the two calculate requests above), none of them an extraction.
    //
    // NOT SETTLED HERE: one short Latin token after non-Latin text is still
    // read — "Как дела? ok" gives `search_filter`. How much ASCII is enough
    // evidence is a separate question from whether there is any.
    if !text.iter().any(u8::is_ascii_alphabetic) {
        return None;
    }

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

    // A TIE GOES TO "none", THE FIRST CLASS. This used `max_by_key`, which
    // returns the LAST of equal maxima, so a model that could not decide
    // between "none" and a tool fired the tool: "iw" ([6,-12,6]) and "oc"
    // ([9,-18,9]) gave `message_intent`, and before the check above every
    // message with no n-gram at all ([0,0,0]) did too — "спасибо", "谢谢" and
    // the empty string put `message_intent`, `search_filter`, `web_search` and
    // `web_fetch` in the router's top four (audit M19, measured 9 Oct 2026).
    // Keeping the FIRST maximum is also what the numbers in this file were
    // measured with: `esp32/check.py` takes Python's `max`, `esp32/slots.c` a
    // strict `>` from class 0, and both keep the first. The Rust was the one
    // implementation that disagreed.
    let mut best = 0;
    for (i, &v) in acc.iter().enumerate().skip(1) {
        if v > acc[best] {
            best = i;
        }
    }
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

    /// A FEW FIXED SHAPES, and deliberately not a claim to match the trainer.
    ///
    /// This test used to be called `the_fold_matches_the_trainer` and never
    /// consulted the trainer — it asserted seven strings someone had typed. The
    /// comparison that earns that name is `esp32/check.py`, which now drives the
    /// trainer, the C and THIS fold over one corpus and fails if any two
    /// disagree. What is left here is a fast smoke test of the rules, so an
    /// obvious break shows up in `cargo test` without a Python round trip.
    #[test]
    fn the_fold_obeys_its_own_rules() {
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
    ///
    /// THIS USED TO READ THE SHIPPED BLOB'S HEADER, which is well formed, so it
    /// passed with the guard deleted (audit T2). It now feeds the guard blobs
    /// that are wrong in each way the guard checks, through the same code path
    /// `predict` takes, and a message long enough to hash many n-grams so that
    /// an unguarded read would index past the end of the weights.
    #[test]
    fn a_malformed_blob_is_refused_rather_than_indexed() {
        // The control: the shipped blob passes the guard, so the refusals below
        // are about the malformation and not a guard that refuses everything.
        assert!(header(BLOB).is_some(), "the shipped blob was refused");

        let message = "free things to do in London this weekend";
        let well_formed = |buckets: u32, classes: u32, weights: usize| {
            let mut b = Vec::new();
            b.extend_from_slice(&buckets.to_le_bytes());
            b.extend_from_slice(&classes.to_le_bytes());
            b.resize(8 + weights, 1);
            b
        };
        let truncated = {
            let mut b = BLOB.to_vec();
            b.truncate(BLOB.len() / 2);
            b
        };
        for (what, blob) in [
            ("empty", Vec::new()),
            ("shorter than the header", vec![0u8; 7]),
            ("truncated", truncated),
            ("one weight short", well_formed(1024, 3, 1024 * 3 - 1)),
            ("one weight long", well_formed(1024, 3, 1024 * 3 + 1)),
            ("wrong class count", well_formed(1024, 2, 1024 * 2)),
            ("zero buckets", well_formed(0, 3, 0)),
            ("counts that overflow", well_formed(u32::MAX, 3, 16)),
        ] {
            assert_eq!(header(&blob), None, "accepted a blob that is {what}");
            assert_eq!(
                predict_with(&blob, message),
                None,
                "predicted from a blob that is {what}"
            );
        }
    }

    /// NON-LATIN SMALL TALK AND THE EMPTY STRING carry no word the model can
    /// see, so the gate abstains rather than letting a tie-break pick a tool.
    /// Before the first fix the first six returned `Some("message_intent")`.
    /// The rest kept their ASCII punctuation through the fold and were voted
    /// on by punctuation n-grams alone, which an all-zero check did not catch:
    /// "привет, как дела?" and "?" gave `search_filter`, "." and the line from
    /// `benchmarks/core/core-ru.json` gave `message_intent`. The last two kept
    /// their digits through the fold and passed a letter-or-digit check:
    /// "спасибо 2" and the `core-zh.json` calculate request each gave
    /// `search_filter`.
    #[test]
    fn a_message_with_no_ngrams_reaches_neither_tool() {
        for quiet in [
            "спасибо",
            "谢谢",
            "こんにちは",
            "שלום",
            "",
            "   ",
            "привет, как дела?",
            "Привет! Как дела?",
            "谢谢？!",
            "?",
            ".",
            "спасибо 2",
            "那把 1280 和 460 这两项加起来是多少？",
        ] {
            assert_eq!(predict(quiet), None, "fired on {quiet:?}");
        }
    }

    /// A MODEL THAT CANNOT DECIDE BETWEEN "none" AND A TOOL DOES NOT FIRE IT.
    /// Both strings tie "none" with `message_intent` exactly — [6,-12,6] and
    /// [9,-18,9] — and `max_by_key`, which keeps the last of equal maxima,
    /// returned `Some("message_intent")` for both.
    #[test]
    fn a_tie_with_none_reaches_neither_tool() {
        for tied in ["iw", "oc"] {
            assert_eq!(predict(tied), None, "a tie fired on {tied:?}");
        }
    }
}
