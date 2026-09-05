//! `search_filter` and `message_intent` — two tools whose whole job is the
//! ARGUMENTS.
//!
//! WHY THESE TWO EXIST, and why they are not like the rest of the catalog. Every
//! other tool here does something: it hashes a file, it opens a page, it writes a
//! spreadsheet. These two compute almost nothing. What they carry is a SCHEMA —
//! a small set of typed, closed-vocabulary slots — and the measurement is whether
//! the model can fill it from a sentence a person actually typed.
//!
//! THAT IS THE PROJECT'S OWN THESIS, NARROWED TO ONE MEASURABLE TASK. The claim
//! on the front page is that the schema is the security boundary and the grammar
//! makes an invalid call unrepresentable. For a free-text argument that buys
//! syntax and nothing more. For a `choice[...]` field it buys the whole answer:
//! the automaton cannot emit a value outside the set, so "price" is `free`,
//! `cheap`, `mid`, `premium` or `any` and there is no sixth thing a 270M model
//! can invent. Slot extraction is where constrained decoding stops being a
//! safety property and starts being a capability.
//!
//! WHY THE RESULT IS ECHOED BACK AS `key=value`. `tacet bench` scores `evidence`
//! against a pool that includes the tool's own output, so a receipt line like
//! `city=istanbul audience=family price=free` turns "did it extract the right
//! slots" into an assertion the benchmark format can already express — without
//! teaching that format to reach inside a call. The receipt is the measurement
//! surface, and it is the reason these tools return text at all.
//!
//! NEITHER TOOL TOUCHES THE NETWORK OR THE DISK. `search_filter` does not search;
//! it normalises a query into a filter that something else would search with.
//! `message_intent` does not read mail; it classifies text it was handed. That is
//! deliberate — both are host-independent and reproducible, which is what lets
//! them sit in a benchmark that runs the same on every machine.

use serde_json::Value;
use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult,
    TraceUpdate, boxed,
};

/// The closed vocabularies. Public so the benchmark and the tests name the same
/// strings the grammar will enforce, rather than two copies that can drift.
pub const AUDIENCE: [&str; 5] = ["family", "kids", "adults", "seniors", "any"];
pub const PRICE: [&str; 5] = ["free", "cheap", "mid", "premium", "any"];
pub const WHEN: [&str; 5] = ["today", "tomorrow", "weekend", "anytime", "any"];
pub const INTENT: [&str; 4] = ["promised_date", "dispute", "paid", "irrelevant"];

fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `key=value` for every slot that was filled, in a fixed order.
///
/// FIXED ORDER AND LOWERCASED, because this string is what a benchmark asserts
/// against: a receipt whose field order depended on the model's argument order
/// would make the same extraction pass or fail by accident.
fn receipt(pairs: &[(&str, Option<&str>)]) -> String {
    pairs
        .iter()
        .filter_map(|(k, v)| v.map(|v| format!("{k}={}", v.to_lowercase())))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// search_filter
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct SearchFilterTool;

impl Tool for SearchFilterTool {
    fn name(&self) -> &str {
        "search_filter"
    }

    /// SHORT, for the reason `checksum` gives: `router::overlap` matches stems
    /// over name + description, so every extra sentence is another chance to
    /// outrank a tool the message was actually about.
    fn description(&self) -> &str {
        "Turns a request for PLACES OR THINGS TO DO into a structured search filter. Call \
         this when the user describes what they are looking for in words — a city, who it is \
         for, what it should cost, when — and fill only the fields the message actually \
         states. Do not guess a field the user did not mention; leave it out."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new(
                "query",
                ArgSchema::text().description("What is being looked for, in the user's own words."),
            )
            .required(),
            Field::new(
                "city",
                ArgSchema::text().description("The place named in the message, if any."),
            ),
            // THE THREE CLOSED FIELDS ARE THE POINT OF THE TOOL. A `choice` turns
            // into a literal alternation in the grammar, so a constrained model
            // CANNOT write a sixth audience — where a free-text field would let
            // it write "families with young children" and leave the caller to
            // guess what that maps to.
            Field::new(
                "audience",
                ArgSchema::choice(AUDIENCE)
                    .description("Who it is for. Leave out when the message does not say."),
            ),
            Field::new(
                "price",
                ArgSchema::choice(PRICE)
                    .description("What it should cost. `free` when the user says free."),
            ),
            Field::new(
                "when",
                ArgSchema::choice(WHEN).description("When it is for, if the message says."),
            ),
        ])
        .description("A structured search filter built from a sentence")
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let trace = ctx.start_chip("filter", "Reading the request…");
            let outcome = match self.work(&args) {
                Ok(o) => o,
                Err(e) => ToolOutcome::failed(&e),
            };
            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(args.to_string())
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            outcome
        })
    }
}

