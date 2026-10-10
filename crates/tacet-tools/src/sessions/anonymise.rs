//! Turning a real session into a corpus fixture without publishing it (RG-28).
//!
//! WHY THIS EXISTS. The corpus under `tests/fixtures/sessions/corpus/` is
//! synthetic: every line was written in the measured *shape*, and the four
//! Claude Code "versions" are one file with the version string edited. RG-28
//! asks for anonymised sessions per agent and version, which means copying a
//! real one off a real machine — and a real session holds the user's prompts,
//! code, paths, branch names and whatever they pasted. The corpus README's old
//! step 2 ("anonymise it … read the file before committing it") left all of
//! that to a person with a text editor and a 40 MB file. This does the
//! mechanical part, so the capture is one command and the reading is a check
//! rather than the whole job.
//!
//! WHAT IS KEPT, WHAT IS NOT. The readers depend on structure — line types,
//! roles, ids and parent links, timestamps, versions, tool names, task ids and
//! statuses — and on a handful of markup tokens inside text (`<USER_REQUEST>`,
//! `<command-name>`, `- [x]`, `file://`). Those survive, and only where they
//! LOOK like structure: a value under a kept key is kept only in a kept key's
//! shape (an identifier, a dotted version, an RFC 3339 time), and inside a
//! tool's data — a call's input or arguments, a result's content, any JSON
//! carried in a string — only the few values the readers take from there
//! (a status they know, a numeric task id, a content block's type). The first
//! version kept by key name alone, at every depth; a review put a CRM answer
//! in a tool_result (`{"id":"jane.doe@…","status":"overdue: Jane Doe owes
//! 40k",…}`) and every value came out verbatim. The second kept every object
//! KEY and every number, so the next review put the CRM answer in the keys
//! (`{"Jane Doe":{"iban_no":4012…},"jdoe@clientbank.com":true}`) and it came
//! out whole, address included. Now a key inside a tool's data survives only
//! when a reader looks it up there (`DATA_KEYS`), a number there only when it
//! is a task id of at most six digits (`TASK_ID_MAX`: the third review found
//! a card number under `id` kept), and a key outside data only when it is one
//! identifier-shaped token; a masked number keeps its digit count. OUTSIDE
//! DATA, NUMBERS AND TOKEN-SHAPED KEYS ARE KEPT, because that is the agents'
//! own format. So a tool's answer must never be read as envelope: the keys
//! that hold one in a line no reader opens (`ALWAYS_DATA` — Codex's
//! `mcp_tool_call_end` and `exec_command_end` payloads, Claude Code's
//! `progress` lines) are data wherever they appear. An envelope that carries
//! tool output under a key not on that list is the gap this design leaves,
//! and why step 2 of the corpus README is reading the output. A markup tag
//! keeps its brackets, and its name only when a reader splits on it
//! (`TAG_NAMES`).
//! Every other string value is
//! MASKED: each word becomes a pseudo-word of the same length and
//! case pattern, the same word always the same pseudo-word within one capture
//! (so a path in a `cwd` field and the same path inside a tool call still
//! agree, and Claude Code's project directory can be recomputed from the
//! masked `cwd`). Home directories become `dev` before masking, so
//! `/Users/<anyone>/…` reads `/Users/dev/…` the way the corpus scan demands,
//! and e-mail addresses are replaced outright.
//!
//! THE SALT IS PER CAPTURE AND NEVER STORED. Without it the masking is a
//! deterministic function of the word, and a dictionary of user names and
//! common words would read a masked fixture back. With a salt drawn from the
//! OS's randomness, printed once and not written anywhere, that dictionary
//! would have to be run once per possible salt — 2^64 of them. The first
//! version drew it from the clock and the pid: microsecond clock on macOS, a
//! commit date to narrow the hour, about 2^49 candidates, few enough to try.
//!
//! WHAT IT CANNOT KNOW. A value under a kept key that is private anyway (a
//! tool literally named after a client), and anything the readers never look
//! at but a person would recognise. That is why the corpus test still scans
//! every file, and why the README still says to read the output.
//!
//! Pure functions over text: no file is opened here, so the secrets gate's
//! "every reader goes through the door" guard has nothing to find. The
//! `corpus_capture` example opens the input through `secrets::read_text`.

use serde_json::Value;

/// Keys whose string values are structure, kept verbatim (after the home
/// rule). Taken from what the three readers look up — `claude.rs`,
/// `codex.rs`, `antigravity.rs` — and the brief's field lists; a string under
/// a key that is not here is masked, so forgetting one costs a fixture its
/// meaning, never that string. (It does not cover keys and numbers outside a
/// tool's data, which are kept: see `ALWAYS_DATA`.)
pub const KEEP_KEYS: &[&str] = &[
    // identity and links
    "uuid",
    "parentUuid",
    "logicalParentUuid",
    "leafUuid",
    "sessionId",
    "session_id",
    "thread_id",
    "forked_from_id",
    "id",
    "taskId",
    "addBlockedBy",
    "blockedBy",
    "tool_use_id",
    "toolUseID",
    "call_id",
    "sourceToolAssistantUUID",
    "promptId",
    "requestId",
    "messageId",
    // shape
    "type",
    "subtype",
    "role",
    "status",
    "stop_reason",
    "source",
    "level",
    "operation",
    "userType",
    "entrypoint",
    "originator",
    "history_mode",
    "model_provider",
    "hookEventName",
    "permissionMode",
    "trigger",
    // models, by name (a tool's name is kept only where it is one: `TOOL_NAMES`)
    "model",
    // time and version
    "timestamp",
    "created_at",
    "updatedAt",
    "version",
    "cli_version",
];

/// Keys whose string values are times: kept only as RFC 3339 is spelled.
const TIME_KEYS: &[&str] = &["timestamp", "created_at", "updatedAt"];

/// Keys whose string values are versions: kept only as dotted numbers with
/// an optional suffix (`2.1.285`, `0.46.0-alpha.3`).
const VERSION_KEYS: &[&str] = &["version", "cli_version"];

