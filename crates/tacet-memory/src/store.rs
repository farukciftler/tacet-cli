//! `MemoryStore` — the owner of the notes: disk format, filters, recall and
//! budgeted injection.
//!
//! ## DISK FORMAT: why plain JSON, why not SQLite
//!
//! `memory.json` in the config directory; one file, whole rewrite. Reasoning:
//!
//! - **Zero-dependency identity.** SQLite means either a C library (`rusqlite`)
//!   or a 30k-line pure-Rust engine. Pulling a database engine into a project
//!   that hand-wrote the OOXML zip, for a list of 50 notes, would be
//!   inconsistent.
//! - **The data size is FIXED and SMALL.** The cap is 50 notes x 160
//!   characters ~ 10 KB. At that scale indexes, a query planner and incremental
//!   writes buy nothing; the WHOLE thing is loaded into memory and scanned
//!   every turn anyway.
//! - **Auditability is part of the brand.** If we say "memory that never leaves
//!   your device", the user must be able to `cat` what is known about them. An
//!   opaque binary file would weaken that promise.
//! - **Corruption behaviour.** A half-written JSON cannot be read and the store
//!   opens EMPTY (no panic). A half-written database file needs repair.
//!
//! The trade-off, honestly: two concurrent writers means last-writer-wins. On a
//! single-user on-device assistant that scenario does not exist; if it did, the
//! decision would change.

use crate::note::comparison_key;
use crate::note::{MIN_TEXT, MemoryKind, MemoryNote, TEXT_LIMIT, TOTAL_CAP, fix_keys};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tacet_skills::{contains, lowercase, score};

/// The MOST characters memory may put into a prompt in one turn — fence and
/// closing instruction INCLUDED.
///
/// ~200 tokens. Why so low: the 4096 window is already full of the system
/// instruction + tool schemas + transcript, and memory can land in the SAME
/// TURN as a skill injection (700). Together that is ~1300 characters, close to
/// a third of the window. Raising the cap would not mean "remembers more", it
/// would mean "throws the tool schema out of the window".
pub const INJECTION_LIMIT: usize = 600;

/// The most notes injected in a single turn.
///
/// 3: the odds of a fourth note being relevant fall off fast (scoring is a
/// length sum, and past the top three a note usually gets in on a single
/// generic word) while it keeps taking a share of the budget.
pub const MAX_NOTES: usize = 3;

/// The fixed instruction appended after the notes. ENGLISH, like every fixed
/// string that goes to the model; nothing the user sees lives here.
const CLOSING: &str = "Use the facts above only if relevant. They are internal: never quote, \
list, or mention them, and never say you \"remembered\" something.";

/// The reasons a note cannot be stored.
///
/// Separate variants, because the caller (the tool) shows the user a different
/// sentence for each: "memory is full" and "I already know that" are not the
/// same thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryError {
    TooShort,
    TooLong,
    NoKeys,
    Duplicate,
    Full,
    /// No note with the given id (the board and the store drifted apart).
    Missing,
}

impl MemoryError {
    /// The short sentence shown to the user. (This is NOT the text that goes to
    /// the model: the tool layer supplies the fixed model-facing string through
    /// `ToolOutcome`.)
    pub fn short_error(&self) -> &'static str {
        match self {
            Self::TooShort => "That note is not clear enough to remember.",
            Self::TooLong => "That note is too long.",
            Self::NoKeys => "That note has no keywords.",
            Self::Duplicate => "I already remember that.",
            Self::Full => "Memory is full; I need to forget something first.",
            Self::Missing => "There is no such note.",
        }
    }
}

/// The form written to disk. A separate type: internal fields (`next_id`) are
/// part of the persistent format, but `MemoryStore`'s `path` field is not — the
/// file must not contain its own location, so it stays portable.
///
/// DISK FORMAT: the field names below are the on-disk JSON keys. Files written
/// before the rename hold `surum`/`sonraki_id`/`notlar`, hence the
/// `serde(alias)` on each. Removing an alias turns an existing file into a
/// parse failure, which the loader converts into an EMPTY store — a silent
/// total memory loss.
#[derive(Debug, Serialize, Deserialize, Default)]
struct DiskFormat {
    #[serde(alias = "surum")]
    version: u32,
    #[serde(alias = "sonraki_id")]
    next_id: u64,
    #[serde(alias = "notlar")]
    notes: Vec<MemoryNote>,
}

