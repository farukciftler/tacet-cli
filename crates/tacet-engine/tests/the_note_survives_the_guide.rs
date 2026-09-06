//! The turn's note must reach the model, whatever the guide costs.
//!
//! WHY THIS EXISTS. The web nudge is one sentence and it is the measured reason
//! a small model reaches for `web_search` at all. Both callers appended it to
//! the GUIDE STRING, so it entered `with_guide` and was subject to
//! `GUIDE_LIMIT`. Sitting at the end, it was the first thing the cap took.
//!
//! That is not a hypothetical overflow. Measured over the eighteen shipped
//! skills, seven produce a guide long enough that guide + nudge crosses 960
//! characters — and the cap then walks back to the previous newline, taking the
//! guide's own closing envelope with it. So on exactly the turns with the most
//! guidance, the highest-priority sentence of the turn was deleted, the guide
//! was left half-closed, and nothing reported either.
//!
//! `with_note` gives it its own budget. These tests hold that.

use tacet_engine::prompt::{GUIDE_LIMIT, NOTE_LIMIT, Prompt};
use tacet_engine::{Template, WEB_NUDGE};

/// A guide at the very edge of its budget — the shape that used to eat the note.
fn a_guide_that_fills_its_budget() -> String {
    let line = "- a rule the model must not break, written out at length.\n";
    let mut g = String::new();
    while g.chars().count() + line.chars().count() <= GUIDE_LIMIT {
        g.push_str(line);
    }
    g
}

#[test]
fn a_full_guide_does_not_delete_the_note() {
    let guide = a_guide_that_fills_its_budget();
    assert!(
        guide.chars().count() + WEB_NUDGE.chars().count() > GUIDE_LIMIT,
        "this test is only meaningful when the two together overflow"
    );
    let text = Prompt::new("sys", "ferry times to Bozcaada")
        .with_guide(&guide)
        .with_note(WEB_NUDGE)
        .text();
    assert!(
        text.contains("Call the web_search tool first"),
        "the note was cut by the guide's budget — the defect this slot exists to fix"
    );
    assert!(
        text.contains("a rule the model must not break"),
        "and the guide is still there"
    );
}

/// The note must arrive in every wire format, not only the one the tests read.
#[test]
fn every_template_carries_the_note() {
    let p = Prompt::new("sys", "ferry times")
        .with_guide("use the tool")
        .with_note(WEB_NUDGE);
    for t in [Template::Plain, Template::ChatML, Template::Gemma] {
        let text = p.text_with_template(t);
        assert!(
            text.contains("Call the web_search tool first"),
            "{t:?} dropped the note"
        );
        assert!(text.contains("use the tool"), "{t:?} dropped the guide");
    }
}

/// A note with no guide still opens the fence. The web nudge fires on messages
/// that match no skill at all — which is most of them.
#[test]
fn a_note_with_no_guide_still_reaches_the_model() {
    let text = Prompt::new("sys", "ferry times")
        .with_note(WEB_NUDGE)
        .text();
    assert!(text.contains("<guidance>"));
    assert!(text.contains("Call the web_search tool first"));
}

/// The note comes AFTER the guide. This file's header says the last blocks
/// carry the most weight in a small model, and the note is the instruction
/// chosen for THIS message.
#[test]
fn the_note_is_the_last_thing_in_the_fence() {
    let text = Prompt::new("sys", "q")
        .with_guide("GUIDE-MARKER")
        .with_note("NOTE-MARKER")
        .text();
    let g = text.find("GUIDE-MARKER").expect("guide present");
    let n = text.find("NOTE-MARKER").expect("note present");
    assert!(g < n, "the note must sit after the guide");
}

#[test]
fn the_note_has_its_own_cap() {
    let long = "x".repeat(NOTE_LIMIT * 3);
    let p = Prompt::new("sys", "q").with_note(&long);
    assert_eq!(
        p.note.as_ref().map(|n| n.chars().count()),
        Some(NOTE_LIMIT),
        "a note that does not fit in two sentences is a skill, not a note"
    );
}
