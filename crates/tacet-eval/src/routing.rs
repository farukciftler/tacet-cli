//! The ROUTING measurement — "was the right tool even SHOWN to the model".
//!
//! WHY IT HAD TO EXIST, and it is the sharpest lesson in this crate. Three
//! measurements live next to each other and until now only two of them were
//! taken:
//!
//! * `case.rs` — Tacet's logic, with `FakeEngine`. Deterministic, runs in CI,
//!   gates the build.
//! * `tool_selection.rs` — the model's CHOICE, with real weights. Minutes per
//!   run, needs a 2 GB file, runs on nobody's laptop during a refactor.
//! * THIS FILE — the ROUTER's choice, with no model at all.
//!
//! The router decides which nine of the catalog reach the prompt, and a tool
//! that is not in those nine CANNOT BE CALLED however well the model reasons —
//! `Explanation` already prints that sentence to the user. So the router sets
//! the CEILING on every number `tool_selection.rs` reports, and it was the one
//! layer with no measurement of its own: a routing regression showed up as "the
//! model got worse", minutes later, on a machine with weights.
//!
//! WHAT IT FOUND ON ITS FIRST RUN is why the file is worth its length. On
//! `What is 125 times 8?` the router put `web_fetch`, `find_file` and
//! `web_search` ahead of `calculate` — the plural "times" is a Web trigger and
//! the arithmetic question scored as an internet question. On `What time is
//! it?` `calendar` outranked `time`. On `read notes.md`, `run_code` came first
//! and `read_document` did not make the top five. None of that needs a model to
//! see, and none of it was visible before this file.
//!
//! THE TWO NUMBERS, and they are not the same claim:
//!
//! * REACH — is the expected tool inside the budget at all. This is a HARD
//!   requirement, not a quality score: below it the model is being asked to
//!   pick a tool it was never shown. The exit code is tied to this one.
//! * RANK — where in the list it sits. The router's own header states the
//!   reason position matters ("in a small model, position is selection
//!   probability"), so a tool that is present at rank 9 is not the same outcome
//!   as one at rank 1, and averaging them into REACH would hide exactly the
//!   regression this file exists to catch.
//!
//! DETERMINISTIC AND FREE. No weights, no network, no temporary files beyond
//! the one sandbox directory the catalog needs. A full pass over both suites is
//! milliseconds, so it belongs in CI next to the logic set — which is the whole
//! point: the cheap measurement gates, the expensive one explains.

use crate::env::Env;
use crate::tool_selection::{SelectionCase, selection_cases, turkish_selection_cases};
use serde::Serialize;
use tacet_tools::memory::SharedMemory;
use tacet_tools::router::Router;

/// One expected (message -> tool) pair, and where the router put that tool.
#[derive(Debug, Clone, Serialize)]
pub struct RoutingOutcome {
    pub case: String,
    pub message: String,
    pub expected: String,
    /// 1-based position in the list the model would see; `None` = not shown.
    pub rank: Option<usize>,
    /// The tools that DID make the budget, in order — so a failure can be read
    /// without rerunning anything.
    pub selected: Vec<String>,
}

impl RoutingOutcome {
    pub fn reached(&self) -> bool {
        self.rank.is_some()
    }
}

/// The report over one suite.
#[derive(Debug, Clone, Serialize)]
pub struct RoutingReport {
    /// "english", "turkish", or "english+turkish".
    pub suite: String,
    pub catalog: Vec<String>,
    /// The budget in force — the number of tools the model is shown.
    pub budget: usize,
    pub outcomes: Vec<RoutingOutcome>,
    pub reached: usize,
    pub total: usize,
    /// Reached AND in the first three. See the module header on why position is
    /// its own number.
    pub top3: usize,
    /// The sum of the ranks of the tools that were reached — the mean is
    /// derived, but the sum is what stays integral in JSON.
    pub rank_sum: usize,
}