impl SearchFilterTool {
    fn work(&self, args: &Value) -> ToolResult<ToolOutcome> {
        self.schema().validate(args)?;
        let query =
            text_arg(args, "query").ok_or_else(|| ToolError::MissingField("query".into()))?;
        let line = receipt(&[
            ("city", text_arg(args, "city")),
            ("audience", text_arg(args, "audience")),
            ("price", text_arg(args, "price")),
            ("when", text_arg(args, "when")),
        ]);
        let filled = line.split_whitespace().count();
        Ok(ToolOutcome::read_ok(
            format!("filter · {filled} field(s)"),
            if line.is_empty() {
                format!("search_filter: query={query} (no filters were stated)")
            } else {
                format!("search_filter: {line}")
            },
        )
        .raw_output(format!("query={query} {line}")))
    }
}

// ---------------------------------------------------------------------------
// message_intent
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct MessageIntentTool;

impl Tool for MessageIntentTool {
    fn name(&self) -> &str {
        "message_intent"
    }

    fn description(&self) -> &str {
        "Classifies the INTENT of a message someone sent — a reply about money owed, a \
         reservation, a request — and pulls out the date or amount it names. Call this when \
         the user pastes a message and wants to know what it means. Fill `promised_date` and \
         `amount` only when the message states them."
    }

    fn schema(&self) -> ArgSchema {
        ArgSchema::object(vec![
            Field::new(
                "text",
                ArgSchema::text().description("The message being classified."),
            )
            .required(),
            // REQUIRED AND CLOSED. The whole value of this tool is that the
            // answer is one of four things and the automaton cannot produce a
            // fifth — a free-text intent field would come back as "seems like
            // they will pay soon", which no caller can branch on.
            Field::new(
                "intent",
                ArgSchema::choice(INTENT).description(
                    "`promised_date` when they name a day they will pay, `dispute` when they \
                     reject the claim, `paid` when they say it is already paid, \
                     `irrelevant` when it is about none of that.",
                ),
            )
            .required(),
            Field::new(
                "promised_date",
                ArgSchema::text()
                    .description("The date named in the message, copied word for word."),
            ),
            Field::new(
                "amount",
                ArgSchema::text().description("The amount named in the message, with its unit."),
            ),
        ])
        .description("What a message means, and the date or amount it names")
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            let trace = ctx.start_chip("intent", "Reading the message…");
            let outcome = match self.work(&args) {
                Ok(o) => o,
                Err(e) => ToolOutcome::failed(&e),
            };
            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(args.to_string())
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            outcome
        })
    }
}

impl MessageIntentTool {
    fn work(&self, args: &Value) -> ToolResult<ToolOutcome> {
        self.schema().validate(args)?;
        let intent =
            text_arg(args, "intent").ok_or_else(|| ToolError::MissingField("intent".into()))?;
        let line = receipt(&[
            ("intent", Some(intent)),
            ("promised_date", text_arg(args, "promised_date")),
            ("amount", text_arg(args, "amount")),
        ]);
        Ok(ToolOutcome::read_ok(
            format!("intent · {intent}"),
            format!("message_intent: {line}"),
        )
        .raw_output(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE CLOSED FIELDS ARE CLOSED. This is the property the whole pair exists
    /// to demonstrate, so it is asserted rather than assumed: the schema refuses
    /// a value outside the set, which is the same set the grammar compiles into
    /// a literal alternation.
    #[test]
    fn a_value_outside_the_vocabulary_is_refused_by_the_schema() {
        let tool = SearchFilterTool;
        let ok = serde_json::json!({"query":"parks","city":"istanbul","price":"free"});
        assert!(tool.schema().validate(&ok).is_ok());
        let bad = serde_json::json!({"query":"parks","price":"gratis"});
        assert!(
            tool.schema().validate(&bad).is_err(),
            "`gratis` is not in PRICE, and the point of a choice field is that it cannot be"
        );
    }

    /// The receipt is what a benchmark asserts against, so its shape is part of
    /// the contract: `key=value`, lowercased, in a fixed order, and only for
    /// slots that were actually filled.
    #[test]
    fn the_receipt_names_only_the_slots_that_were_filled() {
        let tool = SearchFilterTool;
        let out = tool
            .work(&serde_json::json!({
                "query":"free places for kids","city":"Istanbul","audience":"family","price":"free"
            }))
            .expect("valid");
        assert!(out.to_model.contains("city=istanbul"), "{}", out.to_model);
        assert!(out.to_model.contains("audience=family"), "{}", out.to_model);
        assert!(out.to_model.contains("price=free"), "{}", out.to_model);
        assert!(
            !out.to_model.contains("when="),
            "a slot the message never stated must not appear: {}",
            out.to_model
        );
    }

    /// `intent` is required, and a message that names no date or amount still
    /// classifies — the two optional slots are optional.
    #[test]
    fn an_intent_with_no_date_or_amount_is_still_a_result() {
        let tool = MessageIntentTool;
        let out = tool
            .work(&serde_json::json!({"text":"I already paid that last week","intent":"paid"}))
            .expect("valid");
        assert_eq!(out.to_model, "message_intent: intent=paid");
        assert!(
            tool.schema()
                .validate(&serde_json::json!({"text":"x"}))
                .is_err(),
            "intent is required"
        );
    }
}
