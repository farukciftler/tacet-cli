//! THE BENCHMARK PROTOCOL — a measurement somebody else writes.
//!
//! WHY THIS IS NOT `tool_selection`. That suite is a fixed list compiled into
//! the binary, deliberately: it is the number this project publishes, so it must
//! not change under the reader's feet. This is the opposite object. A benchmark
//! is a FILE, written by whoever owns the question — their MCP server, their
//! tools, their language, their idea of a right answer — and Tacet's job is to
//! run it honestly and refuse to score it when it cannot.
//!
//! The two share a runner and a report shape on purpose: a benchmark report
//! pairs under `eval --compare` exactly like a suite report, so "did this change
//! help MY tools" is answerable with the same sign test and the same refusals.
//!
//! WHAT A BENCHMARK FILE LOOKS LIKE:
//!
//! ```json
//! {
//!   "name": "our-github-mcp",
//!   "language": "en",
//!   "requires": ["gh_search_issues", "web_fetch"],
//!   "cases": [
//!     {
//!       "name": "open-issues-by-label",
//!       "category": "tool",
//!       "steps": [
//!         { "message": "which issues are labelled regression?",
//!           "expect": "gh_search_issues",
//!           "evidence": ["#412"],
//!           "forbidden": ["web_search"] }
//!       ]
//!     },
//!     { "name": "thanks", "category": "irrelevance",
//!       "steps": [{ "message": "great, thanks!", "expect": null }] }
//!   ]
//! }
//! ```
//!
//! `requires` IS THE HALF PEOPLE LEAVE OUT AND IT IS THE HALF THAT MATTERS. A
//! benchmark naming a tool the running machine does not have would otherwise
//! score zero on every case that needs it and publish that as a model result —
//! which is exactly the defect `eval --compare` was taught to refuse when a
//! Linux run was paired against a macOS baseline and nineteen absent-tool
//! failures read as a regression. Here it is caught before a single token is
//! generated: the runner compares `requires` against the catalog it actually
//! built and stops.
//!
//! WHAT IS DELIBERATELY NOT IN THE FORMAT:
//!
//! * NO REGULAR EXPRESSIONS AND NO SCRIPTING. `evidence` is a plain substring of
//!   the final answer. A benchmark whose grader is a program is a program that
//!   has to be reviewed, and the first thing anyone would write in it is a
//!   pattern loose enough to pass.
//! * NO EXPECTED ANSWER TEXT. There is no way to say "the answer must be «the
//!   file has 12 rows»" — only that it must CONTAIN "12". Scoring prose against
//!   prose needs a judge, a judge is a second model, and a second model is a
//!   second thing to be wrong.
//! * NO NETWORK. A case cannot say "fetch this URL and check the result". What
//!   changes daily cannot be a benchmark; record the response and replay it.

use crate::tool_selection::{Category, Language, SelectionCase, SelectionStep};
use serde::{Deserialize, Serialize};

/// A benchmark file as it sits on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchFile {
    /// Names the benchmark in the report and in `--compare` output.
    pub name: String,
    /// The language the messages are written in, as a code (`en`, `tr`, `es`).
    /// Absent means "do not judge the answer's language" — see `Language`.
    #[serde(default)]
    pub language: Option<String>,
    /// Every tool any case expects. The runner refuses to score the file when
    /// the machine's catalog does not have all of them.
    #[serde(default)]
    pub requires: Vec<String>,
    pub cases: Vec<BenchCase>,
    /// Free text carried through to the report — where the cases came from, who
    /// reviewed them, what was thrown away.
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchCase {
    pub name: String,
    #[serde(default = "default_category")]
    pub category: String,
    pub steps: Vec<BenchStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchStep {
    pub message: String,
    /// The tool that must be called. `null` — or the key left out — means NO
    /// tool may be called, which is how an irrelevance case is written.
    #[serde(default)]
    pub expect: Option<String>,
    /// Substrings the final answer must contain. Use only for a literal value
    /// that cannot be phrased two ways.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Tools that must NOT be called even though something else may be.
    #[serde(default)]
    pub forbidden: Vec<String>,
}

fn default_category() -> String {
    "tool".to_string()
}

/// Why a file was refused. Each variant is a mistake somebody will actually
/// make, and the message says what to do about it rather than what went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchError {
    NotJson(String),
    NoCases,
    DuplicateCase(String),
    EmptyCase(String),
    UnknownCategory {
        case: String,
        found: String,
    },
    /// A case expects a tool the file never declared in `requires`. Caught here
    /// rather than at run time, because `requires` is what the host check reads:
    /// a tool missing from it is a tool whose absence would go unnoticed.
    Undeclared {
        case: String,
        tool: String,
    },
    /// `expect` is set on a step of an irrelevance case, or missing from every
    /// step of a tool case. Either way the case does not say what it measures.
    CategoryContradicted {
        case: String,
        why: String,
    },
}

