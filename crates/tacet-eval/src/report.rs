//! The eval report — a text table and JSON.
//!
//! TWO FORMATS, ONE SOURCE: the text table is for humans (read in the terminal
//! at the end of a run), the JSON is for machines (a CI threshold, the diff
//! between runs). Both derive from the SAME `EvalReport`; produced separately,
//! one would get updated and the other forgotten, and the question "which
//! number is right" would appear.
//!
//! FAILED CASES FIRST: a report exists to be read. The list of passing cases is
//! not interesting; what is looked at is always what broke.

use crate::runner::CaseOutcome;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct EvalReport {
    pub engine: String,
    pub total: usize,
    pub passed: usize,
    /// 0.0 - 1.0. If there are no cases, 0.0 — counting an empty run as "total
    /// success" would make a change that accidentally filters out the cases look
    /// green.
    pub success_rate: f64,
    /// (passed, total) over the cases that hold TACET responsible.
    ///
    /// WHY IT IS REPORTED SEPARATELY: this line is the one with an absolute
    /// claim on it. With the fake engine it must read 100% — that is the CI
    /// gate — and with a real engine it should STILL read 100%, because a case
    /// about a schema gate or a retry flag does not know which model it is
    /// running under. A single averaged percentage buried that: 69.2% over a
    /// mixed set says nothing about whether the defect is in this repository or
    /// in the weights, and those are fixed in different places by different work.
    pub logic: (usize, usize),
    /// (passed, total) over the cases that hold the MODEL responsible.
    pub behaviour: (usize, usize),
    /// How many FAILURES are attributed to Tacet — the ones that are bugs in
    /// this repository. See `Blame`: this is the number the logic line was
    /// reaching for and could not express, because a case can be ABOUT Tacet and
    /// still fail because the model never called anything.
    pub tacet_faults: usize,
    /// How many failures are attributed to the model.
    pub model_faults: usize,
    pub cases: Vec<CaseOutcome>,
}

impl EvalReport {
    pub fn new(engine: &str, cases: Vec<CaseOutcome>) -> Self {
        let total = cases.len();
        let passed = cases.iter().filter(|c| c.passed).count();
        let success_rate = if total == 0 {
            0.0
        } else {
            passed as f64 / total as f64
        };
        let count = |m: crate::case::Measures| {
            let of_kind: Vec<&CaseOutcome> =
                cases.iter().filter(|c| c.measures == m).collect();
            (
                of_kind.iter().filter(|c| c.passed).count(),
                of_kind.len(),
            )
        };
        let logic = count(crate::case::Measures::Logic);
        let behaviour = count(crate::case::Measures::Behaviour);
        let blamed = |b: crate::runner::Blame| cases.iter().filter(|c| c.blame == Some(b)).count();
        let tacet_faults = blamed(crate::runner::Blame::Tacet);
        let model_faults = blamed(crate::runner::Blame::Model);
        Self {
            engine: engine.to_string(),
            total,
            passed,
            success_rate,
            logic,
            behaviour,
            tacet_faults,
            model_faults,
            cases,
        }
    }

    pub fn all_passed(&self) -> bool {
        self.total > 0 && self.passed == self.total
    }

    /// The text table for humans.
    pub fn table(&self) -> String {
        let name_width = self
            .cases
            .iter()
            .map(|c| c.name.chars().count())
            .max()
            .unwrap_or(4)
            .max(4);

        let mut s = String::new();
        s.push_str(&format!("engine: {}\n\n", self.engine));
        s.push_str(&format!(
            "{:<name_width$}  {:<6}  {}\n",
            "CASE", "STATE", "TOOLS"
        ));
        s.push_str(&format!("{}\n", "-".repeat(name_width + 40)));

        for c in &self.cases {
            let state = if c.passed { "pass" } else { "FAIL" };
            s.push_str(&format!(
                "{:<name_width$}  {state:<6}  {}\n",
                c.name,
                if c.called.is_empty() {
                    "-".to_string()
                } else {
                    c.called.join(", ")
                }
            ));
        }

        let failed: Vec<&CaseOutcome> = self.cases.iter().filter(|c| !c.passed).collect();
        if !failed.is_empty() {
            s.push_str("\nFAILED CASES\n");
            for c in failed {
                s.push_str(&format!("  {}\n", c.name));
                for f in &c.faults {
                    s.push_str(&format!("    - {f}\n"));
                }
            }
        }

        // THE TWO LINES COME FIRST AND THE TOTAL LAST, because the total is the
        // least useful of the three. `LOGIC` carries the absolute claim — it must
        // read 100% whatever engine ran — and a reader who takes only one number
        // away should take that one.
        let pct = |(p, t): (usize, usize)| {
            if t == 0 {
                100.0
            } else {
                p as f64 / t as f64 * 100.0
            }
        };
        s.push_str(&format!(
            "\nLOGIC       {}/{}  ({:.1}%)   <- MUST read 100% on any engine\n",
            self.logic.0,
            self.logic.1,
            pct(self.logic)
        ));
        s.push_str(&format!(
            "BEHAVIOUR   {}/{}  ({:.1}%)   <- the model's, not Tacet's\n",
            self.behaviour.0,
            self.behaviour.1,
            pct(self.behaviour)
        ));
        s.push_str(&format!(
            "TOTAL       {}/{}  ({:.1}%)\n",
            self.passed,
            self.total,
            self.success_rate * 100.0
        ));
        // THE LINE THAT ANSWERS "WHERE DO I LOOK". A case can be ABOUT Tacet and
        // fail because the model never called anything; only the attribution
        // separates the two, and `tacet` is the count that has to reach zero.
        if self.passed < self.total {
            s.push_str(&format!(
                "  of the {} failures: {} tacet · {} model\n",
                self.total - self.passed,
                self.tacet_faults,
                self.model_faults
            ));
        }
        s
    }

    /// The JSON for machines. Serialization cannot fail (all the fields are
    /// plain data); even so, it returns an error string rather than turning into
    /// a panic.
    pub fn json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| {
            format!("{{\"error\":\"the report could not be serialized: {e}\"}}")
        })
    }
}