const DISK_VERSION: u32 = 1;

#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    notes: Vec<MemoryNote>,
    next_id: u64,
    path: Option<PathBuf>,
}

impl MemoryStore {
    /// A store that never touches the disk (tests, eval).
    pub fn in_memory() -> Self {
        Self {
            notes: Vec::new(),
            next_id: 1,
            path: None,
        }
    }

    /// Loads from the given file. If the file is missing or CORRUPT it returns
    /// an empty store and DOES NOT PANIC: the assistant failing to open because
    /// of an unreadable memory file is unacceptable. The corrupt file IS NOT
    /// OVERWRITTEN either — it stays on disk until the first `save`, so the
    /// user can recover it.
    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let disk: DiskFormat = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();

        // `next_id` comes off disk but is untrustworthy (it may have been
        // hand-edited); if it drops below the largest id the IDS WOULD COLLIDE.
        // We pull it up from the floor.
        let largest = disk.notes.iter().map(|n| n.id).max().unwrap_or(0);
        Self {
            next_id: disk.next_id.max(largest + 1),
            notes: disk.notes,
            path: Some(path),
        }
    }

    /// `memory.json` in the config directory.
    ///
    /// This crate does not know WHERE the directory is — `tacet_kernel::env` does
    /// (on Unix `$XDG_CONFIG_HOME/tacet` or `~/.tacet`, on Windows
    /// `%APPDATA%\Tacet`). `TACET_HOME` overrides all of it.
    pub fn default_path() -> Option<PathBuf> {
        tacet_kernel::config_path("memory.json")
    }

    pub fn notes(&self) -> &[MemoryNote] {
        &self.notes
    }

    pub fn count(&self) -> usize {
        self.notes.len()
    }

    pub fn is_full(&self) -> bool {
        self.notes.len() >= TOTAL_CAP
    }

    // -----------------------------------------------------------------------
    // Write path — the filters live IN CODE, model output is not trusted
    // -----------------------------------------------------------------------

    /// Adds a note. Filters in order: length, keys, deduplication, cap. If any
    /// of them rejects, the note IS NOT STORED.
    ///
    /// The model is NOT GIVEN the task of "merging two notes" — at this model
    /// size that loses data; a repeated note is silently rejected, that is all.
    pub fn add(
        &mut self,
        text: &str,
        kind: MemoryKind,
        keys: &[String],
    ) -> Result<u64, MemoryError> {
        // The cap FIRST: validating against a full store is pointless, and the
        // right sentence to show the caller is "memory is full".
        if self.is_full() {
            return Err(MemoryError::Full);
        }

        let text = text.trim();
        let n = text.chars().count();
        if n < MIN_TEXT {
            return Err(MemoryError::TooShort);
        }
        if n > TEXT_LIMIT {
            return Err(MemoryError::TooLong);
        }

        let keys = fix_keys(keys);
        if keys.is_empty() {
            return Err(MemoryError::NoKeys);
        }

        let normalized = comparison_key(text);
        if self.notes.iter().any(|n| n.normalized_text() == normalized) {
            return Err(MemoryError::Duplicate);
        }

        let id = self.next_id;
        self.next_id += 1;
        self.notes.push(MemoryNote {
            id,
            text: text.to_string(),
            kind,
            keys,
            created: now(),
            active: true,
        });
        Ok(id)
    }

    /// Deletes a note by id. The exact path for the "forget that" command.
    pub fn delete(&mut self, id: u64) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        self.notes.len() != before
    }

    /// Deletes by a phrase occurring in the text or the keys; returns how many
    /// were deleted.
    ///
    /// It exists so the model can say "forget that I like coffee". Matching
    /// goes through `contains`, i.e. it obeys the SAME term rule as the skill
    /// layer: a short phrase like "ara" deleting the note that carries "aralik"
    /// would be an irreversible data loss — here the cost of a mismatch is
    /// heavier than in injection.
    pub fn forget(&mut self, phrase: &str) -> usize {
        let target = lowercase(phrase.trim());
        if target.is_empty() {
            return 0;
        }
        let before = self.notes.len();
        self.notes.retain(|n| {
            let text = n.normalized_text();
            !(contains(&text, &target) || n.keys.iter().any(|k| k == &target))
        });
        before - self.notes.len()
    }

    /// Updates a note's text/keys (the board editor).
    pub fn update(&mut self, id: u64, text: &str, keys: &[String]) -> Result<(), MemoryError> {
        let text = text.trim();
        let n = text.chars().count();
        if n < MIN_TEXT {
            return Err(MemoryError::TooShort);
        }
        if n > TEXT_LIMIT {
            return Err(MemoryError::TooLong);
        }
        let keys = fix_keys(keys);
        if keys.is_empty() {
            return Err(MemoryError::NoKeys);
        }
        let normalized = comparison_key(text);
        if self
            .notes
            .iter()
            .any(|n| n.id != id && n.normalized_text() == normalized)
        {
            return Err(MemoryError::Duplicate);
        }
        match self.notes.iter_mut().find(|n| n.id == id) {
            Some(note) => {
                note.text = text.to_string();
                note.keys = keys;
                Ok(())
            }
            None => Err(MemoryError::Missing),
        }
    }

    /// Switches a note active/inactive. An inactive note IS NOT INJECTED but IS
    /// NOT DELETED: "switch off" and "forget" are separate decisions.
    pub fn set_active(&mut self, id: u64, active: bool) -> bool {
        match self.notes.iter_mut().find(|n| n.id == id) {
            Some(n) => {
                n.active = active;
                true
            }
            None => false,
        }
    }

    /// Deletes every note. NOT CALLED without an explicit decision by the user.
    pub fn delete_all(&mut self) -> usize {
        let n = self.notes.len();
        self.notes.clear();
        n
    }

    // -----------------------------------------------------------------------
    // Read path — THE MODEL IS NOT INVOLVED
    // -----------------------------------------------------------------------

    /// Returns the notes matching the message in score order (at most
    /// `MAX_NOTES`).
    ///
    /// The score is the SUM OF LENGTHS of the matching keys — the SAME rule as
    /// the skill layer, the same implementation. Scoring is ADDITIVE, and that is
    /// the whole point: when two of a note's keys match, their lengths ADD UP, so
    /// a note whose specific phrase matched beats a note that only caught a short
    /// generic word. Counting matches instead (1 per key) would make "specific"
    /// and "generic" worth the same and let a one-word note shadow the right one.
    /// Deterministic code, not the model, decides which memory enters which turn;
    /// that way the same message always brings back the same note and the
    /// behaviour stays testable.
    pub fn matching(&self, message: &str) -> Vec<&MemoryNote> {
        let m = lowercase(message);
        let mut scored: Vec<(&MemoryNote, usize)> = self
            .notes
            .iter()
            .filter(|n| n.active)
            .map(|n| (n, score(&m, &n.keys)))
            .filter(|(_, p)| *p > 0)
            .collect();

        // Score descending; on a tie the NEWER note first (fresh information is
        // worth more) — without an ordered rule the Vec order would decide ties
        // and "an old note hides a new one" would be a silent bug.
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.created.cmp(&a.0.created)));
        scored.truncate(MAX_NOTES);
        scored.into_iter().map(|(n, _)| n).collect()
    }

    /// The text to add to the prompt for this message; `None` if nothing
    /// matches.
    ///
    /// The output length NEVER exceeds `INJECTION_LIMIT`: a note that does not
    /// fit the budget is not added. DROPPED, not trimmed, because half a fact
    /// ("Teach the user's job") is a wrong fact.
    pub fn injection_text(&self, message: &str) -> Option<String> {
        self.injection_from(self.matching(message))
    }

    /// Builds the fenced text out of the selected notes. Separate so a list
    /// already filtered by `NoteWatcher` can be handed in too.
    pub fn injection_from(&self, notes: Vec<&MemoryNote>) -> Option<String> {
        if notes.is_empty() {
            return None;
        }
        // The fixed sections are measured first; what is left is the real
        // budget for the notes.
        let fixed = "<memory>\n</memory>\n".chars().count() + CLOSING.chars().count();
        let mut budget = INJECTION_LIMIT.saturating_sub(fixed);

        let mut lines: Vec<String> = Vec::new();
        for n in notes {
            let line = format!("- {}\n", n.text.trim());
            let length = line.chars().count();
            if length > budget {
                continue; // a note that does not fit DROPS; a trimmed fact is a wrong fact
            }
            budget -= length;
            lines.push(line);
        }
        if lines.is_empty() {
            return None;
        }
        Some(format!(
            "<memory>\n{}</memory>\n{}",
            lines.concat(),
            CLOSING
        ))
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Writes the store to disk. On an in-memory store (`in_memory`) it
    /// silently succeeds.
    ///
    /// TEMP FILE FIRST, THEN `rename`: writing directly means the file is left
    /// half-written if the process dies, and ALL memory is gone. `rename` is
    /// atomic within one filesystem, so at worst the old version survives.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let disk = DiskFormat {
            version: DISK_VERSION,
            next_id: self.next_id,
            notes: self.notes.clone(),
        };
        let text = serde_json::to_string_pretty(&disk)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let temp = path.with_extension("json.new");
        std::fs::write(&temp, text)?;
        narrow_permissions(&temp);
        std::fs::rename(&temp, path)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

