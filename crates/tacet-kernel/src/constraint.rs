//! Constrainer — the gate that FORCES generation into a grammar.
//!
//! WHY IT LIVES IN THE KERNEL rather than beside an inference engine. The whole
//! contract is three signatures over `&mut [f32]` and `u32`:
//!
//! ```text
//! fn session(&self) -> Box<dyn ConstraintSession>;
//! fn mask(&self, logits: &mut [f32]);
//! fn advance(&mut self, token: u32) -> Result<(), ConstraintError>;
//! ```
//!
//! Nothing in it names a model, a tokenizer, a device or a file. Any runtime
//! that can hand over a logit slice and take back a token can implement it —
//! llama.cpp through a binding, ONNX, a hand-written loop — and get the same
//! guarantee, which is that a call the schema forbids cannot be produced at
//! all rather than caught afterwards.
//!
//! IT USED TO LIVE IN `tacet-engine`, and that placement quietly made the
//! guarantee look like a property of this project's engine. It was not: it
//! forced anyone who wanted constrained generation to also take a crate full of
//! prompt budgeting, GGUF loading and candle. Moving it here costs nothing —
//! `tacet-kernel` depends on serde and thiserror and nothing else — and turns
//! `tacet-grammar` into a component with no inference dependency at all.
//!
//! TWO LAYERS, because a grammar is INCREMENTAL: `Constrainer` is the shareable,
//! immutable definition that lives for the whole generation (`&self`, `Sync`);
//! `ConstraintSession` is the advancing state of ONE generation. With a single
//! layer either the state would be recomputed from scratch at every token
//! (quadratic cost in length) or the definition itself would be mutable, making
//! it impossible to share one constraint across two generations.
//!
//! DEPENDENCY DIRECTION: nothing here depends on a grammar. The contract lives
//! in the kernel, an implementation lives elsewhere (`tacet-grammar`). With the
//! dependency the other way round the engine would have to know the grammar's
//! internal representation — DFA, stack, token mask bit vector — and would
//! change every time the grammar did.

use thiserror::Error;

/// The only thing a constraint can go wrong about.
///
/// DELIBERATELY ONE VARIANT. This used to be `EngineError`, which also carries
/// `ModelNotLoaded(PathBuf)`, `Tokenization`, `BudgetExceeded` and
/// `ScriptExhausted` — none of which a grammar can produce or handle. An
/// implementor of this trait had to depend on, and match against, four failure
/// modes that were never theirs.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ConstraintError {
    /// The constrainer rejected the incoming token despite the mask. This is a
    /// logic error — masking and advancing have drifted apart — and must not be
    /// swallowed silently.
    #[error("constraint rejected the token: {0}")]
    Violation(u32),
}

/// The definition of the grammar constraining a generation. Shared, immutable.
pub trait Constrainer: Send + Sync {
    /// Opens fresh state for a new generation.
    fn session(&self) -> Box<dyn ConstraintSession>;

    /// Short name for diagnostics/logs ("tool_call", "json").
    fn name(&self) -> &str {
        "constraint"
    }
}

/// The advancing constraint state of a single generation. Lives with the
/// generation loop.
pub trait ConstraintSession: Send {
    /// Sets the logit of every token FORBIDDEN in the current state to
    /// `f32::NEG_INFINITY`. The sampler runs after this, so the constraint
    /// cannot be pierced whichever sampling strategy is chosen (greedy, top-p,
    /// temperature).
    ///
    /// The `logits` slice is as long as the ENTIRE vocabulary; index = token id.
    fn mask(&self, logits: &mut [f32]);

    /// Folds the chosen token into the state. If masking worked correctly no
    /// forbidden token ever arrives here; it can still return an error, because
    /// call sites that bypass the mask (sampler bug, hand-fed token) must not
    /// lead to a silently wrong acceptance.
    fn advance(&mut self, token: u32) -> Result<(), ConstraintError>;

    /// Is the grammar in an accepting state — generation can be cut SAFELY here.
    fn is_done(&self) -> bool;

