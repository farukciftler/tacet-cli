//! ToolExecutor — the SINGLE crossing point between model output and a `Tool`.
//!
//! Swift's `ToolExecutor` held the chip lifecycle; here that job belongs to
//! core's `Reporter`. What is left to this type is narrower and more
//! critical: the decision to ACCEPT a tool call.
//!
//! FIVE GATES, ALL STRUCTURAL:
//!
//! 1. NAME — if it is not in the catalog the tool does not run. The model
//!    cannot run a signature it made up.
//! 2. SCHEMA — the tool never sees arguments that did not pass
//!    `ArgSchema::validate`. The grammar already forces this, but the grammar
//!    can be turned off (eval, CLI, a direct call); the two layers are deliberate.
//! 3. APPROVAL — in a tainted session a call that would push data to the
//!    outside world does not pass without a user decision.
//! 4. CANCELLATION — if the user cancelled the turn no tool even starts.
//! 5. REPEAT — the same (tool, arguments) pair DOES NOT RUN a second time in a
//!    turn. This gate was added later and the reason is plain: until then the
//!    loop ban was held by a single sentence in SYSTEM_INSTRUCTION, and that
//!    sentence cut RECOVERY while cutting the loop — a model that picked the
//!    wrong tool could no longer call the right one (the user had to say
//!    "search the internet"). Once the instruction was relaxed something else
//!    had to hold the loop; trusting text failed silently once in this project.
//!
//! ERROR DETECTION IS NOT DONE WITH TEXT. On the Swift side someone once
//! silently wrote `to_model.contains("tool_failed")`; an unrelated commit that
//!    changed the tool text killed the recovery path and no test broke.
//! Here the decision comes from `ExecutionOutcome::reason` (a closed enum);
//! `is_error()` and `retryable` derive from it, not from text.
//!
//! IF THE WORLD CHANGED THERE IS NO RETRY. Sending the same prompt a second
//! time in a turn that returned `ToolState::Written` creates a second
//! file/record. The flag is TURN SCOPED and STICKY: a recovery attempt cannot
//! drop it, only a real new turn resets it.

use serde_json::Value;
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tacet_kernel::{ERROR_MODEL_TEXT, ToolCatalog, ToolContext, ToolError, ToolOutcome, ToolState};

/// The FIXED text returned to the model when approval is denied.
///
/// Separate from the tool's own error text: this is not a malfunction, it is a
/// decision. So the model does not slip into a "try again" reflex the sentence
/// is final and gives no reason — the rationale belongs to the user and is not
/// opened to the model.
pub const DENIAL_MODEL_TEXT: &str =
    "permission_denied: the user did not allow this data to leave the device";

/// The fixed text returned to the model in a cancelled turn.
pub const CANCEL_MODEL_TEXT: &str = "cancelled: the user stopped this turn";

/// The fixed text returned to the model when the SAME call arrives a second time in a turn.
///
/// It does two things at once: (1) it cuts the loop STRUCTURALLY — the tool
/// DOES NOT RUN a second time, so neither a network nor a file side effect is
/// repeated; (2) it shows the model the way out. The sentence does not say
/// "stop", it says "do SOMETHING ELSE": had it only said "do not call again"
/// the model would fall silent and no answer would reach the user — that was
/// exactly the ugliest face of the observed loop failure.
/// NO CAPITALS — the same register as the neighbouring constants
/// (`DENIAL_MODEL_TEXT`, `CANCEL_MODEL_TEXT`). In this project capitals used
/// for emphasis were MEASURED to be taken as CONTENT by the model (see
/// `SYSTEM_INSTRUCTION` item 3); if the text enters the model's context the
/// same risk applies.
pub const REPEAT_MODEL_TEXT: &str = "duplicate_call: you already made this exact call in this turn and its result is above. \
     Do not repeat it. Either answer the user with what you have, or call a different tool.";

// ---------------------------------------------------------------------------
// Call
// ---------------------------------------------------------------------------

/// A single tool call produced by the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub args: Value,
}

impl ToolCall {
    pub fn new(name: impl Into<String>, args: Value) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }

    /// Parses the call out of the model output; `None` if there is no call.
    ///
    /// TWO FORMATS are accepted because with the grammar on the model produces
    /// canonical JSON, while with the grammar off (free generation, eval, hand
    /// typed CLI input) it more often produces the `name({...})` form. Meeting
    /// both here beats growing a separate "maybe this format too" patch at
    /// every call site.
    ///
    /// Returning `None` IS NOT AN ERROR: the model may have given a plain
    /// answer. That distinction is left to the call site.
    pub fn parse(raw: &str) -> Option<Self> {
        let body = strip_code_fence(raw.trim());

        // Format 1 — canonical: {"tool":"name","args":{...}}
        let parsed_val = serde_json::from_str::<Value>(body)
            .or_else(|_| serde_json::from_str::<Value>(&Self::repair_json(body)));

        if let Ok(Value::Object(object)) = parsed_val {
            let name = object
                .get("tool")
                .or_else(|| object.get("name"))
                .and_then(Value::as_str)?;
            let args = object
                .get("args")
                .or_else(|| object.get("arguments"))
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default()));
            return Some(Self::new(name, args));
        }

        // Format 2 — call form: name({...})
        //
        // THERE MAY BE PLAIN TEXT IN THE PREFIX. The WHOLE text before `(` used
        // to be taken as the tool name; measured on a real model, that turned
        // out to be wrong. Qwen3-4B typically writes its intent first:
        //
        //   "I will run a web search to look up the weather in Istanbul.
        //    web_search({"query":"istanbul weather"})"
        //
        // The old code read the name in that output as "I will ... look up.\n\nweb_search",
        // saw a non-letter character inside it and returned `None` — so even
        // though the model had called the tool CORRECTLY the tool NEVER RAN and
        // the produced call was silently swallowed. The symptom was the most
        // misleading part: it looked like "the model is not calling tools",
        // when in fact it was.
        // The right thing is to take the LAST identifier before `(`. Name
        // validation does not happen here; an unknown name is already rejected
        // at gate 1 (`catalog.find`) and a proper error goes back to the model.
        let closing = body.rfind(')')?;
        // The candidates are tried RIGHT TO LEFT: because constraint production
        // ends at the call's closing parenthesis, the real call is at the END of
        // the text. Parentheses appearing in plain text ("... (mathematics) ...")
        // therefore do not shadow the call.
        for (opening, _) in body[..closing]
            .char_indices()
            .rev()
            .filter(|(_, c)| *c == '(')
        {
            let name = last_identifier(&body[..opening]);
            if name.is_empty() {
                continue;
            }
            let inner = body[opening + 1..closing].trim();
            let args = if inner.is_empty() {
                Value::Object(Default::default())
            } else {
                match serde_json::from_str(inner)
                    .or_else(|_| serde_json::from_str(&Self::repair_json(inner)))
                {
                    Ok(v) => v,
                    // This `(` was not the opening of the call; move to the
                    // candidate on the left.
                    Err(_) => continue,
                }
            };
            return Some(Self::new(name, args));
        }
        None
    }

    /// The one malformation small models produce often enough to be worth
    /// repairing: a trailing comma before a closing brace or bracket.
    ///
    /// DELIBERATELY THE ONLY ONE. Every repair rewrites what the model actually
    /// said before it is parsed, and a rewrite that guesses can turn a call the
    /// model did not make into one it did. A trailing comma is safe to drop
    /// because JSON gives it no meaning at all; anything beyond that (adding a
    /// missing quote, closing an unbalanced brace) requires guessing INTENT and
    /// is not done here.
    ///
    /// STRING-AWARE, and that is the whole difficulty: a comma inside a string
    /// value belongs to the user's text, not to the syntax. `{"q":"a,}"}` must
    /// come out unchanged, escapes included.
    fn repair_json(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let chars: Vec<char> = s.chars().collect();
        let mut in_string = false;
        let mut escape = false;
        let mut i = 0;

        while i < chars.len() {
            let c = chars[i];
            if in_string {
                if escape {
                    escape = false;
                } else if c == '\\' {
                    escape = true;
                } else if c == '"' {
                    in_string = false;
                }
                out.push(c);
                i += 1;
                continue;
            }

            if c == '"' {
                in_string = true;
                out.push(c);
                i += 1;
                continue;
            }

            if c == ',' {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                    i += 1;
                    continue;
                }
            }

            out.push(c);
            i += 1;
        }
        out
    }
}