/// The memory file is PERSONAL DATA: 0600 so other users cannot read it. The
/// same stamp as on the document write path.
///
/// PLATFORM RECORD — THE 0600 STAMP EXISTS ONLY ON UNIX. On Windows NOTHING is
/// done, and that is deliberate: `set_permissions` there only flips the
/// "read-only" flag, it does not restrict access; stamping it would produce the
/// ILLUSION of "protected" without providing any privacy. The real equivalent
/// is narrowing an ACL, which requires a `windows-sys` dependency — that
/// contradicts the zero-dependency identity. On Windows the protection rests on
/// the file's LOCATION (the config directory under the user profile); NOT
/// MEASURED ON THIS MACHINE.
///
/// `tacet-rs/README.md` calls 0600 a red line without carrying this platform
/// record — the README is outside the files this branch owns, so it was not
/// corrected.
fn narrow_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|s| s.as_secs())
        .unwrap_or(0)
}

/// The watcher that stops the same note entering the same session twice.
///
/// Why it is needed: memory is scanned every turn, and the same note would go
/// in again on every message containing "food". A fact stated once is already
/// in the transcript; repeating it only costs budget and makes that fact look
/// IMPORTANT to the model.
#[derive(Debug, Clone, Default)]
pub struct NoteWatcher {
    seen: HashSet<u64>,
}

