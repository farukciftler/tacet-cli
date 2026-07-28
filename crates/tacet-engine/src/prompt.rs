//! Prompt assembly — the ONE place the text sent to the model is produced.
//!
//! The prompt is kept in pieces (system / tool description / history / guide /
//! question) and only joined on a `text()` call. The reason: when the context
//! budget is exceeded the truncation policy has to know which piece may be
//! sacrificed. If the prompt were a single `String`, truncation would amount to
//! "cut from the end" and the first victim would always be the question itself.
//!
//! SKILL DECISION (inherited by the Swift side, taken from measurement): the
//! guide text IS NOT EMBEDDED INTO THE SYSTEM INSTRUCTIONS. Measurement showed
//! that under too much fixed instruction the small on-device model starts
//! EXPLAINING what it would do instead of CALLING the tool. Instead, the ONE
//! skill matching that message is attached to that turn's prompt behind a
//! `<guidance>` fence and capped at `GUIDE_LIMIT` characters. Keeping the guide
//! IMMEDIATELY BEFORE the question is deliberate too: in a small model the last
//! blocks carry the most weight.

use tacet_kernel::ToolCatalog;

/// The wire format of the prompt — which fences the pieces are joined with.
///
/// WHY THE ENGINE DECLARES IT AND THE CLI DOES NOT PICK IT: the template is the
/// format the model was trained on, not a user preference. If it were selectable
/// by hand, picking wrong would silently produce broken output — an
/// instruction-tuned model that does not see its own fences loses the roles and
/// falls into rambling. So `EngineProvider::template` STATES it; the call site
/// only obeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Template {
    /// Tagged plain text. For `FakeEngine` and tests: readable and needs no
    /// tokenizer.
    #[default]
    Plain,
    /// ChatML — the format the Qwen2.5/Qwen3 family was trained on:
    /// `<|im_start|>role\n...<|im_end|>`. The closing `<|im_end|>` is also the
    /// stop token, so this format defines where generation stops as well.
    ChatML,
    /// Gemma — `<start_of_turn>role\n...<end_of_turn>`.
    ///
    /// THIS IS NOT A ONE-TAG CHANGE FROM ChatML; it has two structural
    /// differences, and both are sources of silent breakage:
    ///
    /// 1. **THERE IS NO SYSTEM ROLE.** The Gemma chat template has only `user`
    ///    and `model` turns. Writing `<start_of_turn>system` is a string the
    ///    model has NEVER seen; it steps outside the training distribution and
    ///    the instructions lose their binding force because they turn into
    ///    ordinary text. So the system instructions + tool description + memory
    ///    are embedded at the head of THE FIRST USER TURN.
    /// 2. **THE ASSISTANT ROLE IS NAMED `model`**, not `assistant`.
    ///
    /// The stop token is `<end_of_turn>`; `find_stop_tokens` already looks for
    /// it and finds it, because it is marked special in Gemma's vocabulary.
    Gemma,
}

/// The most characters taken from the guide in a single injection.
///
/// The same number as the Swift side (700). Larger eats the history in a 4096
/// window; smaller was cutting off the concrete `tool(args)` example and the
/// anti-hallucination rules — which is exactly why the guide exists.
pub const GUIDE_LIMIT: usize = 700;