impl RoutingReport {
    fn new(
        suite: &str,
        catalog: Vec<String>,
        budget: usize,
        outcomes: Vec<RoutingOutcome>,
    ) -> Self {
        let total = outcomes.len();
        let reached = outcomes.iter().filter(|o| o.reached()).count();
        let top3 = outcomes
            .iter()
            .filter(|o| o.rank.is_some_and(|r| r <= 3))
            .count();
        let rank_sum = outcomes.iter().filter_map(|o| o.rank).sum();
        Self {
            suite: suite.to_string(),
            catalog,
            budget,
            outcomes,
            reached,
            total,
            top3,
            rank_sum,
        }
    }

    pub fn reach_rate(&self) -> f64 {
        crate::tool_selection::ratio(self.reached, self.total)
    }

    pub fn top3_rate(&self) -> f64 {
        crate::tool_selection::ratio(self.top3, self.total)
    }

    /// The mean rank OVER THE REACHED ONES ONLY. Counting a tool that was never
    /// shown as "rank 10" would blend the two questions this report keeps
    /// apart — and would improve when a case got worse, because dropping out of
    /// the budget entirely is capped at 10 while sliding from 1 to 9 is not.
    pub fn mean_rank(&self) -> f64 {
        if self.reached == 0 {
            return 0.0;
        }
        self.rank_sum as f64 / self.reached as f64
    }

    /// The failures, worst first: never shown before merely badly placed.
    pub fn problems(&self) -> Vec<&RoutingOutcome> {
        let mut out: Vec<&RoutingOutcome> = self
            .outcomes
            .iter()
            .filter(|o| o.rank.is_none_or(|r| r > 3))
            .collect();
        // `None` sorts before any `Some` — exactly the order wanted.
        out.sort_by_key(|o| o.rank.unwrap_or(0));
        out
    }

    pub fn table(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "routing: {} suite · {} tools in catalog · budget {}\n\n",
            self.suite,
            self.catalog.len(),
            self.budget
        ));
        let problems = self.problems();
        if problems.is_empty() {
            s.push_str("every expected tool is in the first three.\n\n");
        } else {
            let width = problems
                .iter()
                .map(|o| o.case.chars().count())
                .max()
                .unwrap_or(4)
                .max(4);
            s.push_str(&format!(
                "{:<width$}  {:<8}  {}\n",
                "CASE", "RANK", "EXPECTED / WHAT CAME FIRST"
            ));
            s.push_str(&format!("{}\n", "-".repeat(width + 50)));
            for o in &problems {
                let rank = match o.rank {
                    Some(r) => format!("{r}"),
                    None => "NOT SHOWN".to_string(),
                };
                let head = o
                    .selected
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                s.push_str(&format!(
                    "{:<width$}  {rank:<8}  {} / {head}\n",
                    o.case, o.expected
                ));
            }
            s.push('\n');
        }
        s.push_str(&format!(
            "REACH       {}/{}  ({:.1}%)   <- a tool not shown CANNOT be called\n",
            self.reached,
            self.total,
            self.reach_rate() * 100.0
        ));
        s.push_str(&format!(
            "TOP 3       {}/{}  ({:.1}%)   <- position is selection probability\n",
            self.top3,
            self.total,
            self.top3_rate() * 100.0
        ));
        s.push_str(&format!(
            "MEAN RANK   {:.2}          (of the {} that were shown)\n",
            self.mean_rank(),
            self.reached
        ));
        s
    }

    pub fn json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| {
            format!("{{\"error\":\"the routing report could not be serialized: {e}\"}}")
        })
    }
}

/// Which suite to route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suite {
    English,
    Turkish,
    Both,
}

impl Suite {
    fn name(self) -> &'static str {
        match self {
            Suite::English => "english",
            Suite::Turkish => "turkish",
            Suite::Both => "english+turkish",
        }
    }

    fn cases(self) -> Vec<SelectionCase> {
        let (english, turkish) = agent_session_cases();
        match self {
            Suite::English => {
                let mut c = selection_cases();
                c.extend(english);
                c
            }
            Suite::Turkish => {
                let mut c = turkish_selection_cases();
                c.extend(turkish);
                c
            }
            Suite::Both => {
                let mut c = selection_cases();
                c.extend(english);
                c.extend(turkish_selection_cases());
                c.extend(turkish);
                c
            }
        }
    }
}

