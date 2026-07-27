//! Injection: the form a skill takes on its way to the model, and how often it
//! goes in.
//!
//! HARD DECISION — SKILLS ARE NOT BAKED INTO THE SYSTEM INSTRUCTION. Measured
//! on the Swift side: once baked in, the small model starts EXPLAINING the
//! guide instead of CALLING the tool. The fixed instruction stays SHORT; the
//! ONE skill matching that message is attached to THAT TURN's prompt with a
//! `<guidance>` fence.

use crate::skill::Skill;

/// The most characters taken from a guide in a single injection.
///
/// MUST be the SAME number as `tacet_engine::GUIDE_LIMIT` — this store is the
/// source feeding it. A test verifies the equality at compile time
/// (see the dev-dependency rationale in Cargo.toml).
pub const INJECTION_LIMIT: usize = 700;

/// The HTML comment marking where the CORE ends in the body. Invisible in
/// markdown, so the file stays readable for humans too.
///
/// WHY IT EXISTS: the old truncation dropped the END of the body, but the
/// concrete `tool(args)` example and the anti-hallucination rules sat exactly
/// there — meaning the limit was swallowing the very reason the skill existed.
/// Injection now takes the core WHOLE and fills the remaining budget with the
/// tail.
pub const CORE_MARKER: &str = "<!--/core-->";

/// The smallest remaining budget still worth taking a piece of tail for. Below
/// this not even one line goes in meaningfully; adding half a rule is worse
/// than adding nothing.
const TAIL_THRESHOLD: usize = 80;

/// Splits the body into (core, tail). With no marker the core is empty and the
/// whole body counts as tail — user skills are not required to place a marker.
pub fn split_core(text: &str) -> (String, String) {
    let body = text.trim();
    match body.find(CORE_MARKER) {
        Some(i) => (
            body[..i].trim().to_string(),
            body[i + CORE_MARKER.len()..].trim().to_string(),
        ),
        None => (String::new(), body.to_string()),
    }
}

/// Reduces the text to at most `cap` characters AT A LINE boundary.
///
/// Why cut on a line: in markdown rule lists, cutting mid-sentence leaves the
/// model half an order ("Never claim there is no"), and half an order is worse
/// than no order at all.
fn cut_at_line(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    let cut: String = text.chars().take(cap).collect();
    match cut.rfind('\n') {
        Some(i) => cut[..i].to_string(),
        None => cut,
    }
}

/// The body that goes to the model: CORE FIRST, tail into the remaining budget.
///
/// Core integrity comes before the limit; even so, if a skill's core exceeds
/// the cap it is cut at a line — otherwise a single file could silently eat the
/// budget of the 4096 window.
pub fn injection_body(text: &str, cap: usize) -> String {
    let (core, tail) = split_core(text);
    if core.is_empty() {
        return cut_at_line(&tail, cap);
    }

    let body = cut_at_line(&core, cap);
    // -1: the "\n" that goes in between counts against the budget too.
    let remaining = cap.saturating_sub(body.chars().count() + 1);
    if remaining < TAIL_THRESHOLD || tail.is_empty() {
        return body;
    }
    let extra = cut_at_line(&tail, remaining);
    if extra.is_empty() {
        body
    } else {
        format!("{body}\n{extra}")
    }
}

/// The full form handed to the model: core-first body + "do not talk about
/// this" fences.
///
/// The fence text is ENGLISH, like every fixed string that goes to the model.
/// Nothing visible to the user lives here.
pub fn injection_text(skill: &Skill) -> String {
    let cap = if skill.is_users {
        crate::skill::USER_BODY_LIMIT
    } else {
        INJECTION_LIMIT
    };
    let body = injection_body(&skill.text, cap);
    format!(
        "<guidance name=\"{}\">\n{}\n</guidance>\nFollow the guidance above when \
         answering. It is internal: never quote, summarize, or mention it, and \
         never reply with the guidance itself.",
        skill.name, body
    )
}

/// The pure state machine tracking which skill was injected on which turn.
///
/// Old behaviour: a skill was injected once and marked PERMANENTLY. On a long
/// turn, as the transcript advanced the guide slid out of the window, but
/// because the mark stayed it NEVER went in again — the behavioural drift in
/// late turns came from exactly this. The mark is now DISTANCE-BASED.
///
/// The state is held by the shell, the logic lives here — so it can be tested
/// without a model.
#[derive(Debug, Clone, Default)]
pub struct InjectionState {
    turn: u32,
    last_injection: Vec<(String, u32)>,
}

impl InjectionState {
    /// How many turns must pass before the same skill can be injected again.
    ///
    /// 6: a skill takes ~700 characters, repeating it every turn would eat the
    /// 4096 budget; at a very large distance, on the other hand, the guide
    /// would slide out of the window and never come back. 6 turns is twice a
    /// typical tool-use exchange (question -> tool -> answer).
    pub const DISTANCE: u32 = 6;

