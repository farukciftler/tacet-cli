//! Constrainer — the gate that FORCES generation into a grammar.
//!
//! DEPENDENCY DIRECTION: tacet-engine DOES NOT DEPEND on tacet-grammar. The
//! contract lives here, tacet-grammar IMPLEMENTS it (grammar -> engine, not
//! engine -> grammar). The reason: the engine does not produce the answer to
//! "which token is forbidden", it only asks. With the dependency the other way
//! round the engine would have to know the grammar's internal representation
//! (DFA, stack, token mask bit vector) and would change every time the grammar
//! did. It also means an engine running unconstrained (free-text turn) never
//! compiles the grammar code at all.
//!
//! TWO LAYERS, because a grammar is INCREMENTAL: `Constrainer` is the shareable,
//! immutable definition that lives for the whole generation (`&self`, `Sync`);
//! `ConstraintSession` is the advancing state of ONE generation. With a single
//! layer either the state would be recomputed from scratch at every token
//! (quadratic cost in length) or the definition itself would be mutable, making
//! it impossible to share one constraint across two generations.

use crate::error::EngineError;

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
    fn advance(&mut self, token: u32) -> Result<(), EngineError>;

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
    fn advance(&mut self, _token: u32) -> Result<(), EngineError> {
        Ok(())
    }
    fn is_done(&self) -> bool {
        false
    }
}
