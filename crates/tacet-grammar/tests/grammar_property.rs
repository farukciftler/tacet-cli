//! The headline claim of this crate, turned from EXAMPLES into a PROPERTY over
//! the schema space.
//!
//! `src/tests.rs` measures the claim on one author-chosen `sample_schema()` and
//! a greedy first-open-token sampler. That leaves the schema SPACE untested: a
//! grammar bug that only shows up on a bounded fractional number, or on a `\u`
//! escape, never appears in a hand-written example because nobody thought to
//! write that example. This file generates the schemas and walks them
//! adversarially instead.
//!
//! Two directions, and both are needed — one alone proves nothing:
//!   SOUNDNESS  whatever the MASK permits, when the automaton says `is_done()`,
//!              must parse as JSON and must conform to the schema.
//!   REFUSAL    every mutation of an accepted output into a known-invalid one
//!              must be refused at a NAMED index, with the offending character's
//!              token CLOSED IN THE MASK at that prefix. That last clause is the
//!              whole point: it is what makes this masking rather than
//!              after-the-fact validation.
//!
//! THE ORACLE IS WRITTEN HERE, NOT BORROWED. `ArgSchema::validate` is too weak
//! to measure this claim: its Object arm walks the SCHEMA's fields (schema.rs
//! `validate_path`), so a key that is not in the schema passes silently — even
//! though `json_schema()` advertises `additionalProperties: false` and the
//! grammar really does enforce it. `conforms()` below rejects unknown keys, and
//! `validate` is then asserted on top as a second, weaker witness.
//!
//! THREE REAL DEFECTS FELL OUT OF WRITING THIS, all fixed in `src/state.rs` and
//! all kept here as focused regression tests at the bottom of the file:
//!   * a LONE UTF-16 SURROGATE (`{"q":"\uD800"}`) was accepted with
//!     `is_done() == true` while `serde_json` refused the same text;
//!   * a BOUNDED NUMBER could park on a prefix with no completion — `20.5` and
//!     `1e9` in the range [10,20] — an allowed set that stays non-empty forever
//!     over a path with no exit;
//!   * an UNBOUNDED NUMBER could overflow f64 (`1e31212121212121212`), which
//!     `serde_json` refuses and Rust's own `f64` parse silently turns into
//!     INFINITY rather than an error.
//!
//! None of the three is visible to `no_reachable_state_locks_up` (src/tests.rs),
//! which only asks whether the allowed set is empty; the first and third are not
//! visible to any test that never hands the output to a JSON parser. Only a walk
//! that must TERMINATE and then be PARSED sees them.
//!
//! WHAT THIS SAMPLER IS NOT. It is deliberately drawn to the tightest corners
//! (it scores candidates by how NARROW the next allowed set is), so it
//! under-samples free-text bodies and long enum prefixes. That is the right
//! trade — the corners are where the bugs are — but nobody should read these
//! tests as uniform coverage of the language.
//!
//! AND "UNDER-SAMPLES" WAS ONCE AN UNDERSTATEMENT OF EXACTLY ZERO. Instrumented
//! against its own generator, the sampler produced 384 walks containing not one
//! backslash: a free body was entered only when the schema forced it and closed
//! on the next character, so `StringStage::Escape`, `Hex`, `LowEscape`, `LowU`,
//! `hex_fits` and the hex filter in `allowed_prefixes` were never visited by the
//! per-step assertions at all. Only the hand-written counterexamples at the
//! bottom of the file covered them — which is why the surrogate defect belongs
//! to THOSE tests, not to this sweep. `escape_bonus` forces one escape per walk
//! where the grammar already offers it, and `Census` asserts the visit count so
//! the coverage cannot silently return to zero: 33 of 384 walks now enter an
//! escape and 43 `\u` escapes are opened and driven into the surrogate block,
//! where `hex_fits` is the only thing deciding which digits stay open. VERIFIED
//! BY BREAKING IT: with `left` written for `left - 1` in `allowed_prefixes`'s
//! hex loop, the sweep tests were all still green before this change and go red
//! after it ("mask/advance drift on token \"D\"").
//!
//! WHY A FREE-TEXT BODY IS NOT THE GRAMMAR'S FAULT: src/tests.rs:704-711 already
//! records that an unbounded `Text` field can be extended forever as far as the
//! grammar is concerned and that "the decision to close belongs to the model".
//! So the walk switches to a CLOSING PHASE after `OPEN_PHASE_STEPS`; a walk that
//! then fails to close is reported with the last tokens and the live allowed set,
//! so the first question can be answered — sampler, or grammar.
//!
//! WHITESPACE AFTER THE CLOSE IS LEGITIMATE. With the stack empty `advance`
//! accepts spaces (state.rs `feed_char`) and `allowed_prefixes` opens
//! can_finish + space, so the mask really does open " " and "\n" at a finished
//! state. The walk therefore STOPS at the first `is_done()` and never consumes
//! them: only then is "every proper prefix of the witness leaves is_done()
//! false" a true statement about the witness.
//!
//! MEASURED ON THIS MACHINE (M-series, macOS arm64, debug `cargo test`):
//! 96 generated schemas x 4 walks = 384 walks over vocabularies of 61 to 82
//! tokens, one `mask` + one `mask_with_terminator` + one FULL advance sweep of
//! the vocabulary at every step. Walks ran 14.4 tokens on average and 60 at the
//! longest (5,529 tokens over the 384). The refusal test walks each schema once
//! more and then mutates that witness 2,426 + 858 ways. The seven tests together
//! are 5.3s in one thread, of which the exhaustive number-prefix search at the
//! bottom is the bulk — so the budget goes almost entirely to the one part that
//! is exhaustive rather than sampled, which is the right place for it. (These
//! figures moved when `escape_bonus` was added: before it, 14.0 tokens, 54 at
//! the longest, 2,411 truncating mutants.)
//!
//! The three sweep tests take a DISJOINT THIRD of the seed table each — the same
//! driver, the same assertions, different schemas — so the table is covered
//! exactly once and the test NAME says which proposition to look at first when
//! it goes red.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;
use tacet_grammar::{Grammar, GrammarState, TokenMask};
use tacet_kernel::{ArgSchema, Field, SchemaKind};

// ---------------------------------------------------------------- budget

