//! THE TOOL BRIDGE: MCP's JSON Schema -> our `ArgSchema`.
//!
//! THIS IS THE MOST DANGEROUS PLACE. `ArgSchema` is deliberately CLOSED and
//! SMALL (see `tacet-kernel::schema`): that is the condition for it to be
//! compilable into a grammar. MCP servers, on the other hand, write full JSON
//! Schema — `oneOf`, `$ref`, `pattern`, `additionalProperties`. When the
//! incoming schema is WIDER than ours there are three options:
//!
//! 1. Narrow it silently and accept the tool -> FORBIDDEN. The grammar FORCES
//!    the model into a shape the server will not accept; the model cannot
//!    produce the right call and nobody can see why. A silent breakage is the
//!    worst breakage.
//! 2. Do a SAFE narrowing -> allowed. The definition of "safe" is below.
//! 3. SKIP the tool and record why -> the default.
//!
//! ## What is a safe narrowing
//!
//! Dropping a constraint can go in two directions:
//!
//! - **Narrowing** (the set the grammar accepts shrinks): may be dangerous,
//!   because the model CANNOT PRODUCE a call the server would consider
//!   legitimate. Only lossless cases are accepted: `["string","null"]` ->
//!   `Text` (in JSON an omitted field already counts as null, and `validate`
//!   behaves the same way), a single-branch `anyOf`/`oneOf` -> that branch.
//! - **Widening** (the grammar accepts more): safe, because the server keeps
//!   doing its own validation and the rejection comes back to the model as a
//!   NORMAL tool error. `pattern`, `format`, `multipleOf`, `uniqueItems` are
//!   dropped this way — but NOT SILENTLY: it is written into
//!   `ToolBridge::notes`.
//!
//! ## Description summary
//!
//! The lesson from the Swift side (`mcp-connection-spec §5.3`): MCP tool
//! descriptions are written for large models and run 100-500 tokens. A server
//! with 20 tools finishes off the 4096 window ON ITS OWN. Swift had the
//! on-device model summarize them; here we do a DETERMINISTIC truncation —
//! calling a model to summarize would make building the tool catalog depend on
//! model quality, and the same server would produce a different definition on
//! every launch (eval would become incomparable).

use serde_json::Value;
use tacet_kernel::{ArgSchema, Field, SchemaKind};

/// The nesting cap. The root object is level 1.
///
/// `mcp-connection-spec §5.2`'s "schema depth filter". Schemas deeper than
/// three levels get expensive both as stack depth in the grammar and as
/// readability in the prompt; besides, a tool description that fits in a 4096
/// window cannot be that deep anyway.
pub const MAX_DEPTH: usize = 3;

/// The upper limit (in characters) of an imported description.
///
/// A two-line sentence is ~160 characters; `§5.3`'s "summarized into 1-2 lines"
/// decision was converted into this number.
pub const DESCRIPTION_LIMIT: usize = 160;

/// Why a tool could not be imported. Shown to the user in the connection detail
/// as "unsupported"; it is not swallowed silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UntranslatableReason {
    /// `oneOf` / `anyOf` / `allOf` / `not` — no equivalent in our closed subset.
    CompositeSchema(String),
    /// `$ref` / `$defs` — requires resolution; the reference may also be
    /// non-local.
    Reference,
    /// The root schema is not an object (MCP `inputSchema` must always be one).
    RootNotObject,
    /// A type we do not recognize or cannot carry (`null`, special types other
    /// than `integer`).
    UnsupportedType(String),
    /// No type was declared at all and it cannot be inferred from the content.
    NoType(String),
    /// `MAX_DEPTH` was exceeded.
    TooDeep,
    /// There is an `enum` but not all of its values are strings.
    MixedEnum,
}