/// THE RECOVERY LAYER — matches a NAMELESS bare JSON object against a schema.
///
/// WHY IT EXISTS: on a real model (Qwen3-4B) in a multi-turn document flow the
/// model prints `{...}` JSON directly without writing the tool NAME —
/// `{"format":"excel","file_name":"plan"}`. That fits neither Format 1 (NO
/// `tool`/`name` key) nor Format 2 (NO parentheses); the call was silently
/// swallowed and NO file was created. The fields (`format`, `file_name`) match
/// the `create_document` schema EXACTLY — the forgotten name is the only thing missing.
///
/// WHY IT IS SAFE — a COMPLETE and UNIQUE schema match. For a tool to count as a candidate:
///  1. ALL keys of the object must be DEFINED in that tool's schema (NO foreign
///     key — if the model carries a field the schema does not have, it is not that tool),
///  2. ALL required fields of the tool must be PRESENT in the object (and not null),
///  3. the tool must have AT LEAST ONE required field (so an arbitrary object
///     does not appear to "fit" a signature-less tool).
///
/// If EXACTLY ONE tool satisfies these conditions it is recovered; with ZERO or
/// MORE THAN ONE candidate `None` is returned and the input counts as PLAIN
/// TEXT. A wrong match breaks irrelevance; NOT recovering under ambiguity is deliberate.
///
/// AN EXTRA STRICT CONDITION — the whole text (after the fence is stripped)
/// must be a SINGLE JSON object. JSON embedded in prose is not recovered: when
/// the model writes its intent and shows an example JSON beside it, that is NOT
/// a call, because the body on its own cannot be parsed as a valid JSON object.
fn recover_nameless_json(raw: &str, catalog: &ToolCatalog) -> Option<ToolCall> {
    let body = strip_code_fence(raw.trim());
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(body) else {
        return None;
    };
    // An empty object points at no tool.
    if object.is_empty() {
        return None;
    }
    // An object carrying a name key was already Format 1's business; only a
    // NAMELESS object should land here. Guarantee it anyway.
    if ["tool", "name"].iter().any(|k| object.contains_key(*k)) {
        return None;
    }

    // (name, how many of the object's values a CLOSED SET accepted) — the
    // second number is what breaks a tie; see below.
    let mut candidates: Vec<(String, usize)> = Vec::new();
    let mut eliminated = 0usize;
    for tool in catalog.tools() {
        let schema = tool.schema();
        let fields = schema.fields();

        // (1) Is there a foreign key? Every key in the object must be defined in the schema.
        if !object.keys().all(|k| fields.iter().any(|a| &a.name == k)) {
            continue;
        }

        // (2)+(3) Required fields: there must be at least one and all must be present.
        let mut has_required = false;
        let mut required_complete = true;
        for a in fields.iter().filter(|a| a.required) {
            has_required = true;
            if object.get(&a.name).is_none_or(|v| v.is_null()) {
                required_complete = false;
                break;
            }
        }
        if !has_required || !required_complete {
            continue;
        }

        // (4) DOES THE OBJECT SURVIVE THE TOOL'S OWN CLOSED SETS? A field
        // declared as a choice accepts nothing outside it, so a value outside
        // the set is proof this object was never meant for this tool. Counting
        // the ones that DO fit is what tells two candidates apart in (5).
        let mut closed_set_hits = 0usize;
        let mut violates = false;
        for field in fields {
            let Some(value) = object.get(&field.name) else {
                continue;
            };
            if let Some(choices) = field.schema.choices() {
                match value.as_str() {
                    Some(text) if choices.iter().any(|c| c == text) => closed_set_hits += 1,
                    _ => {
                        violates = true;
                        break;
                    }
                }
            }
        }
        if violates {
            // REMEMBER THAT SOMETHING WAS ELIMINATED: it changes what the
            // survivors mean. See (5).
            eliminated += 1;
            continue;
        }

        candidates.push((tool.name().to_string(), closed_set_hits));
    }

    // (5) THE TIE-BREAK, and it was measured before it was written.
    //
    // Qwen3-4B answers "Bugün ayın kaçı?" with a bare `{"kind":"date"}` — the
    // right arguments, no tool name. Two tools accept that object: `time`,
    // whose `kind` is the closed set clock|date|weekday|all|diff, and
    // `calendar`, whose `kind` is free text and therefore accepts anything at
    // all. The old rule saw two candidates, called it ambiguous and recovered
    // nothing, so the case failed in seven runs out of seven while the model
    // had already done its part.
    //
    // A value landing inside a CLOSED SET is evidence; a value landing in a
    // free-text field is not. So the candidate that satisfies more closed sets
    // wins — and only when it is ALONE at the top. Two tools with the same
    // evidence stay ambiguous, which is the old behaviour and the safe one.
    let best = candidates.iter().map(|(_, hits)| *hits).max()?;
    let mut winners = candidates.iter().filter(|(_, hits)| *hits == best);
    let winner = winners.next()?;
    if winners.next().is_some() {
        return None;
    }
    // A SURVIVOR THAT SATISFIED NOTHING, WHILE SOMETHING ELSE WAS ELIMINATED,
    // IS THE WRONG READING. `{"kind":"banana"}` eliminates `time` (its closed
    // set has no such value) and leaves `calendar`, whose `kind` is free text
    // and would take anything — so the object would be recovered into an
    // EFFECTFUL calendar call when what it really looks like is a botched
    // `time` call. Winning by being the last one standing is not evidence.
    if winner.1 == 0 && eliminated > 0 {
        return None;
    }
    Some(ToolCall::new(winner.0.clone(), Value::Object(object)))
}

/// The identity of a call WITHIN A TURN: `name|arguments`.
///
/// Because `serde_json` keeps object keys ordered (BTreeMap), the same
/// arguments written in a different order still produce the same text — so the
/// measure of "sameness" depends on the ARGUMENTS THEMSELVES, not on the order
/// the model wrote them in.
/// IF SERIALIZATION FAILS the key does not collapse to the NAME alone: we fall
/// back to the `Debug` form. The reason is to avoid a silent false match — had
/// we reduced the key to the name, two legitimate calls with different
/// arguments (`read_document` on two different files) would look like a repeat.
/// THE NAME IS THE CANONICAL ONE, NOT THE MODEL'S SPELLING (see `execute`).
fn call_key(name: &str, args: &Value) -> String {
    let text = serde_json::to_string(args).unwrap_or_else(|_| format!("{args:?}"));
    format!("{name}|{text}")
}

/// Returns the identifier at the END of the text (`[A-Za-z0-9_]*`).
///
/// Being ASCII limited is deliberate: tool names are ASCII by contract (see the
/// naming rule — symbols are ASCII). Had we accepted Turkish letters too, the
/// end of a word such as "yapacagim" would look like a valid name and carry
/// needless noise to gate 1.
fn last_identifier(text: &str) -> &str {
    let emit = text
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
        .last()
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    &text[emit..]
}

/// Strips the ```` ```json ... ``` ```` fence. The small model puts it there
/// whether asked to or not; cleaning the fence once here is cheaper than
/// repeating the same check at every call site.
/// THE MARKERS A MODEL INVENTS WHEN IT MEANS "I AM CALLING A TOOL".
///
/// MEASURED, qwen3-4b over the 115-case selection suite: seven of the twenty-two
/// failures were the RIGHT tool with the RIGHT arguments in a shape nobody
/// taught it —
///
/// ```text
/// ```tool
/// read_document"path=report.md"
/// <tool_call>
/// read_document (path: "weekly_meal_list.xlsx")
/// ```
///
/// WHY THESE THREE AND NOTHING LOOSER. The suite also produced
/// `edit_document.new_content="…"` and `remember.list`, and an
/// attribute-shaped recovery is exactly what `recover_nameless_json` refuses to
/// build: scanning the same failures for `word.word` also matches
/// `notes.md file.` and `e.g., exchange rate` out of ordinary prose. A marker is
/// the opposite — no answer to a user contains "<tool_call>" by accident. So the
/// marker is the whole licence to look, and the two attribute cases stay
/// unrecovered on purpose.
/// `\`\`\`json` JOINED THEM AFTER THE 184-CASE RUN, and it is the one marker on
/// this list that a model could plausibly write while NOT calling a tool — a
/// fenced JSON block is ordinary prose furniture. It is safe here for the same
/// reason the others are, which is that the marker is only the licence to look:
/// the block still has to name a catalog tool, every key still has to be a field
/// of that tool's schema, and every required field still has to be present. A
/// block explaining a data shape fails the last two.
///
/// The shape it recovers, measured twice in that run:
///
/// ```text
/// \`\`\`json
/// {"action":"read_document","path":"report.md"}
/// \`\`\`
/// ```
///
/// The envelope key needs no special handling: the name is found INSIDE
/// `"action"`'s value, and the scan for arguments starts after it, so `action`
/// is behind the cursor by the time keys are read.
const CALL_MARKERS: [&str; 4] = ["```tool_code", "```tool", "```json", "<tool_call>"];

/// Recovers a call written behind one of `CALL_MARKERS`.
///
/// THE SAFETY CONDITIONS ARE THE ONES ALREADY ARGUED for nameless JSON, because
/// the risk is identical — a wrong match breaks irrelevance. The name must be a
/// tool in the catalog, every key parsed must exist in THAT tool's schema, and
/// every required field must be present. Anything short of that returns `None`
/// and the text stays an answer.
///
/// THE VALUES ARE READ AS STRINGS AND NOTHING ELSE. A recovered call reaches
/// `execute` and is validated against the schema there like any other, so a
/// field wanting a number rejects `"3"` rather than silently coercing — the
/// recovery layer is not a second, quieter validator.
/// Where `needle` occurs in `haystack` as a WHOLE WORD, if it does.
///
/// A name character is a letter, a digit or `_` — the alphabet tool names are
/// written in. `time` is a word in `"time"` and in `time(`, and is not one in
/// `timeout` or `runtime`. Everything else that surrounds a name in a real call
/// (quotes, colons, braces, parens, whitespace) is a boundary already.
fn find_word(haystack: &str, needle: &str) -> Option<usize> {
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(needle) {
        let at = from + rel;
        let before_ok = haystack[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !is_name(c));
        let after_ok = haystack[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_name(c));
        if before_ok && after_ok {
            return Some(at);
        }
        from = at + needle.len();
    }
    None
}

fn recover_marked_call(raw: &str, catalog: &ToolCatalog) -> Option<ToolCall> {
    let marker = CALL_MARKERS.iter().find(|m| raw.contains(**m))?;
    let after = raw.split_once(*marker)?.1;

    // The name is the catalog tool the text mentions EARLIEST after the marker;
    // a marker with no known name behind it is not a call.
    //
    // A WHOLE WORD, NOT A SUBSTRING, and the difference executed a tool.
    // `after.find(name)` matched `time` inside the key `"timeout"`, so this
    // prose —
    //
    //     Sure. The settings block looks like this:
    //     ```json
    //     {"timeout": 30, "kind": "date"}
    //     ```
    //
    // — recovered as a `time` call and RAN, because `kind` then satisfied that
    // tool's required field. Nothing in the text names a tool; the model was
    // showing an example.
    //
    // EARLIEST IN THE TEXT, NOT FIRST IN THE CATALOG, for the same reason. The
    // old `find_map` took whichever tool the catalog happened to list first, so
    // a `time` buried inside a later word outranked the real name sitting at the
    // front of the JSON. Position in the text is the only ordering the model
    // controls, and the only one that means anything here.
    let (name, rest) = catalog
        .tools()
        .iter()
        .filter_map(|t| {
            let n = t.name();
            find_word(after, n).map(|at| (at, n.to_string(), &after[at + n.len()..]))
        })
        .min_by_key(|(at, _, _)| *at)
        .map(|(_, n, rest)| (n, rest))?;
    let tool = catalog.find(&name)?;
    let schema = tool.schema();
    let fields = schema.fields();

    // `key = value` / `key: value`, the value quoted or bare up to the next
    // delimiter. Only keys the schema already declares are collected, which is
    // what keeps prose after the call from becoming an argument.
    let args = scan_key_values(rest, fields);
    if !fields
        .iter()
        .filter(|f| f.required)
        .all(|f| args.contains_key(&f.name))
    {
        return None;
    }
    Some(ToolCall::new(name, Value::Object(args)))
}