/// How many generated schemas. Fixed CONSTANTS and not a derived sequence, so a
/// red run reproduces byte-for-byte: there is no clock, no `HashMap` iteration
/// and no environment input anywhere in this file. MEASURED: 96 schemas x 4
/// walks is 0.34s of the 5.2s this file costs, so the schema count is not what
/// the budget is spent on — every `SchemaKind` variant, every entry of every
/// bound table and both `required` settings are hit well inside the table, and
/// the extra seeds are cheap insurance against a shape nobody thought of.
const SCHEMA_SEEDS: [u64; 96] = [
    0x9ACEA2AE5EEBBFF3,
    0xFE5A1BF1BDD400DC,
    0x55377E601CEE8F1B,
    0x7B987AA6967F6B1E,
    0xCD8AD967AC07BDF2,
    0x0289882808A2A8A9,
    0x03607B081E1C4624,
    0x1378B559D7DC8B1E,
    0xCC5A5EED24C83AB4,
    0xBCE2FEB3C9E77B96,
    0xC9707D05D98B7334,
    0x26BD718110BA3B2B,
    0x9E9128A6D2597157,
    0x98B7AE825A4C40E8,
    0xE18D05E5DC52211F,
    0xD0E1FC945C5D26CD,
    0xB1278A87823D5453,
    0xFC1852BD50610F78,
    0xC9DE4C0F4B2AAB6C,
    0xC6C9E897B9A897CB,
    0x44DCCE1F71631147,
    0xDE9B61796B1B8F39,
    0x1D2B517E04ADD962,
    0x1EA4D9E56DF05879,
    0x223714C43CFF1290,
    0x4672E775F0FBF435,
    0xA9DCED5C6042FA7A,
    0xE3CDFA05793E6F8C,
    0x4F7770C109039408,
    0x609F97BA6160F530,
    0x6E74D702A8C7EC4D,
    0x4843A25F9AF30943,
    0xF538E0E9FDFA8943,
    0x4C27CE087AFA249B,
    0xD562E931FC29B231,
    0x2CF145DF9EF7A926,
    0x98930DC040A9F9C3,
    0x9F7772EC01D23B0B,
    0x317B13F7AC1FD0AF,
    0x5B441F93CD662356,
    0x84591BBE3A989F01,
    0xAE8760EF0C2FB186,
    0x8C34242D17272D9F,
    0xB722E6EBC7C2F21F,
    0xCEF66CEF5F9DDAA2,
    0xD72B3B16D6193546,
    0x1BE1D7D81EFE76CA,
    0xB2E05EAA73430E26,
    0xED0CFE3D19BFC18E,
    0x61B9C3388409C70E,
    0xEA2D75E020969E82,
    0xB732DF728487E468,
    0x7CA510CED6368AD1,
    0x8DE7F4E4DE67701B,
    0xDE9DB3CE265ED5B3,
    0x144ED9FA37FBF2BE,
    0xF9E3B2B87C57CABD,
    0xBC55DBCBBB157FAC,
    0x7C95F189BADD9632,
    0xD593BAA406F53DE7,
    0x91383AA5959C365E,
    0x84787EC25BB845B6,
    0x65D9294DAB550613,
    0x350149B624E871BA,
    0x545FFA978B561679,
    0x718377DF573356A9,
    0xA02D1537CD03FDC4,
    0xDCFE649036D5199D,
    0xD5B3DA8BE9F58374,
    0xAA30EC21A3D1D2CB,
    0x7015C2AA4A0BF8BB,
    0x9B95D0294C8E0C5E,
    0x9D2638BC58670EA4,
    0x87D8575F2CA28E08,
    0xD777510E7F2229C2,
    0xF7159A8CA789A31D,
    0x090889C7B5ABA9A8,
    0x5B1F6DA16F0A5065,
    0x1CC830AF3613DB0E,
    0x9A26E8516723A1CC,
    0x63234BBED1BC4087,
    0x1A5530FF5AB23987,
    0xB615EC083613D44F,
    0xCD66BD3D9588C742,
    0x7840D24DE9C369CF,
    0x0A2E8E3A93713EDD,
    0x1B9FD571DEFF0286,
    0x7177BCBAA47F608D,
    0x10651F59D3593DFE,
    0xE3FB5300064657DF,
    0x6A5EEEA1AECE9824,
    0x8FE86DEE925C0D27,
    0xDF69175BCAB6F3FF,
    0x95DE64F2A624304D,
    0xF4203D1C185A9A65,
    0x02D5CCAE53571086,
];

/// Walks per schema. The walk seed is mixed into the SAMPLER only, never into
/// the schema, so these are four different corners of the SAME grammar; new
/// schemas buy more than new walks do, which is why the seed table is the long
/// list and this number is small.
const WALKS_PER_SCHEMA: u64 = 4;

/// After this many tokens the sampler stops hunting corners and starts closing.
/// A free `Text` body would otherwise never end, and that is not a grammar fault
/// (src/tests.rs:704-711).
const OPEN_PHASE_STEPS: usize = 12;

/// A generous ceiling, not a target. MEASURED over the 384 walks: the longest
/// walk took 54 tokens and the mean was 14.0, so the cap never fires; it exists
/// so a grammar that genuinely cannot close FAILS instead of hanging the suite.
/// It has already earned its keep three times over — every sampler mistake
/// recorded in `closing_score` below was found by this assertion firing.
const STEP_CAP: usize = 200;

/// The character that closes a tool call — outside the grammar, recognised by
/// `mask_with_terminator`. See mask.rs on the Qwen2.5 `"})` token.
const TERMINATOR: char = ')';

// ---------------------------------------------------------------- prng

/// xorshift64* — the same generator, constants and `draw` name as
/// `tacet_eval::analysis::Xorshift64Star`. Copied rather than shared because
/// tacet-grammar must not grow a dependency on tacet-eval for a test, and this
/// workspace adds no random-number crate.
struct Xorshift64Star {
    state: u64,
}

impl Xorshift64Star {
    fn new(seed: u64) -> Self {
        // A zero state is a fixed point of xorshift.
        Self {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    /// Named `draw`, not `next`: a bare `next` on a non-iterator reads like
    /// `Iterator::next` at the call site and clippy says so.
    fn draw(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.draw() % n as u64) as usize
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.draw().is_multiple_of(one_in)
    }
}

// ---------------------------------------------------------------- generator

/// Field names chosen so keys SHARE PREFIXES: that is what forces the `InKey`
/// candidate-narrowing path in the automaton instead of a trivial first-letter
/// decision. `üst` is here on purpose — `CompiledField::name_chars` is a
/// `Vec<char>`, and a name whose first character is multi-byte is the test of
/// that being chars and not bytes.
const NAME_POOL: [&str; 10] = [
    "a", "an", "and", "count", "counter", "c", "query", "q", "name", "üst",
];

/// Enum values, also prefix-colliding. No `"` and no `\`: compile.rs records
/// that a choice containing one becomes unproducible on purpose, so putting one
/// here would generate a schema the grammar is DESIGNED to reject rather than a
/// schema it is wrong about.
const CHOICE_POOL: [&str; 6] = ["all", "allow", "al", "near", "n", "on"];

/// Number bounds. Every entry is inhabited, and every entry with `is_integer`
/// contains an integer — a range like (0.5, 0.7) on an integer, or min > max,
/// is an AUTHORING bug that leaves the value position with an empty allowed set
/// and no way to finish. The grammar cannot be blamed for it and it is left out
/// of the generator rather than assertions being widened to survive it.
const NUMBER_RANGES: [(Option<f64>, Option<f64>); 7] = [
    (None, None),
    (Some(1.0), Some(50.0)),
    (Some(10.0), Some(20.0)),
    (Some(-5.0), Some(5.0)),
    (Some(0.0), None),
    (Some(-20.0), Some(-10.0)),
    (Some(3.0), Some(3.0)),
];

const TEXT_LIMITS: [Option<usize>; 4] = [None, Some(0), Some(1), Some(3)];

/// Array bounds; `min <= max` in every entry, for the reason NUMBER_RANGES
/// gives. `(0,0)` is here because it is the only shape where the item type
/// never opens at all.
const ARRAY_BOUNDS: [(Option<usize>, Option<usize>); 5] = [
    (None, None),
    (Some(0), Some(0)),
    (Some(2), Some(3)),
    (None, Some(2)),
    (Some(1), None),
];

/// The ROOT is always an object — that is the tool contract (schema.rs: "The
/// root schema of a tool is always this"), and it is also what makes
/// `is_done() == allowed_prefixes().can_finish()` hold: a bare `Number` root
/// would report `can_finish` from a non-empty stack.
fn generate_schema(rng: &mut Xorshift64Star) -> ArgSchema {
    let count = rng.below(5); // 0 exercises ArgSchema::empty()
    let mut used: Vec<usize> = Vec::new();
    let mut fields = Vec::new();
    for _ in 0..count {
        let Some(name) = pick_name(rng, &mut used) else {
            break;
        };
        let field = Field::new(name, generate_value(rng, 0));
        fields.push(if rng.chance(2) {
            field.required()
        } else {
            field
        });
    }
    ArgSchema::object(fields)
}

fn pick_name(rng: &mut Xorshift64Star, used: &mut Vec<usize>) -> Option<&'static str> {
    for _ in 0..16 {
        let i = rng.below(NAME_POOL.len());
        if !used.contains(&i) {
            used.push(i);
            return Some(NAME_POOL[i]);
        }
    }
    None
}

