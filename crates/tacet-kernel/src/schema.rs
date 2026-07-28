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
    #[serde(flatten)]
    pub kind: SchemaKind,
    /// The description shown to the model. Must be short and imperative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SchemaKind {
    /// An object with ordered fields. The root schema of a tool is always this.
    Object {
        fields: Vec<Field>,
    },
    /// A homogeneous array.
    Array {
        item: Box<ArgSchema>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<usize>,
    },
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<usize>,
    },
    /// A closed value set. A separate variant from Text: the grammar turns this
    /// into a literal alternation, so the model cannot step outside the set.
    Choice {
        choices: Vec<String>,
    },
    Number {
        /// If true, an integer. The grammar decides on the decimal point from this.
        #[serde(default)]
        is_integer: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    Bool,
}

/// An object field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub schema: ArgSchema,
    /// Requiredness is kept NEXT TO the field, not in a separate `required`
    /// list: information kept in two places drifts apart sooner or later.
    #[serde(default)]
    pub required: bool,
}

impl Field {
    pub fn new(name: impl Into<String>, schema: ArgSchema) -> Self {
        Self {
            name: name.into(),
            schema,
            required: false,
        }
    }

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

    pub fn object(fields: Vec<Field>) -> Self {
        Self::from_kind(SchemaKind::Object { fields })
    }

    pub fn array(item: ArgSchema) -> Self {
        Self::from_kind(SchemaKind::Array {
            item: Box::new(item),
            min: None,
            max: None,
        })
    }

    pub fn text() -> Self {
        Self::from_kind(SchemaKind::Text { max_length: None })
    }

    pub fn choice<S: Into<String>>(choices: impl IntoIterator<Item = S>) -> Self {
        Self::from_kind(SchemaKind::Choice {
            choices: choices.into_iter().map(Into::into).collect(),
        })
    }

    pub fn number() -> Self {
        Self::from_kind(SchemaKind::Number {
            is_integer: false,
            min: None,
            max: None,
        })
    }

    pub fn integer() -> Self {
        Self::from_kind(SchemaKind::Number {
            is_integer: true,
            min: None,
            max: None,
        })
    }

    pub fn bool() -> Self {
        Self::from_kind(SchemaKind::Bool)
    }

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
