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
    ///
    /// # Panics
    ///
    /// On an EMPTY set. A closed set with nothing in it is a value position no
    /// call can fill, and the grammar learns that only after it has opened the
    /// key: MEASURED (audit M15), `enum: []` left the automaton with a live key
    /// and a value with no first character — "the constraint forbade every
    /// token" in the middle of a call. Every caller in this workspace builds the
    /// set from a list it has already checked is non-empty (the MCP bridge
    /// refuses `enum: []` as `MixedEnum`, `shell` and `db_write` refuse an empty
    /// allow-list, `clipboard` is not offered without a helper), so reaching
    /// this is an authoring bug, and it is stopped where the schema is written
    /// rather than where a model trips over it.
    pub fn choice<S: Into<String>>(choices: impl IntoIterator<Item = S>) -> Self {
        let choices: Vec<String> = choices.into_iter().map(Into::into).collect();
        assert!(
            !choices.is_empty(),
            "ArgSchema::choice: an empty closed set has no value a call could carry"
        );
        Self::from_kind(SchemaKind::Choice { choices })
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
                let n = number_value(value, *is_integer)
                    .map_err(|why| InvalidArgument(format!("{path}: {why}")))?;
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

    /// The number `validate` would compare against a `Number` schema's range
    /// for this JSON number TEXT — or `None` where `validate` would refuse the
    /// text outright for this kind.
    ///
    /// WHY THE GRAMMAR ASKS THIS INSTEAD OF PARSING THE NUMBER ITSELF. The two
    /// used to keep separate rules and the grammar's comment said they "agree
    /// exactly". MEASURED (audit M16), they did not: over 200 000 random walks
    /// the grammar closed on `-0` in an integer field 15 287 times, and
    /// `serde_json` reads `-0` as the FLOAT `-0.0`, so `validate` answered
    /// "expected an integer". `18446744073709551616` (u64::MAX + 1),
    /// `-9223372036854775809` (i64::MIN - 1) and `1.7976931348623158e308` were
    /// likewise closed on by the grammar and refused here — the last because
    /// `serde_json`'s float parse rounds differently from Rust's at the very top
    /// of the range, so "ask Rust whether it is finite" was a neighbour of the
    /// rule, not the rule. Parsing with the SAME parser and classifying with the
    /// SAME function `validate` uses is what makes "the grammar cannot produce
    /// a number the validator refuses" true by construction rather than by
    /// comment.
    ///
    /// Text that is not a single JSON number — a prefix like `-` or `1e`, or
    /// anything with a second value after it — is `None`.
    pub fn json_number_value(text: &str, is_integer: bool) -> Option<f64> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        if !value.is_number() {
            return None;
        }
        number_value(&value, is_integer).ok()
    }
}

/// The one classification of a JSON number against a `Number` schema's kind,
/// shared by `validate` (on a parsed call) and `json_number_value` (on the
/// grammar's text). The `Err` is the reason `validate` prints.
///
/// AN INTEGER IS WHAT `serde_json` STORES AS ONE — i64 or u64 — not "an f64
/// with no fraction". That is the narrower reading, and the grammar follows it:
/// `-0`, `18446744073709551616` and `1.0` all have an integral value and none of
/// them reaches a tool as an integer.
fn number_value(value: &serde_json::Value, is_integer: bool) -> Result<f64, &'static str> {
    let n = value.as_f64().ok_or("expected a number")?;
    if is_integer && !value.is_i64() && !value.is_u64() {
        return Err("expected an integer");
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `validate` and `json_number_value` must give the same verdict on the same
    /// number, because the grammar's whole claim rests on the second standing in
    /// for the first. Each spelling here was a MEASURED disagreement (audit M16)
    /// or sits on one of the edges that produced them.
    #[test]
    fn the_text_predicate_and_the_validator_agree_on_every_edge() {
        let integer = ArgSchema::object(vec![Field::new("n", ArgSchema::integer()).required()]);
        let number = ArgSchema::object(vec![Field::new("n", ArgSchema::number()).required()]);
        for (text, as_integer, as_number) in [
            ("0", true, true),
            ("-0", false, true),
            ("-1", true, true),
            ("18446744073709551615", true, true),
            ("18446744073709551616", false, true),
            ("-9223372036854775808", true, true),
            ("-9223372036854775809", false, true),
            ("1.0", false, true),
            ("1e3", false, true),
            ("1.7976931348623157e308", false, true),
            ("1.7976931348623158e308", false, false),
            ("1e309", false, false),
            ("1e-400", false, true),
            ("-", false, false),
            ("1e", false, false),
            ("1 2", false, false),
            ("\"1\"", false, false),
        ] {
            assert_eq!(
                ArgSchema::json_number_value(text, true).is_some(),
                as_integer,
                "integer verdict on {text}"
            );
            assert_eq!(
                ArgSchema::json_number_value(text, false).is_some(),
                as_number,
                "number verdict on {text}"
            );
            // Wherever the text is JSON at all, `validate` on the parsed call is
            // the independent witness for both verdicts.
            let call = format!(r#"{{"n":{text}}}"#);
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&call) {
                assert_eq!(
                    integer.validate(&value).is_ok(),
                    as_integer,
                    "validate: {call}"
                );
                assert_eq!(
                    number.validate(&value).is_ok(),
                    as_number,
                    "validate: {call}"
                );
            }
        }
        // The value handed back is the one the range is compared against.
        assert_eq!(ArgSchema::json_number_value("-12", true), Some(-12.0));
        assert_eq!(ArgSchema::json_number_value("2.5", false), Some(2.5));
        assert!(
            integer.validate(&json!({"n": -0.0})).is_err(),
            "a float zero is not an integer to the tool layer"
        );
    }

    /// An empty closed set is refused where the schema is written (audit M15).
    #[test]
    #[should_panic(expected = "an empty closed set")]
    fn an_empty_closed_set_is_refused_where_it_is_written() {
        let _ = ArgSchema::choice(Vec::<String>::new());
    }
}