/// Depth is capped at 3: deeper adds nesting, not new automaton behaviour, and
/// every extra level multiplies the tokens a walk must write to close.
fn generate_value(rng: &mut Xorshift64Star, depth: usize) -> ArgSchema {
    let leaf_only = depth >= 3;
    let variants = if leaf_only { 4 } else { 6 };
    match rng.below(variants) {
        0 => {
            let limit = TEXT_LIMITS[rng.below(TEXT_LIMITS.len())];
            // There is NO builder for the bound; the struct is written out, the
            // same way src/tests.rs does it.
            ArgSchema {
                kind: SchemaKind::Text { max_length: limit },
                description: None,
            }
        }
        1 => {
            let n = 1 + rng.below(3);
            let mut values: Vec<String> = Vec::new();
            for _ in 0..n {
                let v = CHOICE_POOL[rng.below(CHOICE_POOL.len())];
                if !values.iter().any(|e| e == v) {
                    values.push(v.to_string());
                }
            }
            // An empty choice set is unproducible by construction (nothing may
            // follow the opening quote), so `n >= 1` above is load-bearing.
            ArgSchema::choice(values)
        }
        2 => {
            let (min, max) = NUMBER_RANGES[rng.below(NUMBER_RANGES.len())];
            let base = if rng.chance(2) {
                ArgSchema::integer()
            } else {
                ArgSchema::number()
            };
            base.range(min, max)
        }
        3 => ArgSchema::bool(),
        4 => {
            let (min, max) = ARRAY_BOUNDS[rng.below(ARRAY_BOUNDS.len())];
            ArgSchema::array(generate_value(rng, depth + 1)).length(min, max)
        }
        _ => {
            let count = rng.below(3);
            let mut used: Vec<usize> = Vec::new();
            let mut fields = Vec::new();
            for _ in 0..count {
                let Some(name) = pick_name(rng, &mut used) else {
                    break;
                };
                let field = Field::new(name, generate_value(rng, depth + 1));
                fields.push(if rng.chance(2) {
                    field.required()
                } else {
                    field
                });
            }
            ArgSchema::object(fields)
        }
    }
}

// ---------------------------------------------------------------- vocabulary

fn collect_literals(
    schema: &ArgSchema,
    names: &mut BTreeSet<String>,
    values: &mut BTreeSet<String>,
) {
    match &schema.kind {
        SchemaKind::Object { fields } => {
            for f in fields {
                names.insert(f.name.clone());
                collect_literals(&f.schema, names, values);
            }
        }
        SchemaKind::Array { item, .. } => collect_literals(item, names, values),
        SchemaKind::Choice { choices } => values.extend(choices.iter().cloned()),
        _ => {}
    }
}

/// A vocabulary DERIVED FROM THE SCHEMA, because the grammar is defined over a
/// caller-supplied `&[String]` and there is no tokenizer here. It carries every
/// single character the grammar can ever need plus multi-character tokens that
/// STRADDLE structural boundaries — that is where the mask and the automaton
/// have historically drifted apart (`"})`, mask.rs).
///
/// It also carries, on purpose, the two documented vocabulary edge cases: one
/// EMPTY token (a special/control token, mask.rs — always closed) and two ids
/// with the SAME text (mask.rs `TrieNode::ends` is a Vec for exactly this).
/// MEASURED: 61 to 82 tokens across the 96 generated schemas.
fn vocabulary(schema: &ArgSchema) -> Vec<String> {
    let mut names = BTreeSet::new();
    let mut values = BTreeSet::new();
    collect_literals(schema, &mut names, &mut values);

    let mut set: BTreeSet<String> = BTreeSet::new();
    // Structure, numbers, the letters of true/false, the escape machinery, both
    // whitespace kinds, and a multi-byte body character.
    for c in "{}[],:\"0123456789-+.eE truefals\\uabcdefABCDEF\nç".chars() {
        set.insert(c.to_string());
    }
    for literal in names.iter().chain(values.iter()) {
        for c in literal.chars() {
            set.insert(c.to_string());
        }
        if !literal.is_empty() {
            set.insert(literal.clone());
        }
    }
    for name in &names {
        set.insert(format!("\"{name}\":"));
    }
    for t in [
        "{\"", "\":", "\":\"", "\"}", "\"},", ",\"", "}]", "}}", "\"})", ")", "[]", "[{", "]}",
        "true", "false", "12", "-1", "1e3", "0",
    ] {
        set.insert(t.to_string());
    }

    let mut vocab: Vec<String> = set.into_iter().collect();
    let duplicate = vocab[0].clone();
    vocab.push(duplicate);
    vocab.push(String::new());
    vocab
}

// ---------------------------------------------------------------- the oracle

/// STRICTER THAN `ArgSchema::validate`, and that is the point. `validate`
/// iterates the schema's fields, so `{"query":"a","zzz":1}` passes it; the
/// grammar refuses the `z` outright and `json_schema()` says
/// `additionalProperties: false`. If this checker were not written the whole
/// property would quietly degrade to "the output is JSON-ish".
fn conforms(schema: &ArgSchema, value: &Value, path: &str) -> Result<(), String> {
    match &schema.kind {
        SchemaKind::Object { fields } => {
            let Some(map) = value.as_object() else {
                return Err(format!("{path}: not an object"));
            };
            for key in map.keys() {
                if !fields.iter().any(|f| &f.name == key) {
                    return Err(format!("{path}: key '{key}' is not in the schema"));
                }
            }
            for f in fields {
                match map.get(&f.name) {
                    None => {
                        if f.required {
                            return Err(format!("{path}.{}: required field missing", f.name));
                        }
                    }
                    Some(v) => conforms(&f.schema, v, &format!("{path}.{}", f.name))?,
                }
            }
            Ok(())
        }
        SchemaKind::Array { item, min, max } => {
            let Some(items) = value.as_array() else {
                return Err(format!("{path}: not an array"));
            };
            if min.is_some_and(|n| items.len() < n) || max.is_some_and(|n| items.len() > n) {
                return Err(format!("{path}: {} items is out of bounds", items.len()));
            }
            for (i, v) in items.iter().enumerate() {
                conforms(item, v, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        SchemaKind::Text { max_length } => {
            let Some(s) = value.as_str() else {
                return Err(format!("{path}: not a string"));
            };
            if max_length.is_some_and(|n| s.chars().count() > n) {
                return Err(format!(
                    "{path}: {} chars is over the limit",
                    s.chars().count()
                ));
            }
            Ok(())
        }
        SchemaKind::Choice { choices } => {
            let Some(s) = value.as_str() else {
                return Err(format!("{path}: not a string"));
            };
            if choices.iter().any(|c| c == s) {
                Ok(())
            } else {
                Err(format!("{path}: '{s}' is outside the closed set"))
            }
        }
        SchemaKind::Number {
            is_integer,
            min,
            max,
        } => {
            let Some(n) = value.as_f64() else {
                return Err(format!("{path}: not a number"));
            };
            if *is_integer && !value.is_i64() && !value.is_u64() {
                return Err(format!("{path}: {n} is not an integer"));
            }
            if min.is_some_and(|v| n < v) || max.is_some_and(|v| n > v) {
                return Err(format!("{path}: {n} is out of range"));
            }
            Ok(())
        }
        SchemaKind::Bool => value
            .as_bool()
            .map(|_| ())
            .ok_or_else(|| format!("{path}: not true/false")),
    }
}

// ---------------------------------------------------------------- diagnostics

/// Everything needed to reproduce a red run BY HAND: the seed, a copy-pasteable
/// schema, the exact token walk and where it stopped.
struct Trace<'a> {
    seed: u64,
    schema: &'a ArgSchema,
    tokens: &'a [String],
    output: &'a str,
    state: &'a GrammarState,
}

impl fmt::Display for Trace<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let allowed = self.state.allowed_prefixes();
        let tail: Vec<&String> = self.tokens.iter().rev().take(3).collect();
        writeln!(f, "  seed    0x{:016X}", self.seed)?;
        writeln!(
            f,
            "  schema  {}",
            serde_json::to_string(self.schema).expect("ArgSchema is Serialize")
        )?;
        writeln!(f, "  step    {}", self.tokens.len())?;
        writeln!(f, "  last    {tail:?} (most recent first)")?;
        writeln!(f, "  output  {:?}", self.output)?;
        writeln!(
            f,
            "  allowed {:?} text_body={} can_finish={} done={}",
            allowed.chars().collect::<Vec<_>>(),
            allowed.is_text_body(),
            allowed.can_finish(),
            self.state.is_done()
        )
    }
}

