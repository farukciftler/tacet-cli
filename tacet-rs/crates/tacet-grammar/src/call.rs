//! `CallConstraint` — the BRIDGE between the grammar and the generation loop.
//!
//! WHY IT EXISTS: this crate (compile + PDA + mask) was written but had ZERO
//! effect in production. `tacet-engine` did not even know `tacet-grammar` as a
//! dependency; the constraint argument of `engine.generate(...)` went as `None`
//! at every call site, and the only concrete type implementing `Constrainer` was
//! `FreeConstraint`, which constrains nothing. So the goal of "forcing a small
//! model into valid JSON" stayed on paper. This file closes that gap.
//!
//! DEPENDENCY DIRECTION: the direction `constraint.rs` in `tacet-engine` states
//! up front — grammar depends on engine, engine does NOT depend on grammar. The
//! contract is over there, the implementation here. That way a setup running
//! unconstrained never compiles the grammar code at all.
//!
//! WHAT IS FORCED, WHAT IS NOT (honestly):
//!   FORCED     — once the model has written `tool_name(`, the arguments MUST
//!                conform to the GRAMMAR of that tool's schema. Invalid JSON, a
//!                field not in the schema, a value outside the enum set, a
//!                number out of range: none of them CAN BE PRODUCED any more.
//!                That was the real escape hatch.
//!   NOT FORCED — the model's CHOICE to call a tool. A plain answer is a
//!                legitimate output, so free text has to stay open at the start;
//!                otherwise even "hello" would be forced into a tool call. The
//!                tool NAME is therefore forced only indirectly: output with an
//!                invented name counts as "plain text" as far as the grammar is
//!                concerned and is rejected at gate 1 of `ToolExecutor` (not in
//!                the catalog). That is the second line of defence, not this one.
//!
//! MASKING COST: in the free-text stage NO mask is produced at all (no-op),
//! because the whole vocabulary is valid there and walking the trie would burn
//! milliseconds for nothing. The cost is paid only inside the arguments — and
//! there `TokenMask` eliminates invalid branches in one move.

use crate::{Grammar, GrammarState, TokenMask};
use std::sync::Arc;
use tacet_core::ToolCatalog;
use tacet_engine::{ConstraintSession, Constrainer, EngineError};

/// The immutable body of the constraint. Shared via `Arc` because
/// `Constrainer::session` returns `Box<dyn ConstraintSession>` (that is,
/// `'static`): the session cannot BORROW the constraint, it has to own a share
/// of it.
struct Inner {
    /// The vocabulary trie is INDEPENDENT OF THE GRAMMAR (it only looks at token
    /// texts), so exactly ONE is built for all tools — building one trie per
    /// tool would repeat the same work once per catalog entry.
    mask: TokenMask,
    vocab: Vec<String>,
    /// (tool name, the compiled grammar of its arguments) — in catalog order.
    tools: Vec<(String, Arc<Grammar>)>,
    /// The tokens that CONTAIN a `(`: (id, the part before `(`, the part after
    /// the first `(`).
    ///
    /// WHY A CACHE: in the prefix stage the only real danger is a single token
    /// that CLOSES the tool name and enters the arguments as well (like `("`).
    /// Finding those tokens by rescanning the vocabulary at every step would
    /// bring back exactly the cost the mask exists to avoid. Since tokens with a
    /// parenthesis are a very small minority of the vocabulary, separating them
    /// once is enough.
    paren_tokens: Vec<(usize, String, String)>,
    /// The character length of the longest tool name — the prefix queue is
    /// bounded by it (see `align_prefix`). Growing the queue without bound would
    /// mean rescanning the entire output at every step of a long plain answer.
    longest_name: usize,
}

/// The call constraint derived from the catalog. Built once, shared for the
/// whole generation.
pub struct CallConstraint {
    inner: Arc<Inner>,
}

impl CallConstraint {
    /// Compiles the schema of every tool in the catalog and builds the trie over
    /// the vocabulary.
    ///
    /// `vocab` comes from the engine (`EngineProvider::vocab`): the constraint is
    /// only meaningful with an engine that KNOWS the tokenizer, because the mask
    /// speaks in token ids.
    pub fn new(vocab: &[String], catalog: &ToolCatalog) -> Self {
        let tools: Vec<(String, Arc<Grammar>)> = catalog
            .tools()
            .iter()
            .map(|t| (t.name().to_string(), Grammar::compile(&t.schema())))
            .collect();
        let longest_name = tools.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0);
        let paren_tokens = vocab
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let p = t.find('(')?;
                Some((i, t[..p].to_string(), t[p + 1..].to_string()))
            })
            .collect();
        Self {
            inner: Arc::new(Inner {
                mask: TokenMask::new(vocab),
                vocab: vocab.to_vec(),
                tools,
                paren_tokens,
                longest_name,
            }),
        }
    }
}

