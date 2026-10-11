//! `GrammarState` — a push-down automaton.
//!
//! WHY AN AUTOMATON AND NOT AFTER-THE-FACT VALIDATION: a small model (3B), left
//! free, starts EXPLAINING "shall I do this?" instead of calling a tool.
//! Validating the produced text afterwards catches that regression but does not
//! fix it; the only cure is to make the invalid character impossible before it
//! is produced. So the automaton advances character by character and can answer
//! "what is valid right now" at every step — masking rests exactly on that answer.
//!
//! STACK: because JSON nests, a flat DFA is not enough; open object/array frames
//! are kept on a stack. The frames carry only `usize` indices + small counters
//! (see compile.rs), so cloning the state is cheap — masking does hundreds of
//! clones per step.

use crate::compile::{Grammar, Node};
use crate::error::GrammarError;
use crate::permit::{AllowedSet, SPACES};
use std::sync::Arc;
use tacet_kernel::ArgSchema;

/// Bool values; embedded in the grammar literally.
const LITERALS: [&str; 2] = ["true", "false"];

/// What may follow a JSON escape character.
const ESCAPES: [char; 8] = ['"', '\\', '/', 'b', 'f', 'n', 'r', 't'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectStage {
    /// `{` consumed: either a key starts or the object closes.
    KeyOrClose,
    /// The opening quote of the key was consumed, chars are matching.
    InKey,
    /// The closing quote of the key was consumed, `:` expected.
    AfterKey,
    /// `:` consumed, a value expected.
    AfterColon,
    /// The value frame is on the stack, on top of it.
    InValue,
    /// The value ended: `,` or `}`.
    AfterValue,
    /// `,` consumed: a new key must follow.
    AfterComma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrayStage {
    /// `[` consumed.
    Start,
    /// The item frame is on top.
    InItem,
    /// The item ended: `,` or `]`.
    AfterItem,
    /// `,` consumed: a new item must follow.
    BeforeItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringStage {
    Body,
    /// `\` consumed.
    Escape,
    /// A HIGH surrogate (`\uD800`..`\uDBFF`) was just written: the only legal
    /// continuation is the LOW half of the pair, so only `\` opens here.
    LowEscape,
    /// The `\` of the low half was consumed: only `u` opens.
    LowU,
    /// `\u` consumed. `value` is the hex digits written so far, `left` how many
    /// are still owed (never 0 — the stage ends when the fourth lands), and
    /// `low_half` says this escape must land inside `DC00..DFFF`.
    Hex {
        value: u32,
        left: u8,
        low_half: bool,
    },
}

/// Can a `\u` escape whose first `4 - left` hex digits spell `value` still land
/// on a code unit that is legal HERE?
///
/// WHY IT IS NOT ENOUGH TO ACCEPT FOUR HEX DIGITS (this is a MEASURED defect,
/// not a hypothetical): until this check existed, `{"q":"\uD800"}` was accepted
/// by `advance` with `is_done() == true`, and `serde_json::from_str` on the very
/// same text answered "unexpected end of hex escape"; `{"q":"\uDC00"}` answered
/// "lone leading surrogate in hex escape". So the grammar could GENERATE a
/// string that is not JSON — which is exactly the claim this crate exists to
/// make impossible. A lone surrogate has no UTF-8 encoding, so no JSON parser
/// that produces Rust strings can accept one.
///
/// The reachable final values from the prefix are `[value<<4L, value<<4L + 16^L)`,
/// so the decision is an interval intersection and the mask narrows as early as
/// the SECOND digit: after `\uD` the escape may still become D000..DBFF, but
/// after `\uDC` every reachable value is a lone low surrogate and `C` never opens.
fn hex_fits(value: u32, left: u8, low_half: bool) -> bool {
    let lo = value << (4 * left);
    let hi = lo + (1u32 << (4 * left)) - 1;
    if low_half {
        // Owed the second half of a pair: only DC00..DFFF closes it.
        hi >= 0xDC00 && lo <= 0xDFFF
    } else {
        // A lone LOW surrogate never decodes. A HIGH surrogate does open — but
        // only because writing one FORCES the low half (`StringStage::LowEscape`).
        lo < 0xDC00 || hi > 0xDFFF
    }
}

/// JSON number grammar: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE][+-]?[0-9]+)?`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumberStage {
    Start,
    AfterMinus,
    /// A lone `0` was consumed — no digit MAY follow (JSON forbids a leading zero).
    Zero,
    IntDigit,
    /// `.` consumed, at least one digit is mandatory.
    Dot,
    FracDigit,
    /// `e`/`E` consumed.
    Exp,
    /// `+`/`-` consumed, a digit is mandatory.
    ExpSign,
    ExpDigit,
}

impl NumberStage {
    /// Can the number end here (may we return to the parent frame without
    /// consuming the character).
    fn can_finish(self) -> bool {
        matches!(
            self,
            NumberStage::Zero
                | NumberStage::IntDigit
                | NumberStage::FracDigit
                | NumberStage::ExpDigit
        )
    }
}

/// The single transition table of the number automaton.
///
/// `feed_frame` and `frame_allowed` SHARE this function. Written separately,
/// the two would drift over time, the allowed set would stay wider than what
/// the automaton actually accepts, and the mask would open a dead path for the
/// model — the bug would only surface when generation locked up.
fn number_transition(
    stage: NumberStage,
    c: char,
    fractional: bool,
    min: Option<f64>,
) -> Option<NumberStage> {
    let digit = c.is_ascii_digit();
    match (stage, c) {
        // If the range is entirely non-negative, the minus sign never opens.
        (NumberStage::Start, '-') if min.is_none_or(|v| v < 0.0) => Some(NumberStage::AfterMinus),
        (NumberStage::Start | NumberStage::AfterMinus, '0') => Some(NumberStage::Zero),
        (NumberStage::Start | NumberStage::AfterMinus, _) if ('1'..='9').contains(&c) => {
            Some(NumberStage::IntDigit)
        }
        (NumberStage::IntDigit, _) if digit => Some(NumberStage::IntDigit),
        (NumberStage::Zero | NumberStage::IntDigit, '.') if fractional => Some(NumberStage::Dot),
        (NumberStage::Zero | NumberStage::IntDigit, 'e' | 'E') if fractional => {
            Some(NumberStage::Exp)
        }
        (NumberStage::Dot | NumberStage::FracDigit, _) if digit => Some(NumberStage::FracDigit),
        (NumberStage::FracDigit, 'e' | 'E') => Some(NumberStage::Exp),
        (NumberStage::Exp, '+' | '-') => Some(NumberStage::ExpSign),
        (NumberStage::Exp | NumberStage::ExpSign | NumberStage::ExpDigit, _) if digit => {
            Some(NumberStage::ExpDigit)
        }
        _ => None,
    }
}

/// The characters that may be tried in the number automaton; `number_transition`
/// filters them.
const NUMBER_CANDIDATES: [char; 15] = [
    '-', '+', '.', 'e', 'E', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
];

/// The most digits a number's FRACTION may have.
///
/// WHY A NUMBER NEEDS A BOUND AT ALL — the same defect as `FREE_TEXT_CEILING`,
/// in the one place that constant did not reach. JSON puts no limit on a
/// fraction, and nothing here did either: MEASURED (audit L25), `0.` followed
/// by 200 000 digits was still an open path, as was `1e-` followed by 200 000
/// zeros. Every prefix of those is a valid number, so the call never HAS to
/// end; the README's "the grammar bounds the length of a field the schema
/// leaves open" was true of strings only.
///
/// FOUR HUNDRED, AND NOT SEVENTEEN, because the bound must never make a value
/// unreachable. A BOUNDED number refuses the exponent (`range_fits`), so plain
/// decimal is the only spelling it has, and the smallest positive f64 written
/// that way, `5e-324`, needs 323 zeros and its digit. MEASURED over a spread of
/// subnormals and the normal edge (tests.rs, `every_f64_a_range_can_hold_is_
/// still_writable_in_plain_decimal`): no f64's shortest plain spelling is
/// longer than those 324 places. 400 clears that, so every f64 a range can hold
/// stays writable, and it is still a hard stop three orders of magnitude short
/// of the measured runaway.
pub(crate) const MAX_FRACTION_DIGITS: usize = 400;

/// The most digits an EXPONENT may have, leading zeros included.
///
/// Every finite f64 has an exponent within ±324, so three digits reach all of
/// them; the fourth admits the zero-padded spelling some printers emit
/// (`1e+0308`). JSON allows any number of leading zeros, which is why the bound
/// is on digits and not on the exponent's value: `1e-000…0001` was as
/// unending as a long fraction (audit L25). The exponent is only open for an
/// UNBOUNDED number, so nothing a range needs can be cut off by this.
///
/// THE INTEGER PART HAS NO CONSTANT OF ITS OWN because representability already
/// bounds it, and the bound is the tool layer's: 20 digits for an integer
/// (u64::MAX), 19 after a minus (i64::MIN), and 309 for a number (the last
/// finite f64). JSON forbids a leading zero there, so no padding can stretch it.
pub(crate) const MAX_EXPONENT_DIGITS: usize = 4;

/// The number the tool layer will read from `text` — `ArgSchema::json_number_value`
/// — answered without running `serde_json` where the answer is provably the same.
///
/// WHY A FAST PATH: this runs once per candidate character at every number
/// position of the mask walk. MEASURED (M-series, release, a synthetic 32k
/// vocabulary heavy in digit and `eN` tokens, at `{"n":12`): calling
/// `serde_json` every time took a mask step from 2.1ms to 3.2ms; this path
/// brings most of that back (see `range_fits` for the figure that is left).
///
/// WHY IT IS EXACT, not close. Both sides round CORRECTLY whenever the decimal
/// is short (`rounds_alike`), and Rust's `f64::from_str` is correctly rounded by
/// specification — so both return THE f64 nearest the decimal, sign included
/// (`-0.0` from `-0`). An integer of at most 18 digits fits i64, which is what
/// `validate` asks of it, and its f64 is likewise the correctly rounded one on
/// both sides. Everything else — a long mantissa, a far exponent, `-0` in an
/// integer field, the u64/i64/f64 edges — is handed to `serde_json` itself, so
/// the edges stay the consumer's. `the_fast_path_and_the_validator_agree` in
/// tests.rs walks both over every shape on either side of these lines.
///
/// `bounded == false` (no range to compare against) needs only the YES/NO, and
/// there a far exponent is answered from Rust's parse too: `serde_json` only
/// ever refuses a number for overflow, never for underflow, and its double
/// rounding is an ulp or two — it cannot carry anything below 1e308 past
/// f64::MAX. The value handed back may then be an ulp off the consumer's,
/// which nothing on that path reads — and an unbounded number is the only
/// kind that keeps the exponent, so this is where every `e99`-shaped token in
/// a vocabulary lands.
///
/// `text` IS ALWAYS A COMPLETE JSON NUMBER here — the automaton only asks about
/// a finished prefix or its shortest completion — so the length and the absence
/// of an exponent are all the fast path has to look at. (Rust's parse accepts
/// spellings JSON does not, `1.` and `01`; none of them can reach this.)
pub(crate) fn validator_value(text: &str, is_integer: bool, bounded: bool) -> Option<f64> {
    if is_integer {
        // At most 18 digits after an optional `-`: inside i64 either way.
        if text.len() <= 18 && text != "-0" {
            return text.parse::<f64>().ok();
        }
        return ArgSchema::json_number_value(text, true);
    }
    if rounds_alike(text) {
        return text.parse::<f64>().ok();
    }
    let rust = text.parse::<f64>().ok()?;
    // A cheap REFUSAL before the exact answer: where Rust's correctly rounded
    // parse is already infinite the decimal is past f64::MAX by at least half an
    // ulp, and nothing `serde_json` reaches from there is a number this layer
    // should offer — at worst it is one ulp of the very edge, refused, and a
    // refusal is the safe direction (a shorter spelling is always still open).
    if rust.is_infinite() {
        return None;
    }
    if !bounded && rust.abs() < 1e308 {
        return Some(rust);
    }
    ArgSchema::json_number_value(text, false)
}

/// Is `text` short enough that `serde_json` and Rust's `f64::from_str` provably
/// return the same f64 — at most 15 mantissa digits, and a net power of ten
/// within ±22?
///
/// `serde_json` computes `significand * 10^k` (or `/ 10^-k`) with the
/// significand an exact integer below 2^53 and `10^k` exact up to 10^22: ONE
/// correctly rounded operation, so the nearest f64 to the decimal — which is
/// what Rust returns by specification. Outside those lines `serde_json` rounds
/// twice and the two can differ by an ulp; that is where the M16 edge
/// `1.7976931348623158e308` lives, so those spellings go to `serde_json`.
///
/// THE EXPONENT IS COVERED, NOT ONLY THE PLAIN DECIMAL, because a vocabulary
/// carries exponent tokens (`e5`, `e10`) and every one of them would otherwise
/// cost a `serde_json` parse. One pass over the bytes, because it runs per
/// candidate on the mask's hot path.
fn rounds_alike(text: &str) -> bool {
    let bytes = text.as_bytes();
    let start = usize::from(bytes.first() == Some(&b'-'));
    let (mut digits, mut frac, mut in_frac) = (0usize, 0i32, false);
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        match b {
            b'0'..=b'9' => {
                digits += 1;
                frac += i32::from(in_frac);
            }
            b'.' => in_frac = true,
            b'e' | b'E' => {
                // At most `MAX_EXPONENT_DIGITS` digits reach here, so the
                // parse cannot overflow.
                let Ok(e) = text[i + 1..].parse::<i32>() else {
                    return false;
                };
                return digits <= 15 && (e - frac).abs() <= 22;
            }
            _ => return false,
        }
    }
    digits <= 15 && frac <= 22
}

/// Do the fraction and the exponent of this number prefix fit their bounds?
fn digits_fit(buffer: &str) -> bool {
    let body = buffer.trim_start_matches('-');
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let fraction = mantissa.split_once('.').map_or(0, |(_, f)| f.len());
    let exponent = exponent.map_or(0, |e| e.trim_start_matches(['+', '-']).len());
    fraction <= MAX_FRACTION_DIGITS && exponent <= MAX_EXPONENT_DIGITS
}

/// Can THIS PREFIX still be completed into a number the layer downstream can
/// actually hold — inside the schema's range, and representable at all.
///
/// WHY PREFIX-BASED: if the range were only checked when the number closes, the
/// model would produce `99` in the range `[1,50]` and get stuck — every
/// following character is rejected, the mask empties, generation LOCKS UP. In
/// constrained generation there is no such thing as "erroring out afterwards":
/// every prefix that leads to something invalid must be closed before it is
/// produced.
///
/// Method: the value range reachable from the prefix is computed, and the prefix
/// survives if that range intersects [min, max]. There are two regimes, and
/// which one applies is decided by the stage:
///   * the integer part is still growing — `v` is the integer part and `k` more
///     digits may follow, so the reachable values are the union of the ranges
///     `[v*10^k, v*10^k + 10^k)`;
///   * a fraction has started — nothing may grow but the tail, so the reachable
///     set is the single decade `[v, v + 10^-d)` around the WHOLE prefix.
///
/// The second regime is exact only because the exponent is refused outright for
/// a bounded number; both facts were put here by measurement and both are
/// spelled out at their site below.
fn range_fits(
    buffer: &str,
    stage: NumberStage,
    is_integer: bool,
    min: Option<f64>,
    max: Option<f64>,
) -> bool {
    // THE DIGIT BOUNDS COME FIRST: they are what makes an unbounded number a
    // path that has to end (see `MAX_FRACTION_DIGITS`).
    // (Anything of six characters or fewer fits by construction — a fifth
    // exponent digit needs at least `1e` plus five — so the scan is skipped on
    // the hot path where nearly every number is short.)
    if buffer.len() > 6 && !digits_fit(buffer) {
        return false;
    }
    // REPRESENTABILITY IS CHECKED EVEN WITH NO RANGE AT ALL, and it is asked of
    // the tool layer's own parser rather than reasoned about here.
    //
    // This used to ask Rust's `f64` parse for finiteness, with a comment saying
    // that and `serde_json` "agree exactly on where the edge is". MEASURED
    // (audit M16), they do not, and finiteness was never the whole rule either:
    //   * `-0` in an INTEGER field — `serde_json` reads it as the float -0.0 and
    //     `validate` answers "expected an integer"; the audit's 200 000-walk
    //     fuzz closed on it 15 287 times, the one disagreement left;
    //   * `18446744073709551616` and `-9223372036854775809` — finite, and past
    //     u64/i64, so `serde_json` stores a float and the integer check fails;
    //   * `1.7976931348623158e308` — Rust rounds it down to f64::MAX,
    //     `serde_json`'s faster parse rounds it up to infinity and refuses it.
    // `ArgSchema::json_number_value` runs `serde_json` and the classification
    // `validate` itself uses, so the edge is the consumer's edge by
    // construction. (The first overflow found this way, `1e31212121212121212`,
    // is now also cut by the exponent's digit bound.)
    //
    // Asked of the SHORTEST COMPLETION when the prefix owes a digit (`20.`,
    // `1e`, `1e-` complete with a `0`): every other completion of those is
    // larger in magnitude (a positive exponent) or nearer zero (a negative
    // one), so if the shortest is out of reach, everything is. Pruning it as a
    // PREFIX is sound for the same reason the overflow rule always was: the
    // integer part and a positive exponent only grow, so once a prefix is out
    // of reach no continuation brings it back, the digit that tips it is
    // refused before it is written, and the model is never stranded on it.
    //
    // COST, MEASURED (M-series, release, a synthetic 32k vocabulary heavy in
    // digit and `eN` tokens — the worst case for this path; `validator_value`
    // answers most of it without `serde_json`): a mask step at an unbounded
    // `{"n":12` went from 2.1ms to 2.5ms, at an integer from 0.61ms to 0.67ms,
    // at a bounded number from 0.75ms to 0.78ms. Against the ~55ms a token
    // costs at the 18.2 tok/s measured under `MAX_SPACE_RUN`, that is the price
    // of the grammar no longer offering numbers the tool layer refuses.
    let bounded = min.is_some() || max.is_some();
    let value = match stage {
        // Nothing but a sign yet: every kind can still write a digit.
        NumberStage::Start | NumberStage::AfterMinus => None,
        NumberStage::Dot | NumberStage::Exp | NumberStage::ExpSign => {
            // The shortest completion (`X.0`, `Xe0`, `Xe+0`) has the value of
            // `X` itself. Where `X` is short both parses are exact
            // (`validator_value`), so `X` is asked directly and the hot path
            // allocates nothing; a long `X` gets the completion spelled out,
            // because there the two spellings take different routes through
            // `serde_json` and only the spelled-out one is the consumer's.
            let mantissa = buffer.trim_end_matches(['.', 'e', 'E', '+', '-']);
            let value = if mantissa.len() <= 15 {
                validator_value(mantissa, is_integer, bounded)
            } else {
                validator_value(&format!("{buffer}0"), is_integer, bounded)
            };
            match value {
                Some(v) => Some(v),
                None => return false,
            }
        }
        _ => match validator_value(buffer, is_integer, bounded) {
            Some(v) => Some(v),
            None => return false,
        },
    };
    if min.is_none() && max.is_none() {
        return true;
    }
    let lo = min.unwrap_or(f64::NEG_INFINITY);
    let hi = max.unwrap_or(f64::INFINITY);
    // EXPONENT NOTATION IS REFUSED once the range has a finite end, and this
    // line used to say `return true` instead — "the exact check at close time is
    // enough". MEASURED, that claim was false in the way that matters most: with
    // `number().range(Some(10.0), Some(20.0))` the automaton accepted `1e9`,
    // `15e5` and `15e50`, offered `0`..`9` forever afterwards and could NEVER
    // close. That is the lock-up this whole function exists to prevent, arrived
    // at from the other side — not a dead mask, a mask that stays alive on a
    // path with no exit. An exhaustive search over the reachable states of
    // `{"n":1e9` returns no completion at all.
    //
    // Pruning the exponent properly would mean tracking the mantissa's reachable
    // magnitudes decade by decade. The cheaper trade taken here: with a finite
    // bound the exponent buys NOTHING a model writes — every value inside the
    // range is still reachable in plain decimal (`0.51` for `5.1e-1`), only the
    // SPELLING is gone. An unbounded number keeps `e`/`E`, which is where JSON
    // exponents actually turn up.
    if matches!(
        stage,
        NumberStage::Exp | NumberStage::ExpSign | NumberStage::ExpDigit
    ) {
        return false;
    }
    let negative = buffer.starts_with('-');
    // A fraction has STARTED: the integer part can no longer grow, and (because
    // the exponent is closed above) the reachable set is exactly one decade of
    // the last digit written — `[v, v + 10^-d)` upward, `(v - 10^-d, v]`
    // downward, where `d` is the fraction digits so far.
    //
    // ANCHORED ON THE WHOLE PREFIX, not on the integer part. Anchoring on the
    // integer part is what this function did until it was measured: in the range
    // [10,20] the prefix `20.5` was read as "int part 20, so [20,21) — fits",
    // and `20.5` then had no completion and no way to close. The mirror case
    // `-20.5` in [-20,-10] failed the same way.
    if matches!(stage, NumberStage::Dot | NumberStage::FracDigit) {
        // The value the CLOSE-TIME check will compare — the tool layer's parse
        // of the prefix (`20.` read as `20.0`), not a second parse of our own.
        let Some(v) = value else {
            return true;
        };
        let digits = buffer.split_once('.').map_or(0, |(_, f)| f.len());
        // THE REACH FOLLOWS `10^-d` DOWN INTO THE SUBNORMALS and only reaches
        // 0 near d = 324, where f64 runs out. Two spellings that look right are
        // each wrong in a measured way:
        //   * `10f64.powi(-d)` is already 0 at d = 309 (`powi` underflows its
        //     intermediate), which collapses the interval to the point `v` and
        //     prunes `0.000…` before a subnormal can be written — it turns
        //     `every_f64_a_range_can_hold_is_still_writable_in_plain_decimal`
        //     red;
        //   * clamping to `10^-308` (what this said until review) keeps the
        //     reach at 1e-308 for every d from 309 to 399, so under a subnormal
        //     `lo` such as 5e-324 a run of zeros that could no longer reach
        //     `lo` stayed open until the digit bound shut it: `0.` and 399
        //     zeros, mask empty, `is_done()` false — the model stranded
        //     (`a_subnormal_bound_never_strands_a_run_of_zeros`).
        // The `v >= lo` / `v <= hi` disjunct keeps a prefix that is ALREADY in
        // range alive once the reach is 0. At the digit bound nothing more can
        // be written, so the reach is the point itself.
        let reach = if digits >= MAX_FRACTION_DIGITS {
            0.0
        } else if digits > 308 {
            1e-308 * 10f64.powi(-((digits - 308) as i32))
        } else {
            10f64.powi(-(digits as i32))
        };
        return if negative {
            v >= lo && (v <= hi || v - reach < hi)
        } else {
            v <= hi && (v >= lo || v + reach > lo)
        };
    }
    let body = buffer.trim_start_matches('-');
    let int_part: String = body.chars().take_while(char::is_ascii_digit).collect();
    if int_part.is_empty() {
        // Only `-` was written. For a NUMBER that leaves `-0` and below, and the
        // minus sign only opened because `lo < 0`. For an INTEGER it leaves only
        // -1 and below, because `-0` is refused above — so `-` may open only if
        // the range holds a negative integer. Without this, refusing `-0`
        // would have stranded the model in [-0.5, 0]: `-` open, nothing after it.
        return !is_integer || hi.min(-1.0).floor() >= lo;
    }
    let Ok(v) = int_part.parse::<f64>() else {
        return true;
    };
    // The integer part can only grow in stages that accept another digit.
    // `Zero` is excluded: after a leading zero JSON accepts no digit.
    let can_grow = matches!(stage, NumberStage::IntDigit);
    let max_k = if can_grow { 18 } else { 0 };
    for k in 0..=max_k {
        let c = 10f64.powi(k);
        let base = v * c;
        // For an integer the upper bound is INCLUSIVE (v*c + c - 1), for a
        // fractional one it is EXCLUSIVE ((v+1)*c): only that distinction rules
        // out the prefix `0` in the range `[1,50]`.
        let (low, high, high_incl) = if is_integer {
            (base, base + c - 1.0, true)
        } else {
            (base, (v + 1.0) * c, false)
        };
        let (low, high, low_incl, high_incl) = if negative {
            (-high, -low, high_incl, true)
        } else {
            (low, high, true, high_incl)
        };
        let high_ok = if high_incl { high >= lo } else { high > lo };
        let low_ok = if low_incl { low <= hi } else { low < hi };
        if high_ok && low_ok {
            return true;
        }
        // The ranges shift in one direction only: once we are past the target,
        // more digits will not save it.
        if !negative && low > hi {
            break;
        }
        if negative && high < lo {
            break;
        }
    }
    false
}

#[derive(Debug, Clone)]
enum Frame {
    /// A value that has not started yet: the first character picks the type.
    Value { node: usize },
    Object {
        node: usize,
        /// Field-seen flags; this both prevents skipping required fields and
        /// prevents repeating a key, from the same place.
        seen: Vec<bool>,
        stage: ObjectStage,
        /// The field indices still possible while the key is being written.
        candidates: Vec<usize>,
        progress: usize,
        /// The field whose key has closed and whose value is expected.
        active: usize,
    },
    Array {
        node: usize,
        count: usize,
        stage: ArrayStage,
    },
    Text {
        node: usize,
        length: usize,
        stage: StringStage,
    },
    Choice {
        node: usize,
        candidates: Vec<usize>,
        progress: usize,
    },
    Number {
        node: usize,
        stage: NumberStage,
        buffer: String,
    },
    Literal {
        candidates: Vec<usize>,
        progress: usize,
    },
}

/// The effect of one character on a frame.
enum Step {
    /// The character was consumed, the frame continues.
    Consumed,
    /// The character was consumed and the frame completed.
    ConsumedAndDone,
    /// The frame completed but did NOT consume the character; the parent frame
    /// must try it. (Numbers end like this: `12` only closes when a `,` is seen.)
    DoneNotConsumed,
    /// The frame changed / a new frame was pushed; the same character must be
    /// tried again from the top.
    Retry,
}

/// The live state of constrained generation.
/// The ceiling on ONE string body when the schema declares no bound.
///
/// WHY AN UNBOUNDED FIELD IS NOT ACTUALLY SAFE — the same class of defect as
/// `MAX_SPACE_RUN`, found the same way and one round later. Bounding whitespace
/// closed the instance the model happened to fall into; it did not close the
/// shape. `ArgSchema::text()` produces `max_length: None`, `frame_allowed` then
/// left `open_text_body` on forever, and a free-text field became a path with no
/// obligation to end. Measured on qwen3-4b, `write_code-script`: turns 1 and 2
/// finished at 183 and 223 tokens with `stop=ConstraintDone`; turn 3 was still
/// generating FIVE MINUTES later, heading for the 13 483-token cap.
///
/// EIGHT THOUSAND CHARACTERS IS NOT A GUESS AT WHAT A FIELD NEEDS. It is far
/// above anything measured — the longest legitimate call in the selection suite
/// is ~223 tokens, and `write_code`'s own array is capped at 200 lines — and far
/// below the 14 000 tokens a runaway can spend. A schema that genuinely needs
/// more says so with `.max_length()`, which is respected as before; this is only
/// what "no bound" means.
pub(crate) const FREE_TEXT_CEILING: usize = 8192;

/// The most items an array may hold when the schema declares no `max`.
///
/// THE SAME HOLE AS `FREE_TEXT_CEILING`, ONE SHAPE OVER. `ArgSchema::array()`
/// produces `max: None`, `,` then stayed open after every item, and a list was
/// a path with no obligation to end — MEASURED (audit L25), an array of
/// 100 001 elements was still open, and the README's claim that the grammar
/// "bounds the length of a field the schema leaves open" was true of strings
/// only.
///
/// 1024 IS NOT A GUESS AT WHAT A LIST NEEDS. The largest bound any tool in the
/// catalog declares is 200 (`write_code`'s lines); 1024 is five times that, and
/// at two tokens an item (`1,`) a full array is already past the engine's
/// 2048-token cap — so with the engine in front this never fires on a
/// legitimate call, and without it (the crate is usable on its own) it is what
/// makes the list end. A schema that needs more says so with `.length()`; a
/// declared `min` above the ceiling raises it, so a schema is never left with
/// no array it can close.
pub(crate) const ARRAY_CEILING: usize = 1024;

/// The most items THIS array may hold: the declared `max`, or the ceiling —
/// never below the declared `min`, or the array could not close at all.
fn array_limit(min: Option<usize>, max: Option<usize>) -> usize {
    max.unwrap_or(ARRAY_CEILING.max(min.unwrap_or(0)))
}

/// How much room must be left in a free-text body before a character counts as
/// NEUTRAL.
///
/// NEUTRALITY IS A CLAIM ABOUT THE FUTURE: "this character changes no later
/// decision". Near the ceiling that is false — every character consumed brings
/// the closing quote nearer to being the only option — so the mask's fast path
/// (`plain` in `mask.rs`, which opens every token carrying no break character)
/// would start opening tokens that no longer fit.
///
/// THE SLACK IS LARGER THAN ANY TOKEN, deliberately, because the state cannot
/// see the vocabulary. 512 characters is beyond any real BPE token by two orders
/// of magnitude, so while the slack holds, every token `plain` opens is short
/// enough to fit. Under it the exact walk runs — slower, but only for the last
/// 512 characters of a field that has already written 7 680.
const NEUTRAL_SLACK: usize = 512;

/// How many whitespace characters may sit next to each other.
///
/// WHY A BOUND EXISTS AT ALL — a MEASURED non-termination, not a tidiness rule.
/// JSON puts no limit on whitespace, and neither did this automaton: at a
/// structural position `open_space` was called unconditionally and `feed_frame`
/// consumed a space without advancing anything, so " " was legal forever. That
/// is a cycle in the grammar with no exit pressure, and a small model walked
/// straight into it. Measured on qwen3-4b, the case `calendar-remind`:
///
///   (calendar({"title":"call the dentist","when":"tomorrow 9"          …
///
/// The arguments are COMPLETE and CORRECT. The model then emitted spaces until
/// the generation cap of 14 041 tokens — at 18.2 tok/s that is 12.9 minutes of a
/// user staring at nothing, for a call that was finished after 20 tokens.
///
/// THE HEADLINE CLAIM SURVIVES, AND THIS IS THE GAP BESIDE IT. "An invalid call
/// cannot be produced" was true the whole time: every prefix above is valid JSON
/// so far. What was missing is that a VALID prefix could lead somewhere with no
/// obligation to finish. Unrepresentable-invalid and always-terminating are two
/// different properties, and only the first one had been argued.
///
/// SIXTEEN, NOT ONE. A bound of one would reject `{"a": 1}` written with an
/// indent, and pretty-printed JSON reaching this automaton is not hypothetical —
/// an MCP server's example arguments arrive as text. Sixteen covers a newline
/// plus a deep indent and still turns 14 041 tokens of drift into 16.
const MAX_SPACE_RUN: u32 = 16;

#[derive(Debug, Clone)]

/// One generation's position inside a compiled grammar.
///
/// CHEAP TO CLONE ON PURPOSE: the mask asks "what would happen if this token
/// arrived" by branching a copy, so every piece of state that decides an answer
/// — including the whitespace run — has to live in here rather than beside it.
/// State kept outside is state the mask and the automaton can disagree about.
pub struct GrammarState {
    grammar: Arc<Grammar>,
    stack: Vec<Frame>,
    position: usize,
    /// Whitespace characters consumed since the last one that was not.
    ///
    /// IT IS PART OF THE STATE, so it clones with it. `advance` works on a copy
    /// and `branch` hands out copies to the mask — a counter kept outside would
    /// let the mask offer a space the automaton then refuses, which is the exact
    /// drift `mask.rs` warns about.
    spaces_run: u32,
}

impl GrammarState {
    /// A state at the very start of `grammar` — nothing consumed, the root value
    /// still expected.
    pub fn new(grammar: Arc<Grammar>) -> Self {
        let root = grammar.root;
        Self {
            grammar,
            stack: vec![Frame::Value { node: root }],
            position: 0,
            spaces_run: 0,
        }
    }

    /// Advances the automaton over a produced chunk.
    ///
    /// ATOMIC: on error the state does NOT change at all (the work happens on a
    /// copy). The reason: call sites want to continue with the same state after
    /// an error; a half-advanced automaton would silently produce a wrong mask.
    pub fn advance(&mut self, chunk: &str) -> Result<(), GrammarError> {
        let mut attempt = self.clone();
        for c in chunk.chars() {
            attempt.feed_char(c)?;
            attempt.position += 1;
        }
        *self = attempt;
        Ok(())
    }

    /// A single-character branch: returns the new state, leaves itself
    /// unchanged. Used on the hot masking path while walking the vocabulary trie.
    pub fn branch(&self, c: char) -> Result<Self, GrammarError> {
        let mut s = self.clone();
        s.feed_char(c)?;
        s.position += 1;
        Ok(s)
    }

    /// The characters acceptable right now.
    pub fn allowed_prefixes(&self) -> AllowedSet {
        let mut set = AllowedSet::default();
        // The mask must not offer what `feed_char` will refuse. `open_space` is
        // called from a dozen places below; rather than guarding each one, the
        // exhausted run is subtracted at the end (see `spaces_spent`).
        // THE SAME PREDICATE THE AUTOMATON USES. `feed_char` stops counting
        // inside a string body, so closing the space here would be the mirror of
        // the bug it fixes: the mask refusing what the automaton would accept.
        let spaces_spent = self.spaces_run >= MAX_SPACE_RUN && !self.in_string_body();
        let mut i = self.stack.len();
        // `child_done`: if a lower frame can close without consuming, the parent
        // frame must be looked at "as if my child had finished". The only such
        // frame is a number (`,` after `12` is the parent object's business),
        // but the loop is written generically.
        let mut child_done = false;
        loop {
            if i == 0 {
                set.open_can_finish();
                set.open_space();
                break;
            }
            i -= 1;
            if !self.frame_allowed(i, child_done, &mut set) {
                break;
            }
            child_done = true;
        }
        if spaces_spent {
            set.close_space();
        }
        set
    }

    /// Was a valid, complete JSON produced.
    pub fn is_done(&self) -> bool {
        self.stack.is_empty()
    }

    /// Closes generation; errors if a structure is still open.
    pub fn finish(&self) -> Result<(), GrammarError> {
        if self.is_done() {
            Ok(())
        } else {
            Err(GrammarError::Incomplete)
        }
    }

    /// An uncached mask — for small vocabularies and tests.
    /// On the hot path build the trie ONCE with `TokenMask::new()` and reuse it;
    /// this version rebuilds the trie on every call.
    pub fn mask(&self, vocab: &[String]) -> Vec<bool> {
        crate::TokenMask::new(vocab).mask(self)
    }

    /// Does this character leave the automaton's FUTURE decisions unchanged?
    ///
    /// Inside a string body of unbounded length, ordinary characters affect no
    /// transition: the same set is allowed afterwards as well. On the hot
    /// masking path this case dominates — in a free-text field almost the WHOLE
    /// vocabulary is valid, which would mean cloning the state on each of 32k
    /// branches. MEASURED effect: 6.1ms -> 0.2ms per step.
    pub(crate) fn is_neutral(&self, c: char) -> bool {
        self.in_free_text_run() && !breaks_free_text(c)
    }

    /// Is the automaton inside an UNBOUNDED string body — the one place where
    /// `is_neutral` can be true at all.
    ///
    /// WHY IT IS ITS OWN METHOD rather than folded into `is_neutral`: `TokenMask`
    /// needs the question without a character, because it precomputes the answer
    /// for the WHOLE vocabulary before it has one (see `plain` in `mask.rs`). Two
    /// modules deciding separately what "free text" means is exactly how a mask
    /// starts opening a token the automaton then refuses, so the definition lives
    /// here once and `is_neutral` is written in terms of it.
    ///
    /// `max_length: None` IS LOAD-BEARING. With a bound, each accepted character
    /// moves the state closer to the limit, so no character is neutral and the
    /// precomputed set would be wrong — it would keep opening tokens after the
    /// field was full.
    /// Is the next character content inside a string rather than structure?
    ///
    /// NARROWER THAN `in_free_text_run`, deliberately. That one asks whether the
    /// neutral token set can be reused (so it also demands an unbounded field
    /// with headroom); this one asks only "is a space here content", which is
    /// true of a bounded field and of a field close to its ceiling too.
    pub(crate) fn in_string_body(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(Frame::Text {
                stage: StringStage::Body,
                ..
            })
        )
    }

    pub(crate) fn in_free_text_run(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(Frame::Text {
                node,
                length,
                stage: StringStage::Body,
            }) if matches!(self.grammar.nodes[*node], Node::Text { max_length: None })
                && *length + NEUTRAL_SLACK < FREE_TEXT_CEILING
        )
    }
}