// ---------------------------------------------------------------- the walk

/// Is `text` a token the TERMINATOR rule must open at `state`: everything before
/// its closing `)` is grammar-acceptable and leaves the automaton finished.
///
/// This is the only thing that is true of the difference between
/// `mask_with_terminator` and `mask`. In particular the difference is NOT empty
/// while `!is_done()`: `open_terminator` runs at every node of the trie walk
/// against the BRANCHED state, so at `{"q":"a` the token `"})` is already an
/// extra — which is the exact case mask.rs exists for.
fn opens_on_terminator(state: &GrammarState, text: &str) -> bool {
    let Some(prefix) = text.strip_suffix(TERMINATOR) else {
        return false;
    };
    let mut probe = state.clone();
    probe.advance(prefix).is_ok() && probe.is_done()
}

/// Scores an open token for the OPEN phase: the tightest corner the automaton
/// can be pushed into wins.
fn score(text: &str, next: &GrammarState, spaced: bool) -> i64 {
    let space_only = !text.is_empty() && text.chars().all(char::is_whitespace);
    let allowed = next.allowed_prefixes();
    // A free body is the LOOSEST position there is; treat it as maximally wide so
    // the sampler is pulled away from it and towards enum tails, key tails and
    // number ranges.
    let narrowness = if allowed.is_text_body() {
        1024
    } else {
        allowed.chars().count() as i64
    };
    let multi = if text.chars().count() > 1 { 40 } else { 0 };
    let escapes = if text.contains('"') || text.contains('\\') {
        25
    } else {
        0
    };
    // Whitespace is free at structural positions and worth visiting once, but a
    // run of it would eat the whole open phase.
    let space = if space_only {
        if spaced { -200 } else { 3 }
    } else {
        0
    };
    1000 - narrowness + multi + escapes + space
}

/// Scores an open SINGLE character for the CLOSING phase.
///
/// The context matters, and getting it wrong is not a grammar fault but a hung
/// test. Three versions of this were wrong, each measured against a walk that
/// ran to the 200-token cap:
///   * `}` and `]` scored as closers WITHOUT asking where they were, so inside a
///     free `Text` body the walk wrote 200 braces into the string;
///   * whitespace ranked above an opener, so at `[` with only `{` on offer it
///     stalled on newlines;
///   * a fraction digit ranked above the `,` that ends the number, so it wrote a
///     170-digit fraction instead of moving to the next field.
///
/// Hence the shape below: anything that ENDS the current value beats anything
/// that extends it, and whitespace — the only character that is always legal and
/// never progress — is always the worst move.
fn closing_score(c: char, in_body: bool) -> i64 {
    if c.is_whitespace() {
        return -1000;
    }
    if in_body {
        // Inside a string body every other character just lengthens the string.
        return if c == '"' { 100 } else { -100 };
    }
    match c {
        '}' | ']' => 30,
        // A quote either closes a string or starts the key of a field that still
        // has to be written before the object may close; both are progress.
        '"' => 20,
        // Only reachable when a field or item is still owed, so it is progress.
        ',' => 10,
        '{' | '[' => -10,
        // Digits, letters, `.`, `-`, `:` — the body of a value being written.
        _ => -20,
    }
}

/// What one walk VISITED, so the sweep can prove it was not vacuous.
///
/// A COVERAGE COUNTER IS PART OF THE MEASUREMENT, not decoration. The first
/// version of this file asserted (c) and (d) at every step of every walk and
/// reported a green sweep — while the string ESCAPE machinery (`StringStage::
/// Escape`, `Hex`, `LowEscape`, `LowU`, `hex_fits` and the hex filter in
/// `allowed_prefixes`) was never once entered. MEASURED, by instrumenting this
/// file's own sampler: of 384 walks, 0 contained a backslash and 0 contained a
/// `\u` escape. The sampler is drawn to narrow corners and a free string body
/// is the widest position there is, so it entered a body only when forced and
/// closed it on the very next character. `escape_bonus` below is what changes
/// that, and these counters are what stop it silently going back to zero.
#[derive(Default)]
struct Census {
    /// Walks whose output contains a `\` inside a string body.
    escapes: usize,
    /// Walks that reached `StringStage::Hex` — i.e. wrote `\u` and then had to
    /// satisfy `hex_fits` for four digits, surrogate pairing included.
    unicode_escapes: usize,
}

impl Census {
    fn absorb(&mut self, other: &Census) {
        self.escapes += other.escapes;
        self.unicode_escapes += other.unicode_escapes;
    }
}

/// One adversarial walk. EVERY per-step proposition is asserted here, so a red
/// run points at the proposition by name in the message even though the three
/// sweep tests only differ in which schemas they feed it.
fn adversarial_walk(seed: u64, schema: &ArgSchema) -> String {
    walk_with_census(seed, schema).0
}