/// The source of a conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    /// The SHORT text of a tool result returned to the model. Bulk data does not
    /// live here (see the DataStore bypass channel).
    Tool,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
        }
    }

    /// The ChatML role name. It MUST be ASCII and ENGLISH: these are strings
    /// that appeared in the model's training data, not part of our own
    /// vocabulary. Renaming them would break the anchor the model recognises.
    ///
    /// THE TOOL ROLE IS ALSO `user` — NOT `tool`. Verified against Qwen3's
    /// official template in the GGUF metadata: in the `message.role == "tool"`
    /// branch the fence written is `<|im_start|>user` and the body is wrapped in
    /// `<tool_response>...</tool_response>`. It used to say `tool` here; the
    /// model saw a role name it had never seen in training, took the tool result
    /// for context-free text and called the tool again.
    fn chatml(self) -> &'static str {
        match self {
            Role::User | Role::Tool => "user",
            Role::Assistant => "assistant",
        }
    }

    /// The Gemma role name. The Gemma template has ONLY two turns: `user` and
    /// `model`. Tool output is written as `user` too — inventing a third role
    /// name would be a fence the model has never seen; a tool result is
    /// information arriving at the model FROM OUTSIDE, i.e. it belongs on the
    /// user side.
    fn gemma(self) -> &'static str {
        match self {
            Role::User | Role::Tool => "user",
            Role::Assistant => "model",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Turn {
    pub role: Role,
    pub text: String,
}

impl Turn {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
        }
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
        }
    }
    pub fn tool(text: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            text: text.into(),
        }
    }

    fn write(&self, target: &mut String) {
        target.push_str(self.role.label());
        target.push_str(": ");
        target.push_str(&self.text);
        target.push('\n');
    }
}

/// The prompt to be sent to the model, in pieces.
#[derive(Debug, Clone, Default)]
pub struct Prompt {
    /// Fixed session instructions. NOT TRUNCATED — identity, language anchor and
    /// the tool-calling contract are here; truncated, the model forgets what it
    /// is.
    pub system: String,
    /// The tool description derived from the catalog. Not truncated: a missing
    /// description means the model invents a signature that does not exist.
    pub tools: String,
    /// Notes from persistent memory matching this message, fenced with
    /// `<memory>`. It sits in the system block — an identity/preference fact is
    /// not a conversation turn but fixed context; truncated like history, the
    /// "the user is vegetarian" fact would vanish on the third turn. The shell
    /// fills it from the `tacet_memory` store according to the message.
    pub memory: Option<String>,
    /// Old turns. This is the FIRST thing sacrificed under budget pressure.
    pub history: Vec<Turn>,
    /// The ONE skill guide matching that message, fenced with `<guidance>`.
    /// Immediately before the question.
    pub guide: Option<String>,
    /// This turn's user question. Truncated but NEVER dropped.
    ///
    /// IT MAY BE LEFT EMPTY and that has a specific meaning: "there is nothing
    /// new to ask, continue from where the history ENDED". The second and later
    /// turns of the tool loop are like this — the question is already in the
    /// history, sitting BEFORE the tool call. If it is not left empty the
    /// question gets written a second time AFTER the tool result and the model
    /// takes it for a new, unanswered request and calls the tool again.
    pub question: String,
}

impl Prompt {
    pub fn new(system: impl Into<String>, question: impl Into<String>) -> Self {
        Self {
            system: system.into(),
            question: question.into(),
            ..Default::default()
        }
    }

    /// Derives the tool description FROM THE CATALOG — there is no second,
    /// hand-written list.
    ///
    /// The order is the catalog's `Vec` order: the same catalog must produce a
    /// bit-identical prompt on every run so eval results stay comparable.
    pub fn with_tools(mut self, catalog: &ToolCatalog) -> Self {
        let mut m = String::new();
        for tool in catalog.tools() {
            // SHORT SIGNATURE, NOT the full JSON Schema (see
            // `ArgSchema::short_signature`): the arguments are already forced by
            // the grammar, so the schema is not kept in two places.
            m.push_str("- ");
            m.push_str(tool.name());
            m.push('(');
            m.push_str(&tool.schema().short_signature());
            m.push_str(") — ");
            m.push_str(tool.description().trim());
            m.push('\n');
        }
        self.tools = m;
        self
    }

