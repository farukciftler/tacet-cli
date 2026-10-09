//! The value of this crate is measured by its tests: if the grammar is wrong the
//! model silently produces broken JSON and the error surfaces in the tool layer,
//! far too late.
//!
//! The common backbone of the tests: every accepted text must also parse with
//! serde_json (that is, the grammar does not step OUTSIDE JSON), and every
//! rejected text is either not JSON or does not match the schema (that is, the
//! grammar is not LOOSER than the schema).

use super::*;
use std::sync::Arc;
use tacet_kernel::{ArgSchema, Field};

fn sample_schema() -> ArgSchema {
    ArgSchema::object(vec![
        Field::new("query", ArgSchema::text()).required(),
        Field::new("scope", ArgSchema::choice(["all", "near"])),
        Field::new("count", ArgSchema::integer().range(Some(1.0), Some(50.0))),
        Field::new("deep", ArgSchema::bool()),
    ])
}

fn grammar(schema: &ArgSchema) -> Arc<Grammar> {
    Grammar::compile(schema)
}

/// Feeds the text from start to end and says whether it closed.
fn accepts(schema: &ArgSchema, text: &str) -> bool {
    let g = grammar(schema);
    let mut s = g.state();
    s.advance(text).is_ok() && s.is_done()
}

/// Is the accepted text really valid JSON — the independent witness that the
/// grammar does not deviate from JSON.
fn accepts_and_is_json(schema: &ArgSchema, text: &str) -> bool {
    accepts(schema, text) && serde_json::from_str::<serde_json::Value>(text).is_ok()
}

// ------------------------------------------------------------ basic acceptance