/// THE `agent-sessions` CASES (RG-19), routed here and NOWHERE ELSE.
///
/// NOT IN `selection_cases`, and the reason is the model measurement, not this
/// one: those cases also run against real weights, with the measurement-mode
/// catalog, which never holds an addon tool that reads the user's own history
/// (`catalog::production_catalog`'s note on why `shell` and `db` are absent
/// there applies with more force to another agent's transcript). Added there,
/// they would be eleven cases every model run fails by construction, and every
/// published selection number would move for a reason that is not the model.
///
/// THE PHRASINGS ARE THE R&D BRIEF'S, in both languages, plus the read-by-id
/// shape. A message that names no id expects `sessions_list`: without an id
/// the only useful first call is the listing.
fn agent_session_cases() -> (Vec<SelectionCase>, Vec<SelectionCase>) {
    let case = |name: &str, message: &str, expected: &str| SelectionCase {
        name: name.into(),
        category: crate::tool_selection::Category::Tool,
        steps: vec![crate::tool_selection::SelectionStep::new(
            message,
            Some(expected),
        )],
    };
    let list = tacet_tools::sessions_tool::LIST_TOOL;
    let read = tacet_tools::sessions_tool::READ_TOOL;
    let english = vec![
        case(
            "sessions-claude-continue",
            "Continue my Claude Code session from this morning",
            list,
        ),
        case(
            "sessions-codex-left-off",
            "Where did I leave off in Codex?",
            list,
        ),
        case(
            "sessions-antigravity-list",
            "Show my Antigravity conversations",
            list,
        ),
        case(
            "sessions-pick-up",
            "Pick up where I left off in my last session",
            list,
        ),
        case(
            "sessions-read-by-id",
            "Read my claude code session 11111111-0000-4000-8000-111111111111",
            read,
        ),
        case(
            "sessions-continue-my-session",
            "Let's continue my session in the other agent",
            list,
        ),
    ];
    let turkish = vec![
        case(
            "tr-oturum-claude",
            "Dünkü Claude Code oturumumda ne yapıyordum?",
            list,
        ),
        case("tr-oturum-codex", "Codex'te kaldığım yerden devam et", list),
        case(
            "tr-oturum-antigravity",
            "Şu Antigravity görevine devam edelim",
            list,
        ),
        case(
            "tr-oturum-liste",
            "Diğer ajanlardaki oturumlarımı listele",
            list,
        ),
        case(
            "tr-oturum-oku",
            "11111111-0000-4000-8000-111111111111 numaralı claude code oturumunu oku",
            read,
        ),
    ];
    (english, turkish)
}

/// Runs the routing measurement over a suite.
///
/// THE HISTORY OF A CHAIN CASE IS NOT REPLAYED, and that is deliberate rather
/// than a shortcut: `Router::select` is documented stateless (the same message
/// plus the same catalog always gives the same list), so a later step of a
/// chain is routed by its own sentence and nothing else. Measuring it in
/// isolation is measuring what production does.
pub fn run_routing(suite: Suite) -> Result<RoutingReport, String> {
    run_routing_filtered(suite, None, 0)
}

/// A REMOTE TOOL THE ROUTER HAS NEVER HEARD OF.
///
/// WHY THE PRESSURE MODE EXISTS: the eval catalog has thirteen tools and the
/// budget is nine, so only four can ever be crowded out and REACH reads 100%
/// almost by construction. A real machine does not look like that — connect one
/// MCP server and the catalog is thirty-odd tools competing for the same nine
/// slots, which is the situation the budget was invented for and the ONLY
/// situation where dropping out of it is likely.
///
/// THE PADDING IS FOREIGN-LANGUAGE ON PURPOSE, and it is not decoration: the
/// router's `overlap` tie-breaker is a Latin-alphabet word-stem trick, and its
/// own comment records the failure it was written for — a Japanese question
/// reached no remote tool at all. Padding with English tools would measure the
/// easy half. These names are the shape a real server produces (a prefix plus
/// the operator's own language), so the tools compete the way real ones do.
///
/// IT IS SYNTHETIC AND FIXED. Reading the user's `mcp.json` would make the
/// number depend on whose laptop ran it, which is the one thing a gate may not
/// do.
struct RemoteTool {
    name: String,
    description: String,
}