/// THE TOOL NAME GLUED TO ITS FIRST ARGUMENT — a shape only Turkish produced.
///
/// MEASURED over the 184-case suite: three of the eight Turkish failures were
///
/// ```text
/// find_filepattern: "bütçe"
/// find_filepattern: "markdown", folder: "dizin")
/// ```
///
/// The right tool, the right arguments, and no `(` anywhere — so the grammar
/// never armed and `recover_marked_call` finds no marker to look behind. English
/// never produced it in 115 cases; it appeared only once both languages ran,
/// which is the second thing the combined suite bought.
///
/// WHY IT IS SAFE WITHOUT A MARKER — and the first version of this argument was
/// wrong, so it is worth stating carefully.
///
/// It used to say the licence was "a COINCIDENCE that prose cannot arrange": a
/// tool's name immediately followed, with nothing between, by a field name from
/// that same tool's schema. Prose arranges it constantly, because tool names and
/// field names are ordinary English words that compound. MEASURED against the
/// real catalog with no addons installed:
///
/// ```text
/// "Keep the checksumpath=/etc/hosts entry as it is."
///     -> checksum{path: "/etc/hosts entry as it is."}
/// "Our web_searchquery: budget report is the config key."
///     -> web_search{query: "budget report is the config key."}
/// ```
///
/// The second one runs a NETWORK tool from a sentence about a config key. This
/// path is reached on every generation the shell makes, addon-gated or not, and
/// when it fires the user's actual answer is thrown away for a failed chip.
///
/// THE REAL LICENCE IS POSITION. Every case this function was written for —
/// three Turkish failures out of 184, `find_filepattern: "bütçe"` — is the
/// model's answer BEGINNING with the call. None of them is a tool name buried
/// mid-sentence, and prose cannot arrange the compound at the start of a line
/// without it being, to any reader, an attempt at a call. So the name must open
/// the text or open a line; everything after that is the coincidence argument as
/// before, which is sound once it is not being asked to carry the whole weight.
///
/// A WORD BOUNDARY WOULD NOT DO IT. `find_file` is followed by `p` in the shape
/// this exists to catch, so requiring one on the right would delete the feature;
/// requiring one on the left passes every false positive above, since they all
/// have a space in front. Position is the discriminator, not spelling.
/// Where `needle` opens the text or opens a line, if it does.
///
/// Only whitespace may sit between the previous newline and the name. That is
/// what separates a model answering with a call from a sentence that happens to
/// contain a compound word.
fn find_at_line_start(haystack: &str, needle: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(needle) {
        let at = from + rel;
        let line_start = haystack[..at].rfind('\n').map_or(0, |i| i + 1);
        if haystack[line_start..at].chars().all(char::is_whitespace) {
            return Some(at);
        }
        from = at + needle.len();
    }
    None
}

fn recover_glued_call(raw: &str, catalog: &ToolCatalog) -> Option<ToolCall> {
    // Earliest in the TEXT, not first in the catalog — the same ordering bug
    // `recover_marked_call` documents: a name that happens to sit early in the
    // catalog should not outrank one the model actually wrote first.
    let mut best: Option<(usize, &str)> = None;
    for tool in catalog.tools() {
        if let Some(at) = find_at_line_start(raw, tool.name())
            && best.is_none_or(|(b, _)| at < b)
        {
            best = Some((at, tool.name()));
        }
    }
    let (at, name) = best?;
    for tool in catalog.tools() {
        if tool.name() != name {
            continue;
        }
        let schema = tool.schema();
        let fields = schema.fields();
        let tail = &raw[at + name.len()..];

        // The next characters must BE a field name of this tool, with no
        // separator at all. A space, a paren or a quote means this is either a
        // normal call, prose, or someone else's business.
        if !fields.iter().any(|f| tail.starts_with(&f.name)) {
            continue;
        }
        let args = scan_key_values(tail, fields);
        if fields
            .iter()
            .filter(|f| f.required)
            .all(|f| args.contains_key(&f.name))
            && !args.is_empty()
        {
            return Some(ToolCall::new(name, Value::Object(args)));
        }
    }
    None
}

/// `key = value` / `key: value`, quoted or bare, for keys the schema declares.
///
/// SHARED BY BOTH RECOVERIES ON PURPOSE. Written twice they would drift, and the
/// drift would be silent: one shape would start accepting an argument the other
/// rejects, and nothing would fail until a model happened to write it.
fn scan_key_values(text: &str, fields: &[tacet_kernel::Field]) -> serde_json::Map<String, Value> {
    let mut args = serde_json::Map::new();
    for field in fields.iter() {
        // `key=`, `key:`, and the same two with the quote a JSON key closes
        // with — `"path": "report.md"` puts a `"` between the name and the
        // separator, which is why the bare needles missed the fenced-JSON shape
        // this scan is also asked to read.
        for needle in [
            format!("{}=", field.name),
            format!("{}:", field.name),
            format!("{}\"=", field.name),
            format!("{}\":", field.name),
        ] {
            let Some(at) = text.find(&needle) else {
                continue;
            };
            let tail = text[at + needle.len()..].trim_start();
            let value = if let Some(body) = tail.strip_prefix('"') {
                body.split('"').next().unwrap_or("")
            } else {
                tail.split([',', ')', '\n', '"'])
                    .next()
                    .unwrap_or("")
                    .trim()
            };
            if !value.is_empty() {
                args.insert(field.name.clone(), Value::String(value.to_string()));
                break;
            }
        }
    }
    args
}

fn strip_code_fence(raw: &str) -> &str {
    let Some(remaining) = raw.strip_prefix("```") else {
        return raw;
    };
    // The first line is the language tag ("json"), the body starts after it.
    let body = remaining.split_once('\n').map(|(_, g)| g).unwrap_or("");
    body.trim().strip_suffix("```").unwrap_or(body).trim()
}

// ---------------------------------------------------------------------------
// Outcome
// ---------------------------------------------------------------------------

/// The STRUCTURAL outcome of an execution. Call sites make their decisions from
/// this, not from text (see the file header).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionReason {
    /// The tool ran and returned a successful state.
    Ok,
    /// There is no such tool in the catalog.
    UnknownTool,
    /// The arguments did not fit the schema; the tool NEVER ran.
    InvalidArguments,
    /// The tool ran and returned `Failed`.
    ToolFailed,
    /// The approval gate stayed shut; the tool NEVER ran.
    ApprovalDenied,
    /// The turn was cancelled; the tool NEVER ran.
    Cancelled,
    /// The SAME tool with the SAME arguments had already run this turn; the tool NEVER ran.
    RepeatedCall,
}

impl ExecutionReason {
    /// Is this outcome a malfunction. An approval denial and a cancellation ARE
    /// NOT MALFUNCTIONS: both are decisions the user made, and the recovery path
    /// must not be triggered.
    /// `RepeatedCall` DOES NOT COUNT as a malfunction either, and that is
    /// deliberate: were it counted an error, the call site would open a recovery
    /// turn, the model would produce the same call once more, and the very loop
    /// we are trying to prevent would be built — this time with our encouragement.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            ExecutionReason::UnknownTool
                | ExecutionReason::InvalidArguments
                | ExecutionReason::ToolFailed
        )
    }
}

/// The outcome of executing a tool call.
#[derive(Debug, Clone)]
pub struct ExecutionOutcome {
    pub tool_name: String,
    pub reason: ExecutionReason,
    /// The SHORT text returned to the model. Bulk data never goes here (the bypass channel).
    pub to_model: String,
    /// The chip text visible to the user. May be empty (for unimportant calls).
    pub chip_text: String,
    pub state: ToolState,
    /// Did this call change the outside world (`Written`).
    pub world_changed: bool,
    /// May the SAME PROMPT be sent a second time. FALSE once the world changed
    /// within the turn — otherwise a second file/record is created.
    pub retryable: bool,
    /// The tool's raw output (chip detail, eval evidence). It DOES NOT GO to the model.
    pub raw_output: Option<String>,
}

impl ExecutionOutcome {
    pub fn is_error(&self) -> bool {
        self.reason.is_error()
    }
}

// ---------------------------------------------------------------------------
// The approval gate
// ---------------------------------------------------------------------------

/// The sharing request to be shown to the user. `content` is THE VERY SAME
/// ARGUMENTS that will be sent, not a category summary: the user should see
/// exactly what they are permitting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub source: String,
    pub tool_name: String,
    pub content: String,
}

/// The authority that makes the approval decision. A page in the UI, a fixed
/// answer in eval.
/// `&self`: several concurrent calls may poll the same gate; internal
/// synchronization is the concrete implementation's job (the same call as
/// core's `Reporter`).
pub trait ApprovalGate: Send + Sync {
    fn request(&self, request: &ApprovalRequest) -> bool;
}

/// The headless default: there is nobody to ask, so data DOES NOT LEAVE.
///
/// The default being "deny" is deliberate — letting it through without asking
/// would be the same as having no gate at all.
pub struct SilentDeny;

impl ApprovalGate for SilentDeny {
    fn request(&self, _request: &ApprovalRequest) -> bool {
        false
    }
}

/// For tests and "approved" eval scenarios.
pub struct AlwaysApprove;

