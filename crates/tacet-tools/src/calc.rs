//! `CalcTool` — an expression evaluator; also THE REFERENCE TOOL.
//!
//! This file is deliberately the simplest `Tool` implementation: whoever writes a
//! new tool looks here first. The pattern it shows is — validate the schema,
//! start the chip, do the work, return through a single exit point
//! (`ToolOutcome`). There is no `panic!` or `unwrap` on any path; a tool that
//! crashes takes the whole turn with it.
//!
//! WHY OUR OWN PARSER: the zero-dependency identity. Pulling in a ready-made
//! expression engine would mean taking an unaudited body of code — and its panic
//! behaviour — inside for the sake of four operations. Recursive descent is ~150
//! lines here and entirely under our control.
//!
//! WHY THE BYPASS CHANNEL IS NOT USED: `DataStore` exists to keep bulk data away
//! from the model. The output here is a single number; putting it in the store
//! would mean making the model resolve a reference as well — the channel is used
//! where it pays off.

use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult, ToolState,
    TraceUpdate, boxed,
};

/// The input limit. The model sending a long text as an "expression" is a bug;
/// cutting it off at the limit, without keeping the parser busy, is the more
/// correct answer.
const MAX_LENGTH: usize = 512;

/// The parenthesis nesting limit. Recursive descent consumes stack proportional
/// to input depth: without a limit an input like `"((((..."` overflows the stack,
/// and that is not a catchable error but an outright crash.
const MAX_DEPTH: usize = 32;

const DEFAULT_DIGITS: usize = 6;

/// The limit on the expression text in the chip: a chip is ~one line, a long
/// expression breaks the layout.
const CHIP_EXPRESSION_LIMIT: usize = 48;

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

/// The arithmetic expression evaluator.
pub struct CalcTool;

impl Tool for CalcTool {
    fn name(&self) -> &str {
        "calculate"
    }

    fn description(&self) -> &str {
        // CAME FROM MEASUREMENT: in the "can you add 347 and 268" and "what is it
        // with 20 percent off" cases the model did the arithmetic itself without
        // calling the tool AT ALL. The old text said the right thing but had two
        // gaps: (1) it was the only Turkish description in the catalog while the
        // others were English — mixing languages lowers the weight of the
        // instruction in a small model; (2) it had no "however easy it looks"
        // record, and the model took simple addition to "need no tool".
        "Evaluates a numeric expression: the four operations, parentheses, percent (%) and \
         power (^). Call this for ANY request that contains arithmetic, in any language and \
         HOWEVER EASY it looks - adding two numbers counts. Never do the arithmetic in your \
         head and never write a result you did not get back from this tool."
    }

    fn schema(&self) -> ArgSchema {
        // THE SCHEMA IS THE MODEL'S BOUNDARY: the grammar turns it into a
        // constraint one-to-one, so every detail left out here becomes room for
        // the model to invent. The examples are put in the description
        // deliberately; the model learns the format from the example.
        ArgSchema::object(vec![
            Field::new(
                "expression",
                ArgSchema::text().description(
                    "The expression to evaluate. Example: (12 + 3) * 4  |  250 + 18%  |  2^10",
                ),
            )
            .required(),
            Field::new(
                "digits",
                ArgSchema::integer()
                    .range(Some(0.0), Some(10.0))
                    .description("Number of decimal digits (default 6)."),
            ),
        ])
    }