fn walk_with_census(seed: u64, schema: &ArgSchema) -> (String, Census) {
    let grammar = Grammar::compile(schema);
    let vocab = vocabulary(schema);
    let masker = TokenMask::new(&vocab);
    let mut rng = Xorshift64Star::new(seed);
    let mut state = grammar.state();
    let mut output = String::new();
    let mut tokens: Vec<String> = Vec::new();
    let mut spaced = false;
    let mut census = Census::default();
    // At most one escape is FORCED per walk. Once is enough to reach the stage
    // (an escape opened inside a body drags the automaton through Escape and,
    // for `\u`, through four `hex_fits` decisions and any surrogate pairing it
    // owes); forcing every one would turn the walk into an escape generator and
    // cost the corners this sampler exists for.
    let mut owed_escape = true;
    let mut last_was_escape = false;
    // Hex digits still owed in the `\u` escape being written, 4 down to 0.
    let mut hex_left = 0usize;

    loop {
        let trace = Trace {
            seed,
            schema,
            tokens: &tokens,
            output: &output,
            state: &state,
        };
        let allowed = state.allowed_prefixes();

        // (a) A dead node is a model that locks up: nothing to write and no way
        //     to stop. This is the fault class range_fits exists to prevent.
        assert!(
            !allowed.is_empty() || allowed.can_finish(),
            "DEAD NODE: nothing can be produced and generation cannot end\n{trace}"
        );
        // (b) With an OBJECT root, "the whole input may end here" and "the stack
        //     is empty" are the same statement.
        assert_eq!(
            state.is_done(),
            allowed.can_finish(),
            "is_done() and can_finish() disagree\n{trace}"
        );

        let open = masker.mask(&state);

        // (c) THE MASK AND THE AUTOMATON NEVER DISAGREE, in BOTH directions. A
        //     token opened but rejected by advance is the ConstraintViolation
        //     fault of call.rs; a token accepted by advance but left closed is
        //     the `"})` fault of mask.rs. Empty tokens are never opened — when
        //     EOS becomes free is the caller's decision, not the grammar's.
        let mut accepted: Vec<(usize, GrammarState)> = Vec::new();
        for (id, text) in vocab.iter().enumerate() {
            let mut probe = state.clone();
            let takes_it = !text.is_empty() && probe.advance(text).is_ok();
            assert_eq!(
                open[id], takes_it,
                "mask/advance drift on token {id} {text:?}: mask says {}, advance says {takes_it}\n{trace}",
                open[id]
            );
            if takes_it {
                accepted.push((id, probe));
            }
        }

        // (d) THE TERMINATOR OPENS THE CALL CLOSE AND NOTHING AFTER IT. The
        //     terminator mask is the grammar mask plus exactly those tokens whose
        //     text is P + ")" with P acceptable and finishing — no more, no less.
        let open_term = masker.mask_with_terminator(&state, Some(TERMINATOR));
        for (id, text) in vocab.iter().enumerate() {
            let expected = open[id] || opens_on_terminator(&state, text);
            assert_eq!(
                open_term[id], expected,
                "terminator mask is wrong on token {id} {text:?}\n{trace}"
            );
        }

        if state.is_done() {
            return (output, census);
        }
        assert!(
            tokens.len() < STEP_CAP,
            "the walk never closed within {STEP_CAP} tokens\n{trace}"
        );

        // The CLOSING phase considers only single-character tokens. The
        // schema-derived vocabulary always carries every character the grammar
        // can need, so one is always open; multi-character and boundary-
        // straddling tokens are exercised by the open phase and, at every single
        // step, by the two mask sweeps above — which look at the whole
        // vocabulary regardless of what the sampler then picks.
        let closing = tokens.len() >= OPEN_PHASE_STEPS;
        let in_body = allowed.is_text_body();
        let mut best: Vec<usize> = Vec::new();
        let mut best_score = i64::MIN;
        // THE ONE MOVE THE SCORING WOULD NEVER MAKE. Inside a free body every
        // character but `"` is a step away from finishing, so both phases price
        // `\` below the closing quote and the escape machinery is unreachable
        // (measured: 0 of 384 walks, see `Census`). This overrides the score
        // exactly once per walk, and only where the grammar is already offering
        // the character — the mask sweeps above still decide what is legal, so
        // this steers the sampler and cannot excuse the automaton anything.
        let escape_bonus = |text: &str| -> i64 {
            if owed_escape && in_body && text == "\\" {
                1 << 20
            } else if last_was_escape && (text == "u" || text == "\\") {
                // Having opened `\`, take the branch that reaches Hex rather
                // than a one-character escape like `\n`. After a HIGH surrogate
                // the grammar owes a `\` and then a `u`; both are covered here.
                1 << 20
            } else if hex_left == 4 && (text == "d" || text == "D") {
                // AND STEER THE DIGITS INTO THE SURROGATE BLOCK. Visiting `Hex`
                // is not the same as MAKING ITS RULE BITE: `hex_fits` only
                // constrains anything once the prefix is `D8`-`DB` (a high half
                // that owes a low one) or `DC`-`DF` (a lone low half, refused).
                // Measured — with the walk merely entering the stage, breaking
                // `hex_fits`'s `left - 1` into `left` still left every sweep
                // green. With these two lines the mask/advance sweep catches it.
                1 << 20
            } else if hex_left == 3 && text == "8" {
                1 << 20
            } else {
                0
            }
        };
        for (id, next) in &accepted {
            let text = &vocab[*id];
            let mut chars = text.chars();
            let s = match (closing, chars.next(), chars.next()) {
                (true, Some(c), None) => closing_score(c, in_body) * 8,
                // Ranked below every productive single character but above
                // whitespace (-8000), so a multi-character token is taken only
                // when nothing better is open — a missing character can then
                // never stall the walk.
                (true, _, _) => -2000,
                _ => score(text, next, spaced),
            } + (rng.draw() % 7) as i64
                + escape_bonus(text);
            if s > best_score {
                best_score = s;
                best.clear();
                best.push(*id);
            } else if s == best_score {
                best.push(*id);
            }
        }
        assert!(
            !best.is_empty(),
            "the mask opened nothing while the automaton was not done\n{trace}"
        );
        let chosen = best[rng.below(best.len())];
        let text = vocab[chosen].clone();
        if in_body && text == "\\" {
            census.escapes += 1;
            owed_escape = false;
        }
        if last_was_escape && text == "u" {
            census.unicode_escapes += 1;
            hex_left = 4;
        } else if hex_left > 0
            && text.chars().count() == 1
            && text.chars().all(|c| c.is_ascii_hexdigit())
        {
            hex_left -= 1;
        }
        // `\` is only ever a one-character token here, and a token ENDING in a
        // backslash cannot be anything else: a multi-character token whose last
        // character is `\` leaves the automaton owing an escape too.
        last_was_escape = text.ends_with('\\');
        spaced = text.chars().all(char::is_whitespace);
        state
            .advance(&text)
            .expect("the sweep above already advanced this exact token");
        output.push_str(&text);
        tokens.push(text);
    }
}

/// Runs the walks for one slice of the seed table and returns how many witnesses
/// it produced — plus what they VISITED, so a test that silently stops walking,
/// or that stops reaching the escape machinery, cannot pass.
fn sweep(seeds: &[u64]) -> (usize, Census) {
    let mut witnesses = 0;
    let mut census = Census::default();
    for schema_seed in seeds {
        let mut schema_rng = Xorshift64Star::new(*schema_seed);
        let schema = generate_schema(&mut schema_rng);
        for walk in 0..WALKS_PER_SCHEMA {
            let seed = schema_seed ^ walk.wrapping_mul(0x9E3779B97F4A7C15);
            let (output, visited) = walk_with_census(seed, &schema);
            census.absorb(&visited);

            let value: Value = serde_json::from_str(&output).unwrap_or_else(|e| {
                panic!(
                    "the automaton said done on text serde_json refuses: {output:?}\n  \
                     seed 0x{seed:016X}\n  error {e}"
                )
            });
            if let Err(why) = conforms(&schema, &value, "arg") {
                panic!(
                    "the automaton produced something the schema rejects: {why}\n  \
                     seed 0x{seed:016X}\n  output {output:?}\n  schema {}",
                    serde_json::to_string(&schema).expect("Serialize")
                );
            }
            // The weaker witness on top: the grammar and the tool layer share one
            // contract, so `validate` must agree wherever it has an opinion.
            assert!(
                schema.validate(&value).is_ok(),
                "ArgSchema::validate rejected an accepted output: {output:?}"
            );
            witnesses += 1;
        }
    }
    (witnesses, census)
}

/// The non-vacuity floor for one third of the seed table.
///
/// MEASURED on this machine over the whole table (384 walks): 33 walks entered a
/// string escape from the body, and 43 `\u` escapes were opened — more than 33
/// because the digits are steered into the high-surrogate block and a pair opens
/// a second `\u` for its low half. Per third: 15, 9 and 19. It is not higher
/// because a walk can only escape inside a free string body and most generated
/// schemas never put it in one; forcing more would mean biasing the SCHEMA
/// generator, which is a different measurement. The floor is set below the
/// smallest third so ordinary sampler drift does not go red, while a return to
/// the ZERO this file measured before `escape_bonus` existed does.
const MIN_UNICODE_ESCAPES_PER_THIRD: usize = 3;

/// The assertion every sweep test makes about its own coverage.
fn assert_not_vacuous(seeds: &[u64], witnesses: usize, census: &Census) {
    assert_eq!(witnesses, seeds.len() * WALKS_PER_SCHEMA as usize);
    assert!(
        census.unicode_escapes >= MIN_UNICODE_ESCAPES_PER_THIRD,
        "only {} of {witnesses} walks reached a \\u escape ({} entered any escape at all). \
         The per-step assertions above then never looked at StringStage::Hex, hex_fits or the \
         surrogate pairing — which is exactly the state this file was in, and green, before the \
         escape bonus was added.",
        census.unicode_escapes,
        census.escapes
    );
}

// ---------------------------------------------------------------- sweep tests

const THIRD: usize = SCHEMA_SEEDS.len() / 3;

#[test]
fn no_random_walk_ever_produces_something_the_schema_rejects() {
    let seeds = &SCHEMA_SEEDS[..THIRD];
    let (witnesses, census) = sweep(seeds);
    assert_not_vacuous(seeds, witnesses, &census);
}

#[test]
fn the_mask_and_the_automaton_never_disagree() {
    let seeds = &SCHEMA_SEEDS[THIRD..2 * THIRD];
    let (witnesses, census) = sweep(seeds);
    assert_not_vacuous(seeds, witnesses, &census);
}

#[test]
fn the_terminator_opens_the_call_close_and_nothing_after_it() {
    let seeds = &SCHEMA_SEEDS[2 * THIRD..];
    let (witnesses, census) = sweep(seeds);
    assert_not_vacuous(seeds, witnesses, &census);
}

// ---------------------------------------------------------------- refusal