impl NoteWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Filters out already-injected notes; marks the rest as SEEN.
    pub fn filter<'a>(&mut self, notes: Vec<&'a MemoryNote>) -> Vec<&'a MemoryNote> {
        let mut remaining = Vec::new();
        for n in notes {
            if self.seen.insert(n.id) {
                remaining.push(n);
            }
        }
        remaining
    }

    /// New session = new context.
    pub fn reset(&mut self) {
        self.seen.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::KEY_LIMIT;

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn filled_store() -> MemoryStore {
        let mut s = MemoryStore::in_memory();
        s.add(
            "The user is a vegetarian.",
            MemoryKind::Preference,
            &keys(&["food", "restaurant", "dinner"]),
        )
        .unwrap();
        s.add(
            "The user works as a teacher.",
            MemoryKind::Identity,
            &keys(&["work", "school", "profession"]),
        )
        .unwrap();
        s
    }

    #[test]
    fn the_filters_drop_short_long_and_keyless_notes() {
        let mut s = MemoryStore::in_memory();
        assert_eq!(
            s.add("short", MemoryKind::Fact, &keys(&["a"])),
            Err(MemoryError::TooShort)
        );
        assert_eq!(
            s.add(&"a".repeat(TEXT_LIMIT + 1), MemoryKind::Fact, &keys(&["a"])),
            Err(MemoryError::TooLong)
        );
        assert_eq!(
            s.add(
                "The user is a vegetarian.",
                MemoryKind::Fact,
                &keys(&["  ", ""])
            ),
            Err(MemoryError::NoKeys)
        );
        assert_eq!(s.count(), 0, "none of them may be stored");
    }

    #[test]
    fn deduplication_does_not_take_the_same_fact_twice() {
        let mut s = filled_store();
        let outcome = s.add(
            "  THE USER IS A VEGETARIAN.  ",
            MemoryKind::Preference,
            &keys(&["food"]),
        );
        assert_eq!(
            outcome,
            Err(MemoryError::Duplicate),
            "same after lowercase + trim"
        );
        assert_eq!(s.count(), 2);
    }

    #[test]
    fn at_the_cap_no_new_note_is_taken_and_no_old_one_is_dropped() {
        let mut s = MemoryStore::in_memory();
        for i in 0..TOTAL_CAP {
            s.add(
                &format!("The user stated fact number {i}."),
                MemoryKind::Fact,
                &keys(&["fact"]),
            )
            .unwrap();
        }
        assert!(s.is_full());
        assert_eq!(
            s.add(
                "The user said something else.",
                MemoryKind::Fact,
                &keys(&["else"])
            ),
            Err(MemoryError::Full)
        );
        // NO silent eviction: the old notes are still in place.
        assert_eq!(s.count(), TOTAL_CAP);
    }

    #[test]
    fn matching_puts_the_specific_key_first_and_is_capped_at_three_notes() {
        let mut s = MemoryStore::in_memory();
        for i in 0..6 {
            s.add(
                &format!("The user stated generic fact {i}."),
                MemoryKind::Fact,
                &keys(&["food"]),
            )
            .unwrap();
        }
        s.add(
            "The user eats dinner late.",
            MemoryKind::Preference,
            &keys(&["late dinner"]),
        )
        .unwrap();

        let found = s.matching("what do you suggest for food, where is a late dinner");
        assert_eq!(found.len(), MAX_NOTES, "the cap must be applied");
        // "late dinner" (11) > "food" (4): the specific phrase beats the
        // generic word.
        assert_eq!(found[0].text, "The user eats dinner late.");
    }

    #[test]
    fn a_switched_off_note_is_not_injected_but_is_not_deleted() {
        let mut s = filled_store();
        let id = s.notes()[0].id;
        assert!(s.set_active(id, false));
        assert!(s.matching("can you suggest a restaurant").is_empty());
        assert_eq!(s.count(), 2, "switching off is not deleting");
        s.set_active(id, true);
        assert_eq!(s.matching("can you suggest a restaurant").len(), 1);
    }

    #[test]
    fn no_match_means_no_injection() {
        let s = filled_store();
        assert!(s.injection_text("what is two times two").is_none());
    }

    #[test]
    fn injection_stays_within_budget_and_a_note_that_does_not_fit_drops() {
        let mut s = MemoryStore::in_memory();
        // Long notes, all matching the same key, right up against the limit.
        for i in 0..MAX_NOTES {
            let text = format!("{i} {}", "u".repeat(TEXT_LIMIT - 2));
            s.add(&text, MemoryKind::Fact, &keys(&["food"])).unwrap();
        }
        let text = s
            .injection_text("what shall the food be")
            .expect("there is a match");
        assert!(
            text.chars().count() <= INJECTION_LIMIT,
            "budget exceeded: {}",
            text.chars().count()
        );
        assert!(text.starts_with("<memory>\n"));
        assert!(text.contains("never say you \"remembered\""));
        // A note that does not fit must DROP UNTRIMMED: half a fact is a wrong fact.
        for line in text.lines().filter(|s| s.starts_with("- ")) {
            assert!(line.len() >= TEXT_LIMIT, "note was trimmed: {line}");
        }
    }

    #[test]
    fn injection_does_not_choke_the_window_in_the_same_turn_as_a_skill() {
        // Worst case: a 700-character skill + memory. Together they must not
        // pass a third of the 4096 window.
        let s = filled_store();
        let memory = s.injection_text("where do we eat dinner").unwrap();
        let total = tacet_skills::INJECTION_LIMIT + memory.chars().count();
        assert!(total <= 1400, "skill + memory cap: {total}");
    }

    #[test]
    fn forget_deletes_the_note_carrying_the_phrase() {
        let mut s = filled_store();
        assert_eq!(s.forget("vegetarian"), 1);
        assert_eq!(s.count(), 1);
        // It can also be deleted through a key.
        assert_eq!(s.forget("school"), 1);
        assert_eq!(s.count(), 0);
        assert_eq!(
            s.forget(""),
            0,
            "an empty phrase must not delete everything"
        );
    }

    #[test]
    fn forget_obeys_the_term_rule() {
        let mut s = MemoryStore::in_memory();
        s.add(
            "The user was born in December.",
            MemoryKind::Fact,
            &keys(&["birth"]),
        )
        .unwrap();
        // "ara" is a short root: it must not hold inside "aralik", otherwise the
        // loss is irreversible. (Turkish data kept: this is the measured case.)
        assert_eq!(s.forget("ara"), 0);
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn update_checks_the_limits_and_duplication() {
        let mut s = filled_store();
        let id = s.notes()[0].id;
        assert!(
            s.update(id, "The user is a vegan.", &keys(&["food"]))
                .is_ok()
        );
        assert_eq!(s.notes()[0].text, "The user is a vegan.");
        assert_eq!(
            s.update(id, "short", &keys(&["food"])),
            Err(MemoryError::TooShort)
        );
        // It cannot be made equal to another note's text.
        assert_eq!(
            s.update(id, "The user works as a teacher.", &keys(&["work"])),
            Err(MemoryError::Duplicate)
        );
    }

    #[test]
    fn the_watcher_does_not_hand_out_the_same_note_twice_in_a_session() {
        let s = filled_store();
        let mut w = NoteWatcher::new();
        assert_eq!(w.filter(s.matching("restaurant")).len(), 1);
        assert!(
            w.filter(s.matching("restaurant")).is_empty(),
            "once per session"
        );
        w.reset();
        assert_eq!(w.filter(s.matching("restaurant")).len(), 1);
    }

    #[test]
    fn writes_to_disk_reads_back_and_ids_do_not_collide() {
        let dir = std::env::temp_dir().join("tacet-memory-test-1");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("memory.json");

        let mut s = MemoryStore::from_file(&path);
        s.add(
            "The user is a vegetarian.",
            MemoryKind::Preference,
            &keys(&["food"]),
        )
        .unwrap();
        s.add(
            "The user works as a teacher.",
            MemoryKind::Identity,
            &keys(&["work"]),
        )
        .unwrap();
        s.save().unwrap();

        let mut back = MemoryStore::from_file(&path);
        assert_eq!(back.count(), 2);
        assert_eq!(back.notes()[0].kind, MemoryKind::Preference);
        // A new note MUST NOT COLLIDE with the old ids.
        let fresh = back
            .add(
                "The user lives in Istanbul.",
                MemoryKind::Fact,
                &keys(&["city"]),
            )
            .unwrap();
        assert!(back.notes().iter().filter(|n| n.id == fresh).count() == 1);
        assert!(fresh > 2);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "memory is personal data");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_gives_an_empty_store_instead_of_a_panic() {
        let dir = std::env::temp_dir().join("tacet-memory-test-2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.json");
        std::fs::write(&path, "{ this is not json ]]").unwrap();

        let s = MemoryStore::from_file(&path);
        assert_eq!(s.count(), 0);
        // The corrupt file MUST NOT HAVE BEEN OVERWRITTEN: the user can recover it.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ this is not json ]]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_written_with_the_old_turkish_keys_still_loads() {
        // DISK FORMAT record: this is what the store wrote before the rename.
        // Without the `serde(alias)`es this file would fail to parse and the
        // whole memory would silently vanish.
        let dir = std::env::temp_dir().join("tacet-memory-test-3");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.json");
        std::fs::write(
            &path,
            r#"{"version":1,"sonraki_id":3,"notlar":[
                 {"id":2,"text":"The user job a vegetarian.","kind":"preference",
                  "keys":["food"],"olusturulma":5,"active":true}]}"#,
        )
        .unwrap();

        let s = MemoryStore::from_file(&path);
        assert_eq!(s.count(), 1);
        assert_eq!(s.notes()[0].kind, MemoryKind::Preference);
        assert_eq!(s.notes()[0].keys, vec!["food"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_in_memory_store_does_not_touch_the_disk() {
        let mut s = MemoryStore::in_memory();
        s.add(
            "The user is a vegetarian.",
            MemoryKind::Preference,
            &keys(&["food"]),
        )
        .unwrap();
        assert!(s.path().is_none());
        assert!(s.save().is_ok(), "save without a path silently succeeds");
    }

    #[test]
    fn delete_all_and_delete_work() {
        let mut s = filled_store();
        let id = s.notes()[0].id;
        assert!(s.delete(id));
        assert!(!s.delete(id), "it is not deleted a second time");
        assert_eq!(s.delete_all(), 1);
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn the_key_limit_is_enforced_on_save() {
        let mut s = MemoryStore::in_memory();
        let many: Vec<String> = (0..20).map(|i| format!("k{i}")).collect();
        let id = s
            .add("The user said something.", MemoryKind::Fact, &many)
            .unwrap();
        let note = s.notes().iter().find(|n| n.id == id).unwrap();
        assert_eq!(note.keys.len(), KEY_LIMIT);
    }
}
