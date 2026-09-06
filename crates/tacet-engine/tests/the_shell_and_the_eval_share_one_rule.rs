//! What happens to a pass that was cut off — one rule, one place.
//!
//! WHY THIS EXISTS. The eval and the shell each carried their own copy of this
//! decision and they had already drifted. The eval traded a cut-off pass for the
//! final pass — keeping every tool result the earlier passes collected and
//! giving the model a tools-free pass to say something. The shell KILLED THE
//! TURN: a one-line warning and no answer, with the results thrown away. Four
//! steps of the shipped baseline ended that way in the eval before it was fixed,
//! one of them holding two successful `write_code` calls; the shell was doing it
//! to real users the whole time.
//!
//! The eval's entire claim is that it measures the shell. Two loops implementing
//! one rule is how that stops being true.

use tacet_engine::{MAX_TURNS, StopReason, cut_off_can_be_retried};

#[test]
fn a_cut_off_pass_is_traded_for_the_final_pass() {
    for stop in [StopReason::Length, StopReason::CallTooLong] {
        assert!(
            cut_off_can_be_retried(stop, 0),
            "{stop:?} on the first pass must cost the pass, not the turn — the \
             tool results already collected are the whole point"
        );
        assert!(cut_off_can_be_retried(stop, MAX_TURNS - 2));
    }
}

#[test]
fn the_last_pass_has_nothing_left_to_trade() {
    for stop in [StopReason::Length, StopReason::CallTooLong] {
        assert!(
            !cut_off_can_be_retried(stop, MAX_TURNS - 1),
            "there is no pass after the last one; retrying here would loop"
        );
    }
}

/// A cancel is the one incomplete ending the user ASKED for. Offering another
/// pass would be arguing with Ctrl-C.
#[test]
fn a_cancel_is_never_retried() {
    for pass in 0..MAX_TURNS {
        assert!(!cut_off_can_be_retried(StopReason::Cancelled, pass));
    }
}

/// A complete generation never reaches this decision, but if the rule is ever
/// asked, it must not say "retry" — that would spend a pass on a finished turn.
#[test]
fn a_finished_generation_is_not_a_cut_off() {
    for stop in [
        StopReason::Token,
        StopReason::ConstraintDone,
        StopReason::StopString,
        StopReason::Loop,
    ] {
        assert!(stop.is_complete());
        assert!(!cut_off_can_be_retried(stop, 0));
    }
}
