//! A single skill and the `.md` format.
//!
//! The format was inherited from the Swift side (`Tacet/Skills/*.md`):
//! frontmatter (`name`, `triggers`, `tools`) + markdown body. Files are written
//! "core-first": the concrete `tool(args)` example and the unbreakable rules
//! sit ABOVE the `<!--/core-->` marker, the human reference below it.

use crate::matching::lowercase;

/// Body limit for a skill the user wrote themselves.
///
/// NARROWER than a bundled skill (700): the bundled files were measured,
/// reviewed and written core-first; the user's free text is an unreviewed
/// input and it lands in the most expensive spot of the 4096 window (right in
/// front of the question). 500 comfortably fits a typical "answer in this
/// style" instruction but keeps a long essay away from the prompt.
pub const USER_BODY_LIMIT: usize = 500;

/// A skill: name, triggers and guide text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    /// Lowercased triggers; matching works off these.
    ///
    /// MULTILINGUAL, LATER: this list is DATA, not code. It is English-only
    /// today, which means a user writing in another language gets no skill
    /// injected (the model is left without a guide — weaker, not broken).
    /// Adding per-language trigger lists is a product decision, not a refactor.
    pub triggers: Vec<String>,
    pub text: String,
    /// Names of the tools this guide COMMANDS (frontmatter `tools:`).
    /// If empty the skill is tool-independent and free in every catalog.
    pub tools: Vec<String>,
    /// Whether the user wrote it — on a tie the user's own skill wins.
    pub is_users: bool,
}

impl Skill {
    /// A bundled skill (reviewed, written core-first).
    pub fn package(
        name: impl Into<String>,
        triggers: Vec<String>,
        text: impl Into<String>,
        tools: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            triggers: triggers.iter().map(|t| lowercase(t)).collect(),
            text: text.into(),
            tools,
            is_users: false,
        }
    }

    /// A user skill. The body is trimmed HERE, not at injection time: if
    /// trimming happens in one place the "what is saved is what goes to the
    /// model" guarantee holds; in two places the board would show 900
    /// characters while the model saw 500.
    pub fn users(
        name: impl Into<String>,
        triggers: Vec<String>,
        text: impl Into<String>,
    ) -> Self {
        let raw: String = text.into();
        let trimmed: String = raw.chars().take(USER_BODY_LIMIT).collect();
        Self {
            name: name.into(),
            triggers: triggers.iter().map(|t| lowercase(t)).collect(),
            text: trimmed,
            tools: Vec::new(),
            is_users: true,
        }
    }

    /// Are ALL the tools this skill declares present in the catalog.
    ///
    /// The gate is on "all of them" because a guide can describe a two-step
    /// flow (first `read_document`, then `create_document`); if half of it is
    /// missing the guide cannot be followed anyway, and injecting it makes the
    /// model call a tool that does not exist.
    pub fn has_tools(&self, available: Option<&[String]>) -> bool {
        let Some(available) = available else {
            return true; // passing nil means no filtering (test/preview path)
        };
        if self.tools.is_empty() {
            return true;
        }
        self.tools.iter().all(|t| available.iter().any(|a| a == t))
    }
}

/// Parses frontmatter + body. A file with no triggers DOES NOT COUNT as a skill
/// (`None`): a trigger-less skill can never be selected, but it would sit in
/// the catalog and produce a "why doesn't this work" question.
pub fn parse(default_name: &str, raw: &str) -> Option<Skill> {
    let lines: Vec<&str> = raw.split('\n').map(|s| s.trim_end_matches('\r')).collect();
    let mut name = default_name.to_string();
    let mut triggers: Vec<String> = Vec::new();
    let mut tools: Vec<String> = Vec::new();
    let mut body = raw.trim().to_string();

    if lines.first().map(|s| s.trim()) == Some("---")
        && let Some(closing) = lines
            .iter()
            .skip(1)
            .position(|s| s.trim() == "---")
            .map(|i| i + 1)
    {
        for line in &lines[1..closing] {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "name" => name = value.to_string(),
                "triggers" => triggers = comma_separated(value, true),
                "tools" => tools = comma_separated(value, false),
                _ => {}
            }
        }
        body = lines[(closing + 1)..].join("\n").trim().to_string();
    }

    if triggers.is_empty() {
        return None;
    }
    Some(Skill::package(name, triggers, body, tools))
}

/// "a, b, c" -> ["a","b","c"]; empty parts are dropped.
fn comma_separated(value: &str, lower: bool) -> Vec<String> {
    value
        .split(',')
        .map(|p| {
            let t = p.trim();
            if lower { lowercase(t) } else { t.to_string() }
        })
        .filter(|p| !p.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_reads_name_triggers_and_tools() {
        let s = parse(
            "file-name",
            "---\nname: calc\ntriggers: Calculate, how much is\ntools: calculate\n---\n# Arithmetic\nBody.",
        )
        .expect("valid skill");
        assert_eq!(s.name, "calc");
        assert_eq!(s.triggers, vec!["calculate", "how much is"]);
        assert_eq!(s.tools, vec!["calculate"]);
        assert_eq!(s.text, "# Arithmetic\nBody.");
        assert!(!s.is_users);
    }

    #[test]
    fn file_without_triggers_is_not_a_skill() {
        assert!(parse("x", "---\nname: empty\n---\nBody").is_none());
        assert!(parse("x", "no frontmatter, plain text").is_none());
    }

    #[test]
    fn user_body_is_trimmed_at_500_characters() {
        let long = "a".repeat(900);
        let s = Skill::users("mine", vec!["trigger".into()], long);
        assert_eq!(s.text.chars().count(), USER_BODY_LIMIT);
        assert!(s.is_users);
    }

    #[test]
    fn tool_gate_filters_out_a_missing_tool() {
        let s = parse(
            "x",
            "---\nname: two-step\ntriggers: edit\ntools: read_document, edit_document\n---\nBody",
        )
        .unwrap();
        let full = vec!["read_document".to_string(), "edit_document".to_string()];
        let half = vec!["read_document".to_string()];
        assert!(s.has_tools(Some(&full)));
        assert!(!s.has_tools(Some(&half)), "a half flow must not be injected");
        assert!(s.has_tools(None), "None = no filtering");
    }

    #[test]
    fn a_skill_declaring_no_tools_is_free_in_every_catalog() {
        let s = Skill::users("mine", vec!["trigger".into()], "body");
        assert!(s.has_tools(Some(&[])));
    }
}
