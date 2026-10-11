//! What the grammar lets a model WRITE has to be what the tool layer then READS.
//!
//! The crate's claim is "an invalid call cannot be generated". That claim is
//! only as good as the agreement between two parsers: the automaton deciding
//! character by character what may be written, and `serde_json` +
//! `ArgSchema::validate` deciding, once the call is closed, what it means. The
//! audit of 9 Oct 2026 found two places where they disagreed and the grammar was
//! the looser one — every case below was MEASURED, not imagined:
//!
//!   M15  a closed-set value or a key holding `\` or `"` was written into the
//!        grammar RAW. `C:\temp` was generated as `"C:\temp"`, which JSON reads
//!        as `C:<TAB>emp`, and `validate` answered "invalid choice"; `a"b` left
//!        the automaton with no token to open ("the constraint forbade every
//!        token"); a key spelled `a\nb` decoded to a newline and the field came
//!        back MissingField.
//!   M16  a number the automaton closed on and `serde_json`/`validate` refused:
//!        `-0` in an integer field (15 287 times in a 200 000-walk fuzz),
//!        u64::MAX + 1, i64::MIN - 1, and `1.7976931348623158e308`.
//!
//! So every test here takes a decision the grammar made and hands it to the
//! OTHER parser, never to the grammar again.

use serde_json::Value;
use std::collections::BTreeSet;
use tacet_grammar::{Grammar, GrammarState};
use tacet_kernel::{ArgSchema, Field, SchemaKind};

fn accepts(schema: &ArgSchema, text: &str) -> bool {
    let mut state = Grammar::compile(schema).state();
    state.advance(text).is_ok() && state.is_done()
}

/// Every text the grammar can close on from `state`, without structural
/// whitespace (free between tokens, and it would make the set infinite). Only
/// for positions whose language is finite — a closed set, a key, a literal.
fn every_completion(state: &GrammarState) -> BTreeSet<String> {
    fn walk(
        state: &GrammarState,
        written: &mut String,
        out: &mut BTreeSet<String>,
        budget: &mut usize,
    ) {
        assert!(*budget > 0, "the language here is not finite: {written:?}");
        *budget -= 1;
        if state.is_done() {
            out.insert(written.clone());
            return;
        }
        let allowed = state.allowed_prefixes();
        assert!(
            !allowed.is_text_body(),
            "a free string body is not a finite language: {written:?}"
        );
        // `chars()` leaves the structural whitespace out (it is a flag, not a
        // list), so a space seen here is CONTENT — a space inside a value.
        for c in allowed.chars().collect::<Vec<_>>() {
            let next = state.branch(c).unwrap_or_else(|e| {
                panic!("the mask offered {c:?} after {written:?} and advance refused it: {e}")
            });
            written.push(c);
            walk(&next, written, out, budget);
            written.pop();
        }
    }
    let mut out = BTreeSet::new();
    let mut budget = 100_000;
    walk(state, &mut String::new(), &mut out, &mut budget);
    out
}

// ------------------------------------------------------------------ M15: text