/// Objects whose KEYS are file paths, relative or not. A key is otherwise
/// structure and kept; one with a `/` or `\` in it was masked as a path, but
/// `trackedFileBackups` also keys by a bare file name (`acme_payroll.xlsx`).
const PATH_KEYED_MAPS: &[&str] = &["trackedFileBackups"];

/// The keys under which a tool's DATA sits: what a call was given and what a
/// result said. Anything in there is the user's or a third party's — an API
/// answer, a `package.json`, `gh` output — and is masked whatever its key is
/// called, but for `DATA_KEEP`.
fn is_data(object_type: Option<&str>, in_call: bool, key: &str) -> bool {
    match key {
        "input" | "arguments" | "args" => in_call,
        "content" => object_type == Some("tool_result"),
        "output" => object_type.is_some_and(|t| t.ends_with("_output")),
        // Claude Code's structured copy of a result, beside the message.
        "toolUseResult" => true,
        // Codex's shell and search calls carry what they were given in
        // `action`, not `arguments`: the command, and an `env` map keyed by
        // whatever variable names the user's shell had (a review's
        // `ACME_SECRET_TOKEN_NAME` came out verbatim as an envelope key).
        "action" => matches!(object_type, Some("local_shell_call" | "web_search_call")),
        // Envelopes no reader opens, holding a tool's answer: Codex's
        // `event_msg` `mcp_tool_call_end` (`invocation`, `result`) and
        // `exec_command_end` (`stdout`, `stderr`, `aggregated_output`,
        // `formatted_output`), Claude Code's `progress` lines (`data`).
        // Outside data a key of one identifier-shaped token and every number
        // are kept, so a review's `{"JaneDoe":{"iban_no":4012…}}` under
        // `result.Ok.structuredContent` came out whole (RG-28 third review).
        // `no_reader_looks_up_an_always_data_key` keeps these out of the
        // readers.
        k if ALWAYS_DATA.contains(&k) => true,
        _ => false,
    }
}

/// Keys that are a tool's data wherever they appear. None of the three
/// readers looks one up, so declaring them data costs no reading.
const ALWAYS_DATA: &[&str] = &[
    "invocation",
    "result",
    "stdout",
    "stderr",
    "aggregated_output",
    "formatted_output",
    "data",
];

/// Object KEYS a reader looks up inside a tool's data. Every other key in
/// data is masked like text: a tool's JSON is keyed by its own content as
/// often as by a schema (`{"Jane Doe":{…}}`, `{"jdoe@clientbank.com":true}`,
/// an MCP server's `structuredContent`), and the first two versions kept
/// every key, so a review's CRM answer moved from values to keys and came
/// out whole. Masking a key the readers need does not leak — it changes the
/// reading, which `a_captured_session_keeps_its_shape` catches.
const DATA_KEYS: &[&str] = &[
    // Claude Code's task and todo tools (`claude::replay_plan`) and
    // `toolUseResult.task.id`.
    "taskId",
    "subject",
    "status",
    "addBlockedBy",
    "blockedBy",
    "todos",
    "content",
    "activeForm",
    "task",
    "id",
    // Codex `update_plan`: `codex::plan_from_update` refuses a call with a
    // key it does not know, so a masked key is a plan Codex would refuse.
    "plan",
    "step",
    "explanation",
    // Content blocks (`claude::content_text`, `codex::content_text`).
    "type",
    "text",
    // Antigravity's working directory, PascalCase (`antigravity::CWD_ARG`).
    "Cwd",
];

/// Keys whose values inside data are task ids, kept when they are task-id
/// shaped (`task_id_shaped`), as a string or as a number: Claude Code writes
/// a task id either way, and a masked one would no longer match the same id
/// written the other way. The string and number paths read this one list —
/// `blockedBy` was once in the number list only, so `[3]` was kept and
/// `["3"]` masked. Every other number in data is a value somebody's tool
/// returned (a balance, a card number) and is masked.
const TASK_ID_KEYS: &[&str] = &["taskId", "addBlockedBy", "blockedBy", "id"];

/// The largest task id kept. Claude Code numbers a session's tasks from 1
/// upward; a key as generic as `id` also holds whatever a tool's JSON keys
/// by it, and the review found `{"id":4012888888881881}` (a card number)
/// kept verbatim when the bound was twenty digits.
const TASK_ID_MAX: u64 = 999_999;

/// One to six ASCII digits, no leading zero beyond a lone `0`.
fn task_id_shaped(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| b.is_ascii_digit())
        && s.parse::<u64>()
            .is_ok_and(|n| n <= TASK_ID_MAX && n.to_string() == s)
}

/// Content block types a reader branches on inside a result's content
/// (`claude.rs` takes the text of a `text` block only). Any other `type`
/// in data is masked: it is not structure the readers use, and a hostile
/// one (`"type":"AcmeCorp confidential"`) is the user's text.
const DATA_BLOCK_TYPES: &[&str] = &["text", "image", "document", "tool_reference"];

/// Whether a value in a tool's data is one a reader takes from there:
/// a task's or plan item's status the readers know (`TaskUpdate`,
/// `TodoWrite`, Codex `update_plan`), and a task id, which Claude Code
/// numbers (`TaskUpdate.taskId`, `addBlockedBy`, `toolUseResult.task.id`).
/// A masked digit string would still match itself, but not a task id the
/// same session wrote as a JSON number.
fn data_keep(key: &str, s: &str) -> bool {
    match key {
        "status" => {
            super::Status::from_claude_task(s, false).is_some()
                || super::Status::from_codex_plan(s).is_some()
        }
        k if TASK_ID_KEYS.contains(&k) => task_id_shaped(s),
        "type" => DATA_BLOCK_TYPES.contains(&s),
        _ => false,
    }
}

/// Whether a value under a kept key looks like what that key holds. Ids,
/// roles, types, models: one token of `[A-Za-z0-9_.:-]`, no space, no `@`,
/// no `/`. Times and versions: their own spellings. Anything else under a
/// kept key is text somebody wrote there and is masked like text.
fn structural(key: &str, s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if s.len() > 128 {
        return false;
    }
    let starts_digit = s.starts_with(|c: char| c.is_ascii_digit());
    if TIME_KEYS.contains(&key) {
        return starts_digit
            && s.bytes()
                .all(|b| b.is_ascii_digit() || b"-:.TZ+".contains(&b));
    }
    if VERSION_KEYS.contains(&key) {
        return starts_digit
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-+_".contains(&b));
    }
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}