/// The characters that END a free-text run: they are the ones the automaton
/// still has to think about inside a string body.
///
/// THIS IS THE SAME PREDICATE `AllowedSet::contains` APPLIES to its `text_body`
/// branch (`permit.rs`), and the two must stay identical: `contains` decides what
/// may be produced, this decides what may be skipped. If they ever disagreed in
/// the direction "neutral but not allowed", the mask would open a token the
/// automaton refuses — the drift `mask.rs` and `call.rs` both warn about.
/// `the_neutral_class_is_a_subset_of_what_is_allowed` measures it.
pub(crate) fn breaks_free_text(c: char) -> bool {
    c == '"' || c == '\\' || c.is_control()
}

impl GrammarState {
    // ---- automaton ----

    fn feed_char(&mut self, c: char) -> Result<(), GrammarError> {
        // THE RUN IS COUNTED BEFORE ANYTHING ELSE LOOKS AT `c`, because every
        // STRUCTURAL path below that accepts a space must be bound by the same
        // number — the frame loop, and the "JSON already closed" branch right
        // here. A counter updated per branch would leave whichever branch was
        // forgotten as a way back into the cycle.
        //
        // BUT NOT INSIDE A STRING, and that exception is a fix rather than a
        // subtlety. Counting content spaces made the automaton refuse the 17th
        // space of an indented line while `allowed_prefixes` still offered it
        // through the free-text path — mask and automaton disagreeing, which is
        // the one thing `ConstraintError::Violation` exists to catch. Measured:
        // a `write_code` call carrying a Python line indented past sixteen
        // columns died mid-generation.
        //
        // The bound loses nothing by stepping aside here. The non-termination it
        // was written for happened at a STRUCTURAL position — the arguments were
        // complete, the automaton had a cycle with no exit pressure, and the
        // model drifted for 14 041 tokens. Inside a string the call is not
        // complete (the quote is still open), and a model that wants to run
        // forever can emit any character it likes; bounding one of them buys
        // nothing. Length is bounded there by `max_length` and the free-text
        // ceiling, which is the right instrument for it.
        let space = SPACES.contains(&c);
        if space && !self.in_string_body() {
            if self.spaces_run >= MAX_SPACE_RUN {
                return Err(GrammarError::UnexpectedCharacter {
                    character: c,
                    position: self.position,
                });
            }
            self.spaces_run += 1;
        } else if !space {
            self.spaces_run = 0;
        }
        loop {
            if self.stack.is_empty() {
                // The JSON closed: only whitespace is accepted.
                return if SPACES.contains(&c) {
                    Ok(())
                } else {
                    Err(GrammarError::TrailingInput {
                        position: self.position,
                    })
                };
            }
            match self.feed_frame(c)? {
                Step::Consumed => return Ok(()),
                Step::ConsumedAndDone => {
                    self.stack.pop();
                    self.child_done();
                    return Ok(());
                }
                Step::DoneNotConsumed => {
                    self.stack.pop();
                    self.child_done();
                }
                Step::Retry => {}
            }
        }
    }

