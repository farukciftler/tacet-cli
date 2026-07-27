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
        Self {
            engine: engine.to_string(),
            total,
            passed,
            success_rate,
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

        s.push_str(&format!(
            "\n{}/{} passed  ({:.1}%)\n",
            self.passed,
            self.total,
            self.success_rate * 100.0
        ));
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