    /// Is generation currently inside a STRUCTURAL region (the arguments of a
    /// tool call, i.e. where the grammar actually forces things)?
    ///
    /// WHY THIS EXISTS (measured, write_code/Qwen3-8B): the repeat penalty
    /// exists for the PROSE loop, but code and JSON are REPETITIVE BY NATURE —
    /// indentation, newlines, a second occurrence of the same identifier. Under
    /// greedy sampling a penalty of 1.15 turned the second spelling of
    /// `asal_sayi_kontrol` into `asal_say_kontrol` and both verification
    /// attempts then failed with a syntax error. The loop detector is already
    /// skipped while a constraint is active ("valid JSON may be repetitive");
    /// the same reasoning applies to the penalty — but only INSIDE THE
    /// ARGUMENTS. In the free-text region the penalty stays; removing it there
    /// would bring the prose loop bug back.
    ///
    /// Defaults to `false`: constraints with no notion of a structural region
    /// (Free) do not affect the penalty.
    fn is_structural(&self) -> bool {
        false
    }
}

/// An implementation that constrains nothing.
///
/// Its reason to exist is not to simplify `Option<&dyn Constrainer>` (that
/// already works); it is to run the free-text turn through THE SAME code path
/// as the constrained turn. With two separate loops, sampling/stopping bugs
/// would only ever get fixed in one of them.
pub struct FreeConstraint;

impl Constrainer for FreeConstraint {
    fn session(&self) -> Box<dyn ConstraintSession> {
        Box::new(FreeSession)
    }
    fn name(&self) -> &str {
        "free"
    }
}

struct FreeSession;

impl ConstraintSession for FreeSession {
    fn mask(&self, _logits: &mut [f32]) {}
    fn advance(&mut self, _token: u32) -> Result<(), ConstraintError> {
        Ok(())
    }
    fn is_done(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE CONTRACT NAMES NOTHING FROM AN INFERENCE STACK, and this test is how
    /// that stays true. It implements a constraint with no dependency beyond
    /// this module — no model, no tokenizer, no device — and drives it. If a
    /// future signature starts requiring one, this file stops compiling.
    #[test]
    fn a_constraint_can_be_implemented_with_nothing_but_this_module() {
        /// Allows only token 7, once.
        struct OnlySeven;
        struct OnlySevenSession {
            done: bool,
        }

        impl Constrainer for OnlySeven {
            fn session(&self) -> Box<dyn ConstraintSession> {
                Box::new(OnlySevenSession { done: false })
            }
        }
        impl ConstraintSession for OnlySevenSession {
            fn mask(&self, logits: &mut [f32]) {
                for (id, logit) in logits.iter_mut().enumerate() {
                    if id != 7 {
                        *logit = f32::NEG_INFINITY;
                    }
                }
            }
            fn advance(&mut self, token: u32) -> Result<(), ConstraintError> {
                if token != 7 {
                    return Err(ConstraintError::Violation(token));
                }
                self.done = true;
                Ok(())
            }
            fn is_done(&self) -> bool {
                self.done
            }
        }

        let mut session = OnlySeven.session();
        let mut logits = vec![1.0f32; 16];
        session.mask(&mut logits);
        assert_eq!(logits[7], 1.0, "the allowed token keeps its logit");
        assert!(logits[3].is_infinite(), "a forbidden token is closed");

        assert!(!session.is_done());
        assert_eq!(session.advance(3), Err(ConstraintError::Violation(3)));
        assert!(session.advance(7).is_ok());
        assert!(
            session.is_done(),
            "the grammar accepts and generation may stop"
        );
    }

    /// The free constraint closes nothing and accepts everything — the free-text
    /// turn runs through the same loop as the constrained one.
    #[test]
    fn the_free_constraint_masks_nothing() {
        let mut session = FreeConstraint.session();
        let mut logits = vec![0.5f32; 8];
        session.mask(&mut logits);
        assert!(logits.iter().all(|l| *l == 0.5));
        assert!(session.advance(1234).is_ok());
        assert!(
            !session.is_done(),
            "free text is never structurally finished"
        );
    }
}