impl UntranslatableReason {
    /// The single line written to the record/log. Shown to the user, DOES NOT
    /// GO to the model.
    pub fn short(&self) -> String {
        match self {
            UntranslatableReason::CompositeSchema(k) => format!("a `{k}` schema cannot be carried"),
            UntranslatableReason::Reference => "the schema contains a `$ref`".into(),
            UntranslatableReason::RootNotObject => "the root schema is not an object".into(),
            UntranslatableReason::UnsupportedType(t) => format!("unsupported type: {t}"),
            UntranslatableReason::NoType(f) => format!("the field declares no type: {f}"),
            UntranslatableReason::TooDeep => {
                format!("the schema is deeper than {MAX_DEPTH} levels")
            }
            UntranslatableReason::MixedEnum => "the enum values are not strings".into(),
        }
    }
}

/// Alongside the converted schema, the list of DROPPED constraints.
///
/// If it is not empty the tool is still used; the record exists to tell the
/// user "we took this tool but we cannot enforce that constraint".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConversionNotes(pub Vec<String>);

impl ConversionNotes {
    fn dropped(&mut self, path: &str, constraint: &str) {
        self.0
            .push(format!("{path}: `{constraint}` is not enforced"));
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The conversion outcome.
pub struct Conversion {
    pub schema: ArgSchema,
    pub notes: ConversionNotes,
}

/// MCP `inputSchema` -> `ArgSchema`.
///
/// If it returns an error the tool is SKIPPED; the call site logs the reason.
pub fn convert_schema(input: &Value) -> Result<Conversion, UntranslatableReason> {
    let mut notes = ConversionNotes::default();
    // MCP tools may take no arguments; the server may not have sent
    // `inputSchema` at all, or may have sent `{}`. Both mean "takes no
    // arguments", not an error.
    if input.is_null() || input.as_object().is_some_and(|o| o.is_empty()) {
        return Ok(Conversion {
            schema: ArgSchema::empty(),
            notes,
        });
    }
    if !input.is_object() {
        return Err(UntranslatableReason::RootNotObject);
    }
    if type_name(input).is_some_and(|t| t != "object") {
        return Err(UntranslatableReason::RootNotObject);
    }
    let schema = convert_node(input, "arg", 1, &mut notes)?;
    if !matches!(schema.kind, SchemaKind::Object { .. }) {
        return Err(UntranslatableReason::RootNotObject);
    }
    Ok(Conversion { schema, notes })
}

fn convert_node(
    node: &Value,
    path: &str,
    depth: usize,
    notes: &mut ConversionNotes,
) -> Result<ArgSchema, UntranslatableReason> {
    if depth > MAX_DEPTH {
        return Err(UntranslatableReason::TooDeep);
    }
    let Some(object) = node.as_object() else {
        // A `true`/`false` schema (valid in JSON Schema) tells us nothing; if we
        // accept a typeless field the grammar cannot know what to produce.
        return Err(UntranslatableReason::NoType(path.to_string()));
    };

    if object.contains_key("$ref")
        || object.contains_key("$defs")
        || object.contains_key("definitions")
    {
        return Err(UntranslatableReason::Reference);
    }
    for key in ["allOf", "not"] {
        if object.contains_key(key) {
            return Err(UntranslatableReason::CompositeSchema(key.into()));
        }
    }
    // SAFE NARROWING: a single-branch `anyOf`/`oneOf` is not a choice, only an
    // unnecessary wrapper; descending into that branch eliminates no legitimate
    // call.
    for key in ["anyOf", "oneOf"] {
        if let Some(branches) = object.get(key).and_then(Value::as_array) {
            let meaningful: Vec<&Value> = branches.iter().filter(|b| !is_empty_type(b)).collect();
            if meaningful.len() == 1 {
                return convert_node(meaningful[0], path, depth, notes);
            }
            return Err(UntranslatableReason::CompositeSchema(key.into()));
        }
    }

    // `enum` is looked at before the type: `{"enum":["a","b"]}` is a closed set
    // even without declaring a type, and it maps exactly onto `Choice`.
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        let choices: Option<Vec<String>> = values
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect();
        let choices = choices.ok_or(UntranslatableReason::MixedEnum)?;
        if choices.is_empty() {
            return Err(UntranslatableReason::MixedEnum);
        }
        return Ok(with_description(ArgSchema::choice(choices), object));
    }

    let kind = type_name(node).ok_or_else(|| typeless_reason(object, path))?;

    let schema = match kind.as_str() {
        "object" => {
            let mut fields = Vec::new();
            let required: Vec<&str> = object
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            // The ORDER of `properties` is alphabetical unless serde_json's
            // `preserve_order` feature is on; either way it is deterministic,
            // i.e. the same server compiles to the same grammar on every launch.
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                for (name, child) in properties {
                    let child_path = format!("{path}.{name}");
                    let child_schema = convert_node(child, &child_path, depth + 1, notes)?;
                    let mut field = Field::new(name.clone(), child_schema);
                    if required.contains(&name.as_str()) {
                        field = field.required();
                    }
                    fields.push(field);
                }
            }
            // If `additionalProperties` is a SCHEMA (not a bool) it is typing
            // extra fields we do not know about; `ArgSchema` has no equivalent
            // and ignoring it would let the model invent fields.
            if object
                .get("additionalProperties")
                .is_some_and(|v| v.is_object())
            {
                return Err(UntranslatableReason::CompositeSchema(
                    "additionalProperties".into(),
                ));
            }
            ArgSchema::object(fields)
        }
        "array" => {
            let element = match object.get("items") {
                Some(i) => convert_node(i, &format!("{path}[]"), depth + 1, notes)?,
                // An array whose element is not described: the grammar cannot
                // know what to produce.
                None => return Err(UntranslatableReason::NoType(format!("{path}[]"))),
            };
            if object.get("prefixItems").is_some() {
                return Err(UntranslatableReason::CompositeSchema("prefixItems".into()));
            }
            if object.contains_key("uniqueItems") {
                notes.dropped(path, "uniqueItems");
            }
            ArgSchema::array(element).length(
                number_usize(object.get("minItems")),
                number_usize(object.get("maxItems")),
            )
        }
        "string" => {
            for constraint in ["pattern", "format", "minLength"] {
                if object.contains_key(constraint) {
                    notes.dropped(path, constraint);
                }
            }
            let mut s = ArgSchema::text();
            if let Some(n) = number_usize(object.get("maxLength")) {
                s.kind = SchemaKind::Text {
                    max_length: Some(n),
                };
            }
            s
        }
        "integer" | "number" => {
            for constraint in ["multipleOf", "exclusiveMinimum", "exclusiveMaximum"] {
                if object.contains_key(constraint) {
                    notes.dropped(path, constraint);
                }
            }
            let base = if kind == "integer" {
                ArgSchema::integer()
            } else {
                ArgSchema::number()
            };
            base.range(
                object.get("minimum").and_then(Value::as_f64),
                object.get("maximum").and_then(Value::as_f64),
            )
        }
        "boolean" => ArgSchema::bool(),
        other => return Err(UntranslatableReason::UnsupportedType(other.to_string())),
    };

