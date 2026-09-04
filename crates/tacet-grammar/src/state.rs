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
    // OVERFLOW IS CHECKED EVEN WITH NO RANGE AT ALL, and it has to be first.
    // JSON puts no bound on the magnitude of a number, but every consumer here
    // does: `serde_json` answers "number out of range" for anything that
    // overflows f64, and Rust's own `f64` parse returns INFINITY rather than an
    // error, so the close-time check saw nothing wrong. MEASURED, that gap let
    // an unbounded `Number` produce `{"c":1e31212121212121212}` — accepted with
    // `is_done() == true`, and `serde_json::from_str` refused the very same
    // text. The two agree exactly on where the edge is (`1e308` and a
    // 309-digit integer pass, `1e309` and a 310-digit integer do not), so
    // asking Rust's parse for finiteness is not an approximation of the rule,
    // it IS the rule.
    //
    // Pruning it as a PREFIX is sound because both the exponent and the integer
    // part only ever grow: once a prefix is infinite no continuation brings it
    // back, so the digit that tips it over is refused before it is written and
    // the model is never stranded on it. A negative exponent shrinks towards
    // zero and stays finite, which is why underflow needs no rule.
    //
    // COST, since this now runs even for an unbounded number and the masking
    // path is hot: one short `f64` parse, MEASURED at 4ns on this machine, at
    // most once per entry of NUMBER_CANDIDATES — roughly 60ns per allowed-set
    // computation, against 63µs for a single 32k-vocabulary mask step at a
    // number position. It does not show up at that scale.
    if buffer.parse::<f64>().is_ok_and(|v| !v.is_finite()) {
        return false;
    }
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
        // `"20."` parses as 20.0 in Rust, so `Dot` needs no special case.
        let Ok(v) = buffer.parse::<f64>() else {
            return true;
        };
        let digits = buffer.split_once('.').map_or(0, |(_, f)| f.len());
        // Past ~308 fraction digits `10^-d` underflows to 0 and the interval
        // collapses to the point `v`; f64 cannot tell finer prefixes apart
        // either, and the close-time check compares the same f64, so the two
        // agree. The `v >= lo` / `v <= hi` disjunct keeps a prefix that is
        // ALREADY in range alive even then.
        let reach = 10f64.powi(-(digits.min(308) as i32));
        return if negative {
            v >= lo && (v <= hi || v - reach < hi)
        } else {
            v <= hi && (v >= lo || v + reach > lo)
        };
    }
    let body = buffer.trim_start_matches('-');
    let int_part: String = body.chars().take_while(char::is_ascii_digit).collect();
    if int_part.is_empty() {
        return true; // no digit yet (only '-' was written)
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
#[derive(Debug, Clone)]
pub struct GrammarState {
    grammar: Arc<Grammar>,
    stack: Vec<Frame>,
    position: usize,
}

impl GrammarState {
    pub fn new(grammar: Arc<Grammar>) -> Self {
        let root = grammar.root;
        Self {
            grammar,
            stack: vec![Frame::Value { node: root }],
            position: 0,
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
    pub(crate) fn in_free_text_run(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(Frame::Text {
                node,
                stage: StringStage::Body,
                ..
            }) if matches!(self.grammar.nodes[*node], Node::Text { max_length: None })
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
                        if c == '"' {
                            // Pick the exactly matching candidate; otherwise the
                            // key was left half-written.
                            match candidates
                                .iter()
                                .copied()
                                .find(|i| fields[*i].name_chars.len() == *progress)
                            {
                                Some(i) => {
                                    seen[i] = true;
                                    *active = i;
                                    *stage = ObjectStage::AfterKey;
                                    Step::Consumed
                                }
                                None => return Err(err()),
                            }
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
                match *stage {
                    ArrayStage::Start => {
                        if space {
                            Step::Consumed
                        } else if c == ']' && low == 0 {
                            Step::ConsumedAndDone
                        } else if max.is_some_and(|u| u == 0) {
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
                        } else if c == ',' && max.is_none_or(|u| *count < u) {
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
                let full = max_length.is_some_and(|u| *length >= u);
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
                if c == '"' {
                    if candidates.iter().any(|i| choices[*i].len() == *progress) {
                        Step::ConsumedAndDone
                    } else {
                        return Err(err());
                    }
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
                        let value: f64 = buffer
                            .parse()
                            .map_err(|_| GrammarError::OutOfRange(buffer.clone()))?;
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
                        if max.is_none_or(|u| u > 0) {
                            self.value_start(*item, set);
                        }
                    }
                    ArrayStage::BeforeItem => self.value_start(*item, set),
                    ArrayStage::InItem => {}
                    ArrayStage::AfterItem => {
                        if max.is_none_or(|u| total < u) {
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
                        if max_length.is_none_or(|u| *length < u) {
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
                    && buffer
                        .parse::<f64>()
                        .is_ok_and(|v| min.is_none_or(|x| v >= x) && max.is_none_or(|x| v <= x))
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