    // Pure arithmetic; no personal data is read, it does not hit the approval
    // gate.
    fn taints_session(&self) -> bool {
        false
    }

    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            // The grammar may be disabled, or the call may come from eval/the CLI;
            // schema validation therefore also stands at the tool's own gate.
            if let Err(error) = self.schema().validate(&args) {
                return ToolOutcome::failed(&error);
            }
            let Some(expression) = args.get("expression").and_then(|v| v.as_str()) else {
                return ToolOutcome::failed(&ToolError::MissingField("arg.expression".into()));
            };
            let digits = args
                .get("digits")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_DIGITS);

            let trace = ctx.start_chip("=", "Calculating");

            match calculate(expression) {
                Ok(value) => {
                    let result = number_text(value, digits);
                    let chip = format!("{} = {}", truncate_for_chip(expression), result);
                    ctx.update_chip(
                        trace,
                        TraceUpdate::state(ToolState::Read)
                            .text(chip.clone())
                            .raw_input(expression.trim())
                            .raw_output(result.clone()),
                    );
                    // The text going to the model is SEPARATE from the chip text
                    // and shorter: the model will place the result into its own
                    // sentence and has no need to read the expression again.
                    ToolOutcome::read_ok(chip, result.clone()).raw_output(result)
                }
                Err(error) => {
                    // A single error exit point: a Turkish sentence to the chip,
                    // fixed text to the model.
                    let outcome = ToolOutcome::failed(&error);
                    // EXCEPT WHEN THE MODEL HIT THE ALPHABET, in which case the
                    // fixed text is the wrong thing to say.
                    //
                    // This parser takes digits, `. , ( )` and the four
                    // operators, and nothing else — so `sqrt(81)`, `factorial(10)`
                    // and `10!` all die on a letter, and the model is told
                    // `tool_failed: the action could not be completed`, one
                    // sentence for every cause. It cannot tell "you asked for
                    // something impossible" from "the arithmetic broke", so it
                    // retries the same shape and spends the turn.
                    //
                    // ORDINARY BAD SYNTAX KEEPS THE GENERIC TEXT: `12++*5` uses
                    // only legal characters in an illegal order, and telling it
                    // "this tool has no functions" would be false and would send
                    // it looking for the wrong repair — which is the exact harm
                    // this exception exists to undo.
                    //
                    // `run_code` next door already carves exactly this exception
                    // for exactly this reason: the generic message drags the
                    // model into the wrong repair.
                    //
                    // The vocabulary is AUTHORED HERE and never `format!`-ed
                    // from the error — the error text is not a channel to the
                    // model, and the closed error set exists so a tool cannot
                    // become one. AND IT NAMES NO OTHER TOOL: a redirect saying
                    // "use run_code" risks flipping the four square-root and
                    // factorial cases out of a cluster the model currently gets
                    // right by answering in prose, and that trade has not been
                    // measured.
                    let outcome = match &error {
                        ToolError::InvalidArgument(m) if m.starts_with("no such operation") => {
                            let mut o = outcome;
                            o.to_model = ALPHABET_MODEL_TEXT.to_string();
                            o
                        }
                        _ => outcome,
                    };
                    ctx.update_chip(
                        trace,
                        TraceUpdate::state(outcome.state.clone()).text(outcome.chip_text.clone()),
                    );
                    outcome
                }
            }
        })
    }
}

/// What the model is told when its expression used something this parser does
/// not have. See the exception in `run`.
///
/// NO CAPITALS, the same register as `REPEAT_MODEL_TEXT` and the other constants
/// that enter the model's context: capitals used for emphasis were measured to
/// be taken as CONTENT here.
const ALPHABET_MODEL_TEXT: &str = "calculate_alphabet: this tool evaluates arithmetic only — digits, + - * / % and \
     parentheses. It has no functions, no names and no factorial. Do not retry the same \
     expression.";

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Evaluates an expression. Separate and `pub` so it can also be called directly
/// from outside the tool (eval, CLI, tests).
pub fn calculate(expression: &str) -> ToolResult<f64> {
    if expression.trim().is_empty() {
        return Err(ToolError::InvalidArgument("empty expression".into()));
    }
    if expression.chars().count() > MAX_LENGTH {
        return Err(ToolError::InvalidArgument("expression too long".into()));
    }

    // A char slice: the input may be Turkish/unicode and we walk back by position;
    // working with byte indices risks landing in the middle of a multi-byte
    // character.
    let chars: Vec<char> = expression.chars().collect();
    let mut p = Parser {
        input: &chars,
        position: 0,
        depth: 0,
    };

    let value = p.sum()?;
    p.skip_space();
    if p.position < p.input.len() {
        // THE SAME TWO MISTAKES AS IN `atom`, and `10!` arrives HERE rather than
        // there: the number parses, and the `!` is left over. A letter or a `!`
        // after a complete expression is still the model reaching for an
        // operation this parser does not have.
        let c = p.input[p.position];
        return Err(ToolError::InvalidArgument(
            if c.is_alphabetic() || c == '!' {
                format!("no such operation: '{c}'")
            } else {
                format!("unexpected character: '{c}'")
            },
        ));
    }
    finite(value)
}

