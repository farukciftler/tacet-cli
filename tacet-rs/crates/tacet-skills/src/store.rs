//! `SkillStore` — holds the skills and picks the ONE skill matching a message.
//!
//! Two sources: the defaults that ship with the package (BAKED INTO the build)
//! and the user's `skills/*.md` files under the config directory (see
//! `crate::user_dir`).
//!
//! WHY BAKED IN: package skills are part of the program's behaviour, not its
//! data. Read from disk, the binary would not work on its own and a missing
//! file would be a SILENT loss of behaviour (the model is left without a guide
//! and nobody notices). User skills are the exact opposite: they are data and
//! they come from disk.

use crate::matching::{lowercase, score};
use crate::skill::{Skill, parse};
use std::path::Path;

/// The default skills baked into the build. One line per file; adding a new
/// skill is a deliberate decision, not dropping a file into a directory.
const PACKAGE_FILES: &[(&str, &str)] = &[
    ("calc", include_str!("../skills/calc.md")),
    ("read-document", include_str!("../skills/read-document.md")),
    ("create-document", include_str!("../skills/create-document.md")),
    ("time", include_str!("../skills/time.md")),
];

#[derive(Debug, Clone, Default)]
pub struct SkillStore {
    package: Vec<Skill>,
    user: Vec<Skill>,
}

impl SkillStore {
    /// An empty store — for tests and the "user skills only" scenario.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The default set shipped with the package.
    ///
    /// Named `default_set` rather than `default` so it cannot be confused with
    /// the derived `Default::default()`, which returns an EMPTY store.
    pub fn default_set() -> Self {
        Self {
            package: PACKAGE_FILES
                .iter()
                .filter_map(|(name, raw)| parse(name, raw))
                .collect(),
            user: Vec::new(),
        }
    }