impl ApprovalGate for AlwaysApprove {
    fn request(&self, _request: &ApprovalRequest) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// The executor
// ---------------------------------------------------------------------------

/// A turn ticket. `new_turn()` returns one, `execute` takes one.
///
/// WHY A TICKET: had a plain `bool` held the cancellation flag, cleaning up a
/// cancelled OLD turn (a late `clear_cancel`) would drop the NEW turn's flag —
/// the user's second "stop" would be swallowed. A ticket is a number and a
/// cancellation is recorded as "the turn with this number is cancelled"; what
/// an old turn said does not bind the new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnTicket(pub u64);

pub struct ToolExecutor {
    catalog: ToolCatalog,
    /// The names of the tools that can push data to the outside world. Core's
    /// `Tool` contract has NO such flag and none was added: "does it push data
    /// out" is not a property of the tool itself but a DEPLOYMENT decision (the
    /// same tool does not push anything out in an offline install). That is why
    /// the list is kept here.
    external_tools: HashSet<String>,
    gate: Box<dyn ApprovalGate>,
    /// Session scoped. Set once a personal-data tool runs SUCCESSFULLY; a turn
    /// change does not clear it (the context summary may carry personal data).
    session_tainted: AtomicBool,
    /// Turn scoped, STICKY.
    world_changed: AtomicBool,
    /// The active turn number.
    turn: AtomicU64,
    /// The cancelled turn number. 0 = nothing cancelled (turns start at 1).
    cancelled: AtomicU64,
    /// The sources denied in this session — the same source is not asked about
    /// a second time, so the model cannot enter an insistence loop.
    denied: Mutex<HashSet<String>>,
    /// The (tool, arguments) pairs that have run in THIS TURN.
    ///
    /// WHY A STRUCTURAL GUARD IS NEEDED: the loop ban was held until now by a
    /// single sentence in SYSTEM_INSTRUCTION, and in this project text-based
    /// rules died silently once. On top of that, that sentence cut recovery
    /// while cutting the loop — a model that picked the wrong tool could no
    /// longer call the right one. Since the instruction has been relaxed, loop
    /// prevention must rest on code, not text: the same call DOES NOT PASS a
    /// second time in a turn, whatever the model writes.
    ///
    /// TURN SCOPED: it is legitimate for the user to ask "what time is it" a
    /// second time; getting the same result twice within one turn is not.
    turn_calls: Mutex<HashSet<String>>,
}

impl ToolExecutor {
    pub fn new(catalog: ToolCatalog) -> Self {
        Self {
            catalog,
            external_tools: HashSet::new(),
            gate: Box::new(SilentDeny),
            session_tainted: AtomicBool::new(false),
            world_changed: AtomicBool::new(false),
            turn: AtomicU64::new(1),
            cancelled: AtomicU64::new(0),
            denied: Mutex::new(HashSet::new()),
            turn_calls: Mutex::new(HashSet::new()),
        }
    }

    /// Marks this tool as "pushes data to the outside world".
    pub fn external_tool(mut self, name: impl Into<String>) -> Self {
        self.external_tools.insert(name.into());
        self
    }

    pub fn with_gate(mut self, gate: impl ApprovalGate + 'static) -> Self {
        self.gate = Box::new(gate);
        self
    }

    pub fn catalog(&self) -> &ToolCatalog {
        &self.catalog
    }

    pub fn session_tainted(&self) -> bool {
        self.session_tainted.load(Ordering::SeqCst)
    }

    /// Re-applies the session taint onto a REBUILT executor. The catalog can be
    /// rebuilt mid-session (an addon toggled from inside the shell); the taint
    /// MUST survive that swap — a fresh executor forgetting it would silently
    /// drop the approval gate for the rest of the session. Only sets, never
    /// clears: clearing taint is `reset_chat`'s job alone.
    pub fn inherit_taint(&self, tainted: bool) {
        if tainted {
            self.session_tainted.store(true, Ordering::SeqCst);
        }
    }

    pub fn world_changed(&self) -> bool {
        self.world_changed.load(Ordering::SeqCst)
    }

    /// May the same prompt be sent a second time — the SINGLE decision point of
    /// the recovery path.
    pub fn retry_safe(&self) -> bool {
        !self.world_changed()
    }

    pub fn active_turn(&self) -> TurnTicket {
        TurnTicket(self.turn.load(Ordering::SeqCst))
    }

    /// A real new turn: the side-effect flag is reset, a new ticket is produced.
    pub fn new_turn(&self) -> TurnTicket {
        self.world_changed.store(false, Ordering::SeqCst);
        self.turn_calls.lock().expect("turn calls lock").clear();
        TurnTicket(self.turn.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// A recovery attempt — NOT a real new turn. The ticket, the side-effect
    /// flag and the TURN CALLS are preserved: whether a side effect happened
    /// must not be lost exactly at the moment the retry decision is made; in the
    /// same way a recovery turn must not become an excuse to repeat a failed call verbatim.
    pub fn recovery_attempt(&self) -> TurnTicket {
        self.active_turn()
    }

    /// Cancels the active turn.
    pub fn cancel(&self) {
        self.cancelled
            .store(self.turn.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    pub fn is_cancelled(&self, ticket: TurnTicket) -> bool {
        self.cancelled.load(Ordering::SeqCst) == ticket.0
    }

    /// End of session — everything ends, including the taint and the denial cache.
    pub fn reset_chat(&self) {
        self.session_tainted.store(false, Ordering::SeqCst);
        self.world_changed.store(false, Ordering::SeqCst);
        self.denied.lock().expect("denial cache lock").clear();
        self.turn_calls.lock().expect("turn calls lock").clear();
        self.turn.fetch_add(1, Ordering::SeqCst);
    }

    /// Parses the call out of raw model output and executes it.
    ///
    /// `None` = the model did not call a tool (a plain answer). That is not an error.
    ///
    /// The two canonical formats are tried FIRST; if neither holds, the RECOVERY
    /// layer (matching nameless JSON against a schema, see
    /// `recover_nameless_json`) is tried. Recovery sees the catalog, so it lives
    /// here, outside `parse` — `parse` only takes a string and is responsible
    /// for the two canonical formats.
    pub async fn execute_raw(
        &self,
        raw: &str,
        ticket: TurnTicket,
        ctx: &mut ToolContext,
    ) -> Option<ExecutionOutcome> {
        let call = ToolCall::parse(raw)
            .or_else(|| recover_nameless_json(raw, &self.catalog))
            .or_else(|| recover_marked_call(raw, &self.catalog))
            .or_else(|| recover_glued_call(raw, &self.catalog))?;
        Some(self.execute(&call, ticket, ctx).await)
    }

    /// Runs a single tool call through the gates.
    ///
    /// It DOES NOT RETURN a `Result` (the same rationale as core decision 4):
    /// every path produces an `ExecutionOutcome`, so call sites cannot find an
    /// option called "forget the error path".
    pub async fn execute(
        &self,
        call: &ToolCall,
        ticket: TurnTicket,
        ctx: &mut ToolContext,
    ) -> ExecutionOutcome {
        // GATE 4 — cancellation. First of all: in a cancelled turn no tool
        // should start, not even a cheap one.
        if self.is_cancelled(ticket) {
            return self.outcome(
                &call.name,
                ExecutionReason::Cancelled,
                CANCEL_MODEL_TEXT.to_string(),
                "Cancelled".to_string(),
                ToolState::Failed("cancelled".into()),
                None,
                false,
            );
        }

        // GATE 1 — the name.
        let Some(tool) = self.catalog.find(&call.name) else {
            let error = ToolError::InvalidArgument(format!("unknown tool: {}", call.name));
            return self.outcome(
                &call.name,
                ExecutionReason::UnknownTool,
                ERROR_MODEL_TEXT.to_string(),
                error.short_error(),
                ToolState::Failed(error.short_error()),
                None,
                false,
            );
        };

        // FROM HERE ON THE CATALOG'S SPELLING IS AUTHORITATIVE, NEVER THE MODEL'S.
        //
        // `ToolCatalog::find` forgives ASCII case ON PURPOSE — it is a SPELLING
        // gate and a model that writes `Web_ara(...)` should still be understood.
        // But every gate below matches a `String` EXACTLY: `external_tools`
        // (gate 3), `turn_calls` (gate 5) and the denial cache. Feeding them the
        // model's spelling turned that relaxation into an APPROVAL BYPASS:
        // `Web_fetch` resolved to `web_fetch` at gate 1 and then MISSED gate 3,
        // so in a tainted session data left the device with no question asked —
        // measured end to end, the receiving server really saw the request. A
        // poisoned MCP tool description is enough to make the model capitalise a
        // name. Canonicalising once, here, closes it for every gate at the same time.
        let name = tool.name().to_string();

        // GATE 2 — the schema. The tool validates internally too; this layer
        // guarantees the tool NEVER RUNS (which matters for tools with side effects).
        if let Err(error) = tool.schema().validate(&call.args) {
            return self.outcome(
                &call.name,
                ExecutionReason::InvalidArguments,
                ERROR_MODEL_TEXT.to_string(),
                error.short_error(),
                ToolState::Failed(error.short_error()),
                None,
                false,
            );
        }

        // GATE 5 — REPEAT. AFTER the name and the schema: a malformed call
        // should first learn that it is malformed, not "you already did that".
        //
        // THE QUERY IS HERE, THE RECORD IS AFTER APPROVAL — the two were split deliberately:
        //
        //  * The query must come BEFORE approval, otherwise a repeat of an
        //    approved call would ask the user for approval A SECOND TIME. Someone
        //    who opened the gate once must not be disturbed again just because
        //    the model reproduced the same request.
        //  * The record must come AFTER approval, otherwise a DENIED call is
        //    marked "done" and its repeat tells the model "duplicate_call". The
        //    right answer is "permission_denied": the model must know it was
        //    denied, not that it repeated itself.
        let key = call_key(&name, &call.args);
        if self
            .turn_calls
            .lock()
            .expect("turn calls lock")
            .contains(&key)
        {
            return self.outcome(
                &name,
                ExecutionReason::RepeatedCall,
                REPEAT_MODEL_TEXT.to_string(),
                // THE CHIP TEXT IS EMPTY: the user must not see this as an
                // "operation". The tool did not run, nothing happened in the
                // world; showing a second line on screen would present work that
                // was never done as if it had been.
                String::new(),
                ToolState::Read,
                None,
                false,
            );
        }

        // GATE 3 — approval. Only in a tainted session and only for external
        // tools. It is not asked in a clean session: what is rare gets read,
        // what is frequent stops being read and the gate loses its function.
        if self.external_tools.contains(&name) && self.session_tainted() {
            let content = serde_json::to_string(&call.args).unwrap_or_default();
            let request = ApprovalRequest {
                source: name.clone(),
                tool_name: name.clone(),
                content,
            };
            if !self.ask_approval(&request) {
                let trace = ctx.start_chip("approval", &format!("{} · not sent", request.source));
                ctx.update_chip(
                    trace,
                    tacet_kernel::TraceUpdate::state(ToolState::NeedsPermission)
                        .raw_input(request.content.clone()),
                );
                return self.outcome(
                    &name,
                    ExecutionReason::ApprovalDenied,
                    DENIAL_MODEL_TEXT.to_string(),
                    format!("{} · not sent", request.source),
                    ToolState::NeedsPermission,
                    None,
                    false,
                );
            }
        }

        // The record half of gate 5: the call really runs FROM HERE on.
        self.turn_calls.lock().expect("turn calls lock").insert(key);

        let outcome: ToolOutcome = tool.run(call.args.clone(), ctx).await;

        // The cancellation may have arrived WHILE the tool was running. We still
        // return the outcome, but if a write happened the flag is preserved —
        // "I cancelled it and the file was created anyway" must not be hidden.
        let world = outcome.state.changed_world();
        if world {
            self.world_changed.store(true, Ordering::SeqCst);
            ctx.taint();
        }

        // TAINTING: only Read/Written. NeedsPermission and Failed DO NOT TAINT
        // — no data was read, and tainting the session would tighten the gate
        // for no reason.
        let succeeded = matches!(outcome.state, ToolState::Read | ToolState::Written);
        if succeeded && tool.taints_session() {
            self.session_tainted.store(true, Ordering::SeqCst);
            ctx.taint();
        }

        let reason = match &outcome.state {
            ToolState::Failed(_) => ExecutionReason::ToolFailed,
            ToolState::NeedsPermission => ExecutionReason::ApprovalDenied,
            _ => ExecutionReason::Ok,
        };

        self.outcome(
            &name,
            reason,
            outcome.to_model,
            outcome.chip_text,
            outcome.state,
            outcome.raw_output,
            world,
        )
    }

    fn ask_approval(&self, request: &ApprovalRequest) -> bool {
        {
            let denial = self.denied.lock().expect("denial cache lock");
            if denial.contains(&request.source) {
                return false;
            }
        }
        let accept = self.gate.request(request);
        if !accept {
            self.denied
                .lock()
                .expect("denial cache lock")
                .insert(request.source.clone());
        }
        accept
    }

    #[allow(clippy::too_many_arguments)]
    fn outcome(
        &self,
        name: &str,
        reason: ExecutionReason,
        to_model: String,
        chip_text: String,
        state: ToolState,
        raw_output: Option<String>,
        world_changed: bool,
    ) -> ExecutionOutcome {
        ExecutionOutcome {
            tool_name: name.to_string(),
            reason,
            to_model,
            chip_text,
            state,
            world_changed,
            // It is checked at TURN level, not at this call's level: if a file
            // was written in an earlier step of the turn, this call's innocence
            // does not make a retry safe.
            retryable: !self.world_changed(),
            raw_output,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tacet_kernel::{
        ArgSchema, Field, InMemoryDataStore, SilentReporter, Tool, ToolFuture, boxed,
    };

    // --- Helpers ---

    /// A tool that reads personal data: it taints the session.
    struct PersonalTool;
    impl Tool for PersonalTool {
        fn name(&self) -> &str {
            "personal_read"
        }
        fn description(&self) -> &str {
            "Reads personal data."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![Field::new("what", ArgSchema::text()).required()])
        }
        fn taints_session(&self) -> bool {
            true
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::read_ok("read", "data: 42") })
        }
    }

    /// A tool that sends data out.
    struct ExternalTool;
    impl Tool for ExternalTool {
        fn name(&self) -> &str {
            "send_out"
        }
        fn description(&self) -> &str {
            "Sends the data out."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![Field::new("body", ArgSchema::text()).required()])
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::read_ok("sent", "sent") })
        }
    }

