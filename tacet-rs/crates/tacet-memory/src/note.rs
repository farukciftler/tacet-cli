//! `MemoryNote` — the memory layer's only persistent record, and the limit
//! constants.
//!
//! The limits are HARD because matching notes are injected into the 4096 token
//! window; every character is budget. When the cap is reached there is NO
//! AUTOMATIC EVICTION: deleting silently is the mirror image of the "no silent
//! learning" principle. What is learned and what is forgotten are both the
//! user's call.

use serde::{Deserialize, Serialize};
use tacet_skills::lowercase;

/// Upper limit of a note's text (~40 tokens).
pub const TEXT_LIMIT: usize = 160;

/// Lower limit. A "fact" shorter than this is either noise or a fragment the
/// model half-copied; neither is worth remembering.
pub const MIN_TEXT: usize = 10;

/// The most notes that can be stored. Once full, a new record IS NOT TAKEN.
pub const TOTAL_CAP: usize = 50;

/// Most keys per note. Limited because the match scan runs on EVERY message:
/// 50 notes x unlimited keys would be a tax paid every single turn.
pub const KEY_LIMIT: usize = 8;

/// The note's kind. In v1 it is only a label on the board; it plays NO role in
/// injection priority.
///
/// DISK FORMAT — THIS ENUM IS SERIALIZED. `rename_all = "snake_case"` means the
/// variant names ARE the on-disk values. Records written before the rename say
/// `kimlik`/`tercih`/`iliski`/`olgu`, so each variant carries a `serde(alias)`
/// for its old value. Dropping those aliases silently downgrades every existing
/// note to `Fact` (the `Default`) — do not remove them without a migration that
/// rewrites the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    #[serde(alias = "kimlik")]
    Identity,
    #[serde(alias = "tercih")]
    Preference,
    #[serde(alias = "iliski")]
    Relation,
    #[default]
    #[serde(alias = "olgu")]
    Fact,
}

impl MemoryKind {
    /// Resolves a kind from text. An unknown value returns `None` and the note
    /// IS DROPPED — it is not downgraded to the default: if the model is making
    /// up the kind, the note itself is suspect.
    /// ASCII lowercasing is used, not Turkish: these values are the SCHEMA
    /// LABELS shown to the model ("identity", "preference"), not Turkish words.
    /// Turkish lowercasing would turn "IDENTITY" into "ıdentity" and a valid
    /// kind would be rejected whenever the model wrote it in capitals.
    pub fn resolve(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "identity" => Some(Self::Identity),
            "preference" => Some(Self::Preference),
            "relation" => Some(Self::Relation),
            "fact" => Some(Self::Fact),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Preference => "preference",
            Self::Relation => "relation",
            Self::Fact => "fact",
        }
    }
}

/// DISK FORMAT — the field names below are the on-disk JSON keys. Files written
/// before the rename carry `metin`/`tur`/`anahtarlar`/`olusturulma`/`aktif`,
/// hence the `serde(alias)` on each. Removing an alias makes an existing
/// memory file fail to parse, which the store turns into an EMPTY store — a
/// silent total memory loss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryNote {
    pub id: u64,
    /// A single-sentence fact; in the user's own wording.
    #[serde(alias = "metin")]
    pub text: String,
    #[serde(alias = "tur")]
    pub kind: MemoryKind,
    /// Lowercased triggers. Recall works off these.
    #[serde(alias = "anahtarlar")]
    pub keys: Vec<String>,
    /// Unix seconds. A raw number so the persistent format does not depend on
    /// human calendars.
    #[serde(alias = "olusturulma")]
    pub created: u64,
    /// The user can switch it off; a disabled note IS NOT INJECTED, but it is
    /// not deleted either.
    #[serde(alias = "aktif")]
    pub active: bool,
}

impl MemoryNote {
    /// Deduplication key.
    pub fn normalized_text(&self) -> String {
        comparison_key(&self.text)
    }

    /// Is it storable — text within the limits and at least one key.
    pub fn is_valid(&self) -> bool {
        let n = self.text.trim().chars().count();
        (MIN_TEXT..=TEXT_LIMIT).contains(&n) && !self.keys.is_empty()
    }

    /// The subtitle shown on the board: "food · restaurant · dinner".
    pub fn summary(&self) -> String {
        self.keys.join(" · ")
    }
}

/// The DEDUPLICATION key — SEPARATE from matching and deliberately more
/// FORGIVING.
///
/// Turkish lowercasing does the right thing and turns `I` into `ı`; that is the
/// correct behaviour IN MATCHING. But in DEDUPLICATION the same correctness
/// opens a hole: "KULLANICI VEJETARYENDIR." becomes "kullanıcı vejetaryendır."
/// and never equals "Kullanici vejetaryendir."; the same fact gets stored
/// twice. Whether the model uses capitals is a coin flip, so dotted and
/// dotless i count as THE SAME here. The asymmetry is deliberate: matching
/// wrongly is expensive (the wrong fact is injected), deduplicating too eagerly
/// is cheap (the user already wrote the same thing).
pub fn comparison_key(text: &str) -> String {
    lowercase(text.trim())
        .chars()
        .map(|c| if c == 'ı' { 'i' } else { c })
        .collect()
}

