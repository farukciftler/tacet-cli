//! WHATEVER THE MASK CLOSES, THE AUTOMATON MUST REFUSE — as a property, over
//! every token in a vocabulary, at every prefix that leads to a call.
//!
//! The two halves of this crate answer the same question in two places.
//! `CallSession::mask` decides, before sampling, which tokens are impossible;
//! `CallSession::advance` decides, after sampling, whether the token that
//! arrived was legal. They are separate code paths reading the same text, and
//! when they disagree the failure is silent in one direction and fatal in the
//! other:
//!
//! * the mask closes a token the automaton would have accepted — a legal
//!   continuation is simply unreachable, and nothing anywhere says so;
//! * the mask leaves open a token the automaton refuses — the turn dies with
//!   `ConstraintError::Violation`, which the header on `advance` calls a logic
//!   error precisely because it means these two have drifted apart.
//!
//! THEY HAD DRIFTED. `swallow` asked `align_prefix(queue)` on seeing `(`, while
//! `prefix_mask` asked `format!("{matched}{before}").trim()` — and the trim
//! disagreed at both ends. Trailing: queue `…create_document`, token ` (` — the
//! trim turned `create_document ` back into a name and the mask closed a token
//! that was never a call. Leading: queue `hello`, token ` time(` — the boundary
//! was inside the token, the queue alone could not see it, and the mask left
//! open a token that starts a call it had not checked.
//!
//! The examples below are the shapes that broke it; the walk is what stops the
//! next one, because nobody thinks to write the example that has not happened.

use std::sync::Arc;
use tacet_grammar::CallConstraint;
use tacet_kernel::{ArgSchema, Constrainer, Field, Tool, ToolCatalog, ToolContext, ToolFuture};

// --- A catalog with the two names the shapes below need ---

struct Named(&'static str, ArgSchema);
impl Tool for Named {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "A tool."
    }
    fn schema(&self) -> ArgSchema {
        self.1.clone()
    }
    fn run<'a>(&'a self, _a: serde_json::Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
        unreachable!("this catalog is never executed, only compiled")
    }
}

fn catalog() -> ToolCatalog {
    let mut catalog = ToolCatalog::new();
    catalog.add(Arc::new(Named(
        "create_document",
        ArgSchema::object(vec![Field::new("path", ArgSchema::text()).required()]),
    )));
    catalog.add(Arc::new(Named(
        "time",
        ArgSchema::object(vec![
            Field::new("kind", ArgSchema::choice(["clock", "date"])).required(),
        ]),
    )));
    catalog
}

/// The tokens. A real vocabulary is thousands of subwords, but only the ones
/// carrying a `(` can reach `prefix_mask` at all, and the disagreement lives
/// entirely in how the text AROUND that `(` is read. So the list is built to be
/// adversarial about exactly that: every combination of leading whitespace,
/// trailing whitespace, a split name, and a remainder that does or does not
/// conform.
fn vocab() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for base in [
        "(",
        " (",
        "( ",
        " ( ",
        "({",
        "({\"",
        "(\"",
        " time(",
        " time(\"",
        "time(",
        "ulate(",
        " create_document(",
        "create_document(",
        "_document(",
        "create_document (",
        "(\"path",
        "({\"path\":",
        "()",
        " ()",
        "(}",
        "({}",
    ] {
        v.push(base.to_string());
    }
    // Ordinary text tokens, so the walk has prefixes to stand on.
    for base in [
        "hello",
        " hello",
        "create",
        "_document",
        "time",
        " time",
        "calc",
        "ulate",
        " ",
        "  ",
        "x",
        "\"",
        "{",
        "}",
        ")",
        "path",
        ":",
        ",",
        "the",
        " the",
        "1",
        "\n",
    ] {
        v.push(base.to_string());
    }
    v
}

/// Replays `prefix` on a fresh session, then reports whether `candidate` is
/// closed by the mask and whether the automaton accepts it.
fn probe(
    vocab: &[String],
    constraint: &CallConstraint,
    prefix: &[u32],
    candidate: u32,
) -> (bool, bool) {
    let mut session = constraint.session();
    for t in prefix {
        session
            .advance(*t)
            .unwrap_or_else(|e| panic!("the prefix itself was refused: {e:?}"));
    }
    let mut logits = vec![0.0f32; vocab.len()];
    session.mask(&mut logits);
    let closed = logits[candidate as usize] == f32::NEG_INFINITY;
    let accepted = session.advance(candidate).is_ok();
    (closed, accepted)
}