#[test]
fn a_valid_object_is_accepted() {
    let s = sample_schema();
    assert!(accepts_and_is_json(&s, r#"{"query":"report"}"#));
    assert!(accepts_and_is_json(
        &s,
        r#"{"query":"report","scope":"near"}"#
    ));
    assert!(accepts_and_is_json(
        &s,
        r#"{"query":"a","scope":"all","count":12,"deep":true}"#
    ));
}

#[test]
fn whitespace_is_free_at_structural_positions() {
    let s = sample_schema();
    assert!(accepts_and_is_json(
        &s,
        "{ \"query\" : \"a\" , \"count\" : 3 }"
    ));
    assert!(accepts_and_is_json(&s, "{\n  \"query\": \"a\"\n}"));
}

#[test]
fn field_order_is_free_but_there_is_no_invented_field() {
    let s = sample_schema();
    // The order does not depend on the schema order; the contract is a set, not
    // a list.
    assert!(accepts_and_is_json(&s, r#"{"count":5,"query":"a"}"#));
    // Not even the FIRST letter of a key that is not in the schema can be produced.
    assert!(!accepts(&s, r#"{"query":"a","other":1}"#));
    assert!(!accepts(&s, r#"{"x":1}"#));
}

#[test]
fn a_required_field_cannot_be_skipped() {
    let s = sample_schema();
    assert!(!accepts(&s, "{}"));
    assert!(!accepts(&s, r#"{"scope":"all"}"#));
    // Without the required field written, '}' never opens: the closing character
    // is not allowed.
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"scope":"all""#).unwrap();
    assert!(!d.allowed_prefixes().contains('}'));
    assert!(d.allowed_prefixes().contains(','));
}

#[test]
fn the_same_key_cannot_be_produced_twice() {
    let s = sample_schema();
    assert!(!accepts(&s, r#"{"query":"a","query":"b"}"#));
}

#[test]
fn no_comma_opens_after_the_last_field() {
    // A single-field schema: once the field is written, ',' leads into a dead path.
    let s = ArgSchema::object(vec![Field::new("a", ArgSchema::bool()).required()]);
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"a":true"#).unwrap();
    let allowed = d.allowed_prefixes();
    assert!(allowed.contains('}'));
    assert!(!allowed.contains(','));
    assert!(!accepts(&s, r#"{"a":true,}"#));
}

// ------------------------------------------------------------------------ enum

#[test]
fn a_value_outside_the_enum_cannot_be_produced() {
    let s = sample_schema();
    assert!(!accepts(&s, r#"{"query":"a","scope":"far"}"#));
    assert!(!accepts(&s, r#"{"query":"a","scope":"allx"}"#));
    // A half-written enum does not close either.
    assert!(!accepts(&s, r#"{"query":"a","scope":"ne"}"#));
}

#[test]
fn while_writing_an_enum_only_the_next_letters_are_allowed() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"query":"a","scope":""#).unwrap();
    let allowed = d.allowed_prefixes();
    assert!(allowed.contains('a') && allowed.contains('n'));
    assert!(!allowed.contains('u'));
    assert!(!allowed.is_text_body(), "an enum field is not free text");
    d.advance("al").unwrap();
    let allowed = d.allowed_prefixes();
    assert_eq!(allowed.chars().collect::<Vec<_>>(), vec!['l']);
}

// ------------------------------------------------------------------------ text

#[test]
fn an_escaped_string_is_handled_correctly() {
    let s = ArgSchema::object(vec![Field::new("m", ArgSchema::text()).required()]);
    assert!(accepts_and_is_json(
        &s,
        r#"{"m":"quote: \" and backslash: \\"}"#
    ));
    assert!(accepts_and_is_json(&s, r#"{"m":"line\nbreak\ttab"}"#));
    // An invalid escape letter.
    assert!(!accepts(&s, r#"{"m":"\q"}"#));
    // An unescaped control character is forbidden in JSON.
    assert!(!accepts(&s, "{\"m\":\"a\nb\"}"));
}

#[test]
fn a_unicode_escape_requires_four_hex_digits() {
    let s = ArgSchema::object(vec![Field::new("m", ArgSchema::text()).required()]);
    assert!(accepts_and_is_json(&s, r#"{"m":"ç �"}"#));
    assert!(!accepts(&s, r#"{"m":"\u00e"}"#));
    assert!(!accepts(&s, r#"{"m":"\u00zz"}"#));

    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"m":"\u00"#).unwrap();
    let allowed = d.allowed_prefixes();
    assert!(allowed.contains('a') && allowed.contains('F') && allowed.contains('9'));
    assert!(!allowed.contains('g') && !allowed.contains('"'));
}

#[test]
fn multi_byte_unicode_is_free_in_the_body() {
    let s = ArgSchema::object(vec![Field::new("m", ArgSchema::text()).required()]);
    assert!(accepts_and_is_json(
        &s,
        r#"{"m":"very secret — tacet ✓ 世界"}"#
    ));
}

#[test]
fn the_text_length_limit_forces_closing() {
    let s = ArgSchema::object(vec![Field::new("m", ArgSchema::text()).required()]);
    let bounded = ArgSchema::object(vec![
        Field::new(
            "m",
            ArgSchema {
                kind: tacet_kernel::SchemaKind::Text {
                    max_length: Some(3),
                },
                description: None,
            },
        )
        .required(),
    ]);
    assert!(accepts_and_is_json(&s, r#"{"m":"abcdef"}"#));
    assert!(accepts_and_is_json(&bounded, r#"{"m":"abc"}"#));
    assert!(!accepts(&bounded, r#"{"m":"abcd"}"#));
    // Once the limit is reached the only exit is the closing quote.
    let g = grammar(&bounded);
    let mut d = g.state();
    d.advance(r#"{"m":"abc"#).unwrap();
    let allowed = d.allowed_prefixes();
    assert!(!allowed.is_text_body());
    assert_eq!(allowed.chars().collect::<Vec<_>>(), vec!['"']);
}

// ---------------------------------------------------------------------- number

#[test]
fn the_number_grammar_obeys_json_rules() {
    let s = ArgSchema::object(vec![Field::new("n", ArgSchema::number()).required()]);
    for good in [
        "0", "-0", "12", "-12", "1.5", "-1.5", "0.5", "1e3", "1E+3", "2.5e-4",
    ] {
        assert!(
            accepts_and_is_json(&s, &format!(r#"{{"n":{good}}}"#)),
            "accept: {good}"
        );
    }
    for bad in ["01", "1.", ".5", "+1", "1e", "1e+", "--1", "1.2.3", "0x1"] {
        assert!(!accepts(&s, &format!(r#"{{"n":{bad}}}"#)), "reject: {bad}");
    }
}

#[test]
fn an_integer_cannot_produce_a_fraction_or_an_exponent() {
    let s = ArgSchema::object(vec![Field::new("n", ArgSchema::integer()).required()]);
    assert!(accepts_and_is_json(&s, r#"{"n":42}"#));
    assert!(!accepts(&s, r#"{"n":4.2}"#));
    assert!(!accepts(&s, r#"{"n":4e2}"#));
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"n":4"#).unwrap();
    let allowed = d.allowed_prefixes();
    assert!(!allowed.contains('.') && !allowed.contains('e'));
    // Because the number can close without consuming, the parent frame's '}'
    // permission is visible.
    assert!(allowed.contains('}'));
}

#[test]
fn in_a_non_negative_range_the_minus_sign_never_opens() {
    let s = ArgSchema::object(vec![
        Field::new("n", ArgSchema::integer().range(Some(1.0), Some(50.0))).required(),
    ]);
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"n":"#).unwrap();
    assert!(!d.allowed_prefixes().contains('-'));
    assert!(!accepts(&s, r#"{"n":-5}"#));
    assert!(!accepts(&s, r#"{"n":99}"#));
    assert!(accepts_and_is_json(&s, r#"{"n":50}"#));
}

// ------------------------------------------------------------------------ bool

#[test]
fn bool_produces_only_true_and_false() {
    let s = ArgSchema::object(vec![Field::new("b", ArgSchema::bool()).required()]);
    assert!(accepts_and_is_json(&s, r#"{"b":true}"#));
    assert!(accepts_and_is_json(&s, r#"{"b":false}"#));
    assert!(!accepts(&s, r#"{"b":True}"#));
    assert!(!accepts(&s, r#"{"b":1}"#));
    assert!(!accepts(&s, r#"{"b":"true"}"#));
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"b":"#).unwrap();
    assert_eq!(
        d.allowed_prefixes().chars().collect::<Vec<_>>(),
        vec!['f', 't']
    );
}

// ------------------------------------------------------------ nested structures

#[test]
fn nested_object_and_array() {
    let s = ArgSchema::object(vec![
        Field::new(
            "target",
            ArgSchema::object(vec![
                Field::new("path", ArgSchema::text()).required(),
                Field::new("line", ArgSchema::integer()),
            ]),
        )
        .required(),
        Field::new("tags", ArgSchema::array(ArgSchema::text())),
    ]);
    assert!(accepts_and_is_json(&s, r#"{"target":{"path":"a.txt"}}"#));
    assert!(accepts_and_is_json(
        &s,
        r#"{"target":{"path":"a.txt","line":3},"tags":["x","y"]}"#
    ));
    assert!(accepts_and_is_json(
        &s,
        r#"{"target":{"path":"a"},"tags":[]}"#
    ));
    // The required field of the inner object cannot be skipped either.
    assert!(!accepts(&s, r#"{"target":{}}"#));
    // The type of array items is fixed.
    assert!(!accepts(&s, r#"{"target":{"path":"a"},"tags":[1]}"#));
}

#[test]
fn array_length_bounds_are_enforced() {
    let s = ArgSchema::object(vec![
        Field::new(
            "d",
            ArgSchema::array(ArgSchema::integer()).length(Some(2), Some(3)),
        )
        .required(),
    ]);
    assert!(accepts_and_is_json(&s, r#"{"d":[1,2]}"#));
    assert!(accepts_and_is_json(&s, r#"{"d":[1,2,3]}"#));
    assert!(!accepts(&s, r#"{"d":[]}"#));
    assert!(!accepts(&s, r#"{"d":[1]}"#));
    assert!(!accepts(&s, r#"{"d":[1,2,3,4]}"#));

    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"d":[1,2,3"#).unwrap();
    let allowed = d.allowed_prefixes();
    assert!(allowed.contains(']'));
    assert!(
        !allowed.contains(','),
        "no comma must open at the upper bound"
    );
}

#[test]
fn nested_array_and_object_mixed() {
    let s = ArgSchema::object(vec![
        Field::new(
            "records",
            ArgSchema::array(ArgSchema::object(vec![
                Field::new("name", ArgSchema::text()).required(),
                Field::new("kind", ArgSchema::choice(["file", "dir"])).required(),
            ])),
        )
        .required(),
    ]);
    assert!(accepts_and_is_json(
        &s,
        r#"{"records":[{"name":"a","kind":"file"},{"name":"b","kind":"dir"}]}"#
    ));
    assert!(!accepts(&s, r#"{"records":[{"name":"a"}]}"#));
    assert!(!accepts(&s, r#"{"records":[{"name":"a","kind":"link"}]}"#));
}

#[test]
fn an_empty_schema_accepts_only_an_empty_object() {
    let s = ArgSchema::empty();
    assert!(accepts_and_is_json(&s, "{}"));
    assert!(accepts_and_is_json(&s, "{ }"));
    assert!(!accepts(&s, r#"{"a":1}"#));
}

// ------------------------------------------------------------------ state flow

#[test]
fn advance_does_not_corrupt_the_state_on_error() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"query":"#).unwrap();
    let err = d.advance("42").unwrap_err();
    assert!(matches!(err, GrammarError::UnexpectedCharacter { .. }));
    // Atomicity: the rejected chunk did not dirty the state, the correct value
    // can still be written.
    d.advance(r#""report"}"#).unwrap();
    assert!(d.is_done());
}

#[test]
fn trailing_input_after_closing_is_rejected() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"query":"a"}"#).unwrap();
    assert!(d.is_done());
    assert!(d.finish().is_ok());
    assert!(d.allowed_prefixes().can_finish());
    // If the model starts explaining "shall I do it?", it stops at the first letter.
    assert!(matches!(
        d.advance("Now").unwrap_err(),
        GrammarError::TrailingInput { .. }
    ));
    // Trailing whitespace is harmless.
    d.advance("  \n").unwrap();
}

#[test]
fn a_half_open_structure_does_not_count_as_finished() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"query":"a"#).unwrap();
    assert!(!d.is_done());
    assert_eq!(d.finish().unwrap_err(), GrammarError::Incomplete);
}

#[test]
fn the_allowed_set_is_never_left_dead() {
    // Along a valid generation, at every step either a character must be
    // producible or generation must be able to end; otherwise the model locks up.
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    for c in r#"{"query":"ac","count":7,"deep":false}"#.chars() {
        let allowed = d.allowed_prefixes();
        assert!(
            !allowed.is_empty() || allowed.can_finish(),
            "dead node: {allowed:?}"
        );
        assert!(
            allowed.contains(c),
            "'{c}' should have been allowed: {allowed:?}"
        );
        d = d.branch(c).unwrap();
    }
    assert!(d.is_done());
}

#[test]
fn in_a_number_range_a_dead_prefix_never_opens() {
    // [10,20]: after '2' is written only '0' can follow (20), because 21..29 are
    // out of range and 2 on its own is out of range too.
    let s = ArgSchema::object(vec![
        Field::new("n", ArgSchema::integer().range(Some(10.0), Some(20.0))).required(),
    ]);
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"n":"#).unwrap();
    // No single-digit number can fall in the range, but 1 and 2 survive as prefixes.
    assert_eq!(
        d.allowed_prefixes().chars().collect::<Vec<_>>(),
        vec!['1', '2']
    );

    let mut two = d.branch('2').unwrap();
    let allowed = two.allowed_prefixes();
    assert_eq!(allowed.chars().collect::<Vec<_>>(), vec!['0']);
    assert!(
        !allowed.can_finish(),
        "2 on its own job out of range, it cannot close"
    );

    let one = d.branch('1').unwrap();
    // 1 on its own is out of range too: '}' must not open, but the digits for
    // 10..19 are open.
    assert!(!one.allowed_prefixes().contains('}'));
    assert!(one.allowed_prefixes().contains('9'));

    two.advance("0}").unwrap();
    assert!(two.is_done());
    assert!(!accepts(&s, r#"{"n":21}"#));
    assert!(!accepts(&s, r#"{"n":5}"#));
    assert!(accepts_and_is_json(&s, r#"{"n":20}"#));
}

#[test]
fn no_reachable_state_locks_up() {
    // THE MOST IMPORTANT PROPERTY: in constrained generation a dead state locks
    // the model up — no token can be produced and generation cannot end either.
    // This walks the reachable state space (to a bounded depth) and verifies the
    // proposition "either there job an exit or it can finish" at every node.
    let s = sample_schema();
    let g = grammar(&s);
    let mut budget = 40_000usize;
    walk(&g.state(), 0, &mut budget);

    fn walk(d: &GrammarState, depth: usize, budget: &mut usize) {
        let allowed = d.allowed_prefixes();
        assert!(
            !allowed.is_empty() || allowed.can_finish(),
            "locked state (depth {depth}): {allowed:?}"
        );
        if depth >= 12 || *budget == 0 {
            return;
        }
        // A free string body branches infinitely; a few representative
        // characters are enough.
        let mut candidates: Vec<char> = allowed.chars().collect();
        if allowed.is_text_body() {
            candidates.extend(['a', 'ç', '✓']);
        }
        for c in candidates {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            if let Ok(next) = d.branch(c) {
                walk(&next, depth + 1, budget);
            }
        }
    }
}

// ------------------------------------------------------------------------ mask

fn vocab() -> Vec<String> {
    [
        "",
        "{",
        "}",
        "\"",
        ":",
        ",",
        "[",
        "]",
        "query",
        "scope",
        "count",
        "deep",
        "all",
        "near",
        "true",
        "false",
        "report",
        "0",
        "1",
        "12",
        "99",
        "Now",
        "far",
        "\"query\"",
        "\":",
        "{\"",
        "other",
        "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn allowed_tokens(mask: &[bool], v: &[String]) -> Vec<String> {
    mask.iter()
        .enumerate()
        .filter(|(_, b)| **b)
        .map(|(i, _)| v[i].clone())
        .collect()
}

#[test]
fn the_mask_leaves_only_valid_tokens() {
    let s = sample_schema();
    let g = grammar(&s);
    let d = g.state();
    let v = vocab();
    let m = TokenMask::new(&v);
    let open = allowed_tokens(&m.mask(&d), &v);
    // At the start, only tokens beginning with '{' can be open.
    assert!(open.contains(&"{".to_string()));
    assert!(open.contains(&"{\"".to_string()));
    assert!(!open.contains(&"Now".to_string()));
    assert!(!open.contains(&"query".to_string()));
    assert!(!open.contains(&"}".to_string()));
    // Special tokens with empty text are never opened in the mask.
    assert!(!open.iter().any(|t| t.is_empty()));
    assert_eq!(m.empty_tokens(), &[0]);
    assert_eq!(m.vocab_size(), v.len());
}

#[test]
fn at_a_key_position_the_mask_opens_only_schema_fields() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance("{\"").unwrap();
    let v = vocab();
    let open = allowed_tokens(&d.mask(&v), &v);
    for expected in ["query", "scope", "count", "deep"] {
        assert!(
            open.contains(&expected.to_string()),
            "{expected} must be open"
        );
    }
    assert!(!open.contains(&"other".to_string()));
    assert!(!open.contains(&"all".to_string()));
}

#[test]
fn at_an_enum_position_the_mask_closes_everything_outside_the_set() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"query":"a","scope":""#).unwrap();
    let v = vocab();
    let open = allowed_tokens(&d.mask(&v), &v);
    assert!(open.contains(&"all".to_string()));
    assert!(open.contains(&"near".to_string()));
    assert!(!open.contains(&"far".to_string()));
    assert!(!open.contains(&"report".to_string()));
}

#[test]
fn the_mask_respects_the_range_and_the_type() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"query":"a","count":"#).unwrap();
    let v = vocab();
    let open = allowed_tokens(&d.mask(&v), &v);
    assert!(open.contains(&"12".to_string()));
    assert!(open.contains(&"1".to_string()));
    // 99 is out of range (1..50), 0 is below the lower bound, '-' cannot be negative.
    assert!(!open.contains(&"99".to_string()));
    assert!(!open.contains(&"0".to_string()));
    assert!(!open.contains(&"-".to_string()));
    assert!(!open.contains(&"true".to_string()));
}

#[test]
fn the_mask_validates_a_multi_character_token_end_to_end() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance("{").unwrap();
    let v = vocab();
    let open = allowed_tokens(&d.mask(&v), &v);
    // A token containing several characters is opened only if ALL of it is valid.
    assert!(open.contains(&"\"query\"".to_string()));
    assert!(open.contains(&"\"".to_string()));
    assert!(!open.contains(&"\":".to_string()));
}

#[test]
fn at_the_end_position_the_mask_opens_nothing() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"query":"a"}"#).unwrap();
    let v = vocab();
    // Once finished, no text token is valid; the stop decision (EOS) is the
    // caller's job, the grammar only says "can finish".
    assert!(allowed_tokens(&d.mask(&v), &v).is_empty());
    assert!(d.allowed_prefixes().can_finish());
}

/// REAL-VOCABULARY REGRESSION: a COMBINED token that starts inside the grammar
/// and ends with the `)` that closes the call must be opened in the mask.
///
/// This is the test of a concrete fault measured with Qwen2.5: the natural
/// tokenization of the string `calculate({"expression": "12*8"})` ends with `"})` (a
/// single token). As long as the terminator is handled outside the walk, that
/// token stays closed — that is, the token the model is MOST LIKELY to produce
/// was forbidden.
#[test]
fn the_mask_opens_a_combined_token_that_ends_with_the_terminator() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"query":"a"#).unwrap();

    // The vocabulary has both split and combined closings.
    let v: Vec<String> = ["\"", "}", ")", "\"}", "\"})", "\"}) extra", "a"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let m = TokenMask::new(&v);
    let open = allowed_tokens(&m.mask_with_terminator(&d, Some(')')), &v);

    // Once the value closes with a quote and the object ends, the call may close too.
    assert!(
        open.contains(&"\"}".to_string()),
        "the split closing must be open"
    );
    assert!(
        open.contains(&"\"})".to_string()),
        "the combined closing must be open"
    );
    // We do not descend PAST the terminator: no chatter may be appended after the call.
    assert!(
        !open.contains(&"\"}) extra".to_string()),
        "text after the call is forbidden"
    );
}

/// Without a terminator the behaviour does not change — `mask` is not a call
/// wire, it is a pure grammar question, and `)` is not in its alphabet.
#[test]
fn without_a_terminator_the_mask_does_not_open_the_closing_token() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"query":"a"#).unwrap();
    let v: Vec<String> = ["\"}", "\"})"].iter().map(|s| s.to_string()).collect();
    let m = TokenMask::new(&v);
    let open = allowed_tokens(&m.mask(&d), &v);
    assert!(open.contains(&"\"}".to_string()));
    assert!(!open.contains(&"\"})".to_string()));
}

#[test]
fn the_cached_and_uncached_masks_give_the_same_result() {
    let s = sample_schema();
    let g = grammar(&s);
    let mut d = g.state();
    d.advance(r#"{"query":"#).unwrap();
    let v = vocab();
    let m = TokenMask::new(&v);
    assert_eq!(m.mask(&d), d.mask(&v));
}

#[test]
fn generation_driven_by_the_mask_always_yields_valid_json() {
    // A holistic test: at every step pick the FIRST open token from the mask. If
    // the grammar is correct, whatever greedy choice is made the output must be
    // valid JSON.
    //
    // The schema deliberately contains no free TEXT: a free-text field can be
    // extended forever as far as the grammar is concerned (every character is
    // valid), so the proposition "whatever you pick it closes" does not hold
    // there — the decision to close belongs to the model.
    let s = ArgSchema::object(vec![
        Field::new("scope", ArgSchema::choice(["all", "near"])).required(),
        Field::new("count", ArgSchema::integer().range(Some(1.0), Some(50.0))).required(),
        Field::new("deep", ArgSchema::bool()).required(),
    ]);
    let g = grammar(&s);
    let mut d = g.state();
    let v = vocab();
    let m = TokenMask::new(&v);
    let mut output = String::new();
    for _ in 0..64 {
        let mask = m.mask(&d);
        let Some(i) = mask.iter().position(|b| *b) else {
            break;
        };
        output.push_str(&v[i]);
        d.advance(&v[i]).unwrap();
    }
    assert!(d.is_done(), "generation did not close: {output}");
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    // What was produced must also pass the schema: grammar and validation share
    // the same contract.
    assert!(
        s.validate(&value).is_ok(),
        "the schema rejected it: {output}"
    );
}

// ------------------------------------------------- an open field has to end

/// A digit sequence the grammar cannot be talked out of: the measured runaways
/// of audit L25 were `0.` + 200 000 digits and `1e-` + 200 000 zeros, both
/// still open before `MAX_FRACTION_DIGITS` and `MAX_EXPONENT_DIGITS` existed.
#[test]
fn a_number_has_a_last_digit() {
    use crate::state::{MAX_EXPONENT_DIGITS, MAX_FRACTION_DIGITS};
    let free = ArgSchema::object(vec![Field::new("n", ArgSchema::number()).required()]);
    let g = grammar(&free);

    let full = format!(r#"{{"n":0.{}"#, "1".repeat(MAX_FRACTION_DIGITS));
    let mut d = g.state();
    d.advance(&full).expect("the bound itself is writable");
    let allowed = d.allowed_prefixes();
    assert!(
        !allowed.chars().any(|c| c.is_ascii_digit()),
        "a digit past the fraction bound opened: {allowed:?}"
    );
    assert!(allowed.contains('}') && allowed.contains('e'));
    assert!(!accepts(&free, &format!("{full}1}}")));
    assert!(accepts_and_is_json(&free, &format!("{full}}}")));

    let exponent = format!(r#"{{"n":1e-{}"#, "0".repeat(MAX_EXPONENT_DIGITS));
    let mut d = g.state();
    d.advance(&exponent).expect("the bound itself is writable");
    assert!(!d.allowed_prefixes().chars().any(|c| c.is_ascii_digit()));
    assert!(accepts_and_is_json(&free, &format!("{exponent}}}")));
    assert!(!accepts(&free, &format!("{exponent}1}}")));

    // The integer part has no constant of its own: representability bounds it.
    let integer = ArgSchema::object(vec![Field::new("n", ArgSchema::integer()).required()]);
    let mut d = grammar(&integer).state();
    d.advance(r#"{"n":12345678901234567890"#)
        .expect("20 digits under u64::MAX");
    assert!(!d.allowed_prefixes().chars().any(|c| c.is_ascii_digit()));
}

/// The fraction bound must never cut off a value a RANGE holds: a bounded
/// number refuses the exponent, so plain decimal is its only spelling. Every
/// f64 below is written the way Rust prints it — plain decimal, shortest
/// round-trip — and must be producible in a range a few ulps either side of it.
///
/// NOT THE POINT RANGE `[x, x]`, and that is a measurement, not a softening:
/// the grammar now compares the value `serde_json` will hand the tool, and
/// `serde_json` rounds a 17-digit mantissa twice, so `0.000…22250738585074237`
/// (x = 2.2250738585074237e-308) comes back one ulp off x. In `[x, x]` the
/// VALIDATOR refuses that spelling as out of range, so the grammar refusing it
/// too is the agreement this crate is for. A range one ulp wide is not a shape
/// any tool declares; the four ulps here are the margin that keeps this test
/// about the digit bound rather than about rounding.
#[test]
fn every_f64_a_range_can_hold_is_still_writable_in_plain_decimal() {
    use crate::state::MAX_FRACTION_DIGITS;
    let mut longest = 0;
    let mut values = vec![
        5e-324,
        f64::MIN_POSITIVE,
        f64::MIN_POSITIVE * 1.000_000_000_000_1,
        1.234_567_89e-310,
        2.2e-300,
        0.1,
        1.0 / 3.0,
        123.456,
    ];
    // A spread of subnormals, where the fewest leading digits are saved.
    let mut bits = 1u64;
    while bits < f64::MIN_POSITIVE.to_bits() {
        values.push(f64::from_bits(bits));
        bits = bits * 3 + 7;
    }
    for x in values {
        let text = format!("{x}");
        let digits = text.split_once('.').map_or(0, |(_, f)| f.len());
        longest = longest.max(digits);
        assert!(digits <= MAX_FRACTION_DIGITS, "{x:e} needs {digits} digits");
        let lo = f64::from_bits(x.to_bits().saturating_sub(4));
        let hi = f64::from_bits(x.to_bits() + 4);
        let near = ArgSchema::object(vec![
            Field::new("n", ArgSchema::number().range(Some(lo), Some(hi))).required(),
        ]);
        let call = format!(r#"{{"n":{text}}}"#);
        assert!(
            accepts(&near, &call),
            "the range around {x:e} cannot write {text}"
        );
        let value: serde_json::Value = serde_json::from_str(&call).expect("JSON");
        assert!(near.validate(&value).is_ok(), "validate disagrees on {x:e}");
    }
    // MEASURED: the longest plain spelling in this set is 5e-324's, 324
    // fraction digits — the figure `MAX_FRACTION_DIGITS` is set above.
    assert_eq!(longest, 324);
}

/// An array the schema leaves open still has a last item (audit L25: 100 001
/// elements were still open before `ARRAY_CEILING`).
#[test]
fn an_unbounded_array_has_a_last_item() {
    use crate::state::ARRAY_CEILING;
    let open = ArgSchema::object(vec![
        Field::new("xs", ArgSchema::array(ArgSchema::bool())).required(),
    ]);
    let items = vec!["true"; ARRAY_CEILING].join(",");
    let mut d = grammar(&open).state();
    d.advance(&format!(r#"{{"xs":[{items}"#))
        .expect("the ceiling itself is writable");
    let allowed = d.allowed_prefixes();
    assert!(!allowed.contains(','), "a comma past the ceiling opened");
    assert!(allowed.contains(']'));
    assert!(accepts_and_is_json(
        &open,
        &format!(r#"{{"xs":[{items}]}}"#)
    ));
    assert!(!accepts(&open, &format!(r#"{{"xs":[{items},true]}}"#)));

    // A declared bound is the schema's word and stays exactly as it was ...
    let declared = ArgSchema::object(vec![
        Field::new(
            "xs",
            ArgSchema::array(ArgSchema::bool()).length(None, Some(ARRAY_CEILING + 5)),
        )
        .required(),
    ]);
    let more = vec!["true"; ARRAY_CEILING + 5].join(",");
    assert!(accepts(&declared, &format!(r#"{{"xs":[{more}]}}"#)));
    // ... and a declared MINIMUM above the ceiling raises it, so the array can
    // still close: without that the schema would have no valid array at all.
    let floor = ARRAY_CEILING + 3;
    let at_least = ArgSchema::object(vec![
        Field::new(
            "xs",
            ArgSchema::array(ArgSchema::bool()).length(Some(floor), None),
        )
        .required(),
    ]);
    let exactly = vec!["true"; floor].join(",");
    assert!(accepts_and_is_json(
        &at_least,
        &format!(r#"{{"xs":[{exactly}]}}"#)
    ));
    assert!(!accepts(
        &at_least,
        &format!(r#"{{"xs":[{exactly},true]}}"#)
    ));
}

/// `validator_value`'s fast path must give the tool layer's answer, bit for
/// bit, on every shape on either side of its lines (15 digits, ±22 net power of
/// ten, 18 integer digits). The one difference allowed is the documented one:
/// a refusal where Rust already rounds to infinity.
#[test]
fn the_fast_path_and_the_validator_agree() {
    use crate::state::validator_value;
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut digit = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        char::from(b'0' + (seed % 10) as u8)
    };
    let exponents = [
        None,
        Some(-330i32),
        Some(-24),
        Some(-23),
        Some(-22),
        Some(-1),
        Some(0),
        Some(7),
        Some(22),
        Some(23),
        Some(290),
        Some(307),
        Some(308),
        Some(309),
    ];
    let mut texts: Vec<String> = Vec::new();
    for int_len in 1..=21usize {
        for frac_len in [0usize, 1, 2, 7, 13, 14, 15, 16, 17, 22, 23] {
            for exp in exponents {
                for _ in 0..3 {
                    let mut t = String::new();
                    let first = digit();
                    t.push(if int_len > 1 && first == '0' {
                        '1'
                    } else {
                        first
                    });
                    for _ in 1..int_len {
                        t.push(digit());
                    }
                    if frac_len > 0 {
                        t.push('.');
                        for _ in 0..frac_len {
                            t.push(digit());
                        }
                    }
                    if let Some(e) = exp {
                        t.push_str(&format!("e{e}"));
                    }
                    texts.push(format!("-{t}"));
                    texts.push(t);
                }
            }
        }
    }
    let edge = format!("17976931348623157{}", "0".repeat(292));
    for t in [
        "0",
        "-0",
        "0.0",
        "-0.0",
        "18446744073709551615",
        "18446744073709551616",
        "-9223372036854775808",
        "-9223372036854775809",
        "999999999999999999",
        "-999999999999999999",
        "1.7976931348623157e308",
        "1.7976931348623158e308",
        edge.as_str(),
    ] {
        texts.push(t.to_string());
    }
    let mut narrowed = 0;
    for text in &texts {
        for is_integer in [false, true] {
            if is_integer && text.contains(['.', 'e']) {
                continue;
            }
            let fast = validator_value(text, is_integer, true);
            let exact = ArgSchema::json_number_value(text, is_integer);
            // Unbounded, only the yes/no is read — and it must be the same.
            let open = validator_value(text, is_integer, false);
            assert_eq!(
                open.is_some(),
                fast.is_some(),
                "{text}: the unbounded verdict differs"
            );
            if fast.map(f64::to_bits) == exact.map(f64::to_bits) {
                continue;
            }
            assert!(
                fast.is_none() && text.parse::<f64>().is_ok_and(f64::is_infinite),
                "{text} (integer: {is_integer}): fast path {fast:?}, the tool layer {exact:?}"
            );
            narrowed += 1;
        }
    }
    // MEASURED: 19 417 spellings, and the infinity refusal decided none of
    // them differently — it is a shortcut to the same answer here, and this
    // says so rather than leaving it to be assumed.
    assert!(texts.len() > 19_000, "the table collapsed: {}", texts.len());
    assert_eq!(narrowed, 0);
}