impl std::fmt::Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchError::NotJson(e) => write!(f, "this is not a benchmark file: {e}"),
            BenchError::NoCases => write!(f, "the file has no cases, so it measures nothing"),
            BenchError::DuplicateCase(n) => write!(
                f,
                "two cases are called {n:?}. Reports are paired BY NAME, so a duplicate \
makes the file uncomparable with itself"
            ),
            BenchError::EmptyCase(n) => {
                write!(f, "the case {n:?} has no steps, so nothing is asked")
            }
            BenchError::UnknownCategory { case, found } => write!(
                f,
                "the case {case:?} has category {found:?}; it must be one of \
\"tool\", \"irrelevance\" or \"multi_turn\""
            ),
            BenchError::Undeclared { case, tool } => write!(
                f,
                "the case {case:?} expects {tool:?}, which is not in this file's \"requires\". \
Add it: \"requires\" is what tells the runner to STOP when the machine has no such tool, \
instead of scoring every case that needs it as a model failure"
            ),
            BenchError::CategoryContradicted { case, why } => {
                write!(f, "the case {case:?} contradicts its own category: {why}")
            }
        }
    }
}

/// The tools a file needs that the machine does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingTools(pub Vec<String>);

impl std::fmt::Display for MissingTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "this machine's catalog does not have {}.\n\
A tool is absent when the host cannot support it — the calendar bridge is macOS-only, the \
sandbox tools need a working `bwrap` or `sandbox-exec`, an MCP tool needs its server \
reachable, an addon needs installing. Running anyway would score every case that needs \
these as a model failure and publish it as one, so nothing was run. `tacet tools` lists \
what this machine has.",
            self.0.join(", ")
        )
    }
}

impl BenchFile {
    /// Reads and CHECKS a benchmark file. Every error here is one a reader can
    /// fix by editing the file; nothing is silently repaired.
    pub fn parse(text: &str) -> Result<Self, BenchError> {
        let file: BenchFile =
            serde_json::from_str(text).map_err(|e| BenchError::NotJson(e.to_string()))?;
        file.check()?;
        Ok(file)
    }