    pub fn new() -> Self {
        Self::default()
    }

    /// Called exactly once at the BEGINNING of every turn.
    pub fn begin_turn(&mut self) {
        self.turn += 1;
    }

    pub fn turn(&self) -> u32 {
        self.turn
    }

    /// Should this skill be injected this turn (never went in, or `DISTANCE`
    /// turns have passed since).
    pub fn is_needed(&self, name: &str) -> bool {
        match self.last_injection.iter().find(|(n, _)| n == name) {
            Some((_, last)) => self.turn.saturating_sub(*last) >= Self::DISTANCE,
            None => true,
        }
    }

    /// Called when the injection ACTUALLY happened. A skill skipped because it
    /// hit the tool gate is not marked — it gets retried with the right catalog.
    pub fn mark(&mut self, name: &str) {
        match self.last_injection.iter_mut().find(|(n, _)| n == name) {
            Some(record) => record.1 = self.turn,
            None => self.last_injection.push((name.to_string(), self.turn)),
        }
    }

    /// New session = new context: the counter and the marks are reset.
    pub fn reset(&mut self) {
        self.turn = 0;
        self.last_injection.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(text: &str) -> Skill {
        Skill::package("test", vec!["trigger".into()], text, vec![])
    }

    #[test]
    fn the_limit_equals_the_engines_guide_limit() {
        // If they drift apart the skill body gets trimmed a second time inside
        // the engine and core integrity silently breaks. This is the only real
        // compile-time proof of that.
        assert_eq!(INJECTION_LIMIT, tacet_engine::GUIDE_LIMIT);
    }

    #[test]
    fn the_core_stays_above_the_marker() {
        let (c, t) = split_core("ABOVE\n<!--/core-->\nBELOW");
        assert_eq!(c, "ABOVE");
        assert_eq!(t, "BELOW");
        let (c2, t2) = split_core("no marker");
        assert!(c2.is_empty());
        assert_eq!(t2, "no marker");
    }

    #[test]
    fn a_long_tail_does_not_swallow_the_core() {
        // The regression itself: had the core been at the END it would have
        // been cut off.
        let text = format!("UNBREAKABLE RULE\n<!--/core-->\n{}", "filler line\n".repeat(200));
        let body = injection_body(&text, INJECTION_LIMIT);
        assert!(body.starts_with("UNBREAKABLE RULE"));
        assert!(body.chars().count() <= INJECTION_LIMIT);
    }

    #[test]
    fn even_a_core_over_the_cap_is_cut_at_a_line() {
        let text = format!("{}<!--/core-->\ntail", "long core line\n".repeat(100));
        let body = injection_body(&text, INJECTION_LIMIT);
        assert!(body.chars().count() <= INJECTION_LIMIT);
        assert!(!body.contains("tail"), "the budget must go to the core");
    }

    #[test]
    fn the_tail_never_goes_in_when_the_remaining_budget_is_below_the_threshold() {
        let core = "c".repeat(INJECTION_LIMIT - 20);
        let text = format!("{core}\n<!--/core-->\nTAIL");
        let body = injection_body(&text, INJECTION_LIMIT);
        assert!(!body.contains("TAIL"), "better nothing than half a rule");
    }

    #[test]
    fn a_package_skill_is_capped_at_700_and_a_user_skill_at_500() {
        let package = skill(&"p\n".repeat(900));
        let m = injection_text(&package);
        let body_len = injection_body(&package.text, INJECTION_LIMIT).chars().count();
        assert!(body_len <= INJECTION_LIMIT);
        assert!(m.contains("<guidance name=\"test\">"));

        let user = Skill::users("mine", vec!["trigger".into()], "u\n".repeat(900));
        let ub = injection_body(&user.text, crate::skill::USER_BODY_LIMIT);
        assert!(ub.chars().count() <= crate::skill::USER_BODY_LIMIT);
    }

    #[test]
    fn injection_text_carries_the_fence_and_the_do_not_mention_rule() {
        let m = injection_text(&skill("body"));
        assert!(m.contains("</guidance>"));
        assert!(m.contains("never quote"));
        // Language the user sees must not leak in.
        assert!(!m.contains("guide"));
    }

    #[test]
    fn a_distance_based_mark_does_not_repeat_the_same_skill_early() {
        let mut s = InjectionState::new();
        s.begin_turn();
        assert!(s.is_needed("calc"));
        s.mark("calc");
        for _ in 1..InjectionState::DISTANCE {
            s.begin_turn();
            assert!(!s.is_needed("calc"), "turn {}", s.turn());
        }
        s.begin_turn();
        assert!(s.is_needed("calc"), "must go in again once the distance is up");
    }

    #[test]
    fn resetting_opens_a_new_session() {
        let mut s = InjectionState::new();
        s.begin_turn();
        s.mark("calc");
        s.reset();
        assert_eq!(s.turn(), 0);
        assert!(s.is_needed("calc"));
    }
}