impl tacet_kernel::Tool for RemoteTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn schema(&self) -> tacet_kernel::ArgSchema {
        tacet_kernel::ArgSchema::empty()
    }
    fn run<'a>(
        &'a self,
        _args: serde_json::Value,
        _ctx: &'a mut tacet_kernel::ToolContext,
    ) -> tacet_kernel::ToolFuture<'a> {
        tacet_kernel::boxed(async move { tacet_kernel::ToolOutcome::read_ok("remote", "ok") })
    }
}

/// The synthetic remote catalog, in the order it is added. Twenty entries is
/// what one ordinary server offers.
const REMOTE_SHAPES: &[(&str, &str)] = &[
    ("disk_durumu", "Sunucudaki disk kullanim durumunu raporlar."),
    (
        "servis_durumu",
        "Systemd servislerinin calisma durumunu listeler.",
    ),
    (
        "ag_durumu",
        "Ag arayuzlerinin ve baglantilarin durumunu gosterir.",
    ),
    ("proses_listesi", "Calisan proseslerin listesini dondurur."),
    ("log_oku", "Sunucudaki bir log dosyasini okur."),
    ("docker_listele", "Docker konteynerlerini listeler."),
    ("docker_log_oku", "Bir docker konteynerinin loglarini okur."),
    (
        "docker_konteyner_yonet",
        "Docker konteynerini baslatir veya durdurur.",
    ),
    ("docker_compose_yonet", "Docker compose yiginini yonetir."),
    (
        "dizin_listele",
        "Uzak sunucudaki bir dizinin icerigini listeler.",
    ),
    ("dosya_oku", "Uzak sunucudaki bir dosyayi okur."),
    ("dosya_yaz", "Uzak sunucuda bir dosyaya yazar."),
    ("dosya_sil", "Uzak sunucudaki bir dosyayi siler."),
    (
        "dosya_tasi_kopyala",
        "Uzak sunucuda dosya tasir veya kopyalar.",
    ),
    ("dosya_ara", "Uzak sunucuda dosya arar."),
    (
        "eposta_gonder",
        "Sunucu uzerinden duz metin eposta gonderir.",
    ),
    (
        "html_eposta_gonder",
        "Sunucu uzerinden HTML eposta gonderir.",
    ),
    ("komut_calistir", "Uzak sunucuda kabuk komutu calistirir."),
    (
        "servis_yeniden_baslat",
        "Bir systemd servisini yeniden baslatir.",
    ),
    ("yedek_al", "Sunucudaki bir dizinin yedegini alir."),
];

