//! MRTR — the server asks a question, without a held-open stream (spec §4).
//!
//! In the old protocol a server could push `elicitation/create` in the middle
//! of a call. 2026-07-28 (SEP-2322) removes the push: a `tools/call` may come
//! back with `resultType: "input_required"` and its questions, and the client
//! re-sends THE SAME call with the answers attached. That maps onto this
//! product's approval gate almost exactly — it is the same shape, arriving
//! from the other side of the wire.
//!
//! ELICITATION IS AN INJECTION SURFACE, so the rules here are not decoration:
//!
//! - A question is TEXT ON A SCREEN. It is never interpreted, never executed,
//!   and never handed to the model as an instruction. Control characters are
//!   stripped and the length is capped before anything is shown.
//! - The ANSWER IS TYPED BY THE USER. The model is never consulted to answer a
//!   server's question — a server interrogating the local model through the
//!   user's turn would be `sampling` with extra steps, and `sampling` is
//!   refused permanently.
//! - At most `MAX_INPUT_ROUNDS` cycles per call. A server must not be able to
//!   hold a turn hostage.
//! - With nobody to ask (`--message`, eval, any headless run) the answer is
//!   DECLINE, exactly like the approval gate's `SilentDeny`.
//!
//! ASSUMED — the field names. The envelope is read defensively (`questions` /
//! `inputRequests` / `elicitation`, and `prompt` / `message` / `description`)
//! because the published shape has not been verified against a real
//! 2026-07-28 server yet. Reading several spellings costs nothing; guessing
//! ONE and being wrong costs the whole feature.

use serde_json::{Map, Value};

/// The cap on a question's text. Longer than `SCREEN_LIMIT` because a question
/// is a sentence the user must actually be able to read, but finite: 10 KB of
/// prose in a terminal is an attack, not a question.
pub const QUESTION_LIMIT: usize = 240;

/// The cap on the number of questions shown for one call.
pub const MAX_QUESTIONS: usize = 8;

/// How many `input_required` cycles one call may go through (spec §4).
pub const MAX_INPUT_ROUNDS: usize = 3;

/// The marker a server sets to ask for input.
pub const INPUT_REQUIRED: &str = "input_required";

/// What the far side wants to know. `id` is what the answer is keyed by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    /// Already sanitized: printable, capped, safe to write to a terminal.
    pub prompt: String,
    pub kind: QuestionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionKind {
    Text,
    /// A yes/no — the shape the terminal renders as `y/N`.
    Boolean,
    /// A closed set. The choices are sanitized like the prompt.
    Choice(Vec<String>),
}

/// Who answers a server's question. The terminal implements it; headless runs
/// use `DeclineInput`.
///
/// `None` means DECLINED — the user pressed Esc, or there was nobody to ask.
/// The call is abandoned and the retry is never sent.
pub trait InputAsk: Send + Sync {
    fn ask(&self, server: &str, questions: &[Question]) -> Option<Vec<String>>;
}

/// The headless default: nobody to ask, so nothing is answered.
pub struct DeclineInput;

impl InputAsk for DeclineInput {
    fn ask(&self, _server: &str, _questions: &[Question]) -> Option<Vec<String>> {
        None
    }
}

/// Is this result the server asking for input rather than answering?
pub fn is_input_required(result: &Value) -> bool {
    result
        .get("resultType")
        .and_then(Value::as_str)
        .is_some_and(|t| t == INPUT_REQUIRED)
}