struct Parser<'a> {
    input: &'a [char],
    position: usize,
    depth: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while matches!(self.input.get(self.position), Some(c) if c.is_whitespace()) {
            self.position += 1;
        }
    }

    /// Skips whitespace and looks at the next character (does not consume it).
    fn peek(&mut self) -> Option<char> {
        self.skip_space();
        self.input.get(self.position).copied()
    }

    fn swallow(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    /// sum := product (('+' | '-') product)*
    fn sum(&mut self) -> ToolResult<f64> {
        let (mut left, _) = self.product()?;
        loop {
            let operator = match self.peek() {
                Some(c @ ('+' | '-')) => c,
                _ => return Ok(left),
            };
            self.position += 1;
            let (right, is_percent) = self.product()?;
            // CALCULATOR INTUITION: "250 + 18%" means 250 + 250*0.18 to the user,
            // not 250.18. That is why the percent flag is carried up from the
            // right-hand term. Whoever wants the absolute form writes it with
            // parentheses: "250 + (18%)" -> 250.18.
            let applied = if is_percent { left * right } else { right };
            left = finite(if operator == '+' {
                left + applied
            } else {
                left - applied
            })?;
        }
    }

    /// product := unary (('*' | '/') unary)*
    ///
    /// Carries the percent flag up only for a single-term product: "10%" is a
    /// percent term, "10% * 2" is now a plain number (0.2).
    fn product(&mut self) -> ToolResult<(f64, bool)> {
        let (mut left, mut is_percent) = self.unary()?;
        loop {
            let operator = match self.peek() {
                // Both the user and the model may write multiplication as 'x';
                // accepting a single operator would produce needless failures.
                Some(c @ ('*' | 'x' | 'X' | '×' | '/' | '÷')) => c,
                _ => return Ok((left, is_percent)),
            };
            self.position += 1;
            let (right, _) = self.unary()?;
            is_percent = false;
            left = if matches!(operator, '/' | '÷') {
                if right == 0.0 {
                    return Err(ToolError::Other("Division by zero is not possible.".into()));
                }
                finite(left / right)?
            } else {
                finite(left * right)?
            };
        }
    }

    /// unary := ('+' | '-') unary | atom ('^' unary)? '%'*
    fn unary(&mut self) -> ToolResult<(f64, bool)> {
        match self.peek() {
            Some('-') => {
                self.position += 1;
                let (value, is_percent) = self.unary()?;
                return Ok((-value, is_percent));
            }
            Some('+') => {
                self.position += 1;
                return self.unary();
            }
            _ => {}
        }

        let base = self.atom()?;
        // The exponent binds to the RIGHT (2^3^2 = 2^9) and, by descending into
        // `unary`, also allows negative exponents such as "2^-3".
        let mut value = if self.swallow('^') {
            let (exponent, _) = self.unary()?;
            power(base, exponent)?
        } else {
            base
        };

        let mut is_percent = false;
        while self.peek() == Some('%') {
            self.position += 1;
            value /= 100.0;
            is_percent = true;
        }
        Ok((value, is_percent))
    }

    /// atom := number | '(' sum ')'
    fn atom(&mut self) -> ToolResult<f64> {
        match self.peek() {
            Some('(') => {
                self.position += 1;
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(ToolError::InvalidArgument(
                        "expression nested too deeply".into(),
                    ));
                }
                let value = self.sum()?;
                if !self.swallow(')') {
                    return Err(ToolError::InvalidArgument("unclosed parenthesis".into()));
                }
                self.depth -= 1;
                Ok(value)
            }
            Some(c) if c.is_ascii_digit() || c == '.' || c == ',' => self.number(),
            // TWO DIFFERENT MISTAKES, AND THE MODEL NEEDS TO BE TOLD WHICH.
            //
            // A LETTER or `!` means it reached for something this parser does
            // not have — `sqrt(81)`, `factorial(10)`, `10!` — and retrying the
            // same shape cannot work. Anything else is ordinary bad syntax
            // (`12++*5` uses only legal characters, in an illegal order) and a
            // retry is reasonable. The two used to share one message, and then
            // one fixed sentence downstream, so the model could not tell "you
            // asked for the impossible" from "the arithmetic broke".
            Some(c) if c.is_alphabetic() || c == '!' => Err(ToolError::InvalidArgument(format!(
                "no such operation: '{c}'"
            ))),
            Some(c) => Err(ToolError::InvalidArgument(format!(
                "unexpected character: '{c}'"
            ))),
            None => Err(ToolError::InvalidArgument("expression ended early".into())),
        }
    }

    fn number(&mut self) -> ToolResult<f64> {
        let mut text = String::new();
        let mut saw_decimal = false;
        while let Some(c) = self.input.get(self.position).copied() {
            if c.is_ascii_digit() {
                text.push(c);
            } else if (c == '.' || c == ',') && !saw_decimal {
                // In Turkish notation the decimal separator is a comma; accepting
                // both makes this independent of which format the model produces.
                saw_decimal = true;
                text.push('.');
            } else if c == '_' {
                // Digit grouping; does not affect the value.
            } else {
                break;
            }
            self.position += 1;
        }
        text.parse::<f64>()
            .map_err(|_| ToolError::InvalidArgument(format!("could not read number: '{text}'")))
    }
}

