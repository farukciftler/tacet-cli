//! tacet-eval — the evaluation runner.
//!
//! WHAT IT MEASURES: not the model's intelligence, but TACET'S LOGIC. Was the
//! right tool picked, was a tool called for a greeting, did the `source_ref`
//! channel really keep the bulk data away from the model, did the table markdown
//! chain reach `create_document` unbroken, did an unresolvable date silently
//! fall back to today, did the approval gate open in a tainted session.
//!
//! WHY THE DEFAULT ENGINE IS FAKE: the answer to none of those questions
//! depends on the model. Had the measurement been tied to a GGUF file it would
//! never run in CI, and therefore in practice never be measured at all. A real
//! engine is plugged into the SAME case list via `SingleEngine`; what is
//! measured then is the choice of model.
//!
//! NO NETWORK, NO PERSISTENT WRITES: every case runs in its own temporary
//! directory and the directory is deleted on drop.

pub mod analysis;
pub mod case;
pub mod env;
pub mod report;
pub mod routing;
pub mod runner;
pub mod tool_selection;

pub use case::{EvalCase, FIXED_EPOCH, LONG_FILE, TABLE_FILE, all};
pub use env::{EXTERNAL_TOOL, Env, FakeExternalTool};
pub use report::EvalReport;
pub use routing::{RoutingReport, Suite, missing_expectations, run_routing, run_routing_filtered};
pub use runner::{CaseOutcome, EngineSelector, FakeSelector, SingleEngine, run, run_case};
pub use tool_selection::{
    Category, Language, SelectionCase, SelectionReport, check_answer_quality, ratio, run_selection,
    run_selection_case, run_selection_with_options, selection_cases, turkish_selection_cases,
};
// The turn budget and the system instruction are NO LONGER defined here: both
// are production behaviour, not a measurement setting (see
// tacet-engine/src/session.rs). They are re-exported so the call sites coming
// through the old path (`tacet_eval::MAX_TURNS`) do not break; the single source
// of truth is the engine layer.
pub use tacet_engine::{MAX_TURNS, SYSTEM_INSTRUCTIONS};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_case_set_is_wide_enough_and_the_names_are_unique() {
        let cases = all();
        // THE FLOOR IS A RATCHET, NOT A ROUND NUMBER. It was 15 while the suite
        // held 51, which is not a floor, it is a formality — three quarters of
        // the cases could have vanished under it. Raised to what was measured on
        // the day the suite reached 78. Raise it when you add cases; never lower
        // it to make a deletion pass.
        assert!(
            cases.len() >= 78,
            "at least 78 cases expected, there are {}",
            cases.len()
        );
        let mut names: Vec<&str> = cases.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "the case names must be unique");
    }

    /// EVERY GROUP REACHES `all()`.
    ///
    /// WHY THE FLOOR ABOVE IS NOT ENOUGH: `all()` is a list of
    /// `v.extend(group())` lines, and deleting one is a silent loss of a dozen
    /// claims — the suite gets smaller, everything left still passes, and the
    /// printed percentage goes UP. A count floor cannot see it (78 minus a group
    /// is still comfortably over 15) and a count EQUALITY would have to be edited
    /// on every honest addition, which is how a pin gets weakened into a
    /// rubber stamp. The prefixes are what a group actually contributes.
    #[test]
    fn every_group_of_cases_reaches_the_suite() {
        let cases = all();
        for prefix in [
            "chat-",
            "calc-",
            "time-",
            "read-document-",
            "create-document-",
            "document-",
            "channel-",
            "gate-",
            "grounding-",
            "edit-document-",
            "find-file-",
            "search-",
            "remember-",
            "loop-",
            "cancel-",
            "archive-",
            "checksum-",
            "chain-",
        ] {
            assert!(
                cases.iter().any(|c| c.name.starts_with(prefix)),
                "no case named {prefix:?}* is in all() — a group was dropped from the \
                 list, and every claim in it went with it"
            );
        }
    }

    #[test]
    fn there_are_cases_that_must_not_call_a_tool() {
        // The tool appetite regression is invisible without these cases.
        assert!(all().iter().any(|c| c.expected_tool.is_none()));
    }

    #[test]
    fn all_the_cases_pass_on_the_fake_engine() {
        // What this test means: eval ITSELF is correct. If the cases do not pass
        // on the fake engine, the measuring instrument is broken, not the thing
        // being measured.
        let report = run(&all(), &FakeSelector);
        assert!(report.all_passed(), "\n{}", report.table());
    }

    #[test]
    fn the_run_is_deterministic() {
        // Two runs must give bit-for-bit identical JSON; otherwise evals cannot
        // be compared and the question "did it get fixed" stays unanswered.
        let a = run(&all(), &FakeSelector).json();
        let b = run(&all(), &FakeSelector).json();
        assert_eq!(a, b);
    }

    #[test]
    fn the_report_json_and_table_give_the_same_number() {
        let report = run(&all(), &FakeSelector);
        let json: serde_json::Value = serde_json::from_str(&report.json()).unwrap();
        assert_eq!(json["total"].as_u64().unwrap() as usize, report.total);
        assert_eq!(json["passed"].as_u64().unwrap() as usize, report.passed);
        assert!(
            report
                .table()
                .contains(&format!("{}/{}", report.passed, report.total))
        );
    }

    #[test]
    fn a_broken_case_is_caught() {
        // The proof that the runner REALLY MEASURES: a deliberately wrong claim
        // must come back as FAILED. Otherwise "everything passed" says nothing.
        let broken = EvalCase::new("broken", "Hello")
            .script(&["Hello!"])
            .evidence(&["there is no such text"]);
        let outcome = run_case(&broken, &FakeSelector);
        assert!(!outcome.passed);
        assert!(!outcome.faults.is_empty());
    }

    #[test]
    fn a_case_fails_if_a_tool_is_called_for_a_greeting() {
        let broken =
            EvalCase::new("hungry", "Hello").script(&[r#"time({"kind":"all"})"#, "Hello!"]);
        let outcome = run_case(&broken, &FakeSelector);
        assert!(!outcome.passed, "the tool appetite should have been caught");
    }
}
