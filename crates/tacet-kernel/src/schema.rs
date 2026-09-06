//! `ArgSchema` — the contract for tool arguments.
//!
//! NOT the whole of JSON Schema, only the subset we need. The reason: this type
//! is translated in two directions — tacet-grammar turns it into a constrained
//! generation grammar (the model cannot deviate from the schema), tacet-cli
//! turns it into the tool description inside the prompt. Full JSON Schema
//! (oneOf/allOf/$ref/pattern) is too broad to be translated into a grammar;
//! keeping it small and closed is the precondition for being able to FORCE the
//! model into the schema.
//!
//! Field order is preserved with a `Vec` (not a HashMap): the grammar and the
//! prompt must come out bit-identical on every run, so that eval results stay
//! comparable.

use serde::{Deserialize, Serialize};

/// One argument schema: kind + human description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArgSchema {
    /// What shape the value has. This is what the grammar compiles.
    #[serde(flatten)]
    pub kind: SchemaKind,
    /// The description shown to the model. Must be short and imperative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The shapes an argument may take.
///
/// DELIBERATELY SMALL AND CLOSED — the module header says why: this set is
/// translated into a generation grammar, and a shape that cannot be translated
/// is a shape the model cannot be FORCED into.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SchemaKind {
    /// An object with ordered fields. The root schema of a tool is always this.
    Object {
        /// In declaration order, which is also prompt order and grammar order.
        fields: Vec<Field>,
    },
    /// A homogeneous array.
    Array {
        /// Every element has this schema; arrays here are homogeneous.
        item: Box<ArgSchema>,
        /// Fewest elements accepted. `None` allows the empty array.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<usize>,
        /// Most elements accepted. `None` is unbounded, which the grammar's own
        /// termination bound then has to carry instead.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<usize>,
    },
    /// Free text. The open shape, and therefore the one the termination bound
    /// exists for.
    Text {
        /// Longest string accepted. `None` means the grammar's own ceiling
        /// applies — free text cannot be left genuinely unbounded, or a valid
        /// call has no obligation to end.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<usize>,
    },
    /// A closed value set. A separate variant from Text: the grammar turns this
    /// into a literal alternation, so the model cannot step outside the set.
    Choice {
        /// The whole set, written literally into the grammar. A value outside it
        /// is unrepresentable rather than refused afterwards, so a tool whose
        /// body accepts exactly N values should declare exactly those N.
        choices: Vec<String>,
    },
    /// A number, integral or not.
    Number {
        /// If true, an integer. The grammar decides on the decimal point from this.
        #[serde(default)]
        is_integer: bool,
        /// Inclusive lower bound, enforced DURING generation rather than after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        /// Inclusive upper bound, likewise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    /// `true` or `false`, and nothing else — not `"true"`, not `1`.
    Bool,
}

/// An object field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// The JSON key. It is also what the model reads in the short signature, so
    /// it is part of the prompt, not only of the parse.
    pub name: String,
    /// What this field accepts.
    pub schema: ArgSchema,
    /// Requiredness is kept NEXT TO the field, not in a separate `required`
    /// list: information kept in two places drifts apart sooner or later.
    #[serde(default)]
    pub required: bool,
}

impl Field {
    /// An OPTIONAL field. Call `required()` on the result to make it mandatory —
    /// optional is the default because a required field the model cannot supply
    /// makes the whole call unwritable.
    pub fn new(name: impl Into<String>, schema: ArgSchema) -> Self {
        Self {
            name: name.into(),
            schema,
            required: false,
        }
    }

    /// Marks the field mandatory: the grammar will not let the call close
    /// without it.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

impl ArgSchema {
    fn from_kind(kind: SchemaKind) -> Self {
        Self {
            kind,
            description: None,
        }
    }

    /// An empty object for tools that take no arguments.
    pub fn empty() -> Self {
        Self::object(vec![])
    }

    /// An object with these fields, in this order. A tool's root schema is
    /// always one of these.
    pub fn object(fields: Vec<Field>) -> Self {
        Self::from_kind(SchemaKind::Object { fields })
    }

    /// An unbounded array of `item`. Add bounds with [`ArgSchema::length`].
    pub fn array(item: ArgSchema) -> Self {
        Self::from_kind(SchemaKind::Array {
            item: Box::new(item),
            min: None,
            max: None,
        })
    }

    /// Free text with no declared ceiling — see [`SchemaKind::Text`] on why the
    /// grammar still imposes one.
    pub fn text() -> Self {
        Self::from_kind(SchemaKind::Text { max_length: None })
    }

    /// A closed set. PREFER THIS OVER `text()` wherever the tool's body accepts
    /// a fixed list: the values are compiled into the grammar literally, so an
    /// invalid one cannot be generated instead of being refused a turn later.
    pub fn choice<S: Into<String>>(choices: impl IntoIterator<Item = S>) -> Self {
        Self::from_kind(SchemaKind::Choice {
            choices: choices.into_iter().map(Into::into).collect(),
        })
    }