impl Constrainer for CallConstraint {
    fn session(&self) -> Box<dyn ConstraintSession> {
        Box::new(CallSession {
            inner: Arc::clone(&self.inner),
            stage: Stage::Prefix { queue: String::new() },
        })
    }
    fn name(&self) -> &str {
        "tool_call"
    }
}

/// Which region of the generation we are in.
enum Stage {
    /// The region BEFORE a call. The output may still be the prefix of a tool
    /// name, but it may also be a plain answer; since free text is open there is
    /// no mask (except for the parenthesis-token exception).
    ///
    /// `queue` is the LAST few characters of the produced text (the longest tool
    /// name + 1 context character). The valid tool-name prefix is derived from
    /// it at every step (see `align_prefix`).
    Prefix { queue: String },
    /// `tool_name(` was consumed; the arguments are now subject to the GRAMMAR.
    Args { state: GrammarState },
    /// The arguments closed and `)` was consumed too — the call is complete.
    Closed,
}

struct CallSession {
    inner: Arc<Inner>,
    stage: Stage,
}

/// Is this an identifier (tool name) character. This decides where the sliding
/// window MAY START.
fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Trims `text` down to the LONGEST suffix that can still be completed into a
/// tool name. Returns an empty string if no suffix matches.
///
/// WHY A SLIDING WINDOW (root cause): the old code killed the constraint
/// ONE-WAY (`Stage::Free`) the moment it stopped holding a prefix, and since
/// `mask` did nothing in that stage, THE REST of the generation was left
/// completely unconstrained. The hybrid versions of Qwen3 START generation with
/// `<think>`; the first character (`<`) is the prefix of no tool name, so the
/// constraint died at the very first step. What was seen in the field was
/// exactly this: the model picks the right tool but writes a syntax that
/// violates the grammar, like `web_search(query:"...")`, and `ToolCall::parse`
/// does not count it as a call.
///
/// The fix is to REALIGN instead of dying: after every character the longest
/// valid tool-name prefix at the end of the text is looked for. That way a call
/// that comes after a thinking block, after an introductory sentence, or after
/// an earlier half match is forced as well.
///
/// THE IDENTIFIER BOUNDARY CONDITION: a match may only start after something
/// that is NOT an identifier character (or at the start of the text). Otherwise
/// a word like `xcreate_document(` would count as a call.
///
/// IRRELEVANCE IS PRESERVED: this alignment masks nothing, it merely stays in a
/// position where it CAN CATCH a call if one starts. A plain answer is still
/// completely free.
fn align_prefix(inner: &Inner, queue: &str) -> String {
    let chars: Vec<char> = queue.chars().collect();
    // If the queue has been trimmed (it is full), character 0 is only CONTEXT:
    // the window cannot start there, because we no longer know what came before it.
    let earliest = usize::from(chars.len() > inner.longest_name);
    for start in earliest..chars.len() {
        if start > 0 && is_id_char(chars[start - 1]) {
            continue;
        }
        let s: String = chars[start..].iter().collect();
        if inner.tools.iter().any(|(name, _)| name.starts_with(&s)) {
            return s;
        }
    }
    String::new()
}

/// Can a NEW tool name start at the end of the queue. If the last character is
/// an identifier character, no: the letters continuing from there are part of
/// the current word.
fn can_start_new_name(queue: &str) -> bool {
    queue.chars().next_back().is_none_or(|c| !is_id_char(c))
}

