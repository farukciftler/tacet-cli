//! The filter that strips the RAW TOOL CALL out of the streaming output.
//!
//! THE FAILURE (from a real user session):
//!
//! ```text
//! web_search({"query": "Ortaköy Üsküdar ferry times"})
//!   ⏺ globe · searched · 26 results
//! ```
//!
//! The first line SHOULD NOT HAVE BEEN VISIBLE: the chip is ALREADY printed by
//! the tool, and in a readable form at that.
//!
//! THE ROOT CAUSE. The old code (`is_call` + a one-shot decision) looked at the
//! first word ONCE and tied the whole turn's fate to it: if the head was
//! `name(`, the entire generation was suppressed; otherwise the entire
//! generation was streamed. But the model does NOT ALWAYS put the call at the
//! start of the output — sometimes it writes a sentence first ("Let me look up
//! the ferry times.") and produces the call AFTER it. By then the decision has
//! already been made as "plain text" and the call was poured onto the screen
//! verbatim. That is also why it did not show up in `--message` mode: there the
//! stream is never printed to the screen (see `if !interactive` in `listener`),
//! the answer is printed in one piece at the end of the turn.
//!
//! THE FIX: the decision is not made once, it is made CONTINUOUSLY. The filter
//! watches the stream from start to finish; if anywhere in the text — at the
//! start of a line — it sees a tool name FROM THE CATALOG followed by `(`, it
//! swallows from there until the balanced parenthesis closes, then continues
//! with the text.
//!
//! THE NAMES COME FROM THE CATALOG, not from a guess. An "identifier +
//! parenthesis" rule would also catch legitimate text like `Hello (how are
//! you)`; an exact match against a catalog name reduces that risk to zero. For
//! the same reason A ONE-WORD ANSWER IS NOT SWALLOWED: `Yes.` matches no tool
//! name and is released at the first character — the old code's "undecided" wait
//! in the `Hello` example disappears too.
//!
//! A PARENTHESIS INSIDE A STRING DOES NOT COUNT: the `)` inside
//! `web_search({"query": "closing )"})` looked like it closed the call; JSON
//! strings and the `\` escape are tracked separately.

/// How long a call body may run at most. If generation is cut off halfway (the
/// closing parenthesis never arrives) this stops the rest of the text being
/// swallowed forever: a buffer exceeding the cap is printed RAW — incomplete
/// output beats lost output.
const CALL_CAP: usize = 4000;

struct CallState {
    raw: String,
    depth: i32,
    in_string: bool,
    escape: bool,
}

/// The state machine that strips tool calls out of streaming text.
pub struct CallFilter {
    names: Vec<String>,
    /// The candidate held at the start of a line (whitespace + identifier) — it
    /// is held as long as it is a PREFIX of a tool name, and released the moment
    /// it is not.
    candidate: Option<String>,
    /// Is the next character at the start of a line (a call is looked for ONLY
    /// there).
    line_start: bool,
    call: Option<CallState>,
    /// The whitespace after a call ends is swallowed — so no blank line is left
    /// behind.
    eat_whitespace: bool,
}

impl CallFilter {
    pub fn new(names: Vec<String>) -> Self {
        Self {
            names,
            candidate: None,
            line_start: true,
            call: None,
            eat_whitespace: false,
        }
    }

    /// Filters the chunk; returns the part TO BE PRINTED TO THE SCREEN.
    pub fn feed(&mut self, chunk: &str) -> String {
        let mut output = String::new();
        for c in chunk.chars() {
            self.character(c, &mut output);
        }
        output
    }

    /// The stream is over: an undecided buffer is printed raw. If the answer's
    /// last word is the prefix of a tool name (`web`, say) it must not stay
    /// swallowed.
    pub fn finish(&mut self) -> String {
        let mut output = String::new();
        if let Some(candidate) = self.candidate.take() {
            output.push_str(&candidate);
        }
        if let Some(state) = self.call.take() {
            // A call left half-finished: not worth showing the user (the chip is
            // already there), but this is a generation accident, so swallowing
            // it silently is not right either — printing raw JSON TO THE SCREEN
            // is still worse. We stay silent.
            let _ = state;
        }
        output
    }

    fn character(&mut self, c: char, output: &mut String) {
        if let Some(mut state) = self.call.take() {
            state.raw.push(c);
            if state.in_string {
                if state.escape {
                    state.escape = false;
                } else if c == '\\' {
                    state.escape = true;
                } else if c == '"' {
                    state.in_string = false;
                }
            } else {
                match c {
                    '"' => state.in_string = true,
                    '(' | '{' | '[' => state.depth += 1,
                    ')' | '}' | ']' => state.depth -= 1,
                    _ => {}
                }
            }
            if state.depth <= 0 {
                self.line_start = true;
                self.eat_whitespace = true;
                return;
            }
            if state.raw.len() > CALL_CAP {
                // No closing came: what looked like a call was not one. Print raw.
                output.push_str(&state.raw);
                self.line_start = false;
                return;
            }
            self.call = Some(state);
            return;
        }

        if self.eat_whitespace {
            if c.is_whitespace() {
                return;
            }
            self.eat_whitespace = false;
        }

        if self.candidate.is_some() || self.line_start {
            self.candidate_character(c, output);
            return;
        }
        output.push(c);
        self.line_start = c == '\n';
    }