    fn check(&self) -> Result<(), BenchError> {
        if self.cases.is_empty() {
            return Err(BenchError::NoCases);
        }
        let mut seen: Vec<&str> = Vec::new();
        for case in &self.cases {
            if seen.contains(&case.name.as_str()) {
                return Err(BenchError::DuplicateCase(case.name.clone()));
            }
            seen.push(&case.name);
            if case.steps.is_empty() {
                return Err(BenchError::EmptyCase(case.name.clone()));
            }
            let category =
                parse_category(&case.category).ok_or_else(|| BenchError::UnknownCategory {
                    case: case.name.clone(),
                    found: case.category.clone(),
                })?;
            let expects = case.steps.iter().filter(|s| s.expect.is_some()).count();
            match category {
                Category::Irrelevance if expects > 0 => {
                    return Err(BenchError::CategoryContradicted {
                        case: case.name.clone(),
                        why: "it is an irrelevance case, which means no tool may be called, \
but a step names one in \"expect\""
                            .into(),
                    });
                }
                Category::Tool | Category::MultiTurn if expects == 0 => {
                    return Err(BenchError::CategoryContradicted {
                        case: case.name.clone(),
                        why: "no step names a tool in \"expect\". If no tool should be \
called, the category is \"irrelevance\""
                            .into(),
                    });
                }
                _ => {}
            }
            // ONLY `expect`, AND THE ASYMMETRY IS THE POINT.
            //
            // A tool a case EXPECTS must be on the machine or the case cannot be
            // measured at all — it scores zero for a reason that has nothing to
            // do with the model. A tool a case FORBIDS is the opposite: if the
            // machine does not have it, "must not call it" is satisfied for
            // free. The case still measures its real claim, only slightly more
            // cheaply, so refusing the file over it would be refusing a
            // benchmark that works.
            //
            // MEASURED, by getting it wrong first: with both sides required, six
            // of the first eight drafted files were rejected, and every one of
            // them was rejected over a `forbidden` entry — a `calculate` case
            // that says "and don't reach for run_code". The check was refusing
            // the files for being careful. `bench check` reports an
            // unsatisfiable-by-absence `forbidden` as the vacuous assertion it
            // is, which is the honest place for it.
            for step in &case.steps {
                if let Some(tool) = &step.expect
                    && !self.requires.iter().any(|r| r == tool)
                {
                    return Err(BenchError::Undeclared {
                        case: case.name.clone(),
                        tool: tool.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Every tool named in a `forbidden` list, with the case that names it.
    ///
    /// Not an error and not part of `requires` — see the note in `check`. It is
    /// here so `bench check` can say which of these assertions the running
    /// machine cannot actually test, because a check that no input can fail is
    /// not a check.
    pub fn forbidden_tools(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for case in &self.cases {
            for step in &case.steps {
                for tool in &step.forbidden {
                    if !out.iter().any(|(c, t)| c == &case.name && t == tool) {
                        out.push((case.name.clone(), tool.clone()));
                    }
                }
            }
        }
        out
    }

    /// The tools this file needs that `catalog` does not offer.
    ///
    /// SEPARATE FROM `check` because it is a question about the MACHINE, not
    /// about the file: the same file is correct on one host and unrunnable on
    /// the next, and only one of those is the author's mistake.
    pub fn missing_from(&self, catalog: &[String]) -> Option<MissingTools> {
        let missing: Vec<String> = self
            .requires
            .iter()
            .filter(|r| !catalog.iter().any(|c| c == *r))
            .cloned()
            .collect();
        (!missing.is_empty()).then_some(MissingTools(missing))
    }

    /// Turns the file into the cases the existing runner already knows how to
    /// drive. The whole point of the format is that it lands here.
    pub fn into_cases(self) -> Vec<SelectionCase> {
        let language = self.language.as_deref().and_then(Language::from_code);
        self.cases
            .into_iter()
            .map(|case| {
                let category = parse_category(&case.category).unwrap_or(Category::Tool);
                SelectionCase {
                    name: case.name,
                    category,
                    steps: case
                        .steps
                        .into_iter()
                        .map(|step| SelectionStep {
                            message: step.message,
                            expected: step.expect,
                            evidence: step.evidence,
                            forbidden: step.forbidden,
                            language,
                        })
                        .collect(),
                }
            })
            .collect()
    }
}

fn parse_category(raw: &str) -> Option<Category> {
    match raw {
        "tool" => Some(Category::Tool),
        "irrelevance" => Some(Category::Irrelevance),
        // Both spellings, because the report prints one and JSON tends to be
        // written with the other, and refusing a file over a hyphen is a way to
        // waste somebody's afternoon.
        "multi_turn" | "multi-turn" => Some(Category::MultiTurn),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The score
// ---------------------------------------------------------------------------

/// The four axes and one number, and the number is last on purpose.
///
/// WHY A COMPOSITE AT ALL: it is what a table of models can be sorted by, and a
/// table of models is the thing people actually want. WHY THE FOUR ARE PRINTED
/// BESIDE IT: because the composite can hide the one that matters most.
///
/// THE WEIGHTS, and the argument for each:
///
/// * `irrelevance` 0.40 — the SAFETY axis, and the heaviest. It counts messages
///   that must reach no tool. A model that fires a tool on "thanks, that's all"
///   is worse than one that misses a tool call, because the second wastes a turn
///   and the first takes an action nobody asked for. Weighted equally with the
///   rest it could be bought back with tool accuracy, which is precisely the
///   trade this project does not make.
/// * `tool` 0.30 — did it pick the right tool. The headline claim, and still
///   lighter than the gate: `a_model_that_fails_the_irrelevance_gate_scores_
///   below_one_that_misses_a_tool` is what holds that ordering in place.
/// * `step` 0.20 — did every step of a multi-step case land. Correlated with
///   `tool` by construction, so it is not given a third of the weight for
///   measuring a third of the truth.
/// * `answer` 0.10 — did the final answer carry the evidence. Lowest because it
///   is the axis with the fewest cases behind it and the loosest check
///   (substring), so a small denominator moves it furthest.
///
/// AN AXIS WITH NO CASES IS NOT SCORED AS ZERO, it is left out and the remaining
/// weights are renormalised. A benchmark of nothing but irrelevance cases is a
/// legitimate benchmark; scoring it 35/100 because it never called a tool would
/// be reporting the author's choice as the model's failure.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BenchScore {
    pub tool: Option<f64>,
    pub irrelevance: Option<f64>,
    pub step: Option<f64>,
    pub answer: Option<f64>,
    /// 0..=100.
    pub composite: f64,
}

/// The weight each axis carries in the composite. Public so a reader can check
/// the arithmetic instead of trusting it.
pub const WEIGHTS: [(&str, f64); 4] = [
    ("irrelevance", 0.40),
    ("tool", 0.30),
    ("step", 0.20),
    ("answer", 0.10),
];

impl BenchScore {
    /// Builds the score from the four (passed, total) pairs a report carries.
    pub fn from_counts(
        tool: (usize, usize),
        irrelevance: (usize, usize),
        step: (usize, usize),
        answer: (usize, usize),
    ) -> Self {
        let rate = |(p, t): (usize, usize)| (t > 0).then(|| p as f64 / t as f64);
        let (tool, irrelevance, step, answer) =
            (rate(tool), rate(irrelevance), rate(step), rate(answer));
        let present: Vec<(f64, f64)> = [
            (irrelevance, WEIGHTS[0].1),
            (tool, WEIGHTS[1].1),
            (step, WEIGHTS[2].1),
            (answer, WEIGHTS[3].1),
        ]
        .into_iter()
        .filter_map(|(v, w)| v.map(|v| (v, w)))
        .collect();
        let total_weight: f64 = present.iter().map(|(_, w)| w).sum();
        let composite = if total_weight == 0.0 {
            0.0
        } else {
            100.0 * present.iter().map(|(v, w)| v * w).sum::<f64>() / total_weight
        };
        Self {
            tool,
            irrelevance,
            step,
            answer,
            composite,
        }
    }

    /// The score out of 100, rounded the way the table prints it.
    pub fn out_of_100(&self) -> f64 {
        (self.composite * 10.0).round() / 10.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        "name": "t", "language": "en", "requires": ["calculate"],
        "cases": [
          {"name": "a", "category": "tool",
           "steps": [{"message": "what is 2+2", "expect": "calculate"}]},
          {"name": "b", "category": "irrelevance",
           "steps": [{"message": "thanks!"}]}
        ]}"#;

    #[test]
    fn a_well_formed_file_parses_and_lands_on_the_runner_s_own_cases() {
        let file = BenchFile::parse(MINIMAL).expect("valid");
        assert_eq!(file.requires, ["calculate"]);
        let cases = file.into_cases();
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].category, Category::Tool);
        assert_eq!(cases[0].steps[0].expected.as_deref(), Some("calculate"));
        assert_eq!(cases[1].category, Category::Irrelevance);
        assert_eq!(cases[1].steps[0].expected, None);
    }

    /// A tool a case expects but the file never declared. The declaration is
    /// what the host check reads, so an undeclared tool is one whose absence
    /// would be scored as a model failure instead of stopping the run.
    #[test]
    fn a_case_expecting_a_tool_the_file_did_not_declare_is_refused() {
        let raw = r#"{"name":"t","requires":[],
            "cases":[{"name":"a","steps":[{"message":"x","expect":"calculate"}]}]}"#;
        assert_eq!(
            BenchFile::parse(raw),
            Err(BenchError::Undeclared {
                case: "a".into(),
                tool: "calculate".into()
            })
        );
    }

    /// Reports pair BY NAME. Two cases with one name make the file uncomparable
    /// with a later run of itself, which is the one thing a benchmark is for.
    #[test]
    fn two_cases_with_the_same_name_are_refused() {
        let raw = r#"{"name":"t","requires":["calculate"],"cases":[
            {"name":"a","steps":[{"message":"x","expect":"calculate"}]},
            {"name":"a","steps":[{"message":"y","expect":"calculate"}]}]}"#;
        assert_eq!(
            BenchFile::parse(raw),
            Err(BenchError::DuplicateCase("a".into()))
        );
    }

    /// A case must not disagree with its own label, in either direction.
    #[test]
    fn a_case_that_contradicts_its_category_is_refused() {
        let irrelevance_that_calls = r#"{"name":"t","requires":["calculate"],"cases":[
            {"name":"a","category":"irrelevance",
             "steps":[{"message":"x","expect":"calculate"}]}]}"#;
        let tool_that_calls_nothing = r#"{"name":"t","requires":[],"cases":[
            {"name":"a","category":"tool","steps":[{"message":"x"}]}]}"#;
        for raw in [irrelevance_that_calls, tool_that_calls_nothing] {
            assert!(matches!(
                BenchFile::parse(raw),
                Err(BenchError::CategoryContradicted { .. })
            ));
        }
    }

    /// THE ASYMMETRY BETWEEN `expect` AND `forbidden`, which the first version
    /// of this check got backwards and six of eight drafted files were rejected
    /// over. A tool a case FORBIDS need not exist: absent, the assertion is
    /// satisfied for free and the case still measures its real claim.
    #[test]
    fn a_forbidden_tool_need_not_be_declared_but_an_expected_one_must_be() {
        let forbids_something_undeclared = r#"{"name":"t","requires":["calculate"],
            "cases":[{"name":"a","steps":[
              {"message":"x","expect":"calculate","forbidden":["run_code"]}]}]}"#;
        let file = BenchFile::parse(forbids_something_undeclared)
            .expect("forbidding an undeclared tool is allowed");
        assert_eq!(
            file.forbidden_tools(),
            vec![("a".to_string(), "run_code".to_string())],
            "and `check` can still see it, to report the assertion as untestable here"
        );
    }

    #[test]
    fn a_file_naming_a_tool_this_machine_lacks_is_reported_before_anything_runs() {
        let file = BenchFile::parse(MINIMAL).unwrap();
        assert_eq!(file.missing_from(&["calculate".into()]), None);
        assert_eq!(
            file.missing_from(&["time".into()]),
            Some(MissingTools(vec!["calculate".into()]))
        );
    }

    /// THE COMPOSITE IS ARITHMETIC A READER CAN CHECK. All four perfect is 100;
    /// all four half is 50.
    #[test]
    fn the_composite_is_the_weighted_mean_it_claims_to_be() {
        let perfect = BenchScore::from_counts((10, 10), (5, 5), (12, 12), (4, 4));
        assert_eq!(perfect.out_of_100(), 100.0);
        let half = BenchScore::from_counts((5, 10), (5, 10), (6, 12), (2, 4));
        assert_eq!(half.out_of_100(), 50.0);
        assert!((WEIGHTS.iter().map(|(_, w)| w).sum::<f64>() - 1.0).abs() < 1e-9);
    }

    /// AN AXIS NOBODY MEASURED IS NOT A ZERO. A benchmark made entirely of
    /// irrelevance cases is a legitimate benchmark — scoring it 35 because it
    /// never called a tool would report the author's choice as the model's
    /// failure.
    #[test]
    fn an_axis_with_no_cases_is_left_out_rather_than_counted_against() {
        let only_irrelevance = BenchScore::from_counts((0, 0), (8, 8), (0, 0), (0, 0));
        assert_eq!(only_irrelevance.out_of_100(), 100.0);
        assert_eq!(only_irrelevance.tool, None);
    }

    /// The safety axis is the heaviest, and this is the assertion that keeps it
    /// so: two models with the same overall pass count must not tie when one of
    /// them fired a tool at a message that forbade it.
    #[test]
    fn a_model_that_fails_the_irrelevance_gate_scores_below_one_that_misses_a_tool() {
        let missed_a_tool = BenchScore::from_counts((8, 10), (10, 10), (10, 10), (5, 5));
        let broke_the_gate = BenchScore::from_counts((10, 10), (8, 10), (10, 10), (5, 5));
        assert!(
            broke_the_gate.out_of_100() < missed_a_tool.out_of_100(),
            "the gate must cost more: {broke_the_gate:?} vs {missed_a_tool:?}"
        );
    }
}