/// Exponentiation. `powf` silently produces `inf`/`NaN` (0^-1, (-8)^0.5); since
/// those values would poison every later operation, they are turned into an error
/// right here.
fn power(base: f64, exponent: f64) -> ToolResult<f64> {
    finite(base.powf(exponent))
}

/// The overflow/undefined gate. In Rust an f64 overflow is not a panic but `inf`;
/// if it passed silently we would show the user a result reading "inf".
fn finite(value: f64) -> ToolResult<f64> {
    if value.is_finite() {
        Ok(value)
    } else if value.is_nan() {
        Err(ToolError::Other("This operation is not defined.".into()))
    } else {
        Err(ToolError::Other(
            "The result does not fit the numeric range.".into(),
        ))
    }
}

/// Writes the number for a human: whole numbers without decimals, fractions with
/// the needless zeros trimmed. The `1e15` threshold is where f64 loses integer
/// precision.
fn number_text(value: f64, digits: usize) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let mut s = format!("{value:.digits$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// Shortens the expression so the chip stays on one line.
fn truncate_for_chip(expression: &str) -> String {
    let clean: String = expression.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() <= CHIP_EXPRESSION_LIMIT {
        return clean;
    }
    let head: String = clean.chars().take(CHIP_EXPRESSION_LIMIT - 1).collect();
    format!("{head}…")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {

    /// THE MODEL IS TOLD WHICH MISTAKE IT MADE.
    ///
    /// `sqrt(81)` and `10!` are not arithmetic this parser has; `12++*5` is
    /// arithmetic written wrongly. They used to produce the same error and then
    /// the same fixed sentence downstream, so the model could not tell "retrying
    /// cannot work" from "retrying might".
    #[test]
    fn a_name_or_a_factorial_is_a_different_failure_from_bad_syntax() {
        for impossible in ["sqrt(81)", "factorial(10)", "10!", "2 + max(3,4)"] {
            let e = calculate(impossible).expect_err("must fail");
            assert!(
                format!("{e}").contains("no such operation"),
                "{impossible:?} did not report the alphabet: {e}"
            );
        }
        for bad_syntax in ["12++*5", "(3 + 4", "5 5"] {
            let e = calculate(bad_syntax).expect_err("must fail");
            assert!(
                !format!("{e}").contains("no such operation"),
                "{bad_syntax:?} uses only legal characters and must not be told \
                 the tool has no functions: {e}"
            );
        }
    }
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use tacet_kernel::{InMemoryDataStore, Reporter, ToolTrace, TraceCollector};

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_four_operations_and_precedence() {
        assert!(approx(calculate("2 + 3 * 4").unwrap(), 14.0));
        assert!(approx(calculate("12 x 34").unwrap(), 408.0));
        assert!(
            approx(calculate("10 - 2 - 3").unwrap(), 5.0),
            "must bind to the left"
        );
        assert!(approx(calculate("100 / 4 / 5").unwrap(), 5.0));
        assert!(approx(calculate("-3 + 5").unwrap(), 2.0));
        assert!(approx(calculate("3,5 * 2").unwrap(), 7.0), "comma decimal");
    }

    #[test]
    fn parentheses_change_precedence() {
        assert!(approx(calculate("(2 + 3) * 4").unwrap(), 20.0));
        assert!(approx(calculate("((1+2)*(3+4))").unwrap(), 21.0));
        assert!(calculate("(1 + 2").is_err(), "unclosed parenthesis");
        // The depth gate: a proper error instead of a stack overflow.
        let deep = format!("{}1{}", "(".repeat(200), ")".repeat(200));
        assert!(calculate(&deep).is_err());
    }

    #[test]
    fn the_exponent_binds_right_and_takes_a_negative_exponent() {
        assert!(approx(calculate("2^10").unwrap(), 1024.0));
        assert!(
            approx(calculate("2^3^2").unwrap(), 512.0),
            "must bind to the right"
        );
        assert!(approx(calculate("2^-2").unwrap(), 0.25));
        // -2^2 = -(2^2): the unary minus is applied after the exponent.
        assert!(approx(calculate("-2^2").unwrap(), -4.0));
    }

    #[test]
    fn percent_works_with_calculator_intuition() {
        assert!(approx(calculate("50%").unwrap(), 0.5));
        assert!(approx(calculate("250 + 18%").unwrap(), 295.0));
        assert!(approx(calculate("200 - 10%").unwrap(), 180.0));
        // Parentheses close the percent binding: the escape hatch.
        assert!(approx(calculate("250 + (18%)").unwrap(), 250.18));
        // In a product the percent flag must drop.
        assert!(approx(calculate("100 + 10% * 2").unwrap(), 100.2));
    }

    #[test]
    fn overflow_and_division_by_zero_return_an_error_instead_of_panicking() {
        assert!(calculate("1 / 0").is_err());
        assert!(calculate("5 / (3 - 3)").is_err());
        let e = calculate("9^9^9").expect_err("overflows");
        assert!(
            e.short_error().contains("does not fit"),
            "{}",
            e.short_error()
        );
        assert!(calculate("0^-1").is_err());
        assert!(
            calculate("(-8)^0,5").is_err(),
            "NaN must count as undefined"
        );
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(calculate("").is_err());
        assert!(calculate("   ").is_err());
        assert!(calculate("2 +").is_err());
        assert!(calculate("rm -rf /").is_err());
        assert!(calculate("1.2.3").is_err());
        assert!(calculate(&"1+".repeat(400)).is_err(), "length limit");
    }

    #[test]
    fn the_number_format_stays_readable() {
        assert_eq!(number_text(408.0, 6), "408");
        assert_eq!(number_text(0.5, 6), "0.5");
        assert_eq!(number_text(1.0 / 3.0, 4), "0.3333");
        assert_eq!(number_text(2.0 / 3.0, 0), "1");
        assert_eq!(number_text(-7.25, 6), "-7.25");
    }

    // --- The tool contract ---

    fn context(reporter: Arc<dyn Reporter>) -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            "/tmp/tacet-calc",
            reporter,
        )
    }

    #[test]
    fn the_tool_produces_a_chip_and_short_model_text_on_the_success_path() {
        let collector = Arc::new(TraceCollector::new());
        let mut ctx = context(collector.clone());
        let outcome = execute(CalcTool.run(json!({"expression": "12 x 34"}), &mut ctx));

        assert_eq!(outcome.chip_text, "12 x 34 = 408");
        assert_eq!(
            outcome.to_model, "408",
            "only the result should go to the model"
        );
        assert_eq!(outcome.state, ToolState::Read);
        // A read-only tool must not change the world.
        assert!(!outcome.state.changed_world());
        assert!(!ctx.session_tainted(), "calc reads no personal data");

        let traces: Vec<ToolTrace> = collector.traces();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].text, "12 x 34 = 408");
    }

    #[test]
    fn the_tool_gives_the_error_to_the_chip_in_turkish_and_fixed_text_to_the_model() {
        let mut ctx = context(Arc::new(TraceCollector::new()));
        let outcome = execute(CalcTool.run(json!({"expression": "1 / 0"}), &mut ctx));

        assert_eq!(outcome.to_model, tacet_kernel::ERROR_MODEL_TEXT);
        assert_eq!(outcome.chip_text, "Division by zero is not possible.");
        assert!(matches!(outcome.state, ToolState::Failed(_)));
    }

    #[test]
    fn the_tool_rejects_an_argument_that_does_not_match_the_schema() {
        let mut ctx = context(Arc::new(TraceCollector::new()));
        // Required field missing.
        let a = execute(CalcTool.run(json!({}), &mut ctx));
        assert!(matches!(a.state, ToolState::Failed(_)));
        // Wrong type.
        let b = execute(CalcTool.run(json!({"expression": 12}), &mut ctx));
        assert!(matches!(b.state, ToolState::Failed(_)));
        // Digits out of range.
        let c = execute(CalcTool.run(json!({"expression": "1+1", "digits": 99}), &mut ctx));
        assert!(matches!(c.state, ToolState::Failed(_)));
    }

    #[test]
    fn the_digits_argument_rounds_the_output() {
        let mut ctx = context(Arc::new(TraceCollector::new()));
        let outcome = execute(CalcTool.run(json!({"expression": "1/3", "digits": 3}), &mut ctx));
        assert_eq!(outcome.to_model, "0.333");
    }

    #[test]
    fn the_schema_json_gives_the_expected_contract() {
        let js = CalcTool.schema().json_schema();
        assert_eq!(js["type"], "object");
        assert_eq!(js["required"], json!(["expression"]));
        // An invented key must not be an escape hatch.
        assert_eq!(js["additionalProperties"], json!(false));
        assert_eq!(js["properties"]["digits"]["type"], "integer");
    }

    #[test]
    fn a_long_expression_is_truncated_in_the_chip() {
        let long = "1 + ".repeat(40) + "1";
        let mut ctx = context(Arc::new(TraceCollector::new()));
        let outcome = execute(CalcTool.run(json!({"expression": long}), &mut ctx));
        assert_eq!(outcome.to_model, "41");
        assert!(
            outcome.chip_text.chars().count() < 70,
            "{}",
            outcome.chip_text
        );
        assert!(outcome.chip_text.contains('…'));
    }

    /// Core has no tokio; a minimal executor that is enough for tests (the same
    /// pattern as `no_futures` in the core tests).
    fn execute<F: std::future::Future>(mut f: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn noop(_: *const ()) {}
        fn clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = unsafe { std::pin::Pin::new_unchecked(&mut f) };
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }
}