    /// Adds memory notes to the prompt. Empty/whitespace text is IGNORED: an
    /// empty `<memory>` fence would give the model the meaningless signal "you
    /// have a memory, but it is empty". The budget was already cut at the store
    /// by `tacet_memory::INJECTION_LIMIT`; we do not cut again here so two
    /// different caps cannot silently collide.
    pub fn with_memory(mut self, notes: impl AsRef<str>) -> Self {
        let n = notes.as_ref().trim();
        self.memory = (!n.is_empty()).then(|| n.to_string());
        self
    }

    /// Adds the guide, CAPPED at `GUIDE_LIMIT` characters.
    ///
    /// WIRED TO PRODUCTION (shell phase): on every turn `tacet-cli` picks the ONE
    /// skill matching the message via `SkillStore::matching` and feeds its
    /// `injection_text` output in here. The 700 limit now binds not only a unit
    /// test but the real production path. Where it is wired:
    /// `Prompt::new(...).with_tools(...).with_guide(chosen_skill)`.
    ///
    /// The cut takes FROM THE FRONT (not the end): skill files are written
    /// "core-first" — the concrete call example and the unbreakable rules sit at
    /// the top. Cutting from the end would drop exactly that core and keep the
    /// human reference; taking from the front does the opposite.
    pub fn with_guide(mut self, guide: impl AsRef<str>) -> Self {
        let g = guide.as_ref();
        let truncated: String = g.chars().take(GUIDE_LIMIT).collect();
        self.guide = (!truncated.trim().is_empty()).then_some(truncated);
        self
    }

    pub fn with_history(mut self, turns: impl IntoIterator<Item = Turn>) -> Self {
        self.history = turns.into_iter().collect();
        self
    }

    /// Joins the pieces into one text. The order is fixed and meaningful:
    /// instructions -> tools -> history -> guide -> question. The question stays
    /// LAST.
    ///
    /// Plain format. If the engine declares a template, `text_with_template` is
    /// used; this method is its `Template::Plain` branch and is what the context
    /// budget measurement (`TokenCounter`) and the CLI's `--show-prompt` output
    /// rest on.
    pub fn text(&self) -> String {
        self.text_with_template(Template::Plain)
    }

    /// Produces the prompt in the wire format of the GIVEN template.
    ///
    /// The piece order is the same INDEPENDENTLY of the template; only the
    /// fences change. That way the truncation policy (which piece is sacrificed
    /// first) stays in one place and adding a template does not affect it.
    pub fn text_with_template(&self, template: Template) -> String {
        match template {
            Template::Plain => self.plain_text(),
            Template::ChatML => self.chatml_text(),
            Template::Gemma => self.gemma_text(),
        }
    }

