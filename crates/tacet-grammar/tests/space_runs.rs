//! The space-run bound, and the disagreement it used to cause.
//!
//! THIS FILE REPLACES `tmp_space_probe.rs`, three tests that printed and
//! asserted nothing. They ran green while recording a real defect in their own
//! output — which is the failure mode this repository has a rule against, and
//! the reason the defect survived: nothing could go red.
//!
//! WHAT THEY WERE RECORDING. `MAX_SPACE_RUN` bounds consecutive whitespace so a
//! finished call cannot drift forever at a structural position (measured once at
//! 14 041 tokens of spaces after complete arguments). The counter was applied to
//! every space, including spaces INSIDE a string, where a space is content. So
//! the seventeenth space of an indented line was refused by the automaton while
//! `allowed_prefixes` still offered it through the free-text path: the mask says
//! yes, the automaton says no. A `write_code` call carrying a Python line
//! indented past sixteen columns died in the middle of generating.

use std::sync::Arc;
use tacet_grammar::Grammar;
use tacet_kernel::{ArgSchema, Field};

fn text_field() -> Arc<Grammar> {
    Grammar::compile(&ArgSchema::object(vec![
        Field::new("code", ArgSchema::text()).required(),
    ]))
}

/// THE BUG, as an assertion. Deeply indented content is ordinary text.
#[test]
fn a_string_body_takes_more_than_sixteen_spaces() {
    let g = text_field();
    let mut st = g.state();
    st.advance(r#"{"code":""#).expect("opening the string");
    for i in 0..64 {
        st.advance(" ")
            .unwrap_or_else(|e| panic!("space #{} inside a string was refused: {e:?}", i + 1));
    }
    st.advance(r#"pass"}"#).expect("the call still closes");
    assert!(
        st.is_done(),
        "a valid call with a deep indent must complete"
    );
}

/// AND THE HALF THAT MUST NOT REGRESS. The bound still holds where it was
/// written for: structural whitespace, outside any string.
#[test]
fn structural_whitespace_is_still_bounded() {
    let g = text_field();
    let mut st = g.state();
    st.advance("{").expect("opening the object");
    let refused = (0..64).find(|_| st.advance(" ").is_err());
    assert!(
        refused.is_some(),
        "unbounded structural whitespace is the cycle the bound exists to cut"
    );
}

/// THE PROPERTY BOTH OF THOSE ARE REALLY ABOUT. Whatever the automaton will
/// refuse, the mask must not offer — anywhere, at any depth of indent.
#[test]
fn the_mask_never_offers_a_space_the_automaton_refuses() {
    let g = text_field();
    for opening in [r#"{"code":""#, "{", r#"{"code":"x"#] {
        let mut st = g.state();
        st.advance(opening).expect("prefix is valid");
        for step in 0..40 {
            let offered = st.allowed_prefixes().contains(' ');
            let accepted = st.clone().advance(" ").is_ok();
            assert_eq!(
                offered, accepted,
                "after {:?} plus {step} spaces the mask says {offered} and the automaton says {accepted}",
                opening
            );
            if !accepted {
                break;
            }
            st.advance(" ").expect("just checked it is accepted");
        }
    }
}