    /// A number that may have a fractional part. Bound it with
    /// [`ArgSchema::range`].
    pub fn number() -> Self {
        Self::from_kind(SchemaKind::Number {
            is_integer: false,
            min: None,
            max: None,
        })
    }

    /// A whole number: the grammar will not offer a decimal point.
    pub fn integer() -> Self {
        Self::from_kind(SchemaKind::Number {
            is_integer: true,
            min: None,
            max: None,
        })
    }

    /// `true` or `false`.
    pub fn bool() -> Self {
        Self::from_kind(SchemaKind::Bool)
    }

    /// The sentence the model reads for this field. It is prompt text, so it is
    /// charged against the token budget — short and imperative.
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// A numeric range; silently ignored on a schema that is not a number.
    pub fn range(mut self, low: Option<f64>, high: Option<f64>) -> Self {
        if let SchemaKind::Number { min, max, .. } = &mut self.kind {
            *min = low;
            *max = high;
        }
        self
    }

    /// An array length bound; silently ignored on a schema that is not an array.
    pub fn length(mut self, low: Option<usize>, high: Option<usize>) -> Self {
        if let SchemaKind::Array { min, max, .. } = &mut self.kind {
            *min = low;
            *max = high;
        }
        self
    }

    /// The fields of the root schema (an empty slice if it is not an object) —
    /// the grammar and validation use this often.
    /// The closed set this schema accepts, if it is one.
    ///
    /// EXPOSED FOR THE RECOVERY LAYER in `tacet-tools`: when the model writes a
    /// bare JSON object with no tool name, a value landing inside a closed set
    /// is EVIDENCE about which tool was meant, while the same value landing in
    /// a free-text field is not. Without this the two cannot be told apart and
    /// the call is dropped as ambiguous.
    pub fn choices(&self) -> Option<&[String]> {
        match &self.kind {
            SchemaKind::Choice { choices } => Some(choices),
            _ => None,
        }
    }

    /// The root object's fields — an empty slice if this schema is not an
    /// object, which callers rely on to avoid matching on the kind.
    pub fn fields(&self) -> &[Field] {
        match &self.kind {
            SchemaKind::Object { fields } => fields,
            _ => &[],
        }
    }

