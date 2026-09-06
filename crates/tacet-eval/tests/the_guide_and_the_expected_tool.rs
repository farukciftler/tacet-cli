//! Does the guide a message receives belong to the tool the case expects?
//!
//! WHY THIS EXISTS. A skill's trigger list and the router's profile triggers are
//! two separate vocabularies and nothing compared them. Run against the suite the
//! first time, the disagreement was not a corner case: of 166 steps with an
//! expected tool, **42 English steps matched no skill at all and 13 matched the
//! wrong one**, and every one of the 47 Turkish steps was unguided — the skills
//! had no Turkish triggers, in a suite whose Turkish half is the larger one.
//!
//! Some of those were plain: `create-document`'s trigger list contained the bare
//! word `report`, so "What does the file report.md say?" was handed the guide for
//! WRITING a document. Others were absences nobody could see: `edit-document`
//! triggered on "add a row" and the suite says "Add the row"; `time` triggered on
//! "todays date" and the suite says "today's date". None of this is visible from
//! either list on its own, which is exactly why neither list caught it.
//!
//! WHAT THIS ASSERTS, and what it deliberately does not. It does not claim the
//! guide makes the model call the tool — that is the model measurement, and it is
//! run on real weights. It claims something cheaper and checkable in
//! milliseconds: for every step the suite scores, SOME skill fires, and the skill
//! that wins names the tool the case is about. A message with no guide is the
//! cheap failure (the tool is still in the session); a message with the WRONG
//! guide spends ~700 characters of the window steering the model elsewhere, which
//! is the expensive one. Both are counted, and both are held at zero except for
//! the exceptions named below.
//!
//! TOOLS ARE NOT FILTERED (`has_tools` is passed `None`) on purpose: a collision
//! hidden because one of the pair happened to be addon-gated is still a collision
//! the day the addon is installed.

use tacet_eval::tool_selection::{selection_cases, turkish_selection_cases};
use tacet_skills::{SkillStore, matching};

/// Steps whose guide is knowingly not the expected tool's, with the reason.
///
/// AN ENTRY HERE IS A DEBT, NOT A DISPENSATION. It is here because the fix
/// available today would be worse than the defect: the only trigger that would
/// move `web_search` above `calc` on this sentence is one written for this
/// sentence, and a trigger list tuned to the eval's own wording measures
/// nothing. The router already gets it right — `score_intent` scores web 21
/// against calc 11 on this message — so the structural fix is to let the
/// router's ranking break the disagreement, and that is a change with its own
/// measurement to do.
const KNOWN_DISAGREEMENTS: &[(&str, &str)] = &[(
    "web_search-current",
    "\"How much is the dollar today?\" — `calc`'s \"how much is\" (11) beats \
     web-search's \"the dollar\" (10). Narrowing calc would unguide \
     \"how much is 15% off 80 dollars?\", which is arithmetic.",
)];

struct Verdict {
    case: String,
    message: String,
    detail: String,
}

fn survey() -> (Vec<Verdict>, Vec<Verdict>, usize) {
    let store = SkillStore::default_set();
    let mut unguided = Vec::new();
    let mut misguided = Vec::new();
    let mut total = 0usize;

    for case in selection_cases()
        .into_iter()
        .chain(turkish_selection_cases())
    {
        for step in &case.steps {
            let Some(expected) = &step.expected else {
                continue;
            };
            if KNOWN_DISAGREEMENTS.iter().any(|(n, _)| *n == case.name) {
                continue;
            }
            total += 1;
            let lowered = matching::lowercase(&step.message);
            // The same rule `SkillStore::matching` uses: highest score wins, and
            // `>` keeps the first on a tie.
            let mut best: Option<(&str, usize, &Vec<String>)> = None;
            for s in store.all() {
                let p = matching::score(&lowered, &s.triggers);
                if p > 0 && p > best.map_or(0, |(_, b, _)| b) {
                    best = Some((s.name.as_str(), p, &s.tools));
                }
            }
            match best {
                None => unguided.push(Verdict {
                    case: case.name.clone(),
                    message: step.message.clone(),
                    detail: format!("no skill fires; expected a guide for {expected}"),
                }),
                Some((name, score, tools)) if !tools.iter().any(|t| t == expected) => misguided
                    .push(Verdict {
                        case: case.name.clone(),
                        message: step.message.clone(),
                        detail: format!(
                            "{name} ({score}) won, and it guides {tools:?}, not {expected}"
                        ),
                    }),
                Some(_) => {}
            }
        }
    }
    (unguided, misguided, total)
}

fn render(v: &[Verdict]) -> String {
    v.iter()
        .map(|x| format!("\n  {:<28} {}\n      {:?}", x.case, x.detail, x.message))
        .collect()
}

#[test]
fn every_scored_step_is_handed_the_guide_for_the_tool_it_is_about() {
    let (unguided, misguided, total) = survey();
    assert!(
        misguided.is_empty(),
        "{} of {total} steps get a guide for the WRONG tool — ~700 characters of \
         the window spent steering the model away from the answer:{}\n\n\
         Fix the triggers, or add the step to KNOWN_DISAGREEMENTS with the reason \
         the honest fix is not available.",
        misguided.len(),
        render(&misguided)
    );
    assert!(
        unguided.is_empty(),
        "{} of {total} steps match no skill at all:{}",
        unguided.len(),
        render(&unguided)
    );
}

/// An irrelevance case must NOT be handed a guide. A guide is a tool's
/// instructions; injecting one into "Hello, how are you?" is telling a small
/// model that a tool is in play when the whole point of the case is that none is.
#[test]
fn a_message_that_wants_no_tool_is_handed_no_guide() {
    let store = SkillStore::default_set();
    let mut guided: Vec<String> = Vec::new();
    for case in selection_cases()
        .into_iter()
        .chain(turkish_selection_cases())
    {
        for step in &case.steps {
            if step.expected.is_some() {
                continue;
            }
            let lowered = matching::lowercase(&step.message);
            if let Some(s) = store
                .all()
                .filter(|s| matching::score(&lowered, &s.triggers) > 0)
                .max_by_key(|s| matching::score(&lowered, &s.triggers))
            {
                guided.push(format!("{} → {} · {:?}", case.name, s.name, step.message));
            }
        }
    }
    assert!(
        guided.is_empty(),
        "an irrelevance case was handed a guide:\n  {}",
        guided.join("\n  ")
    );
}

/// The exceptions must be real cases. An entry that no longer names a step in
/// the suite is a silenced assertion nobody will notice.
#[test]
fn every_known_disagreement_still_names_a_case() {
    let names: Vec<String> = selection_cases()
        .into_iter()
        .chain(turkish_selection_cases())
        .map(|c| c.name)
        .collect();
    for (case, why) in KNOWN_DISAGREEMENTS {
        assert!(
            names.iter().any(|n| n == case),
            "KNOWN_DISAGREEMENTS names {case:?}, which is not a case in the suite. \
             Either the case was renamed or it was fixed — in both cases the entry \
             is now silencing nothing. Reason recorded: {why}"
        );
    }
}