    fn candidate_character(&mut self, c: char, output: &mut String) {
        let mut candidate = self.candidate.take().unwrap_or_default();
        let body = candidate.trim_start().to_string();

        if c == '\n' {
            output.push_str(&candidate);
            output.push('\n');
            self.line_start = true;
            return;
        }
        // The leading whitespace of a line is kept in front of the candidate: if
        // the call is swallowed, let us not leave a line behind with nothing but
        // whitespace on it.
        if (c == ' ' || c == '\t') && body.is_empty() {
            candidate.push(c);
            self.candidate = Some(candidate);
            return;
        }
        if c == '(' && self.names.iter().any(|n| n.as_str() == body) {
            // AN EXACT MATCH + an opening parenthesis: this is a tool call. DROP
            // the candidate (and the whitespace in front of it) and start
            // swallowing the body.
            self.call = Some(CallState {
                raw: String::new(),
                depth: 1,
                in_string: false,
                escape: false,
            });
            return;
        }
        if c.is_ascii_alphanumeric() || c == '_' {
            let grown = format!("{body}{c}");
            if self.names.iter().any(|n| n.starts_with(&grown)) {
                candidate.push(c);
                self.candidate = Some(candidate);
                return;
            }
        }
        // Not the prefix of any tool name: the candidate is released.
        output.push_str(&candidate);
        output.push(c);
        self.line_start = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["web_search", "web_fetch", "calculate", "time"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Feeds the stream chunk by chunk and joins it — the real failure lives at
    /// a chunk boundary.
    fn chunked(text: &str, size: usize) -> String {
        let mut f = CallFilter::new(names());
        let mut output = String::new();
        let chars: Vec<char> = text.chars().collect();
        for group in chars.chunks(size) {
            let p: String = group.iter().collect();
            output.push_str(&f.feed(&p));
        }
        output.push_str(&f.finish());
        output
    }

    fn at_every_size(text: &str) -> String {
        let single = chunked(text, 10_000);
        for size in [1, 2, 3, 5, 9] {
            assert_eq!(chunked(text, size), single, "chunk size {size}: {text:?}");
        }
        single
    }

    /// A CALL AT THE START — the case the old code caught too.
    #[test]
    fn a_call_at_the_start_is_swallowed() {
        assert_eq!(at_every_size("web_search({\"query\": \"ferry\"})"), "");
    }

    /// THE REAL FAILURE: if there is plain text BEFORE the call, the call must
    /// still be swallowed.
    #[test]
    fn a_call_after_some_text_is_swallowed() {
        let out =
            at_every_size("Let me look up the ferry times.\nweb_search({\"query\": \"ferry\"})\n");
        assert_eq!(out, "Let me look up the ferry times.\n");
    }

    /// Two calls back to back are swallowed too (the one-shot decision missed
    /// this).
    #[test]
    fn consecutive_calls_are_swallowed() {
        let out = at_every_size("time({\"kind\":\"clock\"})\nweb_search({\"query\":\"a\"})\nDone.");
        assert_eq!(out, "Done.");
    }

    /// A PARENTHESIS INSIDE A STRING does not close the call early.
    #[test]
    fn a_parenthesis_inside_a_string_does_not_fool_it() {
        assert_eq!(at_every_size("web_search({\"query\": \"a) b(\"})"), "");
        assert_eq!(
            at_every_size("web_search({\"query\": \"escape \\\" )\"})xyz"),
            "xyz"
        );
    }

    /// A LEGITIMATE ONE-WORD ANSWER IS NOT SWALLOWED — the explicit requirement.
    #[test]
    fn a_one_word_answer_is_not_swallowed() {
        assert_eq!(at_every_size("Yes."), "Yes.");
        assert_eq!(at_every_size("Yes"), "Yes");
        assert_eq!(at_every_size("Hello"), "Hello");
        // A word that is the PREFIX of a tool name is also free if no `(`
        // follows it.
        assert_eq!(at_every_size("web"), "web");
        assert_eq!(at_every_size("calc it up"), "calc it up");
        assert_eq!(at_every_size("calculate, I said"), "calculate, I said");
    }

    /// A name that is NOT in the catalog + a parenthesis is text; it is not
    /// swallowed.
    #[test]
    fn a_non_tool_parenthesis_is_not_swallowed() {
        assert_eq!(
            at_every_size("Ortaköy (Üsküdar) line"),
            "Ortaköy (Üsküdar) line"
        );
        assert_eq!(at_every_size("note(1) here"), "note(1) here");
    }

    /// A tool name in the MIDDLE of a line does not count as a call: the model
    /// writes the call at the start of a line, while `calculate(` inside a
    /// sentence is plain text.
    #[test]
    fn the_middle_of_a_line_is_not_swallowed() {
        assert_eq!(
            at_every_size("I wrote the result as calculate(2+2)"),
            "I wrote the result as calculate(2+2)"
        );
    }

    /// No blank line is left behind once a call is swallowed.
    #[test]
    fn no_blank_line_is_left_after_a_call() {
        assert_eq!(
            at_every_size("web_search({\"q\":\"a\"})\n\nResult: 3"),
            "Result: 3"
        );
    }
}