    /// Records "the child value completed" in the parent frame.
    fn child_done(&mut self) {
        match self.stack.last_mut() {
            Some(Frame::Object { stage, .. }) => *stage = ObjectStage::AfterValue,
            Some(Frame::Array { stage, count, .. }) => {
                *count += 1;
                *stage = ArrayStage::AfterItem;
            }
            _ => {}
        }
    }

    fn feed_frame(&mut self, c: char) -> Result<Step, GrammarError> {
        let n = self.stack.len() - 1;
        let position = self.position;
        let err = || GrammarError::UnexpectedCharacter {
            character: c,
            position,
        };
        let space = SPACES.contains(&c);

        // Disjoint field borrows: `grammar` is read-only, `stack` is mutable.
        let grammar = &*self.grammar;
        let frame = &mut self.stack[n];
        let mut to_push: Option<Frame> = None;

        let step = match frame {
            // --- a value that has not started: the first character picks the type ---
            Frame::Value { node } => {
                let d = *node;
                if space {
                    Step::Consumed
                } else {
                    match &grammar.nodes[d] {
                        Node::Object { fields } if c == '{' => {
                            *frame = Frame::Object {
                                node: d,
                                seen: vec![false; fields.len()],
                                stage: ObjectStage::KeyOrClose,
                                candidates: Vec::new(),
                                progress: 0,
                                active: 0,
                            };
                            Step::Consumed
                        }
                        Node::Array { .. } if c == '[' => {
                            *frame = Frame::Array {
                                node: d,
                                count: 0,
                                stage: ArrayStage::Start,
                            };
                            Step::Consumed
                        }
                        Node::Text { .. } if c == '"' => {
                            *frame = Frame::Text {
                                node: d,
                                length: 0,
                                stage: StringStage::Body,
                            };
                            Step::Consumed
                        }
                        Node::Choice { choices } if c == '"' => {
                            *frame = Frame::Choice {
                                node: d,
                                candidates: (0..choices.len()).collect(),
                                progress: 0,
                            };
                            Step::Consumed
                        }
                        Node::Number { .. } => {
                            // Let the number frame eat the character: the
                            // sign/digit distinction is made there.
                            *frame = Frame::Number {
                                node: d,
                                stage: NumberStage::Start,
                                buffer: String::new(),
                            };
                            Step::Retry
                        }
                        Node::Bool => {
                            *frame = Frame::Literal {
                                candidates: (0..LITERALS.len()).collect(),
                                progress: 0,
                            };
                            Step::Retry
                        }
                        _ => return Err(err()),
                    }
                }
            }

            // --- object ---
            Frame::Object {
                node,
                seen,
                stage,
                candidates,
                progress,
                active,
            } => {
                let Node::Object { fields } = &grammar.nodes[*node] else {
                    unreachable!("an object frame cannot be opened on a non-object node")
                };
                let has_remaining = seen.iter().any(|s| !s);
                let required_done = fields
                    .iter()
                    .zip(seen.iter())
                    .all(|(f, s)| !f.required || *s);

                match *stage {
                    ObjectStage::KeyOrClose | ObjectStage::AfterComma => {
                        let can_close = *stage == ObjectStage::KeyOrClose && required_done;
                        if space {
                            Step::Consumed
                        } else if c == '}' && can_close {
                            Step::ConsumedAndDone
                        } else if c == '"' && has_remaining {
                            // A key already seen is not a candidate: a repeated
                            // key cannot be produced, the same contract as the
                            // schema.
                            *candidates = (0..fields.len()).filter(|i| !seen[*i]).collect();
                            *progress = 0;
                            *stage = ObjectStage::InKey;
                            Step::Consumed
                        } else {
                            return Err(err());
                        }
                    }
                    ObjectStage::InKey => {
                        // A `"` CLOSES the key only where a candidate is
                        // complete; anywhere else it can only be the second half
                        // of a `\"` in a key's spelling (`json_body`), and falls
                        // through to the narrowing below. The two never compete:
                        // a complete spelling cannot end in the lone `\` that a
                        // following `"` would continue.
                        let complete = candidates
                            .iter()
                            .copied()
                            .find(|i| fields[*i].name_chars.len() == *progress);
                        if let (true, Some(i)) = (c == '"', complete) {
                            seen[i] = true;
                            *active = i;
                            *stage = ObjectStage::AfterKey;
                            Step::Consumed
                        } else {
                            let remaining: Vec<usize> = candidates
                                .iter()
                                .copied()
                                .filter(|i| fields[*i].name_chars.get(*progress) == Some(&c))
                                .collect();
                            if remaining.is_empty() {
                                return Err(err());
                            }
                            *candidates = remaining;
                            *progress += 1;
                            Step::Consumed
                        }
                    }
                    ObjectStage::AfterKey => {
                        if space {
                            Step::Consumed
                        } else if c == ':' {
                            *stage = ObjectStage::AfterColon;
                            Step::Consumed
                        } else {
                            return Err(err());
                        }
                    }
                    ObjectStage::AfterColon => {
                        if space {
                            Step::Consumed
                        } else {
                            to_push = Some(Frame::Value {
                                node: fields[*active].node,
                            });
                            *stage = ObjectStage::InValue;
                            Step::Retry
                        }
                    }
                    ObjectStage::InValue => unreachable!("the value frame must be on top"),
                    ObjectStage::AfterValue => {
                        if space {
                            Step::Consumed
                        } else if c == ',' && has_remaining {
                            // With no field left to write, a comma leads into a
                            // dead path; it is closed here so it never opens in
                            // the mask either.
                            *stage = ObjectStage::AfterComma;
                            Step::Consumed
                        } else if c == '}' && required_done {
                            Step::ConsumedAndDone
                        } else {
                            return Err(err());
                        }
                    }
                }
            }

            // --- array ---
            Frame::Array { node, count, stage } => {
                let Node::Array { item, min, max } = &grammar.nodes[*node] else {
                    unreachable!("an array frame cannot be opened on a non-array node")
                };
                let low = min.unwrap_or(0);
                // `ARRAY_CEILING` stands in for a missing `max` HERE AND IN
                // `frame_allowed` through the same function, so the mask cannot
                // offer the `,` the automaton refuses.
                let high = array_limit(*min, *max);
                match *stage {
                    ArrayStage::Start => {
                        if space {
                            Step::Consumed
                        } else if c == ']' && low == 0 {
                            Step::ConsumedAndDone
                        } else if high == 0 {
                            return Err(err());
                        } else {
                            to_push = Some(Frame::Value { node: *item });
                            *stage = ArrayStage::InItem;
                            Step::Retry
                        }
                    }
                    ArrayStage::InItem => unreachable!("the item frame must be on top"),
                    ArrayStage::AfterItem => {
                        if space {
                            Step::Consumed
                        } else if c == ',' && *count < high {
                            *stage = ArrayStage::BeforeItem;
                            Step::Consumed
                        } else if c == ']' && *count >= low {
                            Step::ConsumedAndDone
                        } else {
                            return Err(err());
                        }
                    }
                    ArrayStage::BeforeItem => {
                        if space {
                            Step::Consumed
                        } else {
                            to_push = Some(Frame::Value { node: *item });
                            *stage = ArrayStage::InItem;
                            Step::Retry
                        }
                    }
                }
            }

            // --- free text ---
            Frame::Text {
                node,
                length,
                stage,
            } => {
                let Node::Text { max_length } = &grammar.nodes[*node] else {
                    unreachable!("a text frame cannot be opened on a non-text node")
                };
                // NO BOUND IN THE SCHEMA STILL MEANS A BOUND. See
                // `FREE_TEXT_CEILING`: an unbounded body is a non-terminating
                // path, and this is the one place that decides a body is full.
                let full = max_length.map_or(*length >= FREE_TEXT_CEILING, |u| *length >= u);
                match *stage {
                    StringStage::Body => {
                        if c == '"' {
                            Step::ConsumedAndDone
                        } else if full {
                            // At the length limit the only exit is the closing quote.
                            return Err(err());
                        } else if c == '\\' {
                            *stage = StringStage::Escape;
                            Step::Consumed
                        } else if c.is_control() {
                            // JSON forbids a control character unescaped.
                            return Err(err());
                        } else {
                            *length += 1;
                            Step::Consumed
                        }
                    }
                    StringStage::Escape => {
                        if c == 'u' {
                            *stage = StringStage::Hex {
                                value: 0,
                                left: 4,
                                low_half: false,
                            };
                            Step::Consumed
                        } else if ESCAPES.contains(&c) {
                            // An escape sequence counts as ONE character: the
                            // length limit must measure the meaningful length of
                            // the produced text.
                            *length += 1;
                            *stage = StringStage::Body;
                            Step::Consumed
                        } else {
                            return Err(err());
                        }
                    }
                    // Mid-pair. The length limit is NOT consulted here: it was
                    // consulted before the `\` that opened the pair, and a pair
                    // half-written is not a place the model may be stranded.
                    StringStage::LowEscape => {
                        if c == '\\' {
                            *stage = StringStage::LowU;
                            Step::Consumed
                        } else {
                            return Err(err());
                        }
                    }
                    StringStage::LowU => {
                        if c == 'u' {
                            *stage = StringStage::Hex {
                                value: 0,
                                left: 4,
                                low_half: true,
                            };
                            Step::Consumed
                        } else {
                            return Err(err());
                        }
                    }
                    StringStage::Hex {
                        value,
                        left,
                        low_half,
                    } => {
                        let Some(d) = c.to_digit(16) else {
                            return Err(err());
                        };
                        let value = (value << 4) | d;
                        let left = left - 1;
                        if !hex_fits(value, left, low_half) {
                            return Err(err());
                        }
                        if left > 0 {
                            *stage = StringStage::Hex {
                                value,
                                left,
                                low_half,
                            };
                        } else if (0xD800..=0xDBFF).contains(&value) {
                            // A high surrogate is not a character yet, only the
                            // first half of one: `length` does not move and the
                            // string cannot close until the low half lands.
                            *stage = StringStage::LowEscape;
                        } else {
                            // A whole surrogate PAIR counts as one character, the
                            // same way `serde_json` decodes it and the same way
                            // `ArgSchema::validate` counts it (`chars().count()`).
                            *length += 1;
                            *stage = StringStage::Body;
                        }
                        Step::Consumed
                    }
                }
            }

            // --- closed value set ---
            Frame::Choice {
                node,
                candidates,
                progress,
            } => {
                let Node::Choice { choices } = &grammar.nodes[*node] else {
                    unreachable!("a choice frame cannot be opened on a non-choice node")
                };
                // As for a key: `"` closes only where a value is complete, and
                // is otherwise the tail of an escaped `\"` (compile.rs
                // `json_body`). Before values were compiled to their JSON
                // spelling this branch refused every `"` that did not close,
                // which is why the choice `a"b` stranded the model.
                if c == '"' && candidates.iter().any(|i| choices[*i].len() == *progress) {
                    Step::ConsumedAndDone
                } else {
                    let remaining: Vec<usize> = candidates
                        .iter()
                        .copied()
                        .filter(|i| choices[*i].get(*progress) == Some(&c))
                        .collect();
                    if remaining.is_empty() {
                        return Err(err());
                    }
                    *candidates = remaining;
                    *progress += 1;
                    Step::Consumed
                }
            }

            // --- number ---
            Frame::Number {
                node,
                stage,
                buffer,
            } => {
                let Node::Number {
                    is_integer,
                    min,
                    max,
                } = &grammar.nodes[*node]
                else {
                    unreachable!("a number frame cannot be opened on a non-number node")
                };
                let fractional = !*is_integer;
                // Transition table + prefix-based range pruning; both are SHARED
                // with `frame_allowed`, so the allowed set cannot be wider than
                // what the automaton actually accepts.
                let next = number_transition(*stage, c, fractional, *min).filter(|s| {
                    let mut candidate = buffer.clone();
                    candidate.push(c);
                    range_fits(&candidate, *s, *is_integer, *min, *max)
                });
                match next {
                    Some(s) => {
                        *stage = s;
                        buffer.push(c);
                        Step::Consumed
                    }
                    None if stage.can_finish() => {
                        // The value the tool layer will see, from its own parse
                        // — see `range_fits` on why not ours.
                        let value =
                            validator_value(buffer, *is_integer, min.is_some() || max.is_some())
                                .ok_or_else(|| GrammarError::OutOfRange(buffer.clone()))?;
                        if min.is_some_and(|v| value < v) || max.is_some_and(|v| value > v) {
                            return Err(GrammarError::OutOfRange(buffer.clone()));
                        }
                        Step::DoneNotConsumed
                    }
                    None => return Err(err()),
                }
            }

            // --- true / false ---
            Frame::Literal {
                candidates,
                progress,
            } => {
                let remaining: Vec<usize> = candidates
                    .iter()
                    .copied()
                    .filter(|i| LITERALS[*i].chars().nth(*progress) == Some(c))
                    .collect();
                if remaining.is_empty() {
                    return Err(err());
                }
                *progress += 1;
                let i = *progress;
                // "true"/"false" share no common prefix: an exact match is a
                // definite end.
                if remaining.iter().any(|r| LITERALS[*r].chars().count() == i) {
                    Step::ConsumedAndDone
                } else {
                    *candidates = remaining;
                    Step::Consumed
                }
            }
        };

        if let Some(new_frame) = to_push {
            self.stack.push(new_frame);
        }
        Ok(step)
    }

