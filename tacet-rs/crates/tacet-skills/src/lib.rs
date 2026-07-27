//! tacet-skills — the skill layer; the on-device counterpart of Claude's
//! SKILL.md logic.
//!
//! Each tool's detailed usage guide is a `.md` file (frontmatter + body). To
//! keep the 4096 token window from swelling, "progressive disclosure": not all
//! of them at once, but the ONE skill matching the current message, attached to
//! THAT TURN's prompt.
//!
//! ## Decisions that will not change (MEASURED on the Swift side)
//!
//! 1. **Skills are NOT baked into the system instruction.** Once baked in, the
//!    small model started EXPLAINING the guide instead of CALLING the tool;
//!    that was the source of the regression. The fixed instruction stays short.
//! 2. **One skill, per-turn injection**, with a `<guidance>` fence, capped at
//!    700 characters. User skill body 500.
//! 3. **Score = SUM OF LENGTHS of the matching triggers**, not the count.
//!    If the count is used, randomness decides the order on a tie.
//!
//! ## Work left to the shell
//!
//! This crate DOES NOT BUILD THE PROMPT. The shell phase picks the skill with
//! `store.matching(message, tools)`, asks `InjectionState` whether it is needed
//! this turn, and hands the `injection_text(...)` output to
//! `tacet_engine::Prompt::with_guide`. It is split this way because building
//! the prompt is the engine's job and picking the skill is this layer's.
//!
//! NO NETWORK: only local files are read here.

pub mod injection;
pub mod matching;
pub mod skill;
pub mod store;

pub use injection::{
    CORE_MARKER, INJECTION_LIMIT, InjectionState, injection_body, injection_text, split_core,
};
pub use matching::{WHOLE_TERM_LIMIT, contains, lowercase, score};
pub use skill::{Skill, USER_BODY_LIMIT, parse};
pub use store::SkillStore;

/// The directory user skills are read from: config directory + `skills`.
///
/// The path itself IS NOT COMPUTED HERE. Memory, MCP and skills have to point
/// at the same directory; the rule lives in a single place, `tacet_core::env`,
/// and that is where the platform difference (XDG / `%APPDATA%`) is known. The
/// `TACET_HOME` variable still overrides it — so tests and the developer shell
/// can run without polluting the real settings directory.
pub fn user_dir() -> Option<std::path::PathBuf> {
    tacet_core::config_path("skills")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_to_end_selection_and_injection() {
        // This layer's contract: message -> skill -> fenced text, within 700.
        let store = SkillStore::default_set();
        let tools: Vec<String> = ["calculate".into(), "create_document".into()].into();
        let s = store.matching("what is 125 times 8", Some(&tools)).unwrap();
        assert_eq!(s.name, "calc");

        let text = injection_text(s);
        assert!(text.contains("<guidance name=\"calc\">"));
        assert!(text.contains("never compute in your head"));
        assert!(text.chars().count() < INJECTION_LIMIT + 200, "fence + body");
    }

    #[test]
    fn user_dir_is_redirected_by_tacet_home() {
        // `temp_dir()` — a hardcoded `/tmp` would be a meaningless path on Windows.
        let home = std::env::temp_dir().join("tacet-skills-test-home");
        // SAFETY: single-threaded test; the env var is only read here.
        unsafe { std::env::set_var("TACET_HOME", &home) };
        assert_eq!(user_dir().unwrap(), home.join("skills"));
        unsafe { std::env::remove_var("TACET_HOME") };
    }
}