/// `name` is a tool's name only in a call: an object of one of these `type`s,
/// or an entry of Antigravity's `tool_calls`. Kept everywhere, it kept the
/// session title Claude Code writes as `name` into `sessions/<pid>.json`
/// (the user's words; the whole-fixture test found `"demo"` surviving).
const TOOL_OBJECT_TYPES: &[&str] = &[
    "tool_use",
    "server_tool_use",
    "function_call",
    "custom_tool_call",
    "tool_search_call",
];
const TOOL_NAMES: &[&str] = &["name", "namespace"];

/// Words that survive masking, compared without ASCII case: the parts of a
/// home directory the corpus scan requires (`/Users/dev/`, `/home/dev/`), and
/// the markers a `task.md` item is blocked by (`handoff::parse_task_md_line`
/// reads `(blocked …)` / `(engelli …)` case-blind; masked, a blocked item read
/// as pending — the whole-fixture test caught it).
const KEEP_WORDS: &[&str] = &["Users", "home", "dev", "blocked", "engelli"];

/// Text a reader matches on, kept whole wherever it appears in a value.
/// Markup tags are kept by shape (`tag_len`); these are the prose ones.
const KEEP_PHRASES: &[&str] = &[
    // Tacet's own note (RG-22): `sessions_tool::is_user_ask` tells it from
    // the user by this prefix.
    super::writeback::NOTE_PREFIX,
    // The Codex reader keeps a plan only when the call's output is exactly
    // this. Masked, every plan in a captured rollout read as refused — the
    // whole-fixture test caught it on its first run.
    super::codex::PLAN_UPDATED,
    // One of Codex's injected-context openings that is not a tag.
    "# AGENTS.md instructions",
    "file://",
];

/// One capture's masking: the salt, and the capturer's user name, which is
/// also replaced inside kept values.
#[derive(Debug, Clone)]
pub struct Scrub {
    salt: u64,
    user: Option<String>,
}

impl Scrub {
    /// `user` is the capturing account's name; one shorter than four
    /// characters is ignored, as the corpus scan ignores it (a user called
    /// `ci` must not turn every `ci` into `dev`).
    pub fn new(salt: u64, user: Option<&str>) -> Self {
        Self {
            salt,
            user: user.filter(|u| u.chars().count() >= 4).map(str::to_string),
        }
    }