/// Normalizes a key list: lowercase, trim, drop the empties, split anything
/// containing a comma, then cap it.
///
/// Splitting on the comma is DELIBERATE: when the model sends "food, restaurant"
/// as a SINGLE key that string occurs in no message and the note is born dead.
pub fn fix_keys<S: AsRef<str>>(raw: &[S]) -> Vec<String> {
    let mut output: Vec<String> = Vec::new();
    for part in raw {
        for one in part.as_ref().split(',') {
            let t = lowercase(one.trim());
            if t.is_empty() || output.contains(&t) {
                continue;
            }
            output.push(t);
            if output.len() == KEY_LIMIT {
                return output;
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(text: &str, keys: &[&str]) -> MemoryNote {
        MemoryNote {
            id: 1,
            text: text.into(),
            kind: MemoryKind::Fact,
            keys: fix_keys(keys),
            created: 0,
            active: true,
        }
    }

    #[test]
    fn validity_limits_are_enforced() {
        assert!(note("The user is a vegetarian.", &["food"]).is_valid());
        assert!(
            !note("short", &["food"]).is_valid(),
            "shorter than 10 characters"
        );
        assert!(!note(&"a".repeat(TEXT_LIMIT + 1), &["food"]).is_valid());
        assert!(
            !note("The user is a vegetarian.", &[]).is_valid(),
            "no keys"
        );
    }

    #[test]
    fn kind_resolution_rejects_a_made_up_value() {
        assert_eq!(
            MemoryKind::resolve(" Identity "),
            Some(MemoryKind::Identity)
        );
        assert_eq!(
            MemoryKind::resolve("PREFERENCE"),
            Some(MemoryKind::Preference)
        );
        assert_eq!(
            MemoryKind::resolve("profession"),
            None,
            "must NOT fall back to the default"
        );
        assert_eq!(MemoryKind::default(), MemoryKind::Fact);
    }

    #[test]
    fn old_turkish_kind_values_still_load_from_disk() {
        // The disk-format record: files written before the rename hold
        // `kimlik`/`tercih`/`iliski`/`olgu`. Without the aliases every one of
        // them would silently become `Fact`.
        assert_eq!(
            serde_json::from_str::<MemoryKind>("\"kimlik\"").unwrap(),
            MemoryKind::Identity
        );
        assert_eq!(
            serde_json::from_str::<MemoryKind>("\"tercih\"").unwrap(),
            MemoryKind::Preference
        );
        assert_eq!(
            serde_json::from_str::<MemoryKind>("\"iliski\"").unwrap(),
            MemoryKind::Relation
        );
        assert_eq!(
            serde_json::from_str::<MemoryKind>("\"olgu\"").unwrap(),
            MemoryKind::Fact
        );
    }

    #[test]
    fn a_note_written_with_the_old_field_names_still_loads() {
        let old = r#"{"id":7,"text":"The user job a vegetarian.","kind":"preference",
                      "keys":["food"],"olusturulma":12,"active":true}"#;
        let n: MemoryNote = serde_json::from_str(old).unwrap();
        assert_eq!(n.kind, MemoryKind::Preference);
        assert_eq!(n.keys, vec!["food"]);
        assert!(n.active);
        assert_eq!(n.created, 12);
    }

    #[test]
    fn keys_are_comma_split_and_capped() {
        assert_eq!(fix_keys(&["Food, Restaurant"]), vec!["food", "restaurant"]);
        assert_eq!(
            fix_keys(&["a", "A", " a "]),
            vec!["a"],
            "duplicates drop out"
        );
        let many: Vec<String> = (0..20).map(|i| format!("k{i}")).collect();
        assert_eq!(fix_keys(&many).len(), KEY_LIMIT);
    }

    #[test]
    fn deduplication_swallows_the_dotted_dotless_i_distinction() {
        // Deduplication swallows the dotted/dotless i distinction
        // (see `comparison_key`). Turkish data kept: it is the measured case.
        assert_eq!(
            note("İSTANBUL'DA YASIYORUM", &["city"]).normalized_text(),
            note("İstanbul'da yasiyorum", &["city"]).normalized_text()
        );
    }

    #[test]
    fn summary_prints_the_keys_readably() {
        assert_eq!(
            note("The user is a vegetarian.", &["food", "restaurant"]).summary(),
            "food · restaurant"
        );
    }
}