/// Pulls the questions out of an `input_required` result. PURE.
pub fn parse_questions(result: &Value) -> Vec<Question> {
    let items = ["questions", "inputRequests", "elicitation", "inputs"]
        .iter()
        .find_map(|key| result.get(*key).and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();

    items
        .iter()
        .enumerate()
        .take(MAX_QUESTIONS)
        .map(|(i, item)| {
            let id = ["id", "name", "key"]
                .iter()
                .find_map(|k| item.get(*k).and_then(Value::as_str))
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                // A question without an id still gets asked — refusing it
                // would strand the call over a missing label. The position is
                // a stable enough key for a single round trip.
                .unwrap_or_else(|| format!("input{i}"));
            let prompt = ["prompt", "message", "description", "title"]
                .iter()
                .find_map(|k| item.get(*k).and_then(Value::as_str))
                .unwrap_or("the server asked for a value");
            let choices: Vec<String> = ["choices", "enum", "options"]
                .iter()
                .find_map(|k| item.get(*k).and_then(Value::as_array))
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.as_str())
                        .take(MAX_QUESTIONS)
                        .map(sanitize)
                        .collect()
                })
                .unwrap_or_default();
            let declared = item.get("type").and_then(Value::as_str).unwrap_or("string");
            let kind = if !choices.is_empty() {
                QuestionKind::Choice(choices)
            } else if declared == "boolean" {
                QuestionKind::Boolean
            } else {
                QuestionKind::Text
            };
            Question {
                id: sanitize(&id),
                prompt: sanitize(prompt),
                kind,
            }
        })
        .collect()
}

/// The `inputResponses` object sent back with the retried call.
pub fn build_responses(questions: &[Question], answers: &[String]) -> Value {
    let mut map = Map::new();
    for (question, answer) in questions.iter().zip(answers) {
        let value = match question.kind {
            // A boolean goes back as a boolean; sending "true" as a string to a
            // schema expecting a boolean fails validation on the far side, and
            // the user would be asked the same question again.
            QuestionKind::Boolean => Value::Bool(matches!(
                answer.trim().to_ascii_lowercase().as_str(),
                "y" | "yes" | "true" | "1"
            )),
            _ => Value::String(answer.clone()),
        };
        map.insert(question.id.clone(), value);
    }
    Value::Object(map)
}

/// Makes text the FAR SIDE wrote safe to put on a terminal: control characters
/// (ANSI escapes included, since they begin with one) are dropped, whitespace
/// is collapsed, and the result is capped.
fn sanitize(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > QUESTION_LIMIT {
        let mut short: String = collapsed.chars().take(QUESTION_LIMIT - 1).collect();
        short.push('…');
        short
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_question_is_read_out_of_several_spellings() {
        let result = json!({
            "resultType": "input_required",
            "questions": [
                {"id": "confirm", "prompt": "confirm cost: creating project (est. $5)", "type": "boolean"},
                {"name": "region", "message": "which region", "enum": ["eu", "us"]},
                {"description": "free text"},
            ]
        });
        assert!(is_input_required(&result));
        let questions = parse_questions(&result);
        assert_eq!(questions.len(), 3);
        assert_eq!(questions[0].kind, QuestionKind::Boolean);
        assert_eq!(
            questions[1].kind,
            QuestionKind::Choice(vec!["eu".into(), "us".into()])
        );
        assert_eq!(questions[2].kind, QuestionKind::Text);
        // A question with no id still gets asked, keyed by position.
        assert_eq!(questions[2].id, "input2");
    }

    #[test]
    fn escapes_and_length_do_not_reach_the_terminal() {
        let long = "x".repeat(10_000);
        let result = json!({"questions": [
            {"id": "a", "prompt": format!("\u{1b}[31mred\u{1b}[0m\nsecond line\u{7}")},
            {"id": "b", "prompt": long},
        ]});
        let questions = parse_questions(&result);
        assert!(
            !questions[0].prompt.contains('\u{1b}') && !questions[0].prompt.contains('\n'),
            "no control character survives: {:?}",
            questions[0].prompt
        );
        // The escape byte itself becomes a space and the collapse tidies it:
        // what is left is the printable rubble, which is harmless on a screen.
        assert_eq!(questions[0].prompt, "[31mred [0m second line");
        assert_eq!(questions[1].prompt.chars().count(), QUESTION_LIMIT);
    }

    #[test]
    fn a_boolean_answer_goes_back_as_a_boolean() {
        let questions = parse_questions(&json!({"questions": [
            {"id": "confirm", "type": "boolean", "prompt": "sure"},
            {"id": "note", "prompt": "why"},
        ]}));
        let responses = build_responses(&questions, &["y".into(), "because".into()]);
        assert_eq!(responses["confirm"], json!(true));
        assert_eq!(responses["note"], json!("because"));
    }

    #[test]
    fn nobody_to_ask_means_declined() {
        assert!(DeclineInput.ask("linear", &[]).is_none());
    }
}
