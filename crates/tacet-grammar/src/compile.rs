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
use tacet_kernel::{ArgSchema, Field, SchemaKind};

/// A compiled object field.
#[derive(Debug, Clone)]
pub(crate) struct CompiledField {
    /// Key matching advances per char; the chars are produced up front, and
    /// they are the JSON SPELLING of the name (`json_body`), not the name — the
    /// automaton matches what is written, `serde_json` decodes it back.
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
        let (root, _) = add(&mut nodes, schema);
        Arc::new(Grammar { nodes, root })
    }

    /// Opens a fresh generation state for this grammar.
    pub fn state(self: &Arc<Self>) -> crate::GrammarState {
        crate::GrammarState::new(Arc::clone(self))
    }
}

/// A literal (a key or a closed-set value) as the characters of its JSON string
/// BODY — the exact characters the model has to write between the quotes.
///
/// WHY NOT THE RAW CHARACTERS, which is what this module used to store with a
/// comment calling the result "a deliberate narrowing". It was not a narrowing,
/// it was a disagreement with the parser on the other side. MEASURED (audit
/// M15): the choice `C:\temp` was generated as `"C:\temp"`, which JSON decodes
/// as `C:<TAB>emp`, and `validate` refused it as "invalid choice"; the choice
/// `a"b` was a prefix the automaton entered and could not leave ("the
/// constraint forbade every token"); a key spelled `a\nb` decoded to a newline
/// and the field came back `MissingField`. And the MCP bridge admits all three
/// shapes, so they were reachable from any connected server.
///
/// ONE SPELLING PER LITERAL, and it is `serde_json`'s: `\"`, `\\`, the five
/// short escapes, `\u00XX` (lowercase) for the remaining C0 controls. JSON has
/// other spellings of the same string (`\u0041` for `A`, `\/` for `/`); they are
/// left unproducible, which IS a narrowing and a harmless one — the decoded
/// value is identical, and a single spelling keeps the key and enum tries as
/// narrow as they were.
///
/// EVERY `char::is_control` IS ESCAPED, not only the C0 range JSON requires:
/// DEL and the C1 block are legal raw in JSON, but the free-text frame refuses
/// them raw (state.rs, "JSON forbids a control character unescaped" — the
/// stricter reading), and a literal must not be the one place a raw C1 `CSI`
/// can reach the output.
pub(crate) fn json_body(literal: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(literal.len());
    for c in literal.chars() {
        match c {
            '"' => out.extend(['\\', '"']),
            '\\' => out.extend(['\\', '\\']),
            '\u{8}' => out.extend(['\\', 'b']),
            '\u{c}' => out.extend(['\\', 'f']),
            '\n' => out.extend(['\\', 'n']),
            '\r' => out.extend(['\\', 'r']),
            '\t' => out.extend(['\\', 't']),
            c if c.is_control() => out.extend(format!("\\u{:04x}", c as u32).chars()),
            c => out.push(c),
        }
    }
    out
}

/// Compiles `schema` into the arena and reports whether ANY value satisfies it.
///
/// WHY THE SECOND HALF OF THE ANSWER: a field whose schema nothing satisfies is
/// a key the model can open and never finish. MEASURED (audit M15): an MCP
/// field declared `enum: []` opened its key, and the value position had no
/// first character — the automaton stranded mid-call. `ArgSchema::choice` now
/// refuses an empty set where it is written, but a schema can still be built as
/// a struct literal or deserialized, so the grammar does not take it on trust:
/// an OPTIONAL field nobody can fill is dropped (its key never opens), an array
/// of such items is held at length zero, and a REQUIRED one makes its object
/// unsatisfiable in turn. A root nobody can satisfy is an authoring bug the
/// grammar cannot repair — it still compiles, and the caller's validation of
/// its own schema is the place that catches it.
fn add(arena: &mut Vec<Node>, schema: &ArgSchema) -> (usize, bool) {
    // Children are added first, the node itself afterwards. That keeps the
    // indices stable and the arena one-directional (acyclic) — the schema is a
    // tree already.
    let (node, inhabited) = match &schema.kind {
        SchemaKind::Object { fields } => {
            let mut inhabited = true;
            let mut compiled: Vec<CompiledField> = Vec::with_capacity(fields.len());
            for f in fields {
                let f: &Field = f;
                let (node, fillable) = add(arena, &f.schema);
                if !fillable {
                    if f.required {
                        inhabited = false;
                    } else {
                        continue;
                    }
                }
                compiled.push(CompiledField {
                    name_chars: json_body(&f.name),
                    node,
                    required: f.required,
                });
            }
            (Node::Object { fields: compiled }, inhabited)
        }
        SchemaKind::Array { item, min, max } => {
            let (item, fillable) = add(arena, item);
            // An item nobody can write leaves `[]` as the only array there is.
            let max = if fillable { *max } else { Some(0) };
            let inhabited = max.is_none_or(|hi| min.unwrap_or(0) <= hi);
            (
                Node::Array {
                    item,
                    min: *min,
                    max,
                },
                inhabited,
            )
        }
        SchemaKind::Text { max_length } => (
            Node::Text {
                max_length: *max_length,
            },
            true,
        ),
        // Values are compiled to their JSON spelling — see `json_body`.
        SchemaKind::Choice { choices } => (
            Node::Choice {
                choices: choices.iter().map(|c| json_body(c)).collect(),
            },
            !choices.is_empty(),
        ),
        SchemaKind::Number {
            is_integer,
            min,
            max,
        } => (
            Node::Number {
                is_integer: *is_integer,
                min: *min,
                max: *max,
            },
            true,
        ),
        SchemaKind::Bool => (Node::Bool, true),
    };
    arena.push(node);
    (arena.len() - 1, inhabited)
}
