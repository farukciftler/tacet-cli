//! `ArgSchema` -> `Grammar` compilation.
//!
//! WHY A SEPARATE "COMPILED" FORM: the automaton state (`GrammarState`) is
//! cloned THOUSANDS of times during token masking — every branch tried in the
//! vocabulary trie is a clone. If the state held an `ArgSchema` directly, every
//! clone would be a deep tree copy. So the schema is flattened once into an
//! arena (Vec<Node>) and the state carries only `usize` indices; `Grammar` is
//! shared via Arc and never cloned.
//!
//! Second reason: string comparisons are done per char (key and enum prefix
//! matching). Producing the char vectors from `str` at every step is waste on
//! the hot path; they are produced once at compile time.

use std::sync::Arc;
use tacet_core::{ArgSchema, Field, SchemaKind};

/// A compiled object field.
#[derive(Debug, Clone)]
pub(crate) struct CompiledField {
    /// Key matching advances per char; the chars are produced up front.
    /// The field name is stored ONLY in this form: the automaton never does a
    /// `str` comparison anywhere, so keeping a second copy would be dead data.
    pub(crate) name_chars: Vec<char>,
    pub(crate) node: usize,
    pub(crate) required: bool,
}

/// A single schema node in the arena.
#[derive(Debug, Clone)]
pub(crate) enum Node {
    Object {
        fields: Vec<CompiledField>,
    },
    Array {
        item: usize,
        min: Option<usize>,
        max: Option<usize>,
    },
    Text {
        max_length: Option<usize>,
    },
    Choice {
        choices: Vec<Vec<char>>,
    },
    Number {
        is_integer: bool,
        min: Option<f64>,
        max: Option<f64>,
    },
    Bool,
}

/// A compiled, immutable grammar. Produced once per schema and shared with
/// `Arc` for the whole generation.
#[derive(Debug)]
pub struct Grammar {
    pub(crate) nodes: Vec<Node>,
    pub(crate) root: usize,
}

impl Grammar {
    /// Flattens the schema into the arena.
    pub fn compile(schema: &ArgSchema) -> Arc<Grammar> {
        let mut nodes = Vec::new();
        let root = add(&mut nodes, schema);
        Arc::new(Grammar { nodes, root })
    }

    /// Opens a fresh generation state for this grammar.
    pub fn state(self: &Arc<Self>) -> crate::GrammarState {
        crate::GrammarState::new(Arc::clone(self))
    }
}

fn add(arena: &mut Vec<Node>, schema: &ArgSchema) -> usize {
    // Children are added first, the node itself afterwards. That keeps the
    // indices stable and the arena one-directional (acyclic) — the schema is a
    // tree already.
    let node = match &schema.kind {
        SchemaKind::Object { fields } => {
            let compiled: Vec<CompiledField> = fields
                .iter()
                .map(|f: &Field| CompiledField {
                    name_chars: f.name.chars().collect(),
                    node: add(arena, &f.schema),
                    required: f.required,
                })
                .collect();
            Node::Object { fields: compiled }
        }
        SchemaKind::Array { item, min, max } => Node::Array {
            item: add(arena, item),
            min: *min,
            max: *max,
        },
        SchemaKind::Text { max_length } => Node::Text { max_length: *max_length },
        // Choice values are embedded in the grammar LITERALLY: the model cannot
        // step outside the set. A choice containing a character that needs
        // escaping (quote, backslash) becomes impossible to produce; that is a
        // deliberate narrowing — enum values should be identifiers/labels, not
        // free text.
        SchemaKind::Choice { choices } => Node::Choice {
            choices: choices.iter().map(|c| c.chars().collect()).collect(),
        },
        SchemaKind::Number { is_integer, min, max } => Node::Number {
            is_integer: *is_integer,
            min: *min,
            max: *max,
        },
        SchemaKind::Bool => Node::Bool,
    };
    arena.push(node);
    arena.len() - 1
}