    /// Loads the `.md` files in a directory as USER skills; returns how many
    /// were loaded.
    ///
    /// If the directory is missing or a file is broken it is SILENTLY skipped
    /// and 0/fewer is returned: the assistant failing to open because of a
    /// half-written user file would be an unacceptable trade. A file that
    /// cannot be parsed means "no skill", not "error".
    pub fn load_from_dir(&mut self, dir: impl AsRef<Path>) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let mut count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("skill")
                .to_string();
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(s) = parse(&name, &raw) else {
                continue;
            };
            // Everything coming off disk is a USER skill: unreviewed text does
            // not deserve the wide budget of a package skill (700).
            self.user.push(Skill::users(s.name, s.triggers, s.text));
            count += 1;
        }
        count
    }

    /// Replaces the user skills wholesale (the board calls this on every save).
    pub fn reload_user(&mut self, skills: Vec<Skill>) {
        self.user = skills
            .into_iter()
            .map(|s| Skill::users(s.name, s.triggers, s.text))
            .collect();
    }

    /// USER FIRST: on an equal score the user's own skill wins (see `matching`).
    pub fn all(&self) -> impl Iterator<Item = &Skill> {
        self.user.iter().chain(self.package.iter())
    }

    pub fn skill(&self, name: &str) -> Option<&Skill> {
        self.all().find(|s| s.name == name)
    }

    pub fn count(&self) -> usize {
        self.user.len() + self.package.len()
    }

    /// Returns the ONE skill best matching the given message (`None` if there
    /// is none).
    ///
    /// ONE, because the point is progressive disclosure: injecting all of them
    /// fills the 4096 window with guides and the model explains the guide
    /// instead of calling a tool. The score is the SUM OF LENGTHS of the
    /// matching triggers (see `matching::score`), not the count.
    ///
    /// If `available_tools` is given, skills whose guide COMMANDS a tool that
    /// is not present are filtered out — the single source of truth is the tool
    /// catalog, not a hand-maintained map. Passing `None` disables filtering.
    pub fn matching(&self, message: &str, available_tools: Option<&[String]>) -> Option<&Skill> {
        let m = lowercase(message);
        let mut best: Option<(&Skill, usize)> = None;
        for s in self.all() {
            if !s.has_tools(available_tools) {
                continue;
            }
            let p = score(&m, &s.triggers);
            // STRICT greater-than: on a tie the one that came FIRST stays, i.e.
            // the user's own, given the order of `all`. We do not leave ties to
            // randomness.
            if p > 0 && p > best.map_or(0, |(_, b)| b) {
                best = Some((s, p));
            }
        }
        best.map(|(s, _)| s)
    }

    /// Merges the skills with the given names into a single text (for
    /// preview/audit; the production path injects ONE skill).
    pub fn merge(&self, names: &[&str]) -> String {
        names
            .iter()
            .filter_map(|n| self.skill(n))
            .map(|s| format!("## {}\n{}", s.name, s.text))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injection::{INJECTION_LIMIT, injection_text};

    fn tools() -> Vec<String> {
        ["calculate", "time", "read_document", "create_document"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn package_skills_ship_baked_in_and_all_of_them_parse() {
        let s = SkillStore::default_set();
        assert_eq!(s.count(), PACKAGE_FILES.len(), "a file failed to parse");
        assert!(s.skill("calc").is_some());
        assert!(s.skill("create-document").is_some());
    }

    #[test]
    fn a_specific_phrase_beats_a_generic_word() {
        let s = SkillStore::default_set();
        // "as a table" (10) is in read-document; create-document has
        // "make a table" but that does not occur in this sentence. Had the
        // count been used the order would have been random.
        let hit = s.matching("show this as a table", Some(&tools())).unwrap();
        assert_eq!(hit.name, "read-document");
    }

    #[test]
    fn the_tool_gate_blocks_a_skill_from_being_selected() {
        let s = SkillStore::default_set();
        let missing = vec!["time".to_string()];
        assert!(s.matching("make this an excel file", Some(&missing)).is_none());
        assert!(s.matching("make this an excel file", Some(&tools())).is_some());
    }

    #[test]
    fn nothing_is_injected_when_there_is_no_match() {
        let s = SkillStore::default_set();
        assert!(s.matching("hello, how are you", Some(&tools())).is_none());
    }

    #[test]
    fn a_short_root_does_not_summon_the_wrong_skill() {
        let s = SkillStore::default_set();
        // The trigger "pdf" sits inside "pdfs"; the whole-term condition must
        // hold. (The measured original was Turkish: "dok" inside "dokuz".)
        assert!(
            s.matching("pdfs everywhere", Some(&tools())).map(|b| b.name.clone())
                != Some("create-document".into())
        );
    }

    #[test]
    fn a_user_skill_wins_on_a_tie() {
        let mut s = SkillStore::default_set();
        // Same trigger: the scores are equal, and the order of `all` puts the
        // user first.
        s.reload_user(vec![Skill::users(
            "my-calc",
            vec!["calculate".into()],
            "Always give the result in a table.",
        )]);
        let hit = s.matching("calculate this", Some(&tools())).unwrap();
        assert_eq!(hit.name, "my-calc");
    }

    #[test]
    fn a_skill_loaded_from_disk_is_a_user_skill() {
        let dir = std::env::temp_dir().join("tacet-skills-test-1");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mine.md"),
            format!("---\nname: mine\ntriggers: sharp trigger\n---\n{}", "g".repeat(900)),
        )
        .unwrap();
        // Broken file: must be skipped silently, loading must not crash.
        std::fs::write(dir.join("broken.md"), "body without triggers").unwrap();
        std::fs::write(dir.join("unreadable.txt"), "---\ntriggers: a\n---\nx").unwrap();

        let mut s = SkillStore::empty();
        assert_eq!(s.load_from_dir(&dir), 1);
        let hit = s.skill("mine").unwrap();
        assert!(hit.is_users);
        assert_eq!(hit.text.chars().count(), crate::skill::USER_BODY_LIMIT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_directory_returns_zero_not_an_error() {
        let mut s = SkillStore::empty();
        assert_eq!(s.load_from_dir("/definitely/missing/dir"), 0);
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn every_package_skills_injection_stays_within_budget() {
        let s = SkillStore::default_set();
        for skill in s.all() {
            let body = crate::injection::injection_body(&skill.text, INJECTION_LIMIT);
            assert!(
                body.chars().count() <= INJECTION_LIMIT,
                "{} overflowed: {}",
                skill.name,
                body.chars().count()
            );
            // The core marker must NOT LEAK to the model; it is a file marker.
            assert!(!injection_text(skill).contains(crate::injection::CORE_MARKER));
        }
    }

    #[test]
    fn merge_only_takes_names_that_exist() {
        let s = SkillStore::default_set();
        let m = s.merge(&["calc", "no-such-thing"]);
        assert!(m.starts_with("## calc"));
        assert!(!m.contains("no-such"));
    }
}