/// `pressure` is how many synthetic remote tools to add to the catalog before
/// routing. `0` is the built-in catalog alone.
pub fn run_routing_filtered(
    suite: Suite,
    only: Option<&str>,
    pressure: usize,
) -> Result<RoutingReport, String> {
    let env = Env::setup().map_err(|e| format!("the environment could not be set up: {e}"))?;
    let memory = SharedMemory::in_memory();
    // The routing measurement runs no model and executes nothing, so the code
    // budget in the `HostCatalog` has no bearing on it; only the tools do.
    let mut catalog = crate::tool_selection::selection_catalog(&env, &memory).catalog;
    // THE `agent-sessions` TOOLS, APPENDED WHERE PRODUCTION APPENDS THEM — after
    // everything, behind their own gate (`catalog::production_catalog_gated`).
    // The routing measurement executes nothing, so the roots are an empty list
    // and no folder of the user's is read; what is measured is the router with
    // the addon installed, which is the harder case for the other 166: two
    // more tools competing for the same nine slots.
    //
    // MEASURED 10 Oct 2026 on one Mac, `eval --routing`: before the two tools
    // and the Sessions profile, 15 tools, REACH 166/166, TOP 3 166/166, MEAN
    // RANK 1.33. After, 17 tools and the eleven cases above: REACH 177/177,
    // TOP 3 177/177, MEAN RANK 1.32 — and a case-by-case diff of the two JSON
    // reports moved none of the 166 (rank sum 220 both). The same 177/177 at
    // `--routing-pressure 20`. The first wording of the two descriptions
    // did move four of them; see `SessionsListTool::description`.
    for tool in tacet_tools::sessions_tool::tools(tacet_tools::sessions_tool::SessionRoots::Only(
        Vec::new(),
    )) {
        catalog.add(tool);
    }
    let mut remote_names: Vec<String> = Vec::new();
    for (name, description) in REMOTE_SHAPES.iter().take(pressure) {
        let full = format!("serverim_{name}");
        remote_names.push(full.clone());
        catalog.add(std::sync::Arc::new(RemoteTool {
            name: full,
            description: (*description).to_string(),
        }));
    }
    // THE RESERVATION IS PART OF PRODUCTION, so the measurement carries it: the
    // shell hands the router the remote names so a question in a language the
    // trigger table has never seen can still reach the server. Measuring
    // without it would measure a router the app does not run.
    let router = if remote_names.is_empty() {
        Router::new()
    } else {
        Router::new().reserving(remote_names)
    };

    let mut cases = suite.cases();
    if let Some(pattern) = only {
        cases.retain(|c| c.name.contains(pattern));
    }

    let mut outcomes = Vec::new();
    let mut budget = 0usize;
    for case in &cases {
        for (i, step) in case.steps.iter().enumerate() {
            // Irrelevance steps have no expected tool: there is nothing for the
            // ROUTER to get wrong. Whether a tool gets CALLED on a greeting is
            // the model's decision and `tool_selection.rs` measures it.
            let Some(expected) = &step.expected else {
                continue;
            };
            let selected = router.select(&step.message, &catalog);
            budget = budget.max(selected.len());
            let names: Vec<String> = selected.iter().map(|t| t.name().to_string()).collect();
            let rank = names.iter().position(|n| n == expected).map(|p| p + 1);
            // A chain case's steps need distinguishing names or the table lists
            // the same identifier three times with three different verdicts.
            let name = if case.steps.len() > 1 {
                format!("{}#{}", case.name, i + 1)
            } else {
                case.name.clone()
            };
            outcomes.push(RoutingOutcome {
                case: name,
                message: step.message.clone(),
                expected: expected.clone(),
                rank,
                selected: names,
            });
        }
    }

    let catalog_names: Vec<String> = catalog.names().into_iter().map(String::from).collect();
    Ok(RoutingReport::new(
        suite.name(),
        catalog_names,
        budget,
        outcomes,
    ))
}