/// Renders a value the way this file needs it: minimal, and with NO exponent
/// notation. `serde_json::to_string` is not used because ryu may render a small
/// f64 as `1e-5`, and a bounded number refuses `e` on purpose (state.rs) — the
/// mutant would then be refused for a spelling reason instead of the reason
/// under test. Rust's `Display` for f64 never uses scientific notation.
fn render(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                u.to_string()
            } else if let Some(i) = n.as_i64() {
                i.to_string()
            } else {
                format!("{}", n.as_f64().expect("a JSON number is one of the three"))
            }
        }
        Value::String(s) => render_string(s),
        Value::Array(items) => {
            let body: Vec<String> = items.iter().map(render).collect();
            format!("[{}]", body.join(","))
        }
        Value::Object(map) => {
            let body: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}:{}", render_string(k), render(v)))
                .collect();
            format!("{{{}}}", body.join(","))
        }
    }
}

fn render_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A mutant plus why it is invalid — the reason is printed on failure so a red
/// run says which family broke, not just "something was accepted".
struct Mutant {
    text: String,
    family: &'static str,
}

/// Every mutation of the witness this file knows how to make. The families that
/// need a schema fact (a bound, a closed set, a required key) are built from the
/// schema; the families that are pure text (truncation, trailing input, a
/// duplicated pair) are built from the rendered witness.
fn mutants(schema: &ArgSchema, value: &Value, text: &str) -> Vec<Mutant> {
    fn push(out: &mut Vec<Mutant>, text: String, family: &'static str) {
        out.push(Mutant { text, family });
    }
    /// The same pair, one field replaced. Written as a function and not a
    /// closure because `out` is borrowed for the whole family loop below.
    fn replace(
        out: &mut Vec<Mutant>,
        map: &serde_json::Map<String, Value>,
        name: &str,
        v: Value,
        family: &'static str,
    ) {
        let mut m = map.clone();
        m.insert(name.to_string(), v);
        out.push(Mutant {
            text: render(&Value::Object(m)),
            family,
        });
    }

    let mut out: Vec<Mutant> = Vec::new();

    // Truncation: every proper prefix. Handled separately by the caller because
    // the refusal is a NON-CLOSE, not a rejected character.
    // Trailing input, including the call terminator, which is not in the grammar.
    push(&mut out, format!("{text}}}"), "trailing brace");
    push(&mut out, format!("{text} extra"), "trailing chatter");
    push(
        &mut out,
        format!("{text}{TERMINATOR}"),
        "trailing terminator",
    );

    let Some(map) = value.as_object() else {
        return out;
    };
    let pairs: Vec<String> = map
        .iter()
        .map(|(k, v)| format!("{}:{}", render_string(k), render(v)))
        .collect();

    // A duplicated pair. serde_json keeps the LAST of two identical keys, so the
    // parsed value still conforms and `conforms()` cannot see this one — the
    // grammar is strictly STRONGER here (it marks a key seen and never offers it
    // again), and the assertion is made anyway for that reason.
    if let Some(first) = pairs.first() {
        let mut all = vec![first.clone()];
        all.extend(pairs.iter().cloned());
        push(&mut out, format!("{{{}}}", all.join(",")), "duplicate key");
    }

    // A key that is not in the schema. `ArgSchema::validate` ACCEPTS this; the
    // grammar refuses the very first letter of the invented key.
    let mut with_unknown = vec!["\"zzz\":1".to_string()];
    with_unknown.extend(pairs.iter().cloned());
    push(
        &mut out,
        format!("{{{}}}", with_unknown.join(",")),
        "unknown key",
    );

    for f in schema.fields() {
        // A required pair dropped.
        if f.required && map.contains_key(&f.name) {
            let mut less = map.clone();
            less.remove(&f.name);
            push(
                &mut out,
                render(&Value::Object(less)),
                "required field dropped",
            );
        }
        let Some(current) = map.get(&f.name) else {
            continue;
        };
        let name = f.name.as_str();
        // A value of the wrong type.
        match &f.schema.kind {
            SchemaKind::Bool => replace(
                &mut out,
                map,
                name,
                Value::String("x".into()),
                "type swapped",
            ),
            _ => replace(&mut out, map, name, Value::Bool(true), "type swapped"),
        }
        match &f.schema.kind {
            SchemaKind::Number {
                is_integer,
                min,
                max,
            } => {
                if let Some(hi) = max {
                    replace(
                        &mut out,
                        map,
                        name,
                        json_number(hi + 1.0),
                        "number over max",
                    );
                }
                if let Some(lo) = min {
                    replace(
                        &mut out,
                        map,
                        name,
                        json_number(lo - 1.0),
                        "number under min",
                    );
                }
                if *is_integer && let Some(n) = current.as_f64() {
                    let candidate = if max.is_none_or(|hi| n + 0.5 <= hi) {
                        n + 0.5
                    } else {
                        n - 0.5
                    };
                    replace(
                        &mut out,
                        map,
                        name,
                        Value::from(candidate),
                        "integer given a fraction",
                    );
                }
            }
            SchemaKind::Choice { .. } => replace(
                &mut out,
                map,
                name,
                Value::String("zzz".into()),
                "outside the enum",
            ),
            SchemaKind::Text {
                max_length: Some(limit),
            } => replace(
                &mut out,
                map,
                name,
                Value::String("a".repeat(limit + 1)),
                "text over the limit",
            ),
            SchemaKind::Array { min, max, .. } => {
                let items = current.as_array().cloned().unwrap_or_default();
                if min.is_some_and(|n| n > 0) {
                    replace(
                        &mut out,
                        map,
                        name,
                        Value::Array(Vec::new()),
                        "array under min",
                    );
                }
                if let (Some(hi), Some(last)) = (max, items.last()) {
                    let mut over = items.clone();
                    while over.len() <= *hi {
                        over.push(last.clone());
                    }
                    replace(&mut out, map, name, Value::Array(over), "array over max");
                }
            }
            _ => {}
        }
    }
    out
}

fn json_number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Value::from(v as i64)
    } else {
        Value::from(v)
    }
}

/// Feeds `text` one character at a time and reports the first index the
/// automaton refuses at, if any.
fn first_refusal(
    grammar: &std::sync::Arc<Grammar>,
    text: &str,
) -> Option<(usize, char, GrammarState)> {
    let mut state = grammar.state();
    for (i, c) in text.chars().enumerate() {
        match state.branch(c) {
            Ok(next) => state = next,
            Err(_) => return Some((i, c, state)),
        }
    }
    None
}