    Ok(with_description(schema, object))
}

/// Why no type was found: is it uninformed, or is it a type we cannot carry
/// like `null`? The distinction is useful in the note shown to the user.
fn typeless_reason(object: &serde_json::Map<String, Value>, path: &str) -> UntranslatableReason {
    match object.get("type") {
        Some(Value::String(s)) => UntranslatableReason::UnsupportedType(s.clone()),
        Some(Value::Array(_)) => UntranslatableReason::UnsupportedType("composite type".into()),
        _ => UntranslatableReason::NoType(path.to_string()),
    }
}

/// The type name. A binary union like `["string","null"]` is reduced to
/// `string` by SAFE NARROWING: in our model `null` means "field absent" and
/// `ArgSchema::validate` treats null like a missing field too — no loss.
fn type_name(node: &Value) -> Option<String> {
    match node.get("type") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(types)) => {
            let non_empty: Vec<&str> = types
                .iter()
                .filter_map(Value::as_str)
                .filter(|t| *t != "null")
                .collect();
            match non_empty.as_slice() {
                [single] => Some((*single).to_string()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A branch that carries no information, like `{}` or `{"type":"null"}`.
fn is_empty_type(node: &Value) -> bool {
    match node.as_object() {
        Some(o) if o.is_empty() => true,
        Some(o) => o.get("type").and_then(Value::as_str) == Some("null"),
        None => false,
    }
}

fn number_usize(v: Option<&Value>) -> Option<usize> {
    v.and_then(Value::as_u64).map(|n| n as usize)
}

fn with_description(schema: ArgSchema, object: &serde_json::Map<String, Value>) -> ArgSchema {
    match object.get("description").and_then(Value::as_str) {
        Some(d) if !d.trim().is_empty() => schema.description(truncate_description(d)),
        _ => schema,
    }
}

/// Reduces a long MCP description to a sentence the prompt can carry (§5.3).
///
/// THE RULE: try the first sentence first (period/question/exclamation); if it
/// is still long, cut at a word boundary and add "…". Cutting mid-word teaches
/// the model a broken token; "…" is a "there is more here" marker and, while it
/// does not stop the model from inventing, it is honest to the user.
pub fn truncate_description(raw: &str) -> String {
    let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= DESCRIPTION_LIMIT {
        return one_line;
    }

    // If the first sentence is short enough it is the best summary — the
    // author's own.
    if let Some(end) = first_sentence_end(&one_line) {
        let first: String = one_line.chars().take(end).collect();
        if first.chars().count() <= DESCRIPTION_LIMIT {
            return first;
        }
    }

    let truncated: String = one_line.chars().take(DESCRIPTION_LIMIT).collect();
    let body = match truncated.rfind(' ') {
        // If the last space is very near the start (one long word) there is no
        // point looking for a word boundary.
        Some(i) if i > DESCRIPTION_LIMIT / 2 => &truncated[..i],
        _ => &truncated,
    };
    format!("{}…", body.trim_end_matches([',', ';', ' ']))
}

/// The end CHARACTER index of the first sentence (punctuation included).
fn first_sentence_end(text: &str) -> Option<usize> {
    let mut counter = 0usize;
    for c in text.chars() {
        counter += 1;
        if matches!(c, '.' | '!' | '?') {
            return Some(counter);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn convert(v: Value) -> Result<ArgSchema, UntranslatableReason> {
        convert_schema(&v).map(|c| c.schema)
    }

    // --- TRANSLATABLE examples ---

    #[test]
    fn a_flat_object_converts_and_requiredness_carries_over() {
        let schema = convert(json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "shell command"},
                "timeout": {"type": "integer", "minimum": 1, "maximum": 600},
            },
            "required": ["command"],
        }))
        .expect("must convert");

        let fields = schema.fields();
        assert_eq!(fields.len(), 2);
        let command = fields
            .iter()
            .find(|f| f.name == "command")
            .expect("command");
        assert!(command.required);
        assert_eq!(command.schema.description.as_deref(), Some("shell command"));
        let t = fields
            .iter()
            .find(|f| f.name == "timeout")
            .expect("timeout");
        assert!(!t.required);
        assert_eq!(
            t.schema.kind,
            SchemaKind::Number {
                is_integer: true,
                min: Some(1.0),
                max: Some(600.0)
            }
        );
    }

    #[test]
    fn an_enum_converts_to_a_choice() {
        let schema = convert(json!({
            "type": "object",
            "properties": { "mode": {"type": "string", "enum": ["read", "write"]} },
        }))
        .expect("must convert");
        assert_eq!(
            schema.fields()[0].schema.kind,
            SchemaKind::Choice {
                choices: vec!["read".into(), "write".into()]
            }
        );
    }

    #[test]
    fn an_array_and_its_limits_carry_over() {
        let schema = convert(json!({
            "type": "object",
            "properties": {
                "paths": {"type": "array", "items": {"type": "string"}, "maxItems": 5},
            },
        }))
        .expect("must convert");
        let SchemaKind::Array { item, max, .. } = &schema.fields()[0].schema.kind else {
            panic!("an array was expected");
        };
        assert_eq!(*max, Some(5));
        assert!(matches!(item.kind, SchemaKind::Text { .. }));
    }

    #[test]
    fn a_tool_without_arguments_falls_to_an_empty_schema() {
        assert_eq!(convert(json!({})).expect("empty").fields().len(), 0);
        assert_eq!(convert(Value::Null).expect("null").fields().len(), 0);
    }

    #[test]
    fn a_nested_object_converts_up_to_the_limit() {
        // root(1) -> outer(2) -> inner(3): exactly at the limit, must pass.
        assert!(
            convert(json!({
                "type": "object",
                "properties": { "outer": {
                    "type": "object",
                    "properties": { "inner": {"type": "string"} },
                }},
            }))
            .is_ok()
        );
    }

    // --- SAFE NARROWINGS ---

    #[test]
    fn a_string_null_pair_narrows_to_text() {
        // In JSON null = "field absent"; `ArgSchema::validate` behaves the same
        // way, so this narrowing eliminates no legitimate call.
        let schema = convert(json!({
            "type": "object",
            "properties": { "note": {"type": ["string", "null"]} },
        }))
        .expect("must convert");
        assert!(matches!(
            schema.fields()[0].schema.kind,
            SchemaKind::Text { .. }
        ));
    }

    #[test]
    fn a_single_branch_anyof_descends_into_that_branch() {
        let schema = convert(json!({
            "type": "object",
            "properties": { "x": {"anyOf": [{"type": "integer"}, {"type": "null"}]} },
        }))
        .expect("must convert");
        assert!(matches!(
            schema.fields()[0].schema.kind,
            SchemaKind::Number {
                is_integer: true,
                ..
            }
        ));
    }

    #[test]
    fn pattern_is_dropped_but_recorded() {
        let conversion = convert_schema(&json!({
            "type": "object",
            "properties": { "sha": {"type": "string", "pattern": "^[0-9a-f]{40}$"} },
        }))
        .expect("must convert");
        // The tool IS ACCEPTED (widening is safe: the server validates for
        // itself), but the dropped constraint stays visible.
        assert!(!conversion.notes.is_empty());
        assert!(
            conversion.notes.0[0].contains("pattern"),
            "{:?}",
            conversion.notes
        );
    }

    // --- UNTRANSLATABLE: NOT accepted silently ---

    #[test]
    fn a_multi_branch_oneof_is_rejected() {
        let error = convert(json!({
            "type": "object",
            "properties": { "target": {"oneOf": [{"type": "string"}, {"type": "integer"}]} },
        }))
        .unwrap_err();
        assert_eq!(error, UntranslatableReason::CompositeSchema("oneOf".into()));
    }

    #[test]
    fn a_ref_is_rejected() {
        let error = convert(json!({
            "type": "object",
            "properties": { "a": {"$ref": "#/$defs/Thing"} },
        }))
        .unwrap_err();
        assert_eq!(error, UntranslatableReason::Reference);
    }

    #[test]
    fn an_allof_is_rejected() {
        let error = convert(json!({
            "type": "object",
            "properties": { "a": {"allOf": [{"type": "string"}]} },
        }))
        .unwrap_err();
        assert_eq!(error, UntranslatableReason::CompositeSchema("allOf".into()));
    }

    #[test]
    fn a_too_deep_schema_is_rejected() {
        let error = convert(json!({
            "type": "object",
            "properties": { "a": { "type": "object", "properties": {
                "b": { "type": "object", "properties": { "c": {"type": "string"} } },
            }}},
        }))
        .unwrap_err();
        assert_eq!(error, UntranslatableReason::TooDeep);
    }

    #[test]
    fn a_typeless_field_is_rejected() {
        let error = convert(json!({
            "type": "object",
            "properties": { "free": {"description": "could be anything"} },
        }))
        .unwrap_err();
        assert_eq!(error, UntranslatableReason::NoType("arg.free".into()));
    }

    #[test]
    fn additional_properties_with_a_schema_is_rejected() {
        let error = convert(json!({
            "type": "object",
            "properties": {},
            "additionalProperties": {"type": "string"},
        }))
        .unwrap_err();
        assert_eq!(
            error,
            UntranslatableReason::CompositeSchema("additionalProperties".into())
        );
    }

    #[test]
    fn a_mixed_enum_is_rejected() {
        let error = convert(json!({
            "type": "object",
            "properties": { "k": {"enum": ["a", 3]} },
        }))
        .unwrap_err();
        assert_eq!(error, UntranslatableReason::MixedEnum);
    }

    #[test]
    fn a_non_object_root_is_rejected() {
        assert_eq!(
            convert(json!({"type": "string"})).unwrap_err(),
            UntranslatableReason::RootNotObject
        );
    }

    #[test]
    fn an_array_without_an_element_is_rejected() {
        assert_eq!(
            convert(json!({"type": "object", "properties": {"a": {"type": "array"}}})).unwrap_err(),
            UntranslatableReason::NoType("arg.a[]".into())
        );
    }

    // --- Does the converted schema REALLY work ---

    #[test]
    fn the_converted_schema_accepts_the_right_argument_and_rejects_the_wrong_one() {
        // "it compiled" is not enough: the produced schema must pass the core's
        // validation as expected, otherwise the grammar forces the model into
        // the wrong place.
        let schema = convert(json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "mode": {"type": "string", "enum": ["fast", "slow"]},
            },
            "required": ["command"],
        }))
        .expect("must convert");

        assert!(schema.validate(&json!({"command": "ls"})).is_ok());
        assert!(
            schema
                .validate(&json!({"command": "ls", "mode": "fast"}))
                .is_ok()
        );
        assert!(
            schema.validate(&json!({"mode": "fast"})).is_err(),
            "required field missing"
        );
        assert!(
            schema
                .validate(&json!({"command": "ls", "mode": "medium"}))
                .is_err(),
            "outside the set"
        );
        assert!(
            schema.validate(&json!({"command": 5})).is_err(),
            "wrong type"
        );
    }

    // --- Description truncation ---

    #[test]
    fn a_short_description_is_untouched() {
        assert_eq!(truncate_description("Reads a file."), "Reads a file.");
    }

    #[test]
    fn line_breaks_are_reduced_to_a_single_line() {
        assert_eq!(truncate_description("Reads\n  a file.\n"), "Reads a file.");
    }

    #[test]
    fn in_a_long_description_the_first_sentence_is_chosen() {
        let raw = format!(
            "A short summary sentence. {}",
            "A very long detail. ".repeat(40)
        );
        let truncated = truncate_description(&raw);
        assert_eq!(truncated, "A short summary sentence.");
    }

    #[test]
    fn if_the_first_sentence_is_long_too_it_is_cut_at_a_word_boundary() {
        let raw = "word ".repeat(80);
        let truncated = truncate_description(&raw);
        assert!(
            truncated.chars().count() <= DESCRIPTION_LIMIT + 1,
            "{truncated}"
        );
        assert!(truncated.ends_with('…'));
        assert!(
            !truncated.contains("wor…"),
            "it must not cut mid-word: {truncated}"
        );
    }

    #[test]
    fn a_single_long_word_does_not_panic() {
        let truncated = truncate_description(&"a".repeat(500));
        assert!(truncated.chars().count() <= DESCRIPTION_LIMIT + 1);
    }
}