/// A tool the catalog does not have cannot be routed to, and the case that
/// expects it is a broken case rather than a router failure. This tells the two
/// apart — see `DISCOVERY_BOUND` in `tool_selection.rs` for why a tool can be
/// legitimately absent on a given machine.
pub fn missing_expectations(report: &RoutingReport) -> Vec<String> {
    let mut out: Vec<String> = report
        .outcomes
        .iter()
        .filter(|o| !report.catalog.contains(&o.expected))
        .map(|o| o.expected.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The instrument itself: two runs must agree bit for bit, or a routing
    /// number cannot be compared with the one from last week.
    #[test]
    fn the_routing_run_is_deterministic() {
        let a = run_routing(Suite::Both).expect("routing runs").json();
        let b = run_routing(Suite::Both).expect("routing runs").json();
        assert_eq!(a, b);
    }

    /// Every case names a tool this machine actually has, EXCEPT the ones the
    /// platform is allowed to withhold. Without this, a typo in a case name
    /// reads as a router regression forever.
    #[test]
    fn the_cases_name_tools_that_exist() {
        let report = run_routing(Suite::Both).expect("routing runs");
        let missing = missing_expectations(&report);
        let allowed = ["run_code", "write_code", "calendar"];
        let unexplained: Vec<&String> = missing
            .iter()
            .filter(|m| !allowed.contains(&m.as_str()))
            .collect();
        assert!(
            unexplained.is_empty(),
            "these cases expect a tool that is not in the catalog: {unexplained:?}"
        );
    }

    /// RG-19: the `agent-sessions` cases are in the run, the catalog they are
    /// routed against holds the two tools, and every one of them lands in the
    /// first three — in both languages, with and without a connected server.
    #[test]
    fn the_agent_session_cases_land_in_the_first_three() {
        use tacet_tools::sessions_tool::{LIST_TOOL, READ_TOOL};
        for pressure in [0, 20] {
            let report = run_routing_filtered(Suite::Both, None, pressure).expect("routing runs");
            for name in [LIST_TOOL, READ_TOOL] {
                assert!(
                    report.catalog.iter().any(|c| c == name),
                    "{name} is not in the routing catalog"
                );
            }
            let cases: Vec<&RoutingOutcome> = report
                .outcomes
                .iter()
                .filter(|o| o.expected == LIST_TOOL || o.expected == READ_TOOL)
                .collect();
            assert_eq!(
                cases.len(),
                11,
                "the eleven session cases are not all routed"
            );
            assert!(
                cases.iter().any(|o| o.case.starts_with("tr-")),
                "no Turkish session case was routed"
            );
            let low: Vec<String> = cases
                .iter()
                .filter(|o| o.rank.is_none_or(|r| r > 3))
                .map(|o| {
                    format!(
                        "{} rank {:?} for {:?}: {:?}",
                        o.case, o.rank, o.message, o.selected
                    )
                })
                .collect();
            assert!(
                low.is_empty(),
                "at pressure {pressure} these session cases fell out of the first three:\n{}",
                low.join("\n")
            );
        }
    }

    /// The report's own arithmetic. A table that disagrees with its JSON is
    /// worse than no table.
    #[test]
    fn the_counts_agree_with_the_outcomes() {
        let report = run_routing(Suite::English).expect("routing runs");
        assert_eq!(report.total, report.outcomes.len());
        assert_eq!(
            report.reached,
            report.outcomes.iter().filter(|o| o.reached()).count()
        );
        assert!(report.top3 <= report.reached);
        assert!(report.table().contains("REACH"));
    }

    /// THE INVARIANT THE BUDGET EXISTS FOR. With the built-in catalog at
    /// thirteen and the budget at nine only four tools can be crowded out, so
    /// REACH there is nearly free. Connect a server and the catalog is thirty-
    /// odd — that is where a tool actually falls out of the prompt, and that is
    /// the number this test holds.
    ///
    /// MEASURED AGAINST A REAL SERVER TOO, not only this synthetic one: on a
    /// machine with twenty live MCP tools the same suite gave the same two
    /// numbers, 154/154 reach and 153/154 in the first three.
    #[test]
    fn a_connected_server_does_not_push_the_expected_tool_out_of_the_budget() {
        let report =
            run_routing_filtered(Suite::Both, None, 20).expect("routing runs under pressure");
        assert!(
            report.catalog.len() > 2 * report.budget,
            "the pressure mode must make the catalog bigger than twice the budget, \
             or it is not measuring pressure: {} tools, budget {}",
            report.catalog.len(),
            report.budget
        );
        let missing = missing_expectations(&report);
        let unreached: Vec<&RoutingOutcome> = report
            .outcomes
            .iter()
            .filter(|o| !o.reached() && !missing.contains(&o.expected))
            .collect();
        assert!(
            unreached.is_empty(),
            "a connected server crowded these out of the prompt:\n{}",
            unreached
                .iter()
                .map(|o| format!("  {} expected {} for {:?}", o.case, o.expected, o.message))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// THE GATE, and the number is deliberately a floor rather than the score
    /// of the day. A tool that is not in the budget cannot be called, so every
    /// point lost here is a point `--tool-selection` can never win back with a
    /// better model.
    #[test]
    fn every_expected_tool_reaches_the_model() {
        let report = run_routing(Suite::Both).expect("routing runs");
        let missing = missing_expectations(&report);
        let unreached: Vec<&RoutingOutcome> = report
            .outcomes
            .iter()
            .filter(|o| !o.reached() && !missing.contains(&o.expected))
            .collect();
        assert!(
            unreached.is_empty(),
            "the router never showed these tools to the model:\n{}",
            unreached
                .iter()
                .map(|o| format!("  {} expected {} for {:?}", o.case, o.expected, o.message))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