    // ---- allowed set ----

    /// Adds the characters frame `idx` allows into `set`.
    /// Returns: can this frame finish without consuming (should the parent frame
    /// be looked at too).
    fn frame_allowed(&self, idx: usize, child_done: bool, set: &mut AllowedSet) -> bool {
        let grammar = &*self.grammar;
        match &self.stack[idx] {
            Frame::Value { node } => {
                set.open_space();
                self.value_start(*node, set);
                false
            }

            Frame::Object {
                node,
                seen,
                stage,
                candidates,
                progress,
                active,
            } => {
                let Node::Object { fields } = &grammar.nodes[*node] else {
                    return false;
                };
                let has_remaining = seen.iter().any(|s| !s);
                let required_done = fields
                    .iter()
                    .zip(seen.iter())
                    .all(|(f, s)| !f.required || *s);
                // If the child closed, the frame is now "after value".
                let effective = if child_done && *stage == ObjectStage::InValue {
                    ObjectStage::AfterValue
                } else {
                    *stage
                };
                match effective {
                    ObjectStage::KeyOrClose | ObjectStage::AfterComma => {
                        set.open_space();
                        if effective == ObjectStage::KeyOrClose && required_done {
                            set.add('}');
                        }
                        if has_remaining {
                            set.add('"');
                        }
                    }
                    ObjectStage::InKey => {
                        for i in candidates {
                            match fields[*i].name_chars.get(*progress) {
                                Some(c) => set.add(*c),
                                None => set.add('"'),
                            }
                        }
                    }
                    ObjectStage::AfterKey => {
                        set.open_space();
                        set.add(':');
                    }
                    ObjectStage::AfterColon => {
                        set.open_space();
                        self.value_start(fields[*active].node, set);
                    }
                    ObjectStage::InValue => {}
                    ObjectStage::AfterValue => {
                        set.open_space();
                        if has_remaining {
                            set.add(',');
                        }
                        if required_done {
                            set.add('}');
                        }
                    }
                }
                false
            }

            Frame::Array { node, count, stage } => {
                let Node::Array { item, min, max } = &grammar.nodes[*node] else {
                    return false;
                };
                let low = min.unwrap_or(0);
                let high = array_limit(*min, *max);
                let (effective, total) = if child_done && *stage == ArrayStage::InItem {
                    (ArrayStage::AfterItem, count + 1)
                } else {
                    (*stage, *count)
                };
                set.open_space();
                match effective {
                    ArrayStage::Start => {
                        if low == 0 {
                            set.add(']');
                        }
                        if high > 0 {
                            self.value_start(*item, set);
                        }
                    }
                    ArrayStage::BeforeItem => self.value_start(*item, set),
                    ArrayStage::InItem => {}
                    ArrayStage::AfterItem => {
                        if total < high {
                            set.add(',');
                        }
                        if total >= low {
                            set.add(']');
                        }
                    }
                }
                false
            }

            Frame::Text {
                node,
                length,
                stage,
            } => {
                let Node::Text { max_length } = &grammar.nodes[*node] else {
                    return false;
                };
                match *stage {
                    StringStage::Body => {
                        set.add('"');
                        if max_length.map_or(*length < FREE_TEXT_CEILING, |u| *length < u) {
                            set.add('\\');
                            set.open_text_body();
                        }
                    }
                    StringStage::Escape => {
                        set.add_all(ESCAPES);
                        set.add('u');
                    }
                    // Owing the low half of a surrogate pair: the string may not
                    // close, and nothing but the pair's own `\u` opens.
                    StringStage::LowEscape => set.add('\\'),
                    StringStage::LowU => set.add('u'),
                    StringStage::Hex {
                        value,
                        left,
                        low_half,
                    } => {
                        // The hex digits are filtered by the SAME `hex_fits` the
                        // automaton uses, for the reason `number_transition`
                        // states: two copies of the rule drift, and a wider
                        // allowed set means the mask opens a dead path.
                        for c in ('0'..='9').chain('a'..='f').chain('A'..='F') {
                            let d = c.to_digit(16).expect("hex digit");
                            if hex_fits((value << 4) | d, left - 1, low_half) {
                                set.add(c);
                            }
                        }
                    }
                }
                false
            }

            Frame::Choice {
                node,
                candidates,
                progress,
            } => {
                let Node::Choice { choices } = &grammar.nodes[*node] else {
                    return false;
                };
                for i in candidates {
                    match choices[*i].get(*progress) {
                        Some(c) => set.add(*c),
                        None => set.add('"'),
                    }
                }
                false
            }

            Frame::Number {
                node,
                stage,
                buffer,
            } => {
                let Node::Number {
                    is_integer,
                    min,
                    max,
                } = &grammar.nodes[*node]
                else {
                    return false;
                };
                let fractional = !*is_integer;
                // The allowed set is produced by asking the automaton ITSELF; no
                // second copy of the rules is kept (no drift risk). There are 16
                // candidates, so the cost is constant.
                for c in NUMBER_CANDIDATES {
                    if let Some(s) = number_transition(*stage, c, fractional, *min) {
                        let mut candidate = buffer.clone();
                        candidate.push(c);
                        if range_fits(&candidate, s, *is_integer, *min, *max) {
                            set.add(c);
                        }
                    }
                }
                // A number can close without consuming: the parent frame has a
                // say too. But first make sure the value really is inside the
                // range — if not, this is not an ending point.
                stage.can_finish()
                    && validator_value(buffer, *is_integer, min.is_some() || max.is_some())
                        .is_some_and(|v| min.is_none_or(|x| v >= x) && max.is_none_or(|x| v <= x))
            }

            Frame::Literal {
                candidates,
                progress,
            } => {
                for i in candidates {
                    if let Some(c) = LITERALS[*i].chars().nth(*progress) {
                        set.add(c);
                    }
                }
                false
            }
        }
    }

    /// The characters a value may start with.
    fn value_start(&self, node: usize, set: &mut AllowedSet) {
        match &self.grammar.nodes[node] {
            Node::Object { .. } => set.add('{'),
            Node::Array { .. } => set.add('['),
            Node::Text { .. } | Node::Choice { .. } => set.add('"'),
            // The FIRST character of a number goes through range pruning too:
            // in the range `[1,50]` starting with `0` is a dead path and must
            // never open.
            Node::Number {
                is_integer,
                min,
                max,
            } => {
                for c in NUMBER_CANDIDATES {
                    if let Some(s) = number_transition(NumberStage::Start, c, !*is_integer, *min)
                        && range_fits(&c.to_string(), s, *is_integer, *min, *max)
                    {
                        set.add(c);
                    }
                }
            }
            Node::Bool => set.add_all(['t', 'f']),
        }
    }
}
