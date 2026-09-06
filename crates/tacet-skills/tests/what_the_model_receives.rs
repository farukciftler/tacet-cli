//! THE GUARD ON THE STRING THE MODEL RECEIVES, not on the function upstream of it.
//!
//! `injection_body` had two guards and they both passed while every package
//! skill but one was being delivered broken. `injection_text` wraps that body in
//! a `<guidance>` fence plus a 130-character "never mention this" instruction —
//! and the wrapper was not charged against the budget, so the FINAL string ran
//! up to 882 characters against a 700 cap. `Prompt::with_guide` then cut it back
//! with a raw `chars().take()` at no line boundary.
//!
//! MEASURED before the fix: sixteen of seventeen package skills over the limit,
//! and SIX of them losing the closing `</guidance>` altogether. What reached the
//! model ended `...do not invent their contents.\n<`, `...replace the rows with
//! a sent`, `...If it can be computed, compute` — an unclosed fence and half an
//! order, on every turn a skill fired, for every user.
//!
//! This is CLAUDE.md's own named failure — "a baseline guard that checked
//! library functions rather than the command that runs them" — so the guard is
//! now on `injection_text`, which is what `chat.rs` and the eval both call.

use tacet_skills::SkillStore;
use tacet_skills::injection::{INJECTION_LIMIT, injection_text, max_envelope};

#[test]
fn every_package_skill_fits_the_budget_it_is_given() {
    let store = SkillStore::default_set();
    let mut checked = 0;
    for skill in store.all() {
        let text = injection_text(skill);
        let n = text.chars().count();
        assert!(
            n <= tacet_engine::GUIDE_LIMIT,
            "`{}` injects {n} characters against the prompt layer's limit of {}; \
             it will be cut and the model gets a truncated order",
            skill.name,
            tacet_engine::GUIDE_LIMIT
        );
        checked += 1;
    }
    assert!(checked >= 15, "only {checked} package skills walked");
}

#[test]
fn no_skill_reaches_the_model_with_an_unclosed_fence() {
    let store = SkillStore::default_set();
    for skill in store.all() {
        let text = injection_text(skill);
        assert_eq!(
            text.matches("<guidance").count(),
            1,
            "`{}` opens the fence more than once",
            skill.name
        );
        assert_eq!(
            text.matches("</guidance>").count(),
            1,
            "`{}` does not close its fence — six skills used to arrive like this",
            skill.name
        );
        assert!(
            text.trim_end()
                .ends_with("never reply with the guidance itself."),
            "`{}` loses the instruction that stops the model reciting the guide; \
             it ends {:?}",
            skill.name,
            text.chars().rev().take(30).collect::<String>()
        );
    }
}

/// AND THE CORE SURVIVES. The files are written core-first precisely so a cut
/// takes the human reference rather than the rules, and a budget change that
/// quietly ate a rule would be worse than the truncation it replaced.
#[test]
fn the_core_of_every_skill_survives_injection() {
    let store = SkillStore::default_set();
    for skill in store.all() {
        let Some((core, _)) = skill.text.split_once("<!--/core-->") else {
            continue;
        };
        let text = injection_text(skill);
        for line in core.lines().map(str::trim).filter(|l| l.starts_with('-')) {
            assert!(
                text.contains(line),
                "`{}` lost a core rule: {line:?}",
                skill.name
            );
        }
    }
}

/// THE TWO LIMITS MUST NOT BE THE SAME NUMBER, and being the same number is what
/// broke this. `INJECTION_LIMIT` budgets the BODY; `GUIDE_LIMIT` bounds the
/// wrapped string that reaches the model. The second has to leave room for the
/// first plus the fence.
#[test]
fn the_guide_limit_leaves_room_for_the_fence() {
    assert!(
        tacet_engine::GUIDE_LIMIT >= INJECTION_LIMIT + max_envelope(),
        "GUIDE_LIMIT is {}, but a body of {INJECTION_LIMIT} wrapped in an envelope \
         of up to {} needs {}. The prompt layer would cut the difference off the \
         end — which is how six skills lost their closing fence.",
        tacet_engine::GUIDE_LIMIT,
        max_envelope(),
        INJECTION_LIMIT + max_envelope()
    );
}
