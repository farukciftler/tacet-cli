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
//! `<command-name>`, `- [x]`, `file://`). Those survive. Every other string
//! value is MASKED: each word becomes a pseudo-word of the same length and
//! case pattern, the same word always the same pseudo-word within one capture
//! (so a path in a `cwd` field and the same path inside a tool call still
//! agree, and Claude Code's project directory can be recomputed from the
//! masked `cwd`). Home directories become `dev` before masking, so
//! `/Users/<anyone>/…` reads `/Users/dev/…` the way the corpus scan demands,
//! and e-mail addresses are replaced outright.
//!
//! THE SALT IS PER CAPTURE AND NEVER STORED. Without it the masking is a
//! deterministic function of the word, and a dictionary of user names and
//! common words would read a masked fixture back. With a random salt that is
//! printed once and not written anywhere, it cannot be.
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
/// `codex.rs`, `antigravity.rs` — and the brief's field lists; a key that is
/// not here is masked, so forgetting one costs a fixture its meaning, never
/// its privacy.
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

    /// A salt nobody chose: the clock and the process id, hashed.
    pub fn fresh_salt() -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        splitmix(fnv(&now.to_le_bytes()) ^ u64::from(std::process::id()))
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
                    self.value(&mut v, None);
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
                self.value(&mut v, None);
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

    fn value(&self, v: &mut Value, key: Option<&str>) {
        match v {
            Value::String(s) => {
                *s = if key.is_some_and(|k| KEEP_KEYS.contains(&k)) {
                    self.kept(s)
                } else {
                    self.string(s)
                };
            }
            // An array inherits its key: `addBlockedBy: ["1"]` is ids.
            Value::Array(items) => {
                for item in items {
                    self.value(item, key);
                }
            }
            Value::Object(map) => {
                // Keys are structure, except where a format keys a map by
                // file path (Claude Code's `trackedFileBackups`).
                let is_call = key == Some("tool_calls")
                    || map
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|t| TOOL_OBJECT_TYPES.contains(&t));
                let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
                for (k, mut child) in entries {
                    match &mut child {
                        Value::String(s) if is_call && TOOL_NAMES.contains(&k.as_str()) => {
                            *s = self.kept(s);
                        }
                        _ => self.value(&mut child, Some(&k)),
                    }
                    let k = if k.contains('/') || k.contains('\\') {
                        self.text(&k)
                    } else {
                        k
                    };
                    map.insert(k, child);
                }
            }
            Value::Number(_) | Value::Bool(_) | Value::Null => {}
        }
    }

    /// A masked string value — unless it is JSON itself: Codex carries a
    /// call's arguments as a JSON string (`update_plan`'s plan is in one), and
    /// masking its braces' contents as prose would lose the plan.
    fn string(&self, s: &str) -> String {
        let t = s.trim_start();
        if (t.starts_with('{') || t.starts_with('['))
            && let Ok(mut inner) = serde_json::from_str::<Value>(s)
        {
            self.value(&mut inner, None);
            return inner.to_string();
        }
        self.text(s)
    }

    /// A kept value, still put through the home rule and the user name.
    fn kept(&self, s: &str) -> String {
        let mut out = homes_to_dev(s);
        if let Some(user) = &self.user {
            out = out.replace(user.as_str(), "dev");
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
            if let Some(len) = tag_len(rest) {
                out.push_str(&rest[..len]);
                rest = &rest[len..];
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

/// The length of a markup tag at the start of `text`: `<TAG>`, `</TAG>`,
/// `<command-name>`. Tag names are format, not content — the readers split a
/// request out of `<USER_REQUEST>…</USER_REQUEST>` by them.
fn tag_len(text: &str) -> Option<usize> {
    let body = text.strip_prefix('<')?;
    let body = body.strip_prefix('/').unwrap_or(body);
    let first = body.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let name = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(body.len());
    (body[name..].starts_with('>')).then(|| text.len() - body.len() + name + 1)
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

    #[test]
    fn the_capturers_name_leaves_kept_values_too() {
        let s = scrub();
        let line = r#"{"type":"user","name":"alice-tool","sessionId":"s"}"#;
        let out = s.jsonl(line);
        assert!(!out.contains("alice"), "{out}");
    }
}
