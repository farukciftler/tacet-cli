//! `AllowedSet` — the answer to "which characters are valid right now".
//!
//! Why the set is NOT a plain `Set<char>`: inside a string body the number of
//! allowed characters is all of Unicode minus a handful of escape characters.
//! Enumerating that would be (a) close to impossible and (b) would mean
//! producing millions of elements at every step. So the set has two parts: a
//! finite character list + a "string body is open" flag. `contains()` evaluates
//! the two together.

use std::collections::BTreeSet;

/// The whitespace characters left free at structural positions in JSON.
pub(crate) const SPACES: [char; 4] = [' ', '\t', '\n', '\r'];

/// The characters acceptable at the next step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowedSet {
    /// Finite, individually enumerable characters (structural marks, digits,
    /// key/enum prefixes). `BTreeSet`: the output is ordered, so tests and eval
    /// comparisons stay stable.
    chars: BTreeSet<char>,
    /// While open: EVERY character except control characters, `"` and `\` is
    /// valid (we are inside the body of a JSON string).
    text_body: bool,
    /// Are whitespace characters free.
    space: bool,
    /// Can the input end right here (valid, complete JSON).
    can_finish: bool,
}

impl AllowedSet {
    pub(crate) fn add(&mut self, c: char) {
        self.chars.insert(c);
    }

    pub(crate) fn add_all(&mut self, cs: impl IntoIterator<Item = char>) {
        self.chars.extend(cs);
    }

    pub(crate) fn open_text_body(&mut self) {
        self.text_body = true;
    }

    pub(crate) fn open_space(&mut self) {
        self.space = true;
    }

    pub(crate) fn open_can_finish(&mut self) {
        self.can_finish = true;
    }

    /// Can this character be produced right now.
    pub fn contains(&self, c: char) -> bool {
        if self.chars.contains(&c) {
            return true;
        }
        if self.space && SPACES.contains(&c) {
            return true;
        }
        // String body: JSON forbids control characters unescaped.
        self.text_body && c != '"' && c != '\\' && !c.is_control()
    }

    /// The individually enumerable allowed characters (EXCLUDING the string body).
    ///
    /// For the prompt text and for debugging; masking uses `contains()` instead
    /// of this, because the string body is invisible here.
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.chars.iter().copied()
    }

    /// Are we writing free text (is the body open).
    pub fn is_text_body(&self) -> bool {
        self.text_body
    }

    pub fn is_space_free(&self) -> bool {
        self.space
    }

    /// Can generation end right here.
    pub fn can_finish(&self) -> bool {
        self.can_finish
    }

    /// Can nothing at all be produced (a dead node). In a healthy grammar an
    /// empty set must not occur without `can_finish`; the tests watch for that.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty() && !self.text_body && !self.space
    }
}