#[test]
fn every_mutation_of_an_accepted_call_is_refused_at_a_named_index() {
    let mut truncations = 0usize;
    let mut refusals = 0usize;
    let mut skipped = 0usize;
    let mut families: BTreeSet<&'static str> = BTreeSet::new();
    for schema_seed in SCHEMA_SEEDS {
        let mut schema_rng = Xorshift64Star::new(schema_seed);
        let schema = generate_schema(&mut schema_rng);
        let grammar = Grammar::compile(&schema);
        let witness = adversarial_walk(schema_seed, &schema);
        let value: Value = serde_json::from_str(&witness).expect("the walk asserts this");
        let canonical = render(&value);
        // The canonical rendering must itself be producible, otherwise every
        // mutant built from it would be refused for the wrong reason.
        let mut probe = grammar.state();
        assert!(
            probe.advance(&canonical).is_ok() && probe.is_done(),
            "the canonical rendering of an accepted witness is not producible: {canonical:?}"
        );

        // TRUNCATION is a different refusal: the text is consumed in full and the
        // structure simply never closes. Because the walk stops at the FIRST
        // is_done(), the witness carries no trailing whitespace and every proper
        // prefix really is incomplete.
        let chars: Vec<char> = canonical.chars().collect();
        for cut in 0..chars.len() {
            let prefix: String = chars[..cut].iter().collect();
            let mut state = grammar.state();
            state.advance(&prefix).unwrap_or_else(|e| {
                panic!("a prefix of an accepted witness was refused: {prefix:?} ({e})")
            });
            assert!(
                !state.is_done(),
                "a proper prefix reported done: {prefix:?}"
            );
            assert_eq!(
                state.finish().unwrap_err(),
                tacet_grammar::GrammarError::Incomplete,
                "a proper prefix did not report Incomplete: {prefix:?}"
            );
            truncations += 1;
        }

        for mutant in mutants(&schema, &value, &canonical) {
            let family = mutant.family;
            // Prove the mutant really IS invalid before demanding a refusal —
            // otherwise the test would be asserting a false refusal. The two
            // families the independent checker cannot see are named explicitly.
            let independently_bad = match serde_json::from_str::<Value>(&mutant.text) {
                Err(_) => true, // not even JSON
                Ok(v) => conforms(&schema, &v, "arg").is_err(),
            };
            if !independently_bad && !matches!(family, "duplicate key" | "unknown key") {
                skipped += 1;
                continue;
            }

            let Some((index, offender, before)) = first_refusal(&grammar, &mutant.text) else {
                let mut state = grammar.state();
                state.advance(&mutant.text).expect("no refusal was found");
                panic!(
                    "the automaton accepted an invalid call ({family}): {:?} done={}",
                    mutant.text,
                    state.is_done()
                );
            };

            // THE CLAUSE THAT MAKES THIS MASKING RATHER THAN VALIDATION: at the
            // prefix, the offending character's own token is CLOSED in the mask,
            // so the model could not have produced it in the first place.
            let single = vec![offender.to_string()];
            let closed = TokenMask::new(&single).mask(&before);
            assert!(
                !closed[0],
                "({family}) the automaton refused {offender:?} at index {index} of {:?} \
                 but the MASK left its token open — that is after-the-fact validation, not masking",
                mutant.text
            );
            refusals += 1;
            families.insert(family);
        }
    }
    // A generator that quietly stopped generating would make every assertion
    // above vacuous, so the census is asserted too. The numbers are MEASURED on
    // this machine and are exact for this fixed seed table; they are written as
    // floors so an unrelated schema-shape change does not turn this red, but a
    // collapse of the corpus does.
    assert!(
        truncations >= 2_300 && refusals >= 800,
        "the corpus collapsed: {truncations} truncations (2426 on this seed table), \
         {refusals} refusals (858)"
    );
    // MEASURED: exactly 1 of the 859 mutants came out still-conforming and was
    // skipped, so the table really is producing invalid calls rather than
    // accidentally valid ones. The bound is a floor, not that number, because a
    // schema-shape change may legitimately move it by a few.
    assert!(
        skipped * 4 < refusals,
        "{skipped} of {refusals} mutants came out still-conforming; the table is not biting"
    );
    // Every family the table can build must actually have been built and
    // refused, otherwise a family could rot into a no-op unnoticed.
    let expected: BTreeSet<&str> = [
        "array over max",
        "array under min",
        "duplicate key",
        "integer given a fraction",
        "number over max",
        "number under min",
        "outside the enum",
        "required field dropped",
        "text over the limit",
        "trailing brace",
        "trailing chatter",
        "trailing terminator",
        "type swapped",
        "unknown key",
    ]
    .into_iter()
    .collect();
    assert_eq!(families, expected, "a mutation family stopped firing");
}

// ---------------------------------------------------- confirmed counterexamples

fn text_schema() -> ArgSchema {
    ArgSchema::object(vec![Field::new("q", ArgSchema::text()).required()])
}

fn accepts(schema: &ArgSchema, text: &str) -> bool {
    let mut state = Grammar::compile(schema).state();
    state.advance(text).is_ok() && state.is_done()
}