    /// The SHORT SIGNATURE shown in the prompt: `expression: text, digits?: integer`.
    ///
    /// WHY NOT THE FULL JSON SCHEMA: a schema cannot be enforced in two places
    /// at once. The arguments are ALREADY enforced by the grammar
    /// (`tacet_grammar::CallConstraint`) — the model cannot step outside the
    /// schema, and because it cannot, it does not need to memorize it. The full
    /// schema ate up nearly the whole 4096-token window in the prompt; the
    /// signature gives the same selection information (which fields exist,
    /// which are required, what type they are) in less than a tenth of the space.
    ///
    /// `?` = optional. The values of a choice type are written out EXPLICITLY
    /// (`'clock'|'date'`): a closed set is the one piece of information the
    /// model cannot invent, and it decides the choice directly — abbreviate it
    /// and the model is forced to guess a valid value.
    pub fn short_signature(&self) -> String {
        self.fields()
            .iter()
            .map(|f| {
                let mark = if f.required { "" } else { "?" };
                format!("{}{}: {}", f.name, mark, f.schema.kind_name())
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The type name of a single field as shown in the prompt. ENGLISH: the rest
    /// of the prompt is English, and in a small model mixing languages is a
    /// needless source of ambiguity.
    fn kind_name(&self) -> String {
        match &self.kind {
            SchemaKind::Object { .. } => "object".into(),
            SchemaKind::Array { item, .. } => format!("array of {}", item.kind_name()),
            SchemaKind::Text { .. } => "text".into(),
            // A closed set is written out; see `short_signature`.
            SchemaKind::Choice { choices } => choices
                .iter()
                .map(|c| format!("'{c}'"))
                .collect::<Vec<_>>()
                .join("|"),
            SchemaKind::Number { is_integer, .. } => {
                if *is_integer {
                    "integer".into()
                } else {
                    "number".into()
                }
            }
            SchemaKind::Bool => "boolean".into(),
        }
    }

    /// The classic JSON Schema equivalent. Only a surface that faces the outside
    /// world (logs, prompt text, compatibility); the internal flow always goes
    /// through `ArgSchema`.
    pub fn json_schema(&self) -> serde_json::Value {
        use serde_json::{Map, Value, json};
        let mut out: Map<String, Value> = match &self.kind {
            SchemaKind::Object { fields } => {
                let mut properties = Map::new();
                let mut required = Vec::new();
                for f in fields {
                    properties.insert(f.name.clone(), f.schema.json_schema());
                    if f.required {
                        required.push(Value::String(f.name.clone()));
                    }
                }
                let mut m = Map::new();
                m.insert("type".into(), json!("object"));
                m.insert("properties".into(), Value::Object(properties));
                m.insert("required".into(), Value::Array(required));
                // A field that is not in the schema is not accepted: the model
                // must not be able to use a key it invented as an escape hatch.
                m.insert("additionalProperties".into(), json!(false));
                m
            }
            SchemaKind::Array { item, min, max } => {
                let mut m = Map::new();
                m.insert("type".into(), json!("array"));
                m.insert("items".into(), item.json_schema());
                if let Some(v) = min {
                    m.insert("minItems".into(), json!(v));
                }
                if let Some(v) = max {
                    m.insert("maxItems".into(), json!(v));
                }
                m
            }
            SchemaKind::Text { max_length } => {
                let mut m = Map::new();
                m.insert("type".into(), json!("string"));
                if let Some(v) = max_length {
                    m.insert("maxLength".into(), json!(v));
                }
                m
            }
            SchemaKind::Choice { choices } => {
                let mut m = Map::new();
                m.insert("type".into(), json!("string"));
                m.insert("enum".into(), json!(choices));
                m
            }
            SchemaKind::Number {
                is_integer,
                min,
                max,
            } => {
                let mut m = Map::new();
                m.insert(
                    "type".into(),
                    json!(if *is_integer { "integer" } else { "number" }),
                );
                if let Some(v) = min {
                    m.insert("minimum".into(), json!(v));
                }
                if let Some(v) = max {
                    m.insert("maximum".into(), json!(v));
                }
                m
            }
            SchemaKind::Bool => {
                let mut m = Map::new();
                m.insert("type".into(), json!("boolean"));
                m
            }
        };
        if let Some(d) = &self.description {
            out.insert("description".into(), Value::String(d.clone()));
        }
        Value::Object(out)
    }

    /// Validates that the incoming arguments conform to the schema.
    ///
    /// Even though the grammar already forces the model, this gate stays: the
    /// grammar can be disabled, and a tool can be called directly (from eval,
    /// from the CLI). If the schema is the single contract, validation must live
    /// in a single place too.
    pub fn validate(&self, value: &serde_json::Value) -> crate::error::ToolResult<()> {
        self.validate_path(value, "arg")
    }

    fn validate_path(&self, value: &serde_json::Value, path: &str) -> crate::error::ToolResult<()> {
        use crate::error::ToolError::{InvalidArgument, MissingField};
        use serde_json::Value;
        match &self.kind {
            SchemaKind::Object { fields } => {
                let object = value
                    .as_object()
                    .ok_or_else(|| InvalidArgument(format!("{path}: expected an object")))?;
                for f in fields {
                    match object.get(&f.name) {
                        Some(Value::Null) | None if f.required => {
                            return Err(MissingField(format!("{path}.{}", f.name)));
                        }
                        Some(v) if !v.is_null() => {
                            f.schema.validate_path(v, &format!("{path}.{}", f.name))?;
                        }
                        _ => {}
                    }
                }
                Ok(())
            }
            SchemaKind::Array { item, min, max } => {
                let array = value
                    .as_array()
                    .ok_or_else(|| InvalidArgument(format!("{path}: expected an array")))?;
                if min.is_some_and(|n| array.len() < n) || max.is_some_and(|n| array.len() > n) {
                    return Err(InvalidArgument(format!("{path}: item count out of bounds")));
                }
                for (i, v) in array.iter().enumerate() {
                    item.validate_path(v, &format!("{path}[{i}]"))?;
                }
                Ok(())
            }
            SchemaKind::Text { max_length } => {
                let s = value
                    .as_str()
                    .ok_or_else(|| InvalidArgument(format!("{path}: expected a string")))?;
                if max_length.is_some_and(|n| s.chars().count() > n) {
                    return Err(InvalidArgument(format!("{path}: string too long")));
                }
                Ok(())
            }
            SchemaKind::Choice { choices } => {
                let s = value
                    .as_str()
                    .ok_or_else(|| InvalidArgument(format!("{path}: expected a string")))?;
                if choices.iter().any(|c| c == s) {
                    Ok(())
                } else {
                    Err(InvalidArgument(format!("{path}: invalid choice '{s}'")))
                }
            }
            SchemaKind::Number {
                is_integer,
                min,
                max,
            } => {
                let n = value
                    .as_f64()
                    .ok_or_else(|| InvalidArgument(format!("{path}: expected a number")))?;
                if *is_integer && !value.is_i64() && !value.is_u64() {
                    return Err(InvalidArgument(format!("{path}: expected an integer")));
                }
                if min.is_some_and(|v| n < v) || max.is_some_and(|v| n > v) {
                    return Err(InvalidArgument(format!("{path}: out of range")));
                }
                Ok(())
            }
            SchemaKind::Bool => value
                .as_bool()
                .map(|_| ())
                .ok_or_else(|| InvalidArgument(format!("{path}: expected true/false"))),
        }
    }
}