    /// A salt nobody chose, from the OS's randomness without a new
    /// dependency: std seeds each `RandomState`'s SipHash keys from the OS
    /// (`getentropy` on macOS, `getrandom` on Linux, `ProcessPrng` on
    /// Windows) once per thread and steps them per call, so two hashers of
    /// the same input give two unpredictable words. Not the clock: see the
    /// module doc for what that cost.
    pub fn fresh_salt() -> u64 {
        use std::hash::{BuildHasher, Hasher};
        let draw = |n: u64| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(n);
            h.finish()
        };
        draw(0) ^ draw(1).rotate_left(32)
    }

    /// One JSONL file: each line anonymised on its own. A line that is not
    /// JSON — a torn last line, a corrupt one, both shapes the corpus keeps
    /// on purpose — is masked as text, so it stays broken in the same place.
    pub fn jsonl(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let (body, end) = match line.strip_suffix('\n') {
                Some(b) => (b, "\n"),
                None => (line, ""),
            };
            match serde_json::from_str::<Value>(body) {
                Ok(mut v) if !body.trim().is_empty() => {
                    self.value(&mut v, None, false);
                    out.push_str(&v.to_string());
                }
                _ => out.push_str(&self.text(body)),
            }
            out.push_str(end);
        }
        out
    }

    /// One JSON document (`.metadata.json`). Not JSON: masked as text.
    pub fn json(&self, text: &str) -> String {
        match serde_json::from_str::<Value>(text) {
            Ok(mut v) => {
                self.value(&mut v, None, false);
                v.to_string()
            }
            Err(_) => self.text(text),
        }
    }

    /// Free text (`task.md`, a walkthrough). Markdown's checkboxes are
    /// punctuation and one-letter words, and survive.
    pub fn markdown(&self, text: &str) -> String {
        self.text(text)
    }

    /// A path as `cwd` holds it, masked the way every occurrence of it is.
    pub fn path(&self, path: &str) -> String {
        self.text(path)
    }

    /// `data` is true inside a tool's data (`is_data`), where only
    /// `data_keep` values survive; outside it, `KEEP_KEYS` values survive in
    /// their key's shape (`structural`).
    fn value(&self, v: &mut Value, key: Option<&str>, data: bool) {
        match v {
            Value::String(s) => {
                *s = match key {
                    Some(k) if data && data_keep(k, s) => s.clone(),
                    Some(k) if !data && KEEP_KEYS.contains(&k) && structural(k, s) => self.kept(s),
                    _ => self.string(s),
                };
            }
            // An array inherits its key: `addBlockedBy: ["1"]` is ids.
            Value::Array(items) => {
                for item in items {
                    self.value(item, key, data);
                }
            }
            Value::Object(map) => {
                let object_type = map.get("type").and_then(Value::as_str).map(str::to_string);
                let is_call = key == Some("tool_calls")
                    || object_type
                        .as_deref()
                        .is_some_and(|t| TOOL_OBJECT_TYPES.contains(&t));
                // Keys are structure, except where a format keys a map by
                // file path (Claude Code's `trackedFileBackups`).
                let path_keyed = key.is_some_and(|k| PATH_KEYED_MAPS.contains(&k));
                let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
                // Kept keys go in first, and a masked key that comes out
                // spelling one already there gets a `_` until it does not:
                // a collision must neither push a key the readers need aside
                // nor drop a value.
                let (mut kept_keys, mut masked) = (Vec::new(), Vec::new());
                for (k, child) in entries {
                    match self.key(&k, data, path_keyed) {
                        Some(new) => masked.push((k, new, child)),
                        None => kept_keys.push((k.clone(), k, child)),
                    }
                }
                let entries = kept_keys.into_iter().chain(masked);
                for (k, mut new_key, mut child) in entries {
                    let child_data = data || is_data(object_type.as_deref(), is_call, &k);
                    match &mut child {
                        // A tool's name, in the agent's own call: what the
                        // reader reports as the entry's tool. A "tool_use"
                        // inside a tool's data is somebody's JSON, not a call.
                        Value::String(s)
                            if is_call
                                && !data
                                && TOOL_NAMES.contains(&k.as_str())
                                && structural(&k, s) =>
                        {
                            *s = self.kept(s);
                        }
                        _ => self.value(&mut child, Some(&k), child_data),
                    }
                    while map.contains_key(&new_key) {
                        new_key.push('_');
                    }
                    map.insert(new_key, child);
                }
            }
            Value::Number(n) => {
                let task_id = key.is_some_and(|k| TASK_ID_KEYS.contains(&k))
                    && n.as_u64().is_some_and(|v| v <= TASK_ID_MAX);
                if data && !task_id {
                    *n = self.number(n);
                }
            }
            Value::Bool(_) | Value::Null => {}
        }
    }

    /// What an object key becomes, or `None` to keep it. Inside data only
    /// `DATA_KEYS` survive. Outside it a key is the agent's format and is
    /// kept when it is one identifier-shaped token, after the same rules as
    /// a kept value (no home, no address, not the capturer's name); a key
    /// with a space, an `@` or a path in it is somebody's text. Claude Code's
    /// `trackedFileBackups` keys by a bare file name (`acme_payroll.xlsx`),
    /// token-shaped, so it is masked by its map's name.
    fn key(&self, k: &str, data: bool, path_keyed: bool) -> Option<String> {
        let masked = if path_keyed {
            self.text(k)
        } else if data {
            if DATA_KEYS.contains(&k) {
                return None;
            }
            self.text(k)
        } else if structural("", k) {
            self.kept(k)
        } else {
            self.text(k)
        };
        (masked != k).then_some(masked)
    }

    /// A number of the same shape — the same digit count in each run, the
    /// integer part not starting with a zero — so a masked card number is
    /// still sixteen digits and a masked salary still six. The same digits
    /// mask the same way within one capture, as words do.
    fn number(&self, n: &serde_json::Number) -> serde_json::Number {
        let text = n.to_string();
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        let mut first = true;
        while let Some(c) = rest.chars().next() {
            if c.is_ascii_digit() {
                let len = rest
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len());
                let mut run = self.word(&rest[..len]);
                if first && run.len() > 1 && run.starts_with('0') {
                    run.replace_range(..1, "1");
                }
                first = false;
                out.push_str(&run);
                rest = &rest[len..];
            } else {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
        // An exponent masked past f64's range does not parse; zero says
        // nothing about the original.
        serde_json::from_str(&out).unwrap_or_else(|_| serde_json::Number::from(0))
    }

    /// A masked string value — unless it is JSON itself: Codex carries a
    /// call's arguments as a JSON string (`update_plan`'s plan is in one), and
    /// masking its braces' contents as prose would lose the plan. JSON in a
    /// string is always walked as data: wherever it sits, it is something a
    /// tool or a person wrote, not the agent's own envelope.
    fn string(&self, s: &str) -> String {
        let t = s.trim_start();
        if (t.starts_with('{') || t.starts_with('['))
            && let Ok(mut inner) = serde_json::from_str::<Value>(s)
        {
            self.value(&mut inner, None, true);
            return inner.to_string();
        }
        self.text(s)
    }

    /// A kept value, still put through the home rule, the e-mail rule and
    /// the user name. The name is matched without ASCII case: `Alice-mbp`
    /// is the user `alice` too. `structural` already turns an address away
    /// (no `@` in any kept shape); the e-mail rule is here so that widening
    /// a shape cannot be what lets one through.
    fn kept(&self, s: &str) -> String {
        let mut out = emails_out(&homes_to_dev(s));
        if let Some(user) = &self.user {
            out = replace_ignore_ascii_case(&out, user, "dev");
        }
        out
    }

    /// Masks prose: homes to `dev`, e-mail addresses out, then every word
    /// that is not kept. Markup tags and `KEEP_PHRASES` are copied whole.
    pub fn text(&self, text: &str) -> String {
        let text = emails_out(&homes_to_dev(text));
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        'outer: while !rest.is_empty() {
            for phrase in KEEP_PHRASES {
                if let Some(after) = rest.strip_prefix(phrase) {
                    out.push_str(phrase);
                    rest = after;
                    continue 'outer;
                }
            }
            if let Some((open, name)) = tag_at(rest) {
                out.push_str(&rest[..open]);
                out.push_str(&self.tag_name(&rest[open..open + name]));
                out.push('>');
                rest = &rest[open + name + 1..];
                continue;
            }
            // A percent escape in a `file://` URI is one character of the
            // path, not part of the next word: `%20app` masked as one word
            // decoded to a different path than the `cwd` it names.
            if let Some(hex) = rest.strip_prefix('%').and_then(|r| r.get(..2))
                && hex.bytes().all(|b| b.is_ascii_hexdigit())
            {
                out.push_str(&rest[..3]);
                rest = &rest[3..];
                continue;
            }
            let c = rest.chars().next().unwrap_or_default();
            if c.is_alphanumeric() {
                let len = rest
                    .find(|c: char| !c.is_alphanumeric())
                    .unwrap_or(rest.len());
                out.push_str(&self.word(&rest[..len]));
                rest = &rest[len..];
            } else {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
        out
    }

    /// A word of the same length and case pattern, ASCII only (a Claude Code
    /// project directory is encoded from ASCII alphanumerics; a masked `ü`
    /// that stayed non-ASCII would change its dash count). One-character
    /// words are kept: `[x]` is a checkbox, and a single letter names nobody.
    fn word(&self, word: &str) -> String {
        if KEEP_WORDS.iter().any(|k| k.eq_ignore_ascii_case(word)) || word.chars().count() <= 1 {
            return word.to_string();
        }
        let seed = fnv(word.as_bytes()) ^ self.salt;
        word.chars()
            .enumerate()
            .map(|(i, c)| {
                let r = splitmix(seed.wrapping_add(i as u64));
                if c.is_ascii_digit() {
                    char::from(b'0' + (r % 10) as u8)
                } else {
                    let letter = char::from(b'a' + (r % 26) as u8);
                    if c.is_uppercase() {
                        letter.to_ascii_uppercase()
                    } else {
                        letter
                    }
                }
            })
            .collect()
    }
}

/// `text` with every ASCII-case-blind occurrence of `needle` replaced.
fn replace_ignore_ascii_case(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let (hay, pat) = (text.to_ascii_lowercase(), needle.to_ascii_lowercase());
    // ASCII lowercasing keeps every byte offset, so offsets in `hay` are
    // offsets in `text`.
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (at, _) in hay.match_indices(&pat) {
        out.push_str(&text[last..at]);
        out.push_str(with);
        last = at + pat.len();
    }
    out.push_str(&text[last..]);
    out
}

/// A markup tag at the start of `text` (`<TAG>`, `</TAG>`, `<command-name>`):
/// the length of its opening (`<` or `</`) and of its name. The brackets are
/// always kept, so a masked tag is still a tag where it was.
fn tag_at(text: &str) -> Option<(usize, usize)> {
    let body = text.strip_prefix('<')?;
    let body = body.strip_prefix('/').unwrap_or(body);
    let first = body.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let name = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(body.len());
    (body[name..].starts_with('>')).then(|| (text.len() - body.len(), name))
}

/// Tag names a reader splits on, kept whole: Antigravity's request wrapper
/// (`antigravity::split_user_input`), Claude Code's slash-command wrappers
/// (`claude::first_prompt` skips text that opens with `<command-` or
/// `<local-command-`), its reminders, and the children of Codex's
/// environment block. Codex's own injected openings come from the reader's
/// list (`codex::INJECTED_PREFIXES`), so the two cannot drift. Any other tag
/// name is the writer's text: the first version kept every name of tag
/// shape, so `Author: <JaneDoe>` kept the author.
const TAG_NAMES: &[&str] = &[
    "USER_REQUEST",
    "ADDITIONAL_METADATA",
    "command-name",
    "command-message",
    "command-args",
    "command-contents",
    "local-command-stdout",
    "local-command-stderr",
    "local-command-caveat",
    "system-reminder",
    "cwd",
    "shell",
    "approval_policy",
    "sandbox_mode",
    "network_access",
    "writable_roots",
    "current_date",
    "timezone",
];

/// The tag prefixes `claude::first_prompt` matches on: a `<command-…>` tag
/// not in `TAG_NAMES` keeps the prefix, so the reader still skips it, and
/// loses the rest.
const TAG_PREFIXES: &[&str] = &["local-command-", "command-"];

impl Scrub {
    fn tag_name(&self, name: &str) -> String {
        let injected = super::codex::INJECTED_PREFIXES.iter().any(|p| {
            p.strip_prefix('<')
                .and_then(|p| p.strip_suffix('>'))
                .is_some_and(|p| p == name)
        });
        if injected || TAG_NAMES.contains(&name) {
            return name.to_string();
        }
        for prefix in TAG_PREFIXES {
            if let Some(rest) = name.strip_prefix(prefix) {
                return format!("{prefix}{}", self.text(rest));
            }
        }
        self.text(name)
    }
}

/// Every home directory, whoever's, as `dev`: `/Users/<x>`, `/home/<x>`,
/// `C:\Users\<x>`, and Claude Code's encoded `-Users-<x>-` / `-home-<x>-`.
/// The same prefixes the corpus scan looks for.
pub fn homes_to_dev(text: &str) -> String {
    let mut out = text.to_string();
    for (prefix, stop) in [
        ("/Users/", "/\"'\\ \n\t:"),
        ("/home/", "/\"'\\ \n\t:"),
        ("C:\\Users\\", "\\\"' \n\t:/"),
        ("-Users-", "-/\"'\\ \n\t:"),
        ("-home-", "-/\"'\\ \n\t:"),
    ] {
        let mut done = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(prefix) {
            done.push_str(&rest[..at + prefix.len()]);
            let after = &rest[at + prefix.len()..];
            let end = after
                .find(|c: char| stop.contains(c))
                .unwrap_or(after.len());
            if end > 0 {
                done.push_str("dev");
            }
            rest = &after[end..];
        }
        done.push_str(rest);
        out = done;
    }
    out
}

/// `local@domain.tld` → `[email]`, by the corpus scan's own definition of
/// one: something address-like before the `@`, a dotted domain after.
fn emails_out(text: &str) -> String {
    let local_char = |c: char| c.is_alphanumeric() || "._-+".contains(c);
    let domain_char = |c: char| c.is_alphanumeric() || c == '.' || c == '-';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        let before = &rest[..at];
        let local_start = before
            .char_indices()
            .rev()
            .take_while(|(_, c)| local_char(*c))
            .last()
            .map(|(i, _)| i);
        let after = &rest[at + 1..];
        let domain_len = after.find(|c: char| !domain_char(c)).unwrap_or(after.len());
        let domain = after[..domain_len].trim_end_matches('.');
        match local_start {
            Some(start) if domain.contains('.') => {
                out.push_str(&before[..start]);
                out.push_str("[email]");
                rest = &after[domain.len()..];
            }
            _ => {
                out.push_str(&rest[..at + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scrub() -> Scrub {
        Scrub::new(42, Some("alice"))
    }

    #[test]
    fn homes_become_dev_whoever_they_belong_to() {
        assert_eq!(
            homes_to_dev("/Users/alice/w and /home/bob/x and C:\\Users\\carol\\y"),
            "/Users/dev/w and /home/dev/x and C:\\Users\\dev\\y"
        );
        assert_eq!(homes_to_dev("-Users-alice-w"), "-Users-dev-w");
        assert_eq!(homes_to_dev("/Users/dev/w"), "/Users/dev/w");
    }

    #[test]
    fn an_email_is_replaced_and_an_at_sign_alone_is_not() {
        assert_eq!(emails_out("mail jane.doe@example.org."), "mail [email].");
        assert_eq!(emails_out("@Override a@b"), "@Override a@b");
    }

    #[test]
    fn a_word_is_masked_the_same_way_everywhere_and_differently_per_salt() {
        let s = scrub();
        let a = s.text("/Users/alice/Secret-Project");
        assert!(a.starts_with("/Users/dev/"), "{a}");
        assert!(!a.contains("Secret") && !a.contains("Project"), "{a}");
        assert!(
            s.text("cd Secret-Project")
                .ends_with(&format!(" {}", &a["/Users/dev/".len()..])),
            "the same words masked differently in prose and in a path"
        );
        assert_ne!(Scrub::new(43, None).text("Secret"), s.text("Secret"));
        // Same length, same case pattern, ASCII.
        let w = s.text("Ünïcode42");
        assert_eq!(w.chars().count(), 9);
        assert!(w.is_ascii() && w.starts_with(|c: char| c.is_ascii_uppercase()));
        assert!(w[7..].bytes().all(|b| b.is_ascii_digit()), "{w}");
    }

    #[test]
    fn markup_the_readers_split_on_survives() {
        let s = scrub();
        let t = s.text("<USER_REQUEST>\nfix it\n</USER_REQUEST><command-name>/x</command-name>");
        assert!(t.starts_with("<USER_REQUEST>\n"), "{t}");
        assert!(
            t.contains("</USER_REQUEST><command-name>/x</command-name>"),
            "{t}"
        );
        assert!(!t.contains("fix"));
        // Codex's injected openings, from the reader's own list.
        for p in super::super::codex::INJECTED_PREFIXES {
            assert!(s.text(p).starts_with(p), "{p}: {}", s.text(p));
        }
        // RG-28 review: a tag-shaped name nobody splits on is the writer's
        // text. Brackets stay; the name does not. A `<command-…>` tag keeps
        // the prefix Claude Code's reader skips it by.
        let t = s.text("Author: <JaneDoe> for </acme-payroll> <command-AcmeRun>");
        for leak in ["JaneDoe", "acme", "payroll", "Acme"] {
            assert!(!t.contains(leak), "{leak}: {t}");
        }
        assert!(t.contains(": <") && t.contains(" </"), "{t}");
        assert_eq!(t.matches('<').count(), 3, "{t}");
        assert_eq!(t.matches('>').count(), 3, "{t}");
        assert!(t.contains(" <command-") && t.ends_with('>'), "{t}");
        let md = s.markdown("- [x] Ship it\n  - [/] Test\n- [ ] Docs");
        assert!(md.starts_with("- [x] "), "{md}");
        assert!(md.contains("\n  - [/] ") && md.contains("\n- [ ] "), "{md}");
        let note = "[Note from Tacet, a local assistant, not from the user] done";
        assert!(s.text(note).starts_with(&note[..note.len() - 5]));
        assert!(
            s.text("file:///Users/bob/a.rs")
                .starts_with("file:///Users/dev/")
        );
    }

    #[test]
    fn structure_is_kept_and_text_is_not() {
        let s = scrub();
        let line = r#"{"type":"assistant","uuid":"u1","parentUuid":"u0","cwd":"/Users/alice/acme","version":"2.1.285","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"TaskUpdate","input":{"taskId":"3","status":"completed","subject":"Call ACME about invoices"}}]}}"#;
        let out: Value = serde_json::from_str(s.jsonl(line).trim_end()).unwrap();
        assert_eq!(out["type"], "assistant");
        assert_eq!(out["parentUuid"], "u0");
        assert_eq!(out["version"], "2.1.285");
        let block = &out["message"]["content"][0];
        assert_eq!(block["name"], "TaskUpdate");
        assert_eq!(block["input"]["taskId"], "3");
        assert_eq!(block["input"]["status"], "completed");
        let subject = block["input"]["subject"].as_str().unwrap();
        assert!(
            !subject.contains("ACME") && subject.len() == 24,
            "{subject}"
        );
        assert_eq!(out["cwd"], s.path("/Users/alice/acme"));

        // A JSON string is walked, not masked as prose.
        let codex = r#"{"type":"response_item","payload":{"type":"function_call","name":"update_plan","arguments":"{\"plan\":[{\"step\":\"Write tests\",\"status\":\"in_progress\"}]}"}}"#;
        let out: Value = serde_json::from_str(s.jsonl(codex).trim_end()).unwrap();
        let args: Value =
            serde_json::from_str(out["payload"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["plan"][0]["status"], "in_progress");
        assert_ne!(args["plan"][0]["step"], "Write tests");

        // A torn line stays torn and loses its words.
        let torn = s.jsonl("{\"type\":\"user\",\"message\":{\"content\":\"secret plan");
        assert!(serde_json::from_str::<Value>(&torn).is_err());
        assert!(!torn.contains("secret"));
    }

    /// RG-28 review: a kept key name is not enough. Under the agent's own
    /// envelope a kept value must look like its key's structure; inside a
    /// tool's data only what a reader takes from there survives.
    #[test]
    fn a_kept_key_keeps_only_a_value_shaped_like_structure() {
        let s = scrub();
        // Envelope: prose under `source`/`status`/`version`/`model` is text.
        let line = r#"{"type":"user","uuid":"u-1","source":"Alice's laptop","status":"waiting on Bob","version":"Q3 board deck","model":"Bob/acme","timestamp":"yesterday at noon","created_at":"2026-10-09T10:00:00+03:00"}"#;
        let out: Value = serde_json::from_str(s.jsonl(line).trim_end()).unwrap();
        assert_eq!(out["uuid"], "u-1");
        assert_eq!(out["created_at"], "2026-10-09T10:00:00+03:00");
        for (key, word) in [
            ("source", "laptop"),
            ("status", "waiting"),
            ("version", "board"),
            ("model", "acme"),
            ("timestamp", "noon"),
        ] {
            let v = out[key].as_str().unwrap();
            assert!(!v.contains(word), "{key}: {v}");
        }

        // Codex: a call's arguments and its output are data. `update_plan`'s
        // statuses survive (the reader needs them); a CRM's `id`, `status`
        // and `type` do not.
        let call = r#"{"type":"response_item","payload":{"type":"function_call","name":"crm_lookup","call_id":"c1","arguments":"{\"id\":\"AcmeCorp-77\",\"status\":\"overdue\",\"type\":\"Acme\"}"}}"#;
        let out = s.jsonl(call);
        assert!(out.contains(r#""name":"crm_lookup""#), "{out}");
        assert!(out.contains(r#""call_id":"c1""#), "{out}");
        for leak in ["Acme", "overdue"] {
            assert!(!out.contains(leak), "{leak}: {out}");
        }
        let output = r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"{\"id\":\"42\",\"status\":\"completed\",\"source\":\"AcmeCRM\",\"version\":\"v9-acme\"}"}}"#;
        let out = s.jsonl(output);
        for leak in ["AcmeCRM", "acme"] {
            assert!(!out.contains(leak), "{leak}: {out}");
        }

        // A "tool_use" inside a result is the result's JSON, not a call: its
        // `name` is masked.
        let nested = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"tool_use","name":"JaneDoe"}]}]}}"#;
        let out = s.jsonl(nested);
        assert!(!out.contains("JaneDoe"), "{out}");
        assert!(out.contains(r#""tool_use_id":"t1""#), "{out}");
    }

    /// RG-28 second review: keys and numbers. Every shape below came out
    /// byte for byte on the version that masked only values.
    #[test]
    fn a_key_or_a_number_in_tool_data_is_masked_unless_a_reader_needs_it() {
        let s = scrub();
        // JSON in a result's string content, keyed by a name and an address.
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"{\"Jane Doe\":{\"iban_no\":4012888888881881,\"salary\":185000},\"jdoe@clientbank.com\":true}"}]}}"#;
        let out = s.jsonl(result);
        for leak in [
            "Jane",
            "iban",
            "salary",
            "4012888888881881",
            "185000",
            "jdoe",
            "clientbank",
            "@",
        ] {
            assert!(!out.contains(leak), "{leak}: {out}");
        }
        // The numbers keep their digit count.
        let v: Value = serde_json::from_str(out.trim_end()).unwrap();
        let inner: Value =
            serde_json::from_str(v["message"]["content"][0]["content"].as_str().unwrap()).unwrap();
        let mut digits: Vec<usize> = inner
            .as_object()
            .unwrap()
            .values()
            .filter_map(Value::as_object)
            .flat_map(|o| o.values())
            .map(|n| n.as_u64().unwrap().to_string().len())
            .collect();
        digits.sort();
        assert_eq!(digits, [6, 16], "{inner}");

        // A call's input keyed by a client, and Claude Code's structured
        // result keyed by a person.
        let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"mcp__crm__q","input":{"customers":{"AcmeCorp Holdings":{"owed":40000}}}}]},"toolUseResult":{"structuredContent":{"Bob Smith":1}}}"#;
        let out = s.jsonl(call);
        for leak in [
            "customers",
            "AcmeCorp",
            "Holdings",
            "owed",
            "40000",
            "structuredContent",
            "Bob",
            "Smith",
        ] {
            assert!(!out.contains(leak), "{leak}: {out}");
        }
        assert!(out.contains(r#""name":"mcp__crm__q""#), "{out}");

        // Codex's shell call: the environment's variable names.
        let shell = r#"{"type":"response_item","payload":{"type":"local_shell_call","call_id":"c9","action":{"type":"exec","command":["make"],"env":{"ACME_SECRET_TOKEN_NAME":"x"}}}}"#;
        let out = s.jsonl(shell);
        assert!(!out.contains("ACME") && !out.contains("SECRET"), "{out}");
        assert!(out.contains(r#""call_id":"c9""#), "{out}");

        // Outside data, a key with a space or an address in it is text.
        let envelope = r#"{"type":"user","jane.doe@clientbank.com":1,"Jane Doe":2,"uuid":"u"}"#;
        let out = s.jsonl(envelope);
        for leak in ["jane", "Jane", "clientbank", "@"] {
            assert!(!out.contains(leak), "{leak}: {out}");
        }
        assert!(out.contains(r#""uuid":"u""#), "{out}");

        // What the readers take from data stays: task ids as numbers and as
        // strings, statuses, the plan's keys.
        let task = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t3","name":"TaskUpdate","input":{"taskId":12,"status":"completed","addBlockedBy":[3,"4"]}}]},"toolUseResult":{"task":{"id":12,"subject":"x"}}}"#;
        let v: Value = serde_json::from_str(s.jsonl(task).trim_end()).unwrap();
        let input = &v["message"]["content"][0]["input"];
        assert_eq!(input["taskId"], 12);
        assert_eq!(input["status"], "completed");
        assert_eq!(input["addBlockedBy"], serde_json::json!([3, "4"]));
        assert_eq!(v["toolUseResult"]["task"]["id"], 12);
    }

    /// RG-28 third review: `id` is a key the readers take task ids from and
    /// a key any tool's JSON uses. A task id is short; a card number under
    /// `id` or `taskId` is not one, as a number or as a string.
    #[test]
    fn a_long_number_under_a_task_id_key_is_masked() {
        let s = scrub();
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"{\"customers\":[{\"id\":4012888888881881},{\"id\":\"5500005555555559\"}]}"}]}}"#;
        let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"TaskUpdate","input":{"taskId":"4012888888881881","addBlockedBy":[5500005555555559]}}]}}"#;
        for line in [result, call] {
            let out = s.jsonl(line);
            for leak in ["4012888888881881", "5500005555555559"] {
                assert!(!out.contains(leak), "{leak}: {out}");
            }
        }
        // Masked to the same digit count, so the shape stays a long id.
        let v: Value = serde_json::from_str(s.jsonl(call).trim_end()).unwrap();
        let id = v["message"]["content"][0]["input"]["taskId"]
            .as_str()
            .unwrap();
        assert_eq!(id.len(), 16, "{id}");
        // The largest kept id and the smallest masked one, both spellings.
        let edge = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t3","name":"TaskUpdate","input":{"taskId":"999999","addBlockedBy":[999999,1000000,"1000000","007"]}}]}}"#;
        let v: Value = serde_json::from_str(s.jsonl(edge).trim_end()).unwrap();
        let input = &v["message"]["content"][0]["input"];
        assert_eq!(input["taskId"], "999999");
        assert_eq!(input["addBlockedBy"][0], 999_999);
        assert_ne!(input["addBlockedBy"][1], 1_000_000);
        assert_ne!(input["addBlockedBy"][2], "1000000");
        assert_ne!(input["addBlockedBy"][3], "007");
    }

    /// RG-28 third review: `blockedBy` was kept as a number and masked as a
    /// string, so the same id written the two ways no longer matched.
    #[test]
    fn blocked_by_keeps_a_task_id_in_either_spelling() {
        let s = scrub();
        // Two digits: a one-character word survives masking anyway.
        let line = r#"{"type":"user","toolUseResult":{"task":{"id":"7","blockedBy":["13",13]}}}"#;
        let v: Value = serde_json::from_str(s.jsonl(line).trim_end()).unwrap();
        assert_eq!(
            v["toolUseResult"]["task"]["blockedBy"],
            serde_json::json!(["13", 13])
        );
    }

    /// RG-28 third review: a tool's answer in an envelope no reader opens —
    /// Codex's `mcp_tool_call_end`, Claude Code's `progress` — kept every
    /// key of one token and every number.
    #[test]
    fn a_tools_answer_in_an_unread_envelope_is_masked() {
        let s = scrub();
        let codex = r#"{"timestamp":"2026-10-09T10:00:00Z","type":"event_msg","payload":{"type":"mcp_tool_call_end","call_id":"c1","invocation":{"server":"crm","tool":"q","arguments":{"JaneDoe":{"iban_no":4012888888881881}}},"result":{"Ok":{"structuredContent":{"JaneDoe":{"salary":185000}}}}}}"#;
        let claude = r#"{"type":"progress","uuid":"p1","data":{"type":"x","result":{"AcmeCorp":{"card":4012888888881881}}}}"#;
        let shell = r#"{"type":"event_msg","payload":{"type":"exec_command_end","call_id":"c2","stdout":"JaneDoe 185000","exit_code":0}}"#;
        for line in [codex, claude, shell] {
            let out = s.jsonl(line);
            for leak in [
                "JaneDoe",
                "iban",
                "4012888888881881",
                "salary",
                "185000",
                "AcmeCorp",
                "card",
            ] {
                assert!(!out.contains(leak), "{leak}: {out}");
            }
        }
        let v: Value = serde_json::from_str(s.jsonl(codex).trim_end()).unwrap();
        assert_eq!(v["payload"]["type"], "mcp_tool_call_end");
        assert_eq!(v["payload"]["call_id"], "c1");
        let v: Value = serde_json::from_str(s.jsonl(shell).trim_end()).unwrap();
        assert_eq!(v["payload"]["exit_code"], 0);
    }

    /// `ALWAYS_DATA` is masked wherever it appears, which is free only while
    /// no reader looks one of those keys up. A reader that starts to would
    /// read a capture differently from its original; this says why first.
    #[test]
    fn no_reader_looks_up_an_always_data_key() {
        let readers = [
            ("claude.rs", include_str!("claude.rs")),
            ("codex.rs", include_str!("codex.rs")),
            ("antigravity.rs", include_str!("antigravity.rs")),
        ];
        for (file, source) in readers {
            for key in ALWAYS_DATA {
                assert!(
                    !source.contains(&format!("\"{key}\"")),
                    "{file} looks up \"{key}\", which the anonymiser masks as a tool's data"
                );
            }
        }
        // The scan can see one.
        assert!(include_str!("codex.rs").contains("\"call_id\""));
    }

    /// A masked key that spells one already in the object must not replace
    /// it: both values survive.
    #[test]
    fn a_masked_key_never_overwrites_another() {
        // A salt and a two-letter word that masks to `id` under it.
        let words: Vec<String> = (b'a'..=b'z')
            .flat_map(|a| (b'a'..=b'z').map(move |b| String::from_utf8(vec![a, b]).unwrap()))
            .collect();
        let (s, clash) = (0..64)
            .map(|salt| Scrub::new(salt, None))
            .find_map(|s| {
                let w = words
                    .iter()
                    .find(|w| *w != "id" && s.word(w) == "id")?
                    .clone();
                Some((s, w))
            })
            .expect("some two-letter word masks to `id`");
        let line = format!(r#"{{"type":"tool_result","content":{{"id":"1","{clash}":"2"}}}}"#);
        let v: Value = serde_json::from_str(&s.json(&line)).unwrap();
        let content = v["content"].as_object().unwrap();
        assert_eq!(content.len(), 2, "{content:?}");
        assert_eq!(content["id"], "1");
    }

    #[test]
    fn a_kept_value_loses_an_address_and_the_user_in_any_case() {
        let s = scrub();
        assert_eq!(s.kept("Alice-MBP.ALICE.local"), "dev-MBP.dev.local");
        assert_eq!(s.kept("x-jane.doe@clientbank.com"), "[email]");
    }

    /// RG-28 review: `trackedFileBackups` keys by file name, and a relative
    /// one has no `/` to give it away.
    #[test]
    fn a_path_keyed_map_loses_relative_keys_too() {
        let s = scrub();
        let line = r#"{"type":"file-history-snapshot","snapshot":{"trackedFileBackups":{"acme_payroll_2026.xlsx":{"version":1},"/Users/alice/acme/b.rs":{"version":2}}}}"#;
        let out = s.jsonl(line);
        assert!(!out.contains("acme"), "{out}");
        assert!(!out.contains("payroll"), "{out}");
        assert!(out.contains(r#""version":1"#), "{out}");
    }

    /// RG-28 review: the salt was the clock and the pid, and macOS's clock
    /// moves in microseconds: two captures drawn in the same microsecond got
    /// the same salt, and the salt space was the hours a commit date allows.
    /// A thousand draws in a row must all differ.
    #[test]
    fn fresh_salts_do_not_repeat() {
        let draws: std::collections::HashSet<u64> =
            (0..1000).map(|_| Scrub::fresh_salt()).collect();
        assert_eq!(draws.len(), 1000);
    }

    #[test]
    fn the_capturers_name_leaves_kept_values_too() {
        let s = scrub();
        let line = r#"{"type":"user","name":"alice-tool","sessionId":"s"}"#;
        let out = s.jsonl(line);
        assert!(!out.contains("alice"), "{out}");
    }
}