/// THE PROPERTY. Over every prefix of every text below and every token in the
/// vocabulary: a token the mask closes must be one the automaton refuses.
///
/// The converse is NOT asserted — the prefix stage deliberately leaves free text
/// open, so there are many tokens the automaton accepts and the mask does not
/// need to close. This direction is the one that costs something when it fails.
#[test]
fn a_token_the_mask_closes_is_a_token_the_automaton_refuses() {
    let vocab = vocab();
    let constraint = CallConstraint::new(&vocab, &catalog());

    // The prefixes are built as TOKEN SEQUENCES, because that is what the
    // session sees; writing them as text would hide the tokenizer boundary,
    // which is where both defects lived.
    let index = |text: &str| -> u32 {
        vocab
            .iter()
            .position(|t| t == text)
            .unwrap_or_else(|| panic!("`{text}` is not in the test vocabulary")) as u32
    };
    let prefixes: Vec<Vec<u32>> = vec![
        vec![],
        vec![index("hello")],
        vec![index("hello"), index(" ")],
        vec![index("create"), index("_document")],
        vec![
            index("hello"),
            index(" "),
            index("create"),
            index("_document"),
        ],
        vec![index("calc")],
        vec![index("time")],
        vec![index(" time")],
        vec![index("the"), index(" time")],
        vec![index("hello"), index("  ")],
        vec![index("x"), index("\n")],
    ];

    let mut closed_seen = 0usize;
    for prefix in &prefixes {
        for candidate in 0..vocab.len() as u32 {
            let (closed, accepted) = probe(&vocab, &constraint, prefix, candidate);
            if closed {
                closed_seen += 1;
            }
            assert!(
                !(closed && accepted),
                "prefix {:?} + token `{}`: the mask closed it and the automaton took it — \
                 a legal continuation the model can never reach",
                prefix
                    .iter()
                    .map(|t| vocab[*t as usize].as_str())
                    .collect::<Vec<_>>(),
                vocab[candidate as usize]
            );
        }
    }
    // A property that closes nothing proves nothing.
    assert!(
        closed_seen > 0,
        "the mask closed no token anywhere in the walk — it is no longer masking"
    );
}

/// THE TRAILING HALF, as a named example. `create_document ` is not a name.
#[test]
fn a_name_followed_by_a_space_is_not_a_call() {
    let vocab = vocab();
    let constraint = CallConstraint::new(&vocab, &catalog());
    let index = |text: &str| vocab.iter().position(|t| t == text).expect("in vocab") as u32;

    // ` ()` and not ` (`: the token has to carry a remainder that does NOT
    // conform, or the mask has no reason to close it either way and the example
    // measures nothing. `create_document` requires `path`, so `()` is not a
    // legal call — but `create_document ()` was never a call to begin with, and
    // that is the point.
    let prefix = vec![index("create"), index("_document")];
    let (closed, accepted) = probe(&vocab, &constraint, &prefix, index(" ()"));
    assert!(
        accepted,
        "` ()` after a name is ordinary prose to the automaton"
    );
    assert!(
        !closed,
        "the mask closed ` ()` — it read `create_document ` as a name, the automaton did not"
    );
}

/// THE LEADING HALF. The word boundary is INSIDE the token, where the queue
/// cannot see it; the mask has to look at the token's own text to find it.
#[test]
fn a_boundary_inside_the_token_still_starts_a_call() {
    let vocab = vocab();
    let constraint = CallConstraint::new(&vocab, &catalog());
    let index = |text: &str| vocab.iter().position(|t| t == text).expect("in vocab") as u32;

    // `hello` ends in a word character, so nothing can start at the end of the
    // queue — but ` time(` carries its own boundary and the automaton opens the
    // arguments on it.
    let prefix = vec![index("hello")];
    let mut session = constraint.session();
    session.advance(prefix[0]).expect("prose");
    session.advance(index(" time(")).expect("the call opens");
    assert!(
        session.is_structural(),
        "` time(` after `hello` starts a call; the automaton says so"
    );

    // AND SINCE IT DOES, THE MASK OWES AN OPINION ON THE REMAINDER. `time`
    // needs `{` after `(`, so ` time("` opens the arguments and immediately
    // breaks them: the automaton refuses the `"` and the turn dies with
    // `ConstraintError::Violation`. Left open, that is the fault this whole
    // mask exists to prevent — and it WAS left open, because the mask decided
    // there was nothing to check from the queue alone.
    let (closed, accepted) = probe(&vocab, &constraint, &prefix, index(" time(\""));
    assert!(
        !accepted,
        "the automaton must refuse `\"` where the grammar wants `{{`"
    );
    assert!(
        closed,
        "the mask left ` time(\"` open after `hello`: a token the automaton \
         refuses is a turn that dies"
    );

    // The same token with a conforming remainder stays open — the rule is about
    // the remainder, not about punishing tokens that carry a boundary.
    let (closed, accepted) = probe(&vocab, &constraint, &prefix, index(" time("));
    assert!(accepted, "the automaton takes it");
    assert!(
        !closed,
        "and `(` alone conforms, so the mask must not close it"
    );
}