    /// The Gemma template. The piece ORDER is the same as ChatML (so the
    /// truncation policy stays in one place); what changes is WHERE the system
    /// block goes.
    ///
    /// The system instructions are NOT a separate turn but the head of the first
    /// user turn. Gemma has no system role (see `Template::Gemma`); writing a
    /// separate turn would push the model outside the training distribution.
    fn gemma_text(&self) -> String {
        let mut m = String::with_capacity(self.rough_length());

        // The system block is embedded INSIDE the first user turn.
        m.push_str("<start_of_turn>user\n");
        m.push_str(self.system.trim());
        if !self.tools.trim().is_empty() {
            m.push_str("\n\n<tools>\n");
            m.push_str(self.tools.trim_end());
            m.push_str("\n</tools>");
        }
        if let Some(h) = &self.memory {
            m.push_str("\n\n<memory>\n");
            m.push_str(h.trim());
            m.push_str("\n</memory>");
        }

        // History + question are turned into a SINGLE list of blocks, then
        // CONSECUTIVE IDENTICAL ROLES ARE MERGED.
        //
        // WHY MERGING IS MANDATORY: the Gemma template has to alternate
        // (user/model) and the tool result also falls on the `user` role (see
        // `Role::gemma`). Without merging, "tool result (user) + question (user)"
        // would produce two `user` turns back to back — a sequence absent from
        // the model's training distribution. The system block is part of this
        // list too, since it is the head of the first `user` turn.
        let mut blocks: Vec<(&'static str, String)> = Vec::with_capacity(self.history.len() + 1);
        for t in &self.history {
            let body = if t.role == Role::Tool {
                // `<tool_response>` is NOT A SPECIAL TOKEN in Gemma, it is plain
                // text. It is written anyway: the point is not a special token
                // but MARKING where the tool result starts and ends.
                format!("<tool_response>\n{}\n</tool_response>", t.text.trim())
            } else {
                t.text.trim().to_string()
            };
            blocks.push((t.role.gemma(), body));
        }
        // If the question is empty no closing user block is added (see the
        // `question` field).
        if !self.question.trim().is_empty() {
            let mut question = String::new();
            self.gemma_write_question(&mut question);
            blocks.push(("user", question));
        }

        // If the first block is `user` it sticks to the same turn as a
        // continuation of the system block.
        let mut open: Option<&'static str> = Some("user");
        for (role, body) in blocks {
            match open {
                Some(a) if a == role => m.push_str("\n\n"),
                _ => {
                    if open.is_some() {
                        m.push_str("<end_of_turn>\n");
                    }
                    m.push_str("<start_of_turn>");
                    m.push_str(role);
                    m.push('\n');
                    open = Some(role);
                }
            }
            m.push_str(&body);
        }
        m.push_str("<end_of_turn>\n");

        // The generation anchor.
        m.push_str("<start_of_turn>model\n");
        m
    }

    /// Guide + question. The guide sits IMMEDIATELY BEFORE the question (see the
    /// SKILL DECISION at the head of this file); kept separate so the same order
    /// is not written out twice at two call sites.
    fn gemma_write_question(&self, m: &mut String) {
        if let Some(g) = &self.guide {
            m.push_str("<guidance>\n");
            m.push_str(g.trim());
            m.push_str("\n</guidance>\n");
        }
        m.push_str(self.question.trim());
    }

    /// ChatML: the system instructions and the tool description merge into a
    /// SINGLE system turn.
    ///
    /// WHY MERGED: the tool description is a fixed contract, not a conversation
    /// turn. Written as a separate turn it would mix into the history and, under
    /// budget pressure, truncation could take it for an "old turn" and drop it —
    /// whereas the description must not be truncated (see the `tools` field).
    fn chatml_text(&self) -> String {
        let mut m = String::with_capacity(self.rough_length());

        m.push_str("<|im_start|>system\n");
        m.push_str(self.system.trim());
        if !self.tools.trim().is_empty() {
            m.push_str("\n\n<tools>\n");
            m.push_str(self.tools.trim_end());
            m.push_str("\n</tools>");
        }
        // Memory in the system block: fixed user context, not a conversation
        // turn.
        if let Some(h) = &self.memory {
            m.push_str("\n\n<memory>\n");
            m.push_str(h.trim());
            m.push_str("\n</memory>");
        }
        m.push_str("<|im_end|>\n");

        // TOOL RESULTS are wrapped in `<tool_response>` and CONSECUTIVE ones
        // merge into a SINGLE `user` turn — the behaviour in Qwen3's official
        // template. Merging is not an idle detail: opening a separate user turn
        // for each tool result produces consecutive user turns in the history and
        // shifts the "last question" the model sees.
        let mut i = 0;
        while i < self.history.len() {
            let t = &self.history[i];
            if t.role == Role::Tool {
                m.push_str("<|im_start|>user");
                while i < self.history.len() && self.history[i].role == Role::Tool {
                    m.push_str("\n<tool_response>\n");
                    m.push_str(self.history[i].text.trim());
                    m.push_str("\n</tool_response>");
                    i += 1;
                }
                m.push_str("<|im_end|>\n");
                continue;
            }
            m.push_str("<|im_start|>");
            m.push_str(t.role.chatml());
            m.push('\n');
            m.push_str(t.text.trim());
            m.push_str("<|im_end|>\n");
            i += 1;
        }

        // The guide sits INSIDE the question, immediately before it — as a
        // separate turn a role boundary would come between them and the weight
        // advantage of the last block in a small model (see the SKILL DECISION at
        // the head of this file) would be lost.
        // IF THE QUESTION IS EMPTY the closing user turn IS NEVER OPENED (see the
        // `question` field): the history already ends with a `<tool_response>`
        // user turn and the model must continue straight from there.
        if !self.question.trim().is_empty() {
            m.push_str("<|im_start|>user\n");
            if let Some(g) = &self.guide {
                m.push_str("<guidance>\n");
                m.push_str(g.trim());
                m.push_str("\n</guidance>\n");
            }
            m.push_str(self.question.trim());
            m.push_str("<|im_end|>\n");
        }

        // The generation anchor: the model writes from here on. There is NO
        // trailing `\n` — in ChatML the single newline after the role line was
        // already given here.
        m.push_str("<|im_start|>assistant\n");

        // TRIED AND REVERTED (20 Jul 2026) — do not write
        // `<think>\n\n</think>\n\n` here. That is Qwen3's official
        // `enable_thinking=false` path and it really does turn thinking off, BUT
        // IT BREAKS THE TOOL CALL FORMAT.
        //
        // A 10-question measurement, same pair, this line the only variable:
        //   Qwen3-4B  7/10 -> 2/10        Qwen3-8B  (unmeasured) -> 4/10
        // The models pick the tool NAME correctly but abandon the syntax:
        //   `{ "expression": "1247 * 89" }`   (no name)
        //   `<web_search(query:"...")>`       (invented angle brackets)
        //   `find_file pattern: "report"`     (no parenthesis/JSON)
        // That is, the `tool({...})` pattern dissolves and `ToolCall::parse`
        // counts none of them as a call — the tool never runs at all.
        //
        // Likely cause: the anchor injects a foreign block between the
        // `Example: calculate({"expression":"12*8"})` in the system instructions
        // and the start of generation, weakening the model's ability to carry the
        // pattern. (Same family as the "the example carries weight" finding
        // documented in `session.rs`.)
        //
        // Thinking is now stripped from the output ONLY by `thinking.rs`. The
        // price is real: a hybrid 8B burns tokens while thinking (27.2s -> 13.1s
        // difference measured). But slow+correct beats fast+broken.
        m
    }

    fn plain_text(&self) -> String {
        let mut m = String::with_capacity(self.rough_length());
        m.push_str("<system>\n");
        m.push_str(self.system.trim());
        m.push_str("\n</system>\n");

        if !self.tools.trim().is_empty() {
            m.push_str("<tools>\n");
            m.push_str(self.tools.trim_end());
            m.push_str("\n</tools>\n");
        }

        if let Some(h) = &self.memory {
            m.push_str("<memory>\n");
            m.push_str(h.trim());
            m.push_str("\n</memory>\n");
        }

        if !self.history.is_empty() {
            m.push_str("<history>\n");
            for t in &self.history {
                t.write(&mut m);
            }
            m.push_str("</history>\n");
        }

        if let Some(g) = &self.guide {
            m.push_str("<guidance>\n");
            m.push_str(g.trim());
            m.push_str("\n</guidance>\n");
        }

        // If the question is empty only the generation anchor is written (see the
        // `question` field).
        if !self.question.trim().is_empty() {
            m.push_str("User: ");
            m.push_str(self.question.trim());
            m.push('\n');
        }
        m.push_str("Assistant: ");
        m
    }

    fn rough_length(&self) -> usize {
        self.system.len()
            + self.tools.len()
            + self.question.len()
            + self.memory.as_ref().map_or(0, String::len)
            + self.guide.as_ref().map_or(0, String::len)
            + self
                .history
                .iter()
                .map(|t| t.text.len() + 12)
                .sum::<usize>()
            + 64
    }
}