/// COUNTEREXAMPLE 1, found by this file and fixed in state.rs.
///
/// Before the fix `advance` accepted `{"q":"\uD800"}` and reported `is_done()`,
/// while `serde_json::from_str` on the same text answered "unexpected end of hex
/// escape"; `{"q":"\uDC00"}` answered "lone leading surrogate in hex escape".
/// A lone surrogate has no UTF-8 encoding, so no JSON parser that produces Rust
/// strings can accept one — the grammar could GENERATE something that is not
/// JSON, which is precisely the claim the crate makes impossible.
#[test]
fn a_lone_surrogate_cannot_be_generated_even_though_four_hex_digits_look_fine() {
    let s = text_schema();
    for good in [
        r#"{"q":"𐀀"}"#,  // a complete pair: U+10000
        r#"{"q":"😀"}"#, // lowercase, U+1F600
        r#"{"q":"A"}"#,
        r#"{"q":"￿"}"#,
        r#"{"q":"😀"}"#, // the same character raw in the body
    ] {
        assert!(accepts(&s, good), "must still be producible: {good}");
        serde_json::from_str::<Value>(good).unwrap_or_else(|e| panic!("{good}: {e}"));
    }
    for bad in [
        r#"{"q":"\uD800"}"#,  // a high surrogate with no low half
        r#"{"q":"\uDC00"}"#,  // a lone low surrogate
        r#"{"q":"\uD800A"}"#, // a high surrogate followed by something else
        r#"{"q":"\uD800A"}"#, // ... or by a raw character
        r#"{"q":"\uDBFF"}"#,
        r#"{"q":"\uDFFF"}"#,
    ] {
        assert!(!accepts(&s, bad), "must be unproducible: {bad}");
        assert!(
            serde_json::from_str::<Value>(bad).is_err(),
            "serde_json is the independent witness here: {bad}"
        );
    }

    // The mask narrows on the SECOND hex digit, not at the fourth: after `\uD`
    // the escape may still become D000..DBFF, but every value under `\uDC` is a
    // lone low surrogate.
    let grammar = Grammar::compile(&s);
    let mut state = grammar.state();
    state
        .advance(r#"{"q":"\uD"#)
        .expect("a high surrogate may start");
    let allowed = state.allowed_prefixes();
    assert!(allowed.contains('7') && allowed.contains('B') && allowed.contains('b'));
    assert!(!allowed.contains('C') && !allowed.contains('c'));
    assert!(!allowed.contains('F') && !allowed.contains('f'));

    // Owing the low half, the string may not close and nothing but `\` opens.
    let mut owing = grammar.state();
    owing
        .advance(r#"{"q":"\uD800"#)
        .expect("the high half is legal");
    assert_eq!(
        owing.allowed_prefixes().chars().collect::<Vec<_>>(),
        vec!['\\'],
        "a half-written surrogate pair has exactly one exit"
    );

    // A pair counts as ONE character, the way serde_json decodes it and the way
    // `ArgSchema::validate` counts it — so a max_length of 1 admits the pair.
    let bounded = ArgSchema::object(vec![
        Field::new(
            "q",
            ArgSchema {
                kind: SchemaKind::Text {
                    max_length: Some(1),
                },
                description: None,
            },
        )
        .required(),
    ]);
    let text = r#"{"q":"😀"}"#;
    assert!(accepts(&bounded, text));
    let value: Value = serde_json::from_str(text).expect("valid JSON");
    assert!(bounded.validate(&value).is_ok(), "the length gates agree");

    // The other hostile escapes, for completeness.
    for bad in [
        r#"{"q":"\q"}"#,
        r#"{"q":"\u00e"}"#,
        r#"{"q":"\u00zz"}"#,
        "{\"q\":\"a\nb\"}",
    ] {
        assert!(!accepts(&s, bad), "must be unproducible: {bad}");
    }
}

/// COUNTEREXAMPLE 2, found by this file and fixed in state.rs.
///
/// Before the fix, `number().range(Some(10.0), Some(20.0))` accepted `20.5`
/// (`range_fits` looked only at the integer part) and `1e9`, `15e5`, `15e50`
/// (the exponent stages returned `true` unconditionally). Each one left the
/// allowed set non-empty forever with NO completion at all — an exhaustive
/// search over the reachable states of `{"n":1e9` returns nothing. That is the
/// same lock-up `range_fits` exists to prevent, reached from the other side:
/// not a dead mask, a live mask over a path with no exit.
#[test]
fn a_bounded_number_never_parks_on_a_prefix_that_cannot_close() {
    let bounded = |min: Option<f64>, max: Option<f64>| {
        ArgSchema::object(vec![
            Field::new("n", ArgSchema::number().range(min, max)).required(),
        ])
    };

    // Every prefix reachable in a bounded number must have a completion.
    for (min, max) in NUMBER_RANGES {
        let schema = bounded(min, max);
        let grammar = Grammar::compile(&schema);
        let mut state = grammar.state();
        state.advance(r#"{"n":"#).expect("the value position opens");
        assert!(
            completes(&state, COMPLETION_BUDGET).is_some(),
            "no completion at all for range ({min:?},{max:?})"
        );
        // EXHAUSTIVE over reachable number prefixes up to four characters — long
        // enough for `20.5` and `1e9`, which is where both defects lived.
        // MEASURED over the seven ranges: depth 4 visits 15,279 prefixes
        // in 2.2s of debug `cargo test`, depth 5 visits 161,885 in 23.7s and
        // finds nothing depth 4 misses. So this is a BUDGET, not a claim about
        // longer prefixes; the one five-character shape that matters, the
        // negative mirror `-20.5`, is asserted by name below.
        reachable_number_prefixes(&state, 4, &mut |s, written| {
            assert!(
                completes(s, COMPLETION_BUDGET).is_some(),
                "range ({min:?},{max:?}): the prefix {written:?} can never close"
            );
        });
    }

    // The two exact prefixes that were accepted before the fix.
    let ten_twenty = bounded(Some(10.0), Some(20.0));
    for dead in [
        r#"{"n":20.5}"#,
        r#"{"n":1e9}"#,
        r#"{"n":15e5}"#,
        r#"{"n":15e50}"#,
    ] {
        assert!(!accepts(&ten_twenty, dead), "must be unproducible: {dead}");
    }
    // ... and their negative mirror.
    let minus = bounded(Some(-20.0), Some(-10.0));
    assert!(!accepts(&minus, r#"{"n":-20.5}"#));
    assert!(accepts(&minus, r#"{"n":-19.5}"#));

    // The VALUES are all still reachable; only the exponent SPELLING is gone.
    // `5.1e-1` was the one completion the old grammar had for the prefix `5.1`
    // in [-5,5]; `0.51` is the same number and is still producible.
    let five = bounded(Some(-5.0), Some(5.0));
    assert!(accepts(&five, r#"{"n":0.51}"#));
    assert!(accepts(&five, r#"{"n":-0.51}"#));
    assert!(!accepts(&five, r#"{"n":5.1e-1}"#));
    assert!(accepts(&ten_twenty, r#"{"n":19.999}"#));
    assert!(accepts(&ten_twenty, r#"{"n":20.0}"#));
    assert!(!accepts(&ten_twenty, r#"{"n":20.1}"#));

    // An UNBOUNDED number keeps the exponent — that is where JSON exponents
    // actually turn up, and nothing about them can lock up without a bound.
    let free = bounded(None, None);
    for good in ["1e3", "1E+3", "2.5e-4", "-0", "0.5"] {
        assert!(accepts(&free, &format!(r#"{{"n":{good}}}"#)), "{good}");
    }
    // Bad spellings stay bad in both.
    for bad in ["01", "1.", ".5", "+1", "1e", "--1", "1.2.3"] {
        assert!(!accepts(&free, &format!(r#"{{"n":{bad}}}"#)), "{bad}");
        assert!(!accepts(&ten_twenty, &format!(r#"{{"n":{bad}}}"#)), "{bad}");
    }
}

/// COUNTEREXAMPLE 3, found by the sweep above and predicted by nobody.
///
/// The two defects above were both foreseen; this one only turned up because a
/// walk had to produce a whole call and then hand it to `serde_json`. An
/// UNBOUNDED `Number` gets no range pruning at all, so the automaton accepted
/// `{"n":1e31212121212121212}` with `is_done() == true` while
/// `serde_json::from_str` answered "number out of range" — the value overflows
/// f64. Rust's own `f64::from_str` returns INFINITY instead of an error, which
/// is exactly why the close-time `buffer.parse()` saw nothing wrong.
///
/// JSON itself puts no bound on the magnitude of a number. Every consumer in
/// this workspace does, and the grammar's contract is with the consumer.
#[test]
fn a_number_too_large_for_f64_cannot_be_generated() {
    let free = ArgSchema::object(vec![Field::new("n", ArgSchema::number()).required()]);
    let long_ok = format!("1{}", "0".repeat(308));
    let long_bad = format!("1{}", "0".repeat(309));
    // MEASURED on both sides: the grammar and serde_json agree on every one of
    // these, which is what makes "ask Rust's parse for finiteness" the RULE and
    // not an approximation of it. Underflow needs no rule — `1e-400` is 0.0 to
    // both of them, and neither complains.
    for good in [
        "1e308",
        "1.5e308",
        "1e-400",
        "1e-31212121212121212",
        long_ok.as_str(),
    ] {
        let text = format!(r#"{{"n":{good}}}"#);
        assert!(accepts(&free, &text), "must stay producible: {good}");
        serde_json::from_str::<Value>(&text).unwrap_or_else(|e| panic!("{good}: {e}"));
    }
    for bad in [
        "1e309",
        "1e400",
        "1.8e308",
        "1e31212121212121212",
        "-1e400",
        long_bad.as_str(),
    ] {
        let text = format!(r#"{{"n":{bad}}}"#);
        assert!(!accepts(&free, &text), "must be unproducible: {bad}");
        assert!(
            serde_json::from_str::<Value>(&text).is_err(),
            "serde_json is the independent witness here: {bad}"
        );
    }

    // The refusal happens in the MASK, one digit before the overflow: at `1e30`
    // the digits 0..8 stay open (1e300..1e308) and `9` is already closed. `}` is
    // in the set because a number closes without consuming, so the parent
    // object's permission shows through.
    let grammar = Grammar::compile(&free);
    let mut state = grammar.state();
    state.advance(r#"{"n":1e30"#).expect("1e30 is finite");
    assert_eq!(
        state.allowed_prefixes().chars().collect::<Vec<_>>(),
        vec!['0', '1', '2', '3', '4', '5', '6', '7', '8', '}']
    );
    // And at the edge itself the number can still CLOSE, so nothing is stranded:
    // no digit opens, the object's `}` does.
    let mut edge = grammar.state();
    edge.advance(r#"{"n":1e308"#).expect("1e308 is finite");
    assert_eq!(
        edge.allowed_prefixes().chars().collect::<Vec<_>>(),
        vec!['}']
    );
}

/// How many states `completes` may visit before giving up. A number closes in a
/// handful of characters, and BFS finds the SHORTEST completion, so the budget
/// only has to cover the levels above it. MEASURED over every prefix this file
/// searches: the longest completion found was 4 characters and the most states
/// visited before finding one was 21. 500 is margin, not requirement — and it is
/// written down rather than tuned silently, because a budget set too low shows
/// up as a FALSE failure that reads exactly like the real defect.
const COMPLETION_BUDGET: usize = 500;

/// Breadth-first: is there ANY continuation that closes, within `budget` states?
/// Breadth-first and not depth-first on purpose — a number's allowed set is ten
/// digits wide, so a depth-first search over it does not terminate in practice.
fn completes(state: &GrammarState, budget: usize) -> Option<String> {
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((state.clone(), String::new()));
    let mut left = budget;
    while let Some((s, written)) = queue.pop_front() {
        if s.is_done() {
            return Some(written);
        }
        if left == 0 {
            return None;
        }
        left -= 1;
        for c in s.allowed_prefixes().chars().collect::<Vec<_>>() {
            if let Ok(next) = s.branch(c) {
                let mut w = written.clone();
                w.push(c);
                queue.push_back((next, w));
            }
        }
    }
    None
}

fn reachable_number_prefixes(
    state: &GrammarState,
    depth: usize,
    check: &mut impl FnMut(&GrammarState, &str),
) {
    fn walk(
        state: &GrammarState,
        depth: usize,
        written: &mut String,
        check: &mut impl FnMut(&GrammarState, &str),
    ) {
        check(state, written);
        if depth == 0 {
            return;
        }
        for c in state.allowed_prefixes().chars().collect::<Vec<_>>() {
            // Only the number's own alphabet; `}` would leave the frame.
            if !c.is_ascii_digit() && !matches!(c, '-' | '+' | '.' | 'e' | 'E') {
                continue;
            }
            if let Ok(next) = state.branch(c) {
                written.push(c);
                walk(&next, depth - 1, written, check);
                written.pop();
            }
        }
    }
    let mut written = String::new();
    walk(state, depth, &mut written, check);
}