    /// A tool that changes the world.
    struct WritingTool;
    impl Tool for WritingTool {
        fn name(&self) -> &str {
            "write_file"
        }
        fn description(&self) -> &str {
            "Writes a file."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![])
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::written("written", "file_created") })
        }
    }

    /// A tool that always fails.
    struct CrashingTool;
    impl Tool for CrashingTool {
        fn name(&self) -> &str {
            "crashing"
        }
        fn description(&self) -> &str {
            "Always crashes."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![])
        }
        fn taints_session(&self) -> bool {
            true
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            boxed(async move { ToolOutcome::failed(&ToolError::OutOfSpace) })
        }
    }

    fn context() -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            std::env::temp_dir(),
            Arc::new(SilentReporter),
        )
    }

    fn executor() -> ToolExecutor {
        let mut k = ToolCatalog::new();
        k.add(Arc::new(PersonalTool))
            .add(Arc::new(ExternalTool))
            .add(Arc::new(WritingTool))
            .add(Arc::new(CrashingTool));
        ToolExecutor::new(k).external_tool("send_out")
    }

    /// A minimal poll loop — this crate MUST NOT PICK an executor (the same
    /// rationale as in core), so there is no tokio in the test either.
    fn run<F: Future<Output = T>, T>(f: F) -> T {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn empty(_: *const ()) {}
        fn clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(clone, empty, empty, empty);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = std::pin::pin!(f);
        loop {
            if let Poll::Ready(v) = std::pin::Pin::as_mut(&mut f).poll(&mut cx) {
                return v;
            }
        }
    }

    use std::future::Future;

    // --- Parsing ---

    /// THE CASE THAT PAID FOR THE TIE-BREAK, with the real catalog.
    ///
    /// Qwen3-4B answers "Bugün ayın kaçı?" with a bare `{"kind":"date"}` — the
    /// right arguments and no tool name. Both `time` and `calendar` declare a
    /// required `kind`, but only `time` declares it as a closed set that
    /// contains "date"; `calendar` takes free text and would accept anything.
    /// The old rule counted two candidates, called it ambiguous and recovered
    /// nothing, and the case failed in seven runs out of seven.
    /// THE REPAIR MUST NOT INVENT A CALL.
    ///
    /// It was added without a test; these are the cases that decide whether a
    /// rewrite of the model's own words is safe. A comma inside a STRING is the
    /// user's text and must survive untouched — dropping one there would change
    /// the arguments a tool is called with, silently.
    #[test]
    fn the_json_repair_drops_only_a_meaningless_comma() {
        // What it exists for.
        assert_eq!(
            ToolCall::repair_json(r#"{"path":"src/main.rs",}"#),
            r#"{"path":"src/main.rs"}"#
        );
        assert_eq!(ToolCall::repair_json(r#"{"a":[1,2,],}"#), r#"{"a":[1,2]}"#);
        // Whitespace and newlines between the comma and the brace.
        assert_eq!(ToolCall::repair_json("{\"a\":1,\n}"), "{\"a\":1\n}");

        // WHAT IT MUST NOT TOUCH.
        for untouched in [
            r#"{"q":"a,}"}"#,               // a comma before `}` INSIDE a string
            r#"{"q":"a,]"}"#,               // and before `]`
            r#"{"q":"say \", }\" twice"}"#, // an escaped quote, then the pattern
            r#"{"a":1,"b":2}"#,             // an ordinary separating comma
            r#"{"a":[1,2],"b":3}"#,
            r#"{}"#,
        ] {
            assert_eq!(
                ToolCall::repair_json(untouched),
                untouched,
                "the repair changed something it had no business changing"
            );
        }

        // AND IT MUST NOT CHANGE A CALL THAT ALREADY PARSES. Valid JSON in,
        // identical JSON out — the repair is only ever reached after a parse
        // failure, but nothing enforces that from inside, so it is checked here.
        for valid in [
            r#"{"tool":"time","args":{"kind":"date"}}"#,
            r#"{"expression":"12*8","digits":2}"#,
        ] {
            assert_eq!(ToolCall::repair_json(valid), valid);
            assert!(serde_json::from_str::<Value>(&ToolCall::repair_json(valid)).is_ok());
        }
    }

    /// And the end-to-end path: a call a small model would botch now runs.
    #[test]
    fn a_call_with_a_trailing_comma_is_still_a_call() {
        let call = ToolCall::parse(r#"web_search({"query":"istanbul weather",})"#).expect("parsed");
        assert_eq!(call.name, "web_search");
        assert_eq!(call.args["query"], "istanbul weather");

        let canonical = ToolCall::parse(r#"{"tool":"calculate","args":{"expression":"2+2",},}"#)
            .expect("parsed");
        assert_eq!(canonical.name, "calculate");
        assert_eq!(canonical.args["expression"], "2+2");
    }

    #[test]
    fn a_nameless_object_goes_to_the_tool_whose_closed_set_accepts_it() {
        let store = std::sync::Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (catalog, _, _) = crate::catalog::production_catalog(&store, &memory, Some(0));

        // The recovery is reached through the executor's call path, not through
        // `parse` — a nameless object is not a parse, it is a rescue.
        let call = recover_nameless_json(r#"{ "kind": "date" }"#, &catalog).expect("recovered");
        assert_eq!(call.name, "time");
        assert_eq!(call.args["kind"], "date");

        // A value OUTSIDE the closed set is proof the object was not meant for
        // that tool at all. With `calendar` present and taking free text, the
        // recovery must not simply fall back to it.
        assert!(
            recover_nameless_json(r#"{"kind":"banana"}"#, &catalog).is_none(),
            "a value no closed set accepts must not be handed to whichever tool \
             happens to take free text"
        );

        // AND AMBIGUITY IS STILL AMBIGUITY: two tools with equally good evidence
        // recover nothing, which is the old behaviour and the safe one.
        let ambiguous = recover_nameless_json(r#"{"path":"a.md"}"#, &catalog);
        assert!(
            ambiguous.is_none()
                || ambiguous.as_ref().map(|c| c.name.as_str()) == Some("read_document"),
            "an object matching several free-text tools must not pick one at random: {ambiguous:?}"
        );
    }

    #[test]
    fn canonical_json_is_parsed() {
        let c = ToolCall::parse(r#"{"tool":"calculate","args":{"expression":"2+2"}}"#).unwrap();
        assert_eq!(c.name, "calculate");
        assert_eq!(c.args["expression"], "2+2");
    }

    #[test]
    fn the_call_format_is_parsed() {
        let c = ToolCall::parse(r#"calculate({"expression":"2+2"})"#).unwrap();
        assert_eq!(c.name, "calculate");
        assert_eq!(c.args["expression"], "2+2");
    }

    #[test]
    fn the_code_fence_is_stripped() {
        let c = ToolCall::parse("```json\n{\"tool\":\"time\",\"args\":{}}\n```").unwrap();
        assert_eq!(c.name, "time");
    }

    #[test]
    fn a_plain_answer_is_not_a_call() {
        assert!(ToolCall::parse("Hello, how can I help you?").is_none());
        assert!(ToolCall::parse("").is_none());
    }

    /// REAL MODEL OUTPUT (Qwen3-4B, verbatim shape; the model's preamble was
    /// Turkish in the captured run and is rendered in English here). The model
    /// writes its intent first and produces the call after it; the old parser
    /// swallowed this silently.
    #[test]
    fn a_call_with_a_plain_text_prefix_is_parsed() {
        let raw = "I will run a web search to look up the weather in Istanbul.\n\n\
                   web_search({\"query\":\"istanbul weather\"})";
        let c = ToolCall::parse(raw).expect("the prefixed call could not be parsed");
        assert_eq!(c.name, "web_search");
        assert_eq!(c.args["query"], "istanbul weather");
    }

    /// Parentheses in the prefix must not shadow the call: candidates are tried FROM THE RIGHT.
    #[test]
    fn a_parenthesis_in_the_prefix_does_not_shadow_the_call() {
        let raw =
            "First I need a calculation (simple arithmetic): calculate({\"expression\":\"2+2\"})";
        let c = ToolCall::parse(raw).expect("the call was not found");
        assert_eq!(c.name, "calculate");
        assert_eq!(c.args["expression"], "2+2");
    }

    /// There is a parenthesis but no identifier BEFORE it — that is not a call.
    #[test]
    fn a_parenthesis_without_an_identifier_does_not_count_as_a_call() {
        assert!(ToolCall::parse("This is an explanation (in parentheses).").is_none());
        assert!(ToolCall::parse("({\"a\":1})").is_none());
    }

    // --- Recovery (nameless JSON -> schema match) ---

    /// A REAL FAILURE: the model forgets the name and prints a bare `{...}`.
    /// `personal_read` makes 'what' required; the object carries only that -> a
    /// UNIQUE and clear match.
    #[test]
    fn nameless_json_is_recovered_when_it_fits_exactly_one_schema() {
        let y = executor();
        let c = recover_nameless_json(r#"{"what":"calendar"}"#, y.catalog())
            .expect("must be recovered");
        assert_eq!(c.name, "personal_read");
        assert_eq!(c.args["what"], "calendar");
    }

    /// Nameless JSON wrapped in a code fence must be recovered too (the small model adds the fence).
    #[test]
    fn nameless_json_is_recovered_even_inside_a_code_fence() {
        let y = executor();
        let c = recover_nameless_json("```json\n{\"body\":\"hello\"}\n```", y.catalog())
            .expect("must be recovered");
        assert_eq!(c.name, "send_out");
        assert_eq!(c.args["body"], "hello");
    }

    /// If a required field is missing NO recovery happens: there is no 'what',
    /// and 'title' is not required by any tool -> no candidate.
    #[test]
    fn a_missing_required_field_is_not_recovered() {
        let y = executor();
        assert!(recover_nameless_json(r#"{"title":"x"}"#, y.catalog()).is_none());
    }

    /// A foreign key breaks the match: 'what' belongs to personal_read, 'body'
    /// to send_out; together they fit NO single schema.
    #[test]
    fn a_foreign_key_is_not_recovered() {
        let y = executor();
        assert!(recover_nameless_json(r#"{"ne":"x","body":"y"}"#, y.catalog()).is_none());
    }

    /// An empty object, plain text and JSON inside prose: none of them count as
    /// a call (irrelevance is preserved).
    #[test]
    fn an_empty_object_plain_text_and_embedded_json_are_not_recovered() {
        let y = executor();
        assert!(recover_nameless_json("{}", y.catalog()).is_none());
        assert!(recover_nameless_json("Hello, how are you?", y.catalog()).is_none());
        assert!(recover_nameless_json("Thanks!", y.catalog()).is_none());
        // The whole text is NOT a single JSON object -> unparseable -> rejected.
        assert!(
            recover_nameless_json(
                "Example: {\"what\":\"x\"} is written like this.",
                y.catalog()
            )
            .is_none()
        );
    }

    /// If two tools share the same single required field the match is AMBIGUOUS -> rejected.
    #[test]
    fn an_ambiguous_match_is_not_recovered() {
        struct A;
        impl Tool for A {
            fn name(&self) -> &str {
                "tool_a"
            }
            fn description(&self) -> &str {
                "a"
            }
            fn schema(&self) -> ArgSchema {
                ArgSchema::object(vec![Field::new("x", ArgSchema::text()).required()])
            }
            fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
                boxed(async move { ToolOutcome::read_ok("a", "a") })
            }
        }
        struct B;
        impl Tool for B {
            fn name(&self) -> &str {
                "tool_b"
            }
            fn description(&self) -> &str {
                "b"
            }
            fn schema(&self) -> ArgSchema {
                ArgSchema::object(vec![Field::new("x", ArgSchema::text()).required()])
            }
            fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
                boxed(async move { ToolOutcome::read_ok("b", "b") })
            }
        }
        let mut k = ToolCatalog::new();
        k.add(Arc::new(A)).add(Arc::new(B));
        assert!(recover_nameless_json(r#"{"x":"1"}"#, &k).is_none());
    }

    /// End to end: nameless JSON really runs the tool when it goes through `execute_raw`.
    #[test]
    fn nameless_json_runs_the_tool_when_it_goes_through_execute_raw() {
        let y = executor();
        let mut ctx = context();
        let s = run(y.execute_raw(r#"{"what":"calendar"}"#, y.active_turn(), &mut ctx))
            .expect("must be recovered and run");
        assert_eq!(s.reason, ExecutionReason::Ok);
        assert_eq!(s.to_model, "data: 42");
    }

    // --- The gates ---

    #[test]
    fn an_unknown_tool_does_not_run_and_returns_the_fixed_text() {
        let y = executor();
        let mut ctx = context();
        let s = run(y.execute(
            &ToolCall::new("yok_boyle", serde_json::json!({})),
            y.active_turn(),
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::UnknownTool);
        assert!(s.is_error());
        assert_eq!(s.to_model, ERROR_MODEL_TEXT);
    }

    #[test]
    fn on_a_schema_violation_the_tool_never_runs() {
        let y = executor();
        let mut ctx = context();
        // "what" is required; it is not given.
        let s = run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({})),
            y.active_turn(),
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::InvalidArguments);
        // Since the tool did not run the session was not tainted either.
        assert!(!y.session_tainted());
    }

    #[test]
    fn a_successful_personal_tool_taints_the_session() {
        let y = executor();
        let mut ctx = context();
        assert!(!y.session_tainted());
        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "calendar"})),
            y.active_turn(),
            &mut ctx,
        ));
        assert!(y.session_tainted());
        assert!(ctx.session_tainted());
    }

    #[test]
    fn a_failed_tool_does_not_taint_the_session() {
        // CrashingTool has taints_session()==true but returns Failed: no data
        // was read, so there is no reason to tighten the gate.
        let y = executor();
        let mut ctx = context();
        let s = run(y.execute(
            &ToolCall::new("crashing", serde_json::json!({})),
            y.active_turn(),
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::ToolFailed);
        assert!(!y.session_tainted());
    }

    #[test]
    fn in_a_clean_session_an_external_tool_passes_without_asking() {
        let y = executor(); // the default gate: SilentDeny
        let mut ctx = context();
        let s = run(y.execute(
            &ToolCall::new("send_out", serde_json::json!({"body": "hello"})),
            y.active_turn(),
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::Ok);
    }

    #[test]
    fn in_a_tainted_session_an_external_tool_hits_the_gate() {
        let y = executor();
        let mut ctx = context();
        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "calendar"})),
            y.active_turn(),
            &mut ctx,
        ));
        let s = run(y.execute(
            &ToolCall::new("send_out", serde_json::json!({"body": "data: 42"})),
            y.active_turn(),
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::ApprovalDenied);
        assert_eq!(s.to_model, DENIAL_MODEL_TEXT);
        // A denial is NOT a malfunction: the recovery path must not be triggered.
        assert!(!s.is_error());
    }

    #[test]
    fn if_approval_is_given_it_passes_even_in_a_tainted_session() {
        let mut k = ToolCatalog::new();
        k.add(Arc::new(PersonalTool)).add(Arc::new(ExternalTool));
        let y = ToolExecutor::new(k)
            .external_tool("send_out")
            .with_gate(AlwaysApprove);
        let mut ctx = context();
        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        let s = run(y.execute(
            &ToolCall::new("send_out", serde_json::json!({"body": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::Ok);
    }

    #[test]
    fn a_source_denied_once_is_never_asked_again() {
        /// A gate that counts HOW MANY TIMES IT WAS ASKED — the only evidence of the denial cache.
        struct CountingGate(Arc<std::sync::atomic::AtomicUsize>);
        impl ApprovalGate for CountingGate {
            fn request(&self, _i: &ApprovalRequest) -> bool {
                self.0.fetch_add(1, Ordering::SeqCst);
                false
            }
        }
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let mut k = ToolCatalog::new();
        k.add(Arc::new(PersonalTool)).add(Arc::new(ExternalTool));
        let y = ToolExecutor::new(k)
            .external_tool("send_out")
            .with_gate(CountingGate(Arc::clone(&counter)));
        let mut ctx = context();
        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        for _ in 0..3 {
            run(y.execute(
                &ToolCall::new("send_out", serde_json::json!({"body": "x"})),
                y.active_turn(),
                &mut ctx,
            ));
        }
        // Three attempts, ONE question: the model cannot enter an insistence loop.
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    /// A counting version of the external tool: only "the tool did not run"
    /// proves the gate held. `Ok`/`Err` on the outcome would not — a tool that
    /// ran and then failed looks the same from outside.
    struct CountingExternal(Arc<std::sync::atomic::AtomicUsize>);
    impl Tool for CountingExternal {
        fn name(&self) -> &str {
            "send_out"
        }
        fn description(&self) -> &str {
            "Sends the data out."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![Field::new("body", ArgSchema::text()).required()])
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            self.0.fetch_add(1, Ordering::SeqCst);
            boxed(async move { ToolOutcome::read_ok("sent", "sent") })
        }
    }

    /// THE APPROVAL BYPASS. `ToolCatalog::find` forgives ASCII case on purpose,
    /// but `external_tools` matched the model's spelling EXACTLY — so
    /// `Web_fetch`/`Send_out` resolved at gate 1 and then MISSED gate 3, and in a
    /// tainted session the data left the device with no question asked (measured
    /// end to end: the receiving server really saw the request). A poisoned MCP
    /// tool description is enough to make the model capitalise a name.
    #[test]
    fn a_capitalised_external_tool_still_hits_the_approval_gate() {
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut k = ToolCatalog::new();
        k.add(Arc::new(PersonalTool))
            .add(Arc::new(CountingExternal(Arc::clone(&counter))));
        let y = ToolExecutor::new(k).external_tool("send_out");
        let mut ctx = context();

        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "calendar"})),
            y.active_turn(),
            &mut ctx,
        ));
        for spelling in ["Send_out", "SEND_OUT", "sEnD_OuT"] {
            let s = run(y.execute(
                &ToolCall::new(spelling, serde_json::json!({"body": "data: 42"})),
                y.new_turn(),
                &mut ctx,
            ));
            assert_eq!(
                s.reason,
                ExecutionReason::ApprovalDenied,
                "the gate was skipped for {spelling}"
            );
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "THE TOOL REALLY RAN — data left the device without a question"
        );
    }

    /// The denial cache is keyed by name too: a case variant must not be a fresh
    /// question, or the model gets an insistence loop for free.
    #[test]
    fn a_case_variant_does_not_reopen_a_denied_source() {
        struct CountingGate(Arc<std::sync::atomic::AtomicUsize>);
        impl ApprovalGate for CountingGate {
            fn request(&self, _i: &ApprovalRequest) -> bool {
                self.0.fetch_add(1, Ordering::SeqCst);
                false
            }
        }
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut k = ToolCatalog::new();
        k.add(Arc::new(PersonalTool)).add(Arc::new(ExternalTool));
        let y = ToolExecutor::new(k)
            .external_tool("send_out")
            .with_gate(CountingGate(Arc::clone(&asked)));
        let mut ctx = context();

        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        for spelling in ["send_out", "SEND_OUT", "Send_Out"] {
            run(y.execute(
                &ToolCall::new(spelling, serde_json::json!({"body": "x"})),
                y.new_turn(),
                &mut ctx,
            ));
        }
        assert_eq!(
            asked.load(Ordering::SeqCst),
            1,
            "a case variant asked the user again"
        );
    }

    /// Gate 5 is keyed by name too: rewriting the name in another case is not a
    /// new call, it is the same call.
    #[test]
    fn a_case_variant_is_still_a_repeated_call() {
        let (y, counter) = counting();
        let mut ctx = context();
        let ticket = y.new_turn();

        let first = run(y.execute(
            &ToolCall::new("say", serde_json::json!({"x": "a"})),
            ticket,
            &mut ctx,
        ));
        assert_eq!(first.reason, ExecutionReason::Ok);

        let second = run(y.execute(
            &ToolCall::new("Say", serde_json::json!({"x": "a"})),
            ticket,
            &mut ctx,
        ));
        assert_eq!(second.reason, ExecutionReason::RepeatedCall);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "the case variant ran the tool a second time"
        );
    }

    // --- Side effect / retry ---

    #[test]
    fn once_the_world_changes_retry_is_forbidden() {
        let y = executor();
        let mut ctx = context();
        assert!(y.retry_safe());
        let s = run(y.execute(
            &ToolCall::new("write_file", serde_json::json!({})),
            y.active_turn(),
            &mut ctx,
        ));
        assert!(s.world_changed);
        assert!(!s.retryable);
        assert!(!y.retry_safe());
    }

    #[test]
    fn even_an_innocent_call_after_a_side_effect_cannot_be_retried() {
        let y = executor();
        let mut ctx = context();
        run(y.execute(
            &ToolCall::new("write_file", serde_json::json!({})),
            y.active_turn(),
            &mut ctx,
        ));
        let s = run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        assert!(!s.world_changed);
        // The call is innocent but the TURN is not.
        assert!(!s.retryable);
    }

    #[test]
    fn a_recovery_attempt_does_not_drop_the_side_effect_flag() {
        let y = executor();
        let mut ctx = context();
        run(y.execute(
            &ToolCall::new("write_file", serde_json::json!({})),
            y.active_turn(),
            &mut ctx,
        ));
        let ticket = y.recovery_attempt();
        assert_eq!(ticket, y.active_turn());
        assert!(!y.retry_safe());
        // A real new turn resets it.
        y.new_turn();
        assert!(y.retry_safe());
    }

    #[test]
    fn a_new_turn_does_not_clear_the_taint_a_chat_reset_does() {
        let y = executor();
        let mut ctx = context();
        run(y.execute(
            &ToolCall::new("personal_read", serde_json::json!({"what": "x"})),
            y.active_turn(),
            &mut ctx,
        ));
        y.new_turn();
        assert!(
            y.session_tainted(),
            "the context summary may carry personal data"
        );
        y.reset_chat();
        assert!(!y.session_tainted());
    }

    // --- Cancellation ---

    #[test]
    fn in_a_cancelled_turn_no_tool_runs() {
        let y = executor();
        let mut ctx = context();
        let ticket = y.active_turn();
        y.cancel();
        let s = run(y.execute(
            &ToolCall::new("write_file", serde_json::json!({})),
            ticket,
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::Cancelled);
        assert!(!s.world_changed);
        assert!(!y.world_changed(), "the cancelled tool never ran");
    }

    #[test]
    fn cancelling_an_old_turn_does_not_bind_the_new_turn() {
        let y = executor();
        let mut ctx = context();
        let old = y.active_turn();
        y.cancel();
        assert!(y.is_cancelled(old));

        let new = y.new_turn();
        assert!(
            !y.is_cancelled(new),
            "cancelling an old turn must not drop the new one"
        );
        let s = run(y.execute(
            &ToolCall::new("write_file", serde_json::json!({})),
            new,
            &mut ctx,
        ));
        assert_eq!(s.reason, ExecutionReason::Ok);
    }

    // --- Raw execution ---

    #[test]
    fn plain_text_is_not_executed() {
        let y = executor();
        let mut ctx = context();
        assert!(run(y.execute_raw("Hello!", y.active_turn(), &mut ctx)).is_none());
    }

    // --- GATE 5: the repeated call ---

    /// A tool that counts HOW MANY TIMES IT RAN. The claim is not "what result
    /// came back" but "did the tool really run": the cost of the loop failure is
    /// exactly the tool that RUNS AGAIN (a second file, a second network request).
    struct CountingTool(Arc<std::sync::atomic::AtomicUsize>);
    impl Tool for CountingTool {
        fn name(&self) -> &str {
            "say"
        }
        fn description(&self) -> &str {
            "Increments the call counter."
        }
        fn schema(&self) -> ArgSchema {
            ArgSchema::object(vec![Field::new("x", ArgSchema::text()).required()])
        }
        fn run<'a>(&'a self, _a: Value, _c: &'a mut ToolContext) -> ToolFuture<'a> {
            self.0.fetch_add(1, Ordering::SeqCst);
            boxed(async move { ToolOutcome::read_ok("counted", "ok") })
        }
    }

    fn counting() -> (ToolExecutor, Arc<std::sync::atomic::AtomicUsize>) {
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let catalog: ToolCatalog = [Arc::new(CountingTool(Arc::clone(&counter))) as Arc<dyn Tool>]
            .into_iter()
            .collect();
        (ToolExecutor::new(catalog), counter)
    }

    /// A REGRESSION TEST — the loop failure. The instruction's "do not call any
    /// more tools" sentence was relaxed; what holds the loop now is THIS gate.
    #[test]
    fn the_same_call_does_not_run_a_second_time_in_a_turn() {
        let (y, counter) = counting();
        let mut ctx = context();
        let ticket = y.new_turn();
        let call = ToolCall::new("say", serde_json::json!({"x": "a"}));

        let first = run(y.execute(&call, ticket, &mut ctx));
        assert_eq!(first.reason, ExecutionReason::Ok);

        for _ in 0..3 {
            let s = run(y.execute(&call, ticket, &mut ctx));
            assert_eq!(s.reason, ExecutionReason::RepeatedCall);
            // The recovery path MUST NOT OPEN: had it counted as an error the
            // call site would give the model one more turn and the loop we are
            // trying to prevent would be built.
            assert!(s.to_model.contains("duplicate_call"));
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "the tool should have run only once"
        );
    }

    /// The gate is ARGUMENT sensitive: calling the same tool with different
    /// arguments is legitimate (like reading two separate files) and must not be blocked.
    #[test]
    fn a_call_with_different_arguments_does_not_count_as_a_repeat() {
        let (y, counter) = counting();
        let mut ctx = context();
        let ticket = y.new_turn();
        for x in ["a", "b", "c"] {
            let s = run(y.execute(
                &ToolCall::new("say", serde_json::json!({"x": x})),
                ticket,
                &mut ctx,
            ));
            assert_eq!(s.reason, ExecutionReason::Ok);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    /// The gate is TURN SCOPED: it is legitimate for the user to ask the same
    /// thing again in a second message. What is forbidden is getting the same
    /// result twice within a single message.
    #[test]
    fn a_new_turn_clears_the_repeat_record() {
        let (y, counter) = counting();
        let mut ctx = context();
        let call = ToolCall::new("say", serde_json::json!({"x": "a"}));

        run(y.execute(&call, y.new_turn(), &mut ctx));
        run(y.execute(&call, y.new_turn(), &mut ctx));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            2,
            "it must run twice in separate turns"
        );

        // A recovery attempt is NOT a real new turn: it must not be an excuse to
        // repeat a failed call verbatim.
        let s = run(y.execute(&call, y.recovery_attempt(), &mut ctx));
        assert_eq!(s.reason, ExecutionReason::RepeatedCall);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    /// Argument ORDER must not break sameness — the model may write the same
    /// call in two different orders and that does not make it a new call.
    #[test]
    fn argument_order_does_not_break_sameness() {
        let a = ToolCall::new("say", serde_json::json!({"x": "1", "y": "2"}));
        let b = ToolCall::new("say", serde_json::json!({"y": "2", "x": "1"}));
        assert_eq!(call_key(&a.name, &a.args), call_key(&b.name, &b.args));
        // A different value, a different key.
        let c = ToolCall::new("say", serde_json::json!({"x": "9", "y": "2"}));
        assert_ne!(call_key(&a.name, &a.args), call_key(&c.name, &c.args));
    }

    #[test]
    fn raw_json_is_executed() {
        let y = executor();
        let mut ctx = context();
        let s = run(y.execute_raw(
            r#"{"tool":"personal_read","args":{"what":"calendar"}}"#,
            y.active_turn(),
            &mut ctx,
        ))
        .unwrap();
        assert_eq!(s.reason, ExecutionReason::Ok);
        assert_eq!(s.to_model, "data: 42");
    }

    /// PROSE THAT COMPOUNDS A TOOL NAME WITH A FIELD NAME IS NOT A CALL.
    ///
    /// `recover_glued_call` used to argue that its shape was "a coincidence
    /// prose cannot arrange". Prose arranges it constantly, because tool names
    /// and field names are ordinary English words. Measured against the real
    /// catalog with no addons installed, all three of these ran a tool — and the
    /// third one runs a NETWORK tool from a sentence about a config key.
    #[test]
    fn a_compound_word_mid_sentence_is_not_a_glued_call() {
        let store = std::sync::Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (catalog, _, _) =
            crate::catalog::production_catalog_with(&store, &memory, Some(0), true);

        for prose in [
            "Keep the checksumpath=/etc/hosts entry as it is.",
            "Set archivepath: backups.zip, action: list in the config.",
            "Our web_searchquery: budget report is the config key.",
            "I looked at the find_filepattern: field in the docs.",
        ] {
            assert!(
                recover_glued_call(prose, &catalog).is_none(),
                "prose ran a tool: {prose:?} -> {:?}",
                recover_glued_call(prose, &catalog)
            );
        }
    }

    /// AND THE SHAPE IT EXISTS FOR STILL WORKS — three of the eight Turkish
    /// failures in the 184-case suite, where the model answered WITH the call.
    #[test]
    fn a_call_that_opens_the_answer_is_still_recovered() {
        let store = std::sync::Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (catalog, _, _) =
            crate::catalog::production_catalog_with(&store, &memory, Some(0), true);

        for raw in [
            "find_filepattern: \"bütçe\"",
            "find_filepattern: \"markdown\", folder: \"dizin\")",
            "Here you go.\nfind_filepattern: \"rapor\"",
        ] {
            let call = recover_glued_call(raw, &catalog)
                .unwrap_or_else(|| panic!("a real glued call was refused: {raw:?}"));
            assert_eq!(call.name, "find_file");
        }
    }

    /// A TOOL NAME BURIED IN A LONGER WORD IS NOT A CALL.
    ///
    /// This prose ran a tool. `recover_marked_call` looked for the name with a
    /// plain substring search, `"timeout"` contains `time`, and `kind` then
    /// satisfied that tool's required field — so an illustrative JSON block
    /// became an executed call:
    ///
    /// ```text
    /// Sure. The settings block looks like this:
    /// ```json
    /// {"timeout": 30, "kind": "date"}
    /// ```
    /// ```
    ///
    /// It is the fourth defect of one shape found in this codebase: a substring
    /// test matching the thing around what it meant to match, rather than the
    /// thing. The others were `name(` matching an echoed tool signature,
    /// `<tool_response>` matching the system prompt's description of it, and a
    /// whitespace fold that disagreed with its own trainer.
    ///
    /// KEY ORDER USED TO DECIDE IT, which is how thin the ice was:
    /// `{"kind":"date","timeout":30}` was refused and the same object with the
    /// keys swapped was executed, because the catalog was scanned in its own
    /// order rather than the text's.
    #[test]
    fn prose_that_merely_contains_a_tool_name_inside_a_word_is_not_a_call() {
        let store = std::sync::Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (catalog, _, _) =
            crate::catalog::production_catalog_with(&store, &memory, Some(0), true);

        for prose in [
            "Sure. The settings block looks like this:\n```json\n{\"timeout\": 30, \"kind\": \"date\"}\n```\nThat is all.",
            "Sure. The settings block looks like this:\n```json\n{\"kind\": \"date\", \"timeout\": 30}\n```\nThat is all.",
            "A run takes some time; the runtime is logged.\n```json\n{\"kind\": \"date\"}\n```",
        ] {
            assert!(
                recover_marked_call(prose, &catalog).is_none(),
                "prose recovered as a call: {prose:?}"
            );
        }
    }

    /// The whole-word rule, at the level it is decided.
    #[test]
    fn a_name_is_found_only_on_word_boundaries() {
        assert_eq!(find_word("{\"tool\": \"time\"}", "time"), Some(10));
        assert_eq!(find_word("time(", "time"), Some(0));
        assert_eq!(find_word("{\"timeout\": 30}", "time"), None);
        assert_eq!(find_word("the runtime is logged", "time"), None);
        // The earliest WHOLE-word occurrence wins over an earlier partial one.
        assert_eq!(find_word("timeout then time", "time"), Some(13));
    }

    /// THE SEVEN SHAPES A REAL MODEL ACTUALLY WROTE, and the prose that must not
    /// be mistaken for them.
    ///
    /// Every accepted fixture here is copied verbatim from the qwen3-4b run of
    /// the 115-case selection suite, where each one was the right tool with the
    /// right arguments and was thrown away because it did not start with
    /// `name(`. Every rejected fixture is either prose from the same run or the
    /// attribute shape this layer deliberately does not touch.
    #[test]
    fn a_marked_call_is_recovered_and_prose_is_not() {
        let store = std::sync::Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (catalog, _, _) =
            crate::catalog::production_catalog_with(&store, &memory, Some(0), true);

        for (raw, tool, key, value) in [
            (
                "```tool\nread_document\"path=report.md\"\n```",
                "read_document",
                "path",
                "report.md",
            ),
            (
                "```tool\nread_document\"path: readme.md\"\n```",
                "read_document",
                "path",
                "readme.md",
            ),
            (
                "<tool_call>\nread_document (path: \"weekly_meal_list.xlsx\")\n</tool_call>",
                "read_document",
                "path",
                "weekly_meal_list.xlsx",
            ),
        ] {
            let call = recover_marked_call(raw, &catalog)
                .unwrap_or_else(|| panic!("not recovered: {raw:?}"));
            assert_eq!(call.name, tool, "wrong tool for {raw:?}");
            assert_eq!(
                call.args.get(key).and_then(|v| v.as_str()),
                Some(value),
                "wrong argument for {raw:?}: {:?}",
                call.args
            );
        }

        // NOT A CALL. The first two are sentences the same model wrote as
        // ANSWERS; the third and fourth are the attribute shape, which is left
        // alone because scanning for it also matches ordinary prose.
        for raw in [
            "I cannot provide the current dollar value (e.g., exchange rate between currencies).",
            "This is a newly added section to the notes.md file.",
            "remember.list",
            "edit_document.new_content=\"## New Section\"",
            // A marker with no tool name behind it.
            "<tool_call>\nplease do the thing\n</tool_call>",
        ] {
            assert!(
                recover_marked_call(raw, &catalog).is_none(),
                "prose recovered as a call: {raw:?}"
            );
        }
    }

    /// A marked call MISSING a required argument is not a call.
    ///
    /// The same rule `recover_nameless_json` states: an incomplete match is
    /// ambiguity, and under ambiguity this layer does nothing. Without it the
    /// recovery would hand `execute` a call the schema then rejects, turning a
    /// readable answer into a tool error.
    #[test]
    fn a_marked_call_without_its_required_argument_is_left_as_text() {
        let store = std::sync::Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (catalog, _, _) =
            crate::catalog::production_catalog_with(&store, &memory, Some(0), true);

        assert!(
            recover_marked_call("<tool_call>\nread_document\n</tool_call>", &catalog).is_none(),
            "a call with no path is not a read_document call"
        );
    }

    /// THE SHAPE ONLY TURKISH PRODUCED, and the prose that must survive it.
    ///
    /// Fixtures copied verbatim from the 184-case run. English never wrote this
    /// in 115 cases; it appeared only once both languages ran together.
    #[test]
    fn a_tool_name_glued_to_its_argument_is_recovered() {
        let store = std::sync::Arc::new(crate::data_store::SharedStore::new());
        let memory = crate::memory::SharedMemory::in_memory();
        let (catalog, _, _) =
            crate::catalog::production_catalog_with(&store, &memory, Some(0), true);

        let call = recover_glued_call("find_filepattern: \"bütçe\"", &catalog)
            .expect("the tool name glued to its own field is a call");
        assert_eq!(call.name, "find_file");
        assert_eq!(
            call.args.get("pattern").and_then(|v| v.as_str()),
            Some("bütçe"),
            "{:?}",
            call.args
        );

        // NOT A CALL. The first is the same tool name in an ordinary sentence —
        // the coincidence this recovery rests on is the field name touching the
        // tool name, and a space breaks it. The rest are prose from the same run.
        for raw in [
            "I will use find_file to look for the budget report.",
            "find_file pattern: \"budget\"",
            "Bütçe raporu hangi klasörde?",
            "This is a newly added section to the notes.md file.",
        ] {
            assert!(
                recover_glued_call(raw, &catalog).is_none(),
                "recovered from prose: {raw:?}"
            );
        }
    }
}

/// A CALL FENCED AS `json` WITH THE NAME IN AN ENVELOPE KEY.
///
/// MEASURED in the 184-case run, twice: the model wrote
/// ```` ```json {"action":"read_document","path":"report.md"} ``` ```` — the
/// right tool, the right argument, and no `(`, so the grammar never armed.
#[cfg(test)]
mod json_fence {
    use super::*;
    use crate::memory::SharedMemory;

    fn catalog() -> ToolCatalog {
        let store = std::sync::Arc::new(crate::data_store::SharedStore::new());
        let memory = SharedMemory::in_memory();
        let (catalog, _, _) = crate::catalog::production_catalog(&store, &memory, Some(0));
        catalog
    }

    #[test]
    fn the_shape_the_model_actually_wrote_is_recovered() {
        let raw = "```json\n{\"action\":\"read_document\",\"path\":\"report.md\"}\n```";
        let call = recover_marked_call(raw, &catalog()).expect("this is a call");
        assert_eq!(call.name, "read_document");
        assert_eq!(call.args["path"], Value::String("report.md".into()));
    }

    /// THE ASSERTION THAT KEEPS THE NEW MARKER HONEST. `\`\`\`json` is the one
    /// marker on the list a model writes without calling anything, so a fenced
    /// block that names a tool but does not carry its required argument must
    /// stay an answer. Without this the marker would be a way to turn prose
    /// about a tool into a call to it.
    #[test]
    fn a_json_block_that_is_only_talking_about_a_tool_is_not_a_call() {
        let c = catalog();
        for raw in [
            // Names the tool, gives it nothing.
            "```json\n{\"tools\":[\"read_document\",\"find_file\"]}\n```",
            // Names the tool, and the only key is not in its schema.
            "```json\n{\"example\":\"read_document\",\"colour\":\"blue\"}\n```",
            // No catalog tool named at all.
            "```json\n{\"name\":\"Ada\",\"path\":\"report.md\"}\n```",
        ] {
            assert!(
                recover_marked_call(raw, &c).is_none(),
                "not a call, and must not become one: {raw}"
            );
        }
    }
}