impl CallSession {
    /// Processes a single character. Advancing per character is mandatory: one
    /// token carries several characters and the stage transition (seeing `(`)
    /// may happen in the MIDDLE of a token.
    fn swallow(&mut self, c: char) -> Result<(), EngineError> {
        let inner = Arc::clone(&self.inner);
        match &mut self.stage {
            Stage::Prefix { queue } => {
                // `(` is the moment of separation: if the window just before it
                // is a COMPLETE tool name, the call starts. If not, nothing
                // happens — the constraint DOES NOT DIE, plain text keeps
                // flowing and a later call can still be caught.
                if c == '(' {
                    let name = align_prefix(&inner, queue);
                    if let Some((_, grammar)) = inner.tools.iter().find(|(n, _)| *n == name) {
                        self.stage = Stage::Args { state: grammar.state() };
                        return Ok(());
                    }
                }
                queue.push(c);
                // The queue is kept at a fixed size: the longest tool name + 1
                // context character. The context character is required in order
                // to measure the identifier boundary.
                while queue.chars().count() > inner.longest_name + 1 {
                    let first = queue.chars().next().map(char::len_utf8).unwrap_or(0);
                    queue.drain(..first);
                }
                Ok(())
            }
            Stage::Args { state } => {
                // A `)` arriving while the grammar is in an accepting state
                // closes the call. This is checked first: `)` is not valid inside
                // the grammar of the arguments, so letting it fall through would
                // be an error.
                if state.is_done() && c == ')' {
                    self.stage = Stage::Closed;
                    return Ok(());
                }
                state.advance(&c.to_string()).map_err(|_| {
                    // Getting here means masking and advancing have drifted
                    // apart; swallowing it silently would legitimize an invalid
                    // call.
                    EngineError::ConstraintViolation(c as u32)
                })
            }
            Stage::Closed => Ok(()),
        }
    }
}

impl CallSession {
    /// In the prefix stage, eliminates ONLY parenthesis tokens (see `mask`).
    ///
    /// The elimination condition is kept narrow: a token is closed if, together
    /// with what comes before the `(`, it forms a COMPLETE tool name AND the
    /// remainder after `(` does not conform to that tool's grammar. If the name
    /// does not match, the token leads to plain text anyway, so it is harmless;
    /// closing it would stop the model from writing normal sentences containing
    /// a `(`.
    fn prefix_mask(&self, queue: &str, logits: &mut [f32]) {
        let matched = align_prefix(&self.inner, queue);
        // If the window is empty AND the queue ends in the middle of a word, the
        // leading letters of the token are the continuation of that word; a new
        // name cannot start there.
        if matched.is_empty() && !can_start_new_name(queue) {
            return;
        }
        for (id, before, remainder) in &self.inner.paren_tokens {
            if *id >= logits.len() {
                continue;
            }
            let name = format!("{matched}{before}");
            let name = name.trim();
            let Some((_, grammar)) = self.inner.tools.iter().find(|(n, _)| n == name) else {
                continue;
            };
            if !remainder_conforms(grammar, remainder) {
                logits[*id] = f32::NEG_INFINITY;
            }
        }
    }
}

/// Whether the remaining characters after `tool_name(` SURVIVE in the grammar.
///
/// The same logic as the `Stage::Args` branch of `swallow`: a `)` arriving in an
/// accepting state closes the call, and what follows is no longer the grammar's
/// business.
fn remainder_conforms(grammar: &Arc<Grammar>, remainder: &str) -> bool {
    let mut state = grammar.state();
    for c in remainder.chars() {
        if state.is_done() && c == ')' {
            return true;
        }
        if state.advance(&c.to_string()).is_err() {
            return false;
        }
    }
    true
}

impl ConstraintSession for CallSession {
    /// The inside of the arguments is STRUCTURAL: the engine skips the repetition
    /// penalty here (code and JSON are naturally repetitive — the rationale is in
    /// the measurement recorded on the trait definition).
    fn is_structural(&self) -> bool {
        matches!(self.stage, Stage::Args { .. })
    }

    fn mask(&self, logits: &mut [f32]) {
        let state = match &self.stage {
            Stage::Args { state } => state,
            // PREFIX STAGE: free text is legitimate, so there is NO general mask.
            // The only exception are tokens that close the tool name and enter
            // the arguments within the SAME token (like `("`). If those are not
            // masked, the grammar rejects the token afterwards and generation
            // fails with `ConstraintViolation` — that was exactly the observed
            // fault: after `read_document` was written, the single incoming token
            // carried both `(` and `"`, whereas the grammar expects `{` after
            // `(`. Masking and advancing drifting apart is prevented here;
            // catching the error afterwards is too late.
            Stage::Prefix { queue } => {
                self.prefix_mask(queue, logits);
                return;
            }
            Stage::Closed => return,
        };
        // The terminator `)` joins the mask DURING THE WALK, not afterwards.
        //
        // The previous version was a suffix check of the form
        // `vocab[id].starts_with(')')`, and it was wrong in both directions.
        // What it missed: tokens like `"})` that START inside the grammar and
        // END with the terminator — and in Qwen2.5 the natural tokenization of a
        // valid call ends exactly like that, so the most likely token was left
        // closed. What it opened too much: every token that appends chatter after
        // the call, like `))` or `) ...`. The check performed inside the walk
        // fixes both: the prefix must PASS the grammar, and the remainder must be
        // EXACTLY the terminator.
        let allowed = self.inner.mask.mask_with_terminator(state, Some(')'));
        for (id, logit) in logits.iter_mut().enumerate() {
            // An index outside the vocabulary: the constraint can say nothing
            // about it, and closing it is the safest choice.
            if id >= allowed.len() || !allowed[id] {
                *logit = f32::NEG_INFINITY;
            }
        }
    }