/// The closed set the grammar can write, DECODED BY SERDE, is exactly the set
/// the schema declares — no more (a raw `\t` decoding to a TAB), no fewer (a
/// value with a quote nobody can finish).
#[test]
fn a_closed_set_with_a_backslash_or_a_quote_decodes_to_exactly_the_declared_set() {
    let declared = [
        r"C:\temp",
        r#"a"b"#,
        r"\",
        r#"""#,
        r#"\"#,
        "plain",
        "ç ü 世界",
        "tab\there",
        "bell\u{7}",
    ];
    let schema = ArgSchema::object(vec![
        Field::new("dir", ArgSchema::choice(declared)).required(),
    ]);
    let grammar = Grammar::compile(&schema);
    let mut state = grammar.state();
    state
        .advance(r#"{"dir":"#)
        .expect("the value position opens");

    let mut decoded = BTreeSet::new();
    for tail in every_completion(&state) {
        let text = format!(r#"{{"dir":{tail}"#);
        let value: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
            panic!("the grammar closed on text that is not JSON: {text} ({e})")
        });
        schema
            .validate(&value)
            .unwrap_or_else(|e| panic!("the grammar wrote a value validate refuses: {text} ({e})"));
        decoded.insert(value["dir"].as_str().expect("a string").to_string());
    }
    let expected: BTreeSet<String> = declared.iter().map(|s| s.to_string()).collect();
    assert_eq!(decoded, expected);

    // The two measured spellings, by name. The RAW `C:\temp` is a TAB to JSON,
    // so it must be unproducible; the escaped one is the only way to say it.
    assert!(!accepts(&schema, r#"{"dir":"C:\temp"}"#));
    assert!(accepts(&schema, r#"{"dir":"C:\\temp"}"#));
    assert!(accepts(&schema, r#"{"dir":"a\"b"}"#));
    // A control character is spelled the way serde_json spells it.
    assert!(accepts(&schema, r#"{"dir":"tab\there"}"#));
    assert!(accepts(&schema, r#"{"dir":"bell\u0007"}"#));
}

/// The same defect on the other literal the grammar writes: a key.
#[test]
fn a_key_with_a_backslash_a_quote_or_a_control_character_decodes_to_its_name() {
    for name in [r"a\nb", r#"x"y"#, "tab\there", r"\"] {
        let schema = ArgSchema::object(vec![Field::new(name, ArgSchema::bool()).required()]);
        let grammar = Grammar::compile(&schema);
        let tails = every_completion(&grammar.state());
        assert_eq!(
            tails.len(),
            2,
            "{name:?}: exactly true and false, got {tails:?}"
        );
        for text in tails {
            let value: Value =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON: {text} ({e})"));
            let keys: Vec<&String> = value.as_object().expect("an object").keys().collect();
            assert_eq!(keys, vec![name], "{text} decodes to another key");
            schema
                .validate(&value)
                .unwrap_or_else(|e| panic!("{text}: validate says {e}"));
        }
    }
}

/// An empty closed set can still arrive by a door `ArgSchema::choice` does not
/// guard — a struct literal, or a deserialized schema. Such a field has no
/// value a call could carry, so its KEY must never open: MEASURED before the
/// fix, `enum: []` opened the key and stranded the model on the value.
#[test]
fn a_field_with_no_possible_value_is_never_offered() {
    let nothing = ArgSchema {
        kind: SchemaKind::Choice { choices: vec![] },
        description: None,
    };
    let schema = ArgSchema::object(vec![
        Field::new("never", nothing.clone()),
        Field::new("kept", ArgSchema::bool()),
        // An array of the empty set can still be written — as `[]` and nothing
        // else, which is what the item type permits.
        Field::new("list", ArgSchema::array(nothing)),
    ]);
    let grammar = Grammar::compile(&schema);
    let mut state = grammar.state();
    state.advance(r#"{""#).expect("a key may start");
    let allowed: Vec<char> = state.allowed_prefixes().chars().collect();
    assert!(!allowed.contains(&'n'), "the dead key opened: {allowed:?}");
    assert!(allowed.contains(&'k') && allowed.contains(&'l'));
    assert!(accepts(&schema, r#"{"kept":true,"list":[]}"#));
    assert!(!accepts(&schema, r#"{"list":[""]}"#));
    let mut list = grammar.state();
    list.advance(r#"{"list":["#).expect("the array opens");
    assert_eq!(
        list.allowed_prefixes()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<Vec<_>>(),
        vec![']']
    );
}

/// M15, one level further down (review of 10 Oct 2026). The test above
/// covers a dead field directly in the root; the rule that makes an OBJECT
/// dead when one of its REQUIRED fields is (`inhabited = false` in `add`) was
/// held by nothing — set to `true`, every test passed and the optional key
/// `opt` below was offered: the model could open it and strand inside, with no
/// value its required `dead` child could take.
#[test]
fn an_optional_object_with_a_dead_required_field_is_never_offered() {
    let nothing = ArgSchema {
        kind: SchemaKind::Choice { choices: vec![] },
        description: None,
    };
    let schema = ArgSchema::object(vec![
        Field::new(
            "opt",
            ArgSchema::object(vec![Field::new("dead", nothing).required()]),
        ),
        Field::new("kept", ArgSchema::bool()),
    ]);
    let grammar = Grammar::compile(&schema);
    let mut state = grammar.state();
    state.advance(r#"{""#).expect("a key may start");
    let allowed: Vec<char> = state.allowed_prefixes().chars().collect();
    assert!(
        !allowed.contains(&'o'),
        "the dead object's key opened: {allowed:?}"
    );
    assert!(allowed.contains(&'k'));
    assert!(accepts(&schema, r#"{"kept":true}"#));
}

/// M15: EVERY control character in a literal is escaped, C1 and DEL included
/// — `json_body`'s comment says a literal must not be the one place a raw C1
/// `CSI` (U+009B) can reach the output. Narrowed to the C0 range JSON
/// requires, every test passed while the grammar accepted the raw byte (review
/// of 10 Oct 2026).
#[test]
fn a_c1_or_del_in_a_choice_is_written_escaped_never_raw() {
    for (raw, escaped) in [("a\u{9b}b", "a\\u009bb"), ("a\u{7f}b", "a\\u007fb")] {
        let schema = ArgSchema::object(vec![Field::new("v", ArgSchema::choice([raw])).required()]);
        assert!(
            !accepts(&schema, &format!(r#"{{"v":"{raw}"}}"#)),
            "{raw:?} was accepted raw"
        );
        let written = format!(r#"{{"v":"{escaped}"}}"#);
        assert!(accepts(&schema, &written), "{written} was refused");
        let back: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(back["v"], raw, "the escape does not decode to the choice");
    }
}

// --------------------------------------------------------------- M16: numbers

fn number_field(schema: ArgSchema) -> ArgSchema {
    ArgSchema::object(vec![Field::new("n", schema).required()])
}

/// Every number the grammar can close on from `{"n":<prefix>` within `depth`
/// more characters, handed to serde_json and `validate`. EXHAUSTIVE rather than
/// sampled: the audit's 200 000 random walks found `-0` in an integer field,
/// and a two-character exhaustive search finds it on the first prefix it tries.
fn every_closing_number_is_valid(schema: &ArgSchema, prefix: &str, depth: usize) -> usize {
    fn walk(
        schema: &ArgSchema,
        state: &GrammarState,
        written: &mut String,
        depth: usize,
        checked: &mut usize,
    ) {
        let mut closed = state.clone();
        if closed.advance("}").is_ok() && closed.is_done() {
            let text = format!(r#"{{"n":{written}}}"#);
            let value: Value = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("the grammar closed on non-JSON {text} ({e})"));
            if let Err(e) = schema.validate(&value) {
                panic!("the grammar closed on {text} and validate refused it: {e}");
            }
            *checked += 1;
        }
        if depth == 0 {
            return;
        }
        for c in state.allowed_prefixes().chars().collect::<Vec<_>>() {
            if !c.is_ascii_digit() && !matches!(c, '-' | '+' | '.' | 'e' | 'E') {
                continue;
            }
            let next = state.branch(c).expect("the mask and the automaton agree");
            written.push(c);
            walk(schema, &next, written, depth - 1, checked);
            written.pop();
        }
    }
    let mut state = Grammar::compile(schema).state();
    state
        .advance(&format!(r#"{{"n":{prefix}"#))
        .unwrap_or_else(|e| panic!("the prefix {prefix:?} is not producible: {e}"));
    let mut written = prefix.to_string();
    let mut checked = 0;
    walk(schema, &state, &mut written, depth, &mut checked);
    checked
}

#[test]
fn an_integer_field_cannot_write_minus_zero() {
    let free = number_field(ArgSchema::integer());
    assert!(!accepts(&free, r#"{"n":-0}"#));
    let mut state = Grammar::compile(&free).state();
    state
        .advance(r#"{"n":-"#)
        .expect("a negative integer may start");
    let allowed: Vec<char> = state.allowed_prefixes().chars().collect();
    assert_eq!(
        allowed,
        ('1'..='9').collect::<Vec<_>>(),
        "the mask after '-'"
    );
    // A NUMBER field keeps it: serde reads `-0` as -0.0, which is a number.
    assert!(accepts(&number_field(ArgSchema::number()), r#"{"n":-0}"#));

    // Refusing `-0` must not strand a range whose only negative reach WAS `-0`:
    // in [-0.5, 0] the only integer is 0, so `-` may not open at all.
    let only_zero = number_field(ArgSchema::integer().range(Some(-0.5), Some(0.0)));
    let mut start = Grammar::compile(&only_zero).state();
    start.advance(r#"{"n":"#).expect("the value opens");
    assert_eq!(
        start.allowed_prefixes().chars().collect::<Vec<_>>(),
        vec!['0']
    );
    assert!(accepts(&only_zero, r#"{"n":0}"#));
    // ... and in [-1.5, -1.2] there is no integer at all after `-`.
    let none = number_field(ArgSchema::integer().range(Some(-1.5), Some(-1.2)));
    let mut start = Grammar::compile(&none).state();
    start.advance(r#"{"n":"#).expect("the value opens");
    assert!(!start.allowed_prefixes().contains('-'));
}

#[test]
fn an_integer_field_stops_at_the_edges_of_i64_and_u64() {
    let free = number_field(ArgSchema::integer());
    assert!(accepts(&free, r#"{"n":18446744073709551615}"#));
    assert!(!accepts(&free, r#"{"n":18446744073709551616}"#));
    assert!(accepts(&free, r#"{"n":-9223372036854775808}"#));
    assert!(!accepts(&free, r#"{"n":-9223372036854775809}"#));
    // And the refusal is in the MASK, one digit before the edge is crossed.
    let mut state = Grammar::compile(&free).state();
    state
        .advance(r#"{"n":1844674407370955161"#)
        .expect("in range");
    let digits: Vec<char> = state
        .allowed_prefixes()
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    assert_eq!(digits, ('0'..='5').collect::<Vec<_>>());
    // A 21st digit never opens, whatever the first 20 were.
    let mut state = Grammar::compile(&free).state();
    state
        .advance(r#"{"n":10000000000000000000"#)
        .expect("1e19 fits in u64");
    assert!(!state.allowed_prefixes().chars().any(|c| c.is_ascii_digit()));
}

#[test]
fn a_number_field_stops_where_serde_json_stops() {
    let free = number_field(ArgSchema::number());
    // MEASURED: Rust's `f64::from_str` rounds `1.7976931348623158e308` down to
    // f64::MAX, serde_json's parse rounds it to infinity and refuses it.
    assert!(accepts(&free, r#"{"n":1.7976931348623157e308}"#));
    assert!(!accepts(&free, r#"{"n":1.7976931348623158e308}"#));
    assert!(serde_json::from_str::<Value>(r#"{"n":1.7976931348623158e308}"#).is_err());
}

/// THE PROPERTY, exhaustively, at every edge the audit measured and at the
/// starts of every bounded shape the property sweep generates.
#[test]
fn every_number_the_grammar_can_close_on_is_one_validate_accepts() {
    let ranges: [(Option<f64>, Option<f64>); 8] = [
        (None, None),
        (Some(1.0), Some(50.0)),
        (Some(10.0), Some(20.0)),
        (Some(-5.0), Some(5.0)),
        (Some(0.0), None),
        (Some(-20.0), Some(-10.0)),
        (Some(3.0), Some(3.0)),
        (Some(-0.5), Some(0.0)),
    ];
    let mut checked = 0;
    for (min, max) in ranges {
        for base in [ArgSchema::integer(), ArgSchema::number()] {
            checked += every_closing_number_is_valid(&number_field(base.range(min, max)), "", 3);
        }
    }
    let integer = number_field(ArgSchema::integer());
    checked += every_closing_number_is_valid(&integer, "184467440737095516", 3);
    checked += every_closing_number_is_valid(&integer, "-92233720368547758", 3);
    let number = number_field(ArgSchema::number());
    checked += every_closing_number_is_valid(&number, "1.797693134862315", 5);
    checked += every_closing_number_is_valid(&number, "17976931348623157", 3);
    // Not vacuous: MEASURED, the search closes on 181 143 numbers (1.5s in a
    // debug build). The floor is far below that so a range-table edit does not
    // turn this red, and far above zero so a search that stopped searching does.
    assert!(
        checked > 100_000,
        "only {checked} closing numbers were checked"
    );
}