    fn advance(&mut self, token: u32) -> Result<(), EngineError> {
        let text = self
            .inner
            .vocab
            .get(token as usize)
            .cloned()
            .ok_or(EngineError::ConstraintViolation(token))?;
        for c in text.chars() {
            self.swallow(c)?;
        }
        Ok(())
    }

    fn is_done(&self) -> bool {
        // ONLY a completed call safely cuts generation short. Returning `true`
        // in the free-text stage would chop the model's sentence in half.
        matches!(self.stage, Stage::Closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tacet_core::{
        ArgSchema, Field, Tool, ToolContext, ToolFuture, ToolOutcome, boxed,
    };

    /// A tool with one required field and one enum field — the two most
    /// important claims of the constraint (missing required field, value outside
    /// the set) are measured on it.
    struct DocumentTool;

    impl Tool for DocumentTool {
        fn name(&self) -> &str {
            "create_document"
        }
        fn description(&self) -> &str {
            "Produces a document."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![
                Field::new("format", ArgSchema::choice(["excel", "markdown"])).required(),
                Field::new("file_name", ArgSchema::text()).required(),
            ])
        }
        fn run<'a>(
            &'a self,
            _args: serde_json::Value,
            _ctx: &'a mut ToolContext,
        ) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::read_ok("ok", "ok") })
        }
    }

    /// Code point = token id; the convention of FakeEngine's generation path.
    fn vocab() -> Vec<String> {
        (0..0x1000u32)
            .map(|i| char::from_u32(i).map(String::from).unwrap_or_default())
            .collect()
    }

    fn constraint() -> CallConstraint {
        let mut c = tacet_core::ToolCatalog::new();
        c.add(Arc::new(DocumentTool));
        CallConstraint::new(&vocab(), &c)
    }

    /// Whether a character can be produced RIGHT NOW — asks the mask.
    fn allowed(session: &dyn ConstraintSession, c: char) -> bool {
        let mut logits = vec![0.0f32; 0x1000];
        session.mask(&mut logits);
        logits[c as usize] != f32::NEG_INFINITY
    }

    fn feed(session: &mut Box<dyn ConstraintSession>, text: &str) {
        for c in text.chars() {
            session.advance(c as u32).expect("valid text must be accepted");
        }
    }

    #[test]
    fn a_valid_call_is_accepted_from_start_to_end() {
        let k = constraint();
        let mut s = k.session();
        feed(&mut s, r#"create_document({"format":"excel","file_name":"report"})"#);
        assert!(s.is_done(), "a complete call should have finished in an accepting state");
    }

    /// THE CONSTRAINT'S REAL JOB: the object CANNOT BE CLOSED before the required
    /// field is filled. This is the generation-stage counterpart of the
    /// `document-schema-violation` case in eval — over there the schema gate (GATE 2) is
    /// measured, here the fact that the grammar makes the same violation
    /// impossible before it is even produced.
    #[test]
    fn an_object_cannot_be_closed_with_a_required_field_missing() {
        let k = constraint();
        let mut s = k.session();
        feed(&mut s, r#"create_document({"format":"excel""#);
        assert!(!allowed(s.as_ref(), '}'), "`}}` can be produced with `file_name` missing");
        // Once we go on and fill the field, it must be closable.
        feed(&mut s, r#","file_name":"x""#);
        assert!(allowed(s.as_ref(), '}'), "cannot close after the field was filled");
    }

    /// Enum values are embedded in the grammar literally: a letter outside the
    /// set CANNOT BE PRODUCED.
    #[test]
    fn the_enum_set_cannot_be_stepped_out_of() {
        let k = constraint();
        let mut s = k.session();
        feed(&mut s, r#"create_document({"format":""#);
        assert!(allowed(s.as_ref(), 'e'), "the first letter of excel must be open");
        assert!(allowed(s.as_ref(), 'm'), "the first letter of markdown must be open");
        assert!(!allowed(s.as_ref(), 'z'), "a letter outside the set can be produced");
    }

    /// A plain answer is a legitimate output: nothing is masked at the start,
    /// otherwise even "hello" would be forced into a tool call.
    #[test]
    fn a_plain_answer_is_not_constrained() {
        let k = constraint();
        let mut s = k.session();
        assert!(allowed(s.as_ref(), 'M'));
        feed(&mut s, "Hello, how can I help you?");
        assert!(allowed(s.as_ref(), 'X'), "there must be no mask in free text");
        assert!(!s.is_done(), "free text must note count as finished by the constraint");
    }

    /// A name that is not in the catalog DOES NOT TURN INTO a call: the output
    /// falls back to plain text and the decision is made by gate 1 of
    /// `ToolExecutor` (see the file header).
    #[test]
    fn an_unknown_tool_name_does_not_count_as_a_call() {
        let k = constraint();
        let mut s = k.session();
        feed(&mut s, r#"imaginary_tool({"x":"#);
        assert!(allowed(s.as_ref(), 'q'), "an unknown name must be plain text");
        assert!(!s.is_done());
    }

    /// ROOT-CAUSE TEST (observed fault): the hybrid versions of Qwen3 START
    /// generation with `<think>`. That first character is the prefix of no tool
    /// name; the old code killed the constraint ONE-WAY at such a moment and the
    /// real call following the thinking was produced UNCONSTRAINED — in the field
    /// kwargs syntax like `web_search(query:"...")` came out exactly this way.
    #[test]
    fn the_constraint_survives_a_thinking_block() {
        let k = constraint();
        let mut s = k.session();
        feed(&mut s, "<think>The user wants a document.</think>\n\n");
        feed(&mut s, "create_document(");
        assert!(allowed(s.as_ref(), '{'), "did note enter the call grammar");
        assert!(!allowed(s.as_ref(), 'f'), "kwargs syntax (`format:`) can still be produced");
    }

    /// Leading whitespace/newlines MUST NOT KILL the constraint: the model may
    /// start its answer with whitespace, and that is legitimate.
    #[test]
    fn leading_whitespace_does_not_kill_the_constraint() {
        let k = constraint();
        let mut s = k.session();
        feed(&mut s, "\n\n  ");
        feed(&mut s, "create_document(");
        assert!(!allowed(s.as_ref(), 'f'), "the constraint died after leading whitespace");
        feed(&mut s, r#"{"format":"excel","file_name":"x"})"#);
        assert!(s.is_done());
    }

    /// A call that starts in the MIDDLE of plain text is forced too. The model
    /// can open with "Sure, " and write the call afterwards; if the constraint
    /// does not reach that far, it is of no use at all.
    #[test]
    fn a_call_in_the_middle_of_text_is_forced_too() {
        let k = constraint();
        let mut s = k.session();
        feed(&mut s, "Sure, creating it right away: create_document(");
        assert!(!allowed(s.as_ref(), 'f'), "a call in mid-text was left unconstrained");
        assert!(allowed(s.as_ref(), '{'));
    }

    /// The identifier boundary condition: `xcreate_document(` is NOT a tool call,
    /// and the sliding window must not mistake it for one.
    #[test]
    fn without_an_identifier_boundary_it_does_not_count_as_a_call() {
        let k = constraint();
        let mut s = k.session();
        feed(&mut s, "xcreate_document(");
        assert!(allowed(s.as_ref(), 'q'), "a match inside an identifier counted as a call");
    }

    /// When the tool name's prefix stops matching, the constraint falls back to
    /// plain text — a half match must not force a call.
    #[test]
    fn when_the_prefix_breaks_it_stays_free() {
        let k = constraint();
        let mut s = k.session();
        feed(&mut s, "create_docu");
        feed(&mut s, "X");
        assert!(allowed(s.as_ref(), '('), "in free text a parenthesis job free too");
        feed(&mut s, r#"({"format":"zzz"})"#);
        assert!(!s.is_done(), "free text must note close like a call");
    }
}
