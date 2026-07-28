//! The production catalog — the single source of the tool list the model
//! ACTUALLY sees.
//!
//! WHY HERE AND NOT IN `tacet-cli`: the catalog used to be built only in the
//! shell, while eval built its own short list (5 tools). Two lists drifting apart
//! produces silent bugs of the "eval passed but the app behaves differently"
//! class; worst of all, it makes the TOOL SELECTION measurement meaningless — the
//! model makes its choice from the catalog, and if the catalog differs, the
//! measured choice is a different choice. The list now lives in one place and
//! both the shell and eval take it from here.
//!
//! `CodeState` is handed OUT: the attempt counter has to be reset ON EVERY TURN
//! and only the side that knows the turn boundary (the shell) can do that. Once
//! the tool is lost inside an `Arc` in the catalog there is no other way to reach
//! it.

use crate::calc::CalcTool;
use crate::clipboard::ClipboardTool;
use crate::create_document::CreateDocumentTool;
use crate::data_store::SharedStore;
use crate::db::DbTool;
use crate::edit_document::EditDocumentTool;
use crate::find_file::FindFileTool;
use crate::git::GitTool;
use crate::http_call::HttpCallTool;
use crate::memory::{MemoryTool, SharedMemory};
use crate::read_document::ReadDocumentTool;
use crate::run_code::{CodeState, RunCodeTool};
use crate::shell::ShellTool;
use crate::time::TimeTool;
use crate::web_search::{WebFetchTool, WebSearchTool};
use std::sync::Arc;
use tacet_kernel::ToolCatalog;

/// The diagnostic text returned when code execution could not be discovered —
/// given as a second value rather than a `Result` so the call site can inform the
/// user.
pub struct CodeDiagnosis(pub String);

/// WHICH ADDON GATES ARE OPEN for one build of the catalog.
///
/// WHY A STRUCT AND NOT FIVE BOOLEANS: five positional `bool` arguments is a
/// call site nobody can read and every mistake in it is silent — swapping `db`
/// and `clipboard` compiles, and the failure surfaces as a tool the user never
/// opened standing in the catalog. That is a fail-OPEN, the one direction this
/// file exists to prevent.
///
/// `Default` IS "EVERYTHING CLOSED", and that is the load-bearing property: a
/// gate added to this struct tomorrow starts CLOSED at every call site that was
/// written before it existed. There is deliberately no `open()` constructor for
/// production use — see `read`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AddonGates {
    pub web_search: bool,
    pub shell: bool,
    pub db: bool,
    pub clipboard: bool,
    pub http: bool,
}

impl AddonGates {
    /// Every gate closed — the state of a default install.
    pub fn closed() -> Self {
        Self::default()
    }

    /// Every gate open. FOR TESTS. Production never builds this: a gate is a
    /// question about the user's registry, and a constant cannot answer it.
    pub fn all_open() -> Self {
        Self {
            web_search: true,
            shell: true,
            db: true,
            clipboard: true,
            http: true,
        }
    }

    /// Only the web gate — the shape `production_catalog_with` has always had.
    pub fn web(open: bool) -> Self {
        Self {
            web_search: open,
            ..Self::closed()
        }
    }

    /// THE PRODUCTION READ. Every gate is asked THE SAME WAY, through
    /// `tacet_web::addon::is_open` — the one function that owns the question
    /// "is this addon installed and on". No gate is re-derived here from the
    /// tool's own state (whether a `sqlite3` exists, whether the host list
    /// parses): a tool that can answer "am I usable" is not the same as an
    /// addon the user opened, and letting the tool answer for the gate is how
    /// an addon nobody installed ends up in the catalog.
    ///
    /// THE ASK IS BY ADDON CONSTANT, NEVER BY TOOL NAME. `Definition::tools`
    /// exists to explain the gate, not to enforce it: were the lookup done by
    /// tool name, a name misspelled in that table would make the tool "belong
    /// to no addon" and it would be added ungated — fail-open again.
    ///
    /// FIVE READS OF THE REGISTRY, not one. `is_open` reads the file itself and
    /// that is accepted: the file is under a kilobyte and this runs once per
    /// session. The cost is that an edit landing mid-read can be seen by some
    /// questions and not others; the worst outcome is one tool's presence
    /// disagreeing with another's for the length of one session, and a corrupt
    /// read is CLOSED for all of them.
    pub fn read() -> Self {
        use tacet_web::addon;
        Self {
            web_search: addon::is_open(addon::WEB_SEARCH),
            shell: addon::is_open(addon::SHELL),
            db: addon::is_open(addon::DB),
            clipboard: addon::is_open(addon::CLIPBOARD),
            http: addon::is_open(addon::HTTP),
        }
    }
}

/// The production catalog.
///
/// `fixed_epoch`: so eval can be deterministic, the `time` tool can be pinned to
/// a fixed "now". `None` = the real clock (the production path).
///
/// THE WEB TOOLS ARE BEHIND THE ADDON GATE: `web_search`/`web_fetch` are in the
/// catalog only if the user HAS INSTALLED the web search addon and the addon is
/// ON (see `tacet_web::addon`). The gate is read here, not at the caller —
/// leaving the gate to the caller meant a second catalog builder could skip it.
///
/// MEASUREMENT MODE DOES NOT ASK THE GATE — and that is not a dodge, it follows
/// from what `fixed_epoch` already means. That parameter means "a deterministic
/// run": the machine's clock is not read, because what is measured must not vary
/// with the machine. The addon record is exactly MACHINE STATE — reading it would
/// run the same eval set with two different catalogs on two machines and make
/// tool SELECTION scores incomparable.
///
/// THE PRODUCTION PATH PASSES `None` (shell: `session_catalog`), i.e. in
/// production the gate is ALWAYS asked; the `the_production_branch_reads_the_addon_gate`
/// test below measures that. The web tools that appear in measurement mode are
/// already DRIED OUT on the eval side (see `tacet_eval::tool_selection::TO_BE_DRIED`)
/// — they do not go on the network, they merely stand in the selection list as
/// name/description/schema.
///
/// THE OTHER FOUR ADDON TOOLS ARE NOT IN MEASUREMENT MODE, and unlike the web
/// pair they cannot be. `web_search`/`web_fetch` have a fixed name, description
/// and schema, so they can stand in a selection list without a server behind
/// them. `shell` does not: its description LISTS THE USER'S OWN COMMANDS, so
/// two machines would measure two different prompts. `db` and `clipboard` do
/// not exist at all unless a `sqlite3` or a clipboard helper is on the machine.
/// Dried-out stand-ins for those would measure a catalog no user ever sees.
pub fn production_catalog(
    store: &Arc<SharedStore>,
    memory: &SharedMemory,
    fixed_epoch: Option<i64>,
) -> (ToolCatalog, Option<Arc<CodeState>>, Option<CodeDiagnosis>) {
    let gates = if fixed_epoch.is_some() {
        AddonGates::web(true)
    } else {
        AddonGates::read()
    };
    production_catalog_gated(store, memory, fixed_epoch, gates)
}

/// The variant with the WEB gate supplied from outside — for tests, not the
/// production path.
///
/// WHY A SEPARATE BRANCH: if the only way to measure the gate were
/// `production_catalog`, the test would either read the real user's `addons.json`
/// (a machine-dependent result) or move the process-wide `TACET_HOME` variable —
/// a class of failure that steps on other tests running in parallel and has
/// already happened in this repo.
///
/// IT LEAVES EVERY OTHER GATE CLOSED. That is what keeps this signature honest
/// as the number of addons grows: a caller that names only the web gate gets
/// only the web gate, and never a tool it did not ask about.
pub fn production_catalog_with(
    store: &Arc<SharedStore>,
    memory: &SharedMemory,
    fixed_epoch: Option<i64>,
    web_enabled: bool,
) -> (ToolCatalog, Option<Arc<CodeState>>, Option<CodeDiagnosis>) {
    production_catalog_gated(store, memory, fixed_epoch, AddonGates::web(web_enabled))
}

/// The catalog with EVERY gate supplied from outside. The body; the two
/// functions above are the two ways of answering the gate question.
pub fn production_catalog_gated(
    store: &Arc<SharedStore>,
    memory: &SharedMemory,
    fixed_epoch: Option<i64>,
    gates: AddonGates,
) -> (ToolCatalog, Option<Arc<CodeState>>, Option<CodeDiagnosis>) {
    let mut c = ToolCatalog::new();
    // TIME ZONE: if a fixed epoch was given (eval/test) we stay in UTC —
    // measurement varying with the machine's zone would break determinism. On the
    // production path the offset is READ from the operating system (see
    // `time::local_offset_minutes`): the UTC default returned the wrong time in
    // the field and the model, having called the right tool, gave the user an
    // answer three hours behind. If it cannot be read we stay in UTC rather than
    // guess; the `tz=` field in the output states the truth either way.
    let time = match fixed_epoch {
        Some(e) => TimeTool::new().fixed_epoch(e),
        None => match crate::time::local_offset_minutes() {
            Some(min) => TimeTool::new().offset_minutes(min),
            None => TimeTool::new(),
        },
    };
    // THE CATALOG ORDER IS A DECISION, not alphabetical and not the order things
    // were written.
    //
    // The catalog is larger than the router's budget (`MAX_TOOLS`). When no
    // trigger matches, the selection is ENTIRELY down to this order and the tools
    // at the end are NEVER SHOWN to the model.
    //
    // THE SIZE IS NOT A SINGLE NUMBER, it depends on four independent conditions
    // — whoever changes this must account for all of them:
    //   * `calendar` exists on macOS only.
    //   * `run_code`/`write_code` are added only if shield discovery succeeds.
    //   * `web_search`/`web_fetch`, `shell`, `db`, `clipboard`, `http` are added
    //     only while their OWN addon gate is open. The default install has NO
    //     addon and none of them appear.
    //   * even with the gate open, `shell`/`db`/`clipboard`/`http` may still fail
    //     their own discovery (no allowlist, no `sqlite3`, no clipboard helper).
    //
    // A NUMBER LIVING IN A COMMENT GOES STALE: when `write_code` was added the
    // note here still said "10 tools / the last two" and the number of dropped
    // tools had silently gone from two to three; when the addon gate arrived,
    // "11 tools" was for a while taken as the only correct number. NO COUNT IS
    // WRITTEN HERE ANY MORE — what protects is the
    // `the_catalog_is_larger_than_the_router_budget` test, which measures the
    // states rather than describing them.
    //
    // The order is arranged by the question "which tool COULD BE the right
    // answer when the message carries no hint at all":
    //
    //   * calculate / time — the most frequent right answer to short, hintless
    //     questions.
    //   * the document trio and find_file — the body of work of an on-device
    //     assistant.
    //   * run_code — the GENERAL PURPOSE escape hatch; it has a high chance of
    //     being the right answer to a hintless request. It used to be LAST and in
    //     measurement it fell off the budget in cases like "list the primes".
    //   * shell — run_code's OPTED-IN sibling and the same class of answer ("run
    //     the tests", "build it"), so it sits with the code tools rather than
    //     among the trigger-only tools. A user who went through the install and
    //     the approval screen for it means to reach it.
    //   * web_search — a request needing the internet usually says so in words.
    //   * remember / web_fetch / db / clipboard / http — LAST, because none of
    //     them is ever the right answer without an explicit trigger: remember
    //     needs "remember/forget", web_fetch and http need an address, db needs a
    //     query, clipboard needs the word. With no hint, dropping them is
    //     correct, and all four are absent entirely on a default install.
    //
    // edit_document resolves in THREE STAGES (explicit path -> session watcher ->
    // most recently changed document); that order is defined in the tool itself,
    // not in the catalog.
    c.add(Arc::new(CalcTool))
        .add(Arc::new(time))
        .add(Arc::new(ReadDocumentTool::with_store(Arc::clone(store))))
        .add(Arc::new(CreateDocumentTool::new()))
        .add(Arc::new(EditDocumentTool::new()))
        .add(Arc::new(FindFileTool::new()));
    // The calendar bridge is macOS-ONLY today (it speaks through osascript to
    // the Calendar/Reminders apps); on other systems the tool simply does not
    // exist — absence over a tool that always errors, the same rule as the
    // web addon gate.
    #[cfg(target_os = "macos")]
    c.add(Arc::new(crate::calendar::CalendarTool::new()));

    // run_code is added ONLY if it can be run safely: without an interpreter plus
    // a network shield measurement it becomes a trap for the model. `write_code`
    // is built from THE SAME discovery (the shield is measured once) and shares
    // THE SAME attempt counter: the budget is per model-code run in the turn, not
    // per tool — the counter still comes out as a SINGLE handle and the shell
    // signature does not change.
    let (state, diagnosis) = match RunCodeTool::discover() {
        Some(tool) => {
            let s = tool.turn_state();
            let write = crate::write_code::WriteCodeTool::new(
                tool.shield().clone(),
                tool.interpreters().to_vec(),
                Arc::clone(&s),
            );
            c.add(Arc::new(tool));
            c.add(Arc::new(write));
            (Some(s), None)
        }
        None => (None, Some(CodeDiagnosis(RunCodeTool::diagnose()))),
    };

    // THE `shell` GATE. Two conditions, and BOTH are required: the addon is open
    // (asked here, by addon constant) and the tool can build itself from the
    // allowlist in the registry (asked by `discover`, which refuses an empty or
    // unusable list — see `with_commands`). Neither substitutes for the other:
    // `discover` alone would be a gate a tool answers about itself, and the
    // addon gate alone would put a process launcher with nothing to launch in
    // front of the model.
    if gates.shell
        && let Some(tool) = ShellTool::discover()
    {
        c.add(Arc::new(tool));
    }

    // `git` SITS AFTER THE CODE TOOLS AND BEFORE `web_search` — deliberately, by
    // the same rule as the order note above: it is never the right answer to a
    // message that carries no hint at all ("what is 12% of 40" does not want a
    // repository). Placed earlier it would push `run_code`/`write_code` off the
    // budget of 8, and that regression was measured once already. It climbs when
    // the message earns it: "summarize my changes" scores under the General
    // profile and the tool's name and description carry that profile's hints.
    //
    // UNCONDITIONAL, unlike the two above it: `git` needs no interpreter and no
    // network shield. If the binary is missing the tool says so as an ordinary
    // result, so a machine without git loses an answer, not a catalog entry —
    // making the catalog depend on it would make the tool list machine-dependent
    // for no gain.
    c.add(Arc::new(GitTool::new()));

    // THE ADDON GATE. If the addon is not installed (the default state) these
    // tools DO NOT APPEAR in the catalog at all: the model cannot call them, no
    // grammar is generated for them, they do not enter the router budget. The
    // "data never leaves the device" default is thus enforced not as a RUNTIME
    // check but as THE ABSENCE of the tool — if there is nothing to check there is
    // no check to forget.
    //
    // THE ORDER IS PRESERVED: web_search stays BEFORE `remember`, web_fetch right
    // after it (the reasoning is in the order note above).
    if gates.web_search {
        c.add(Arc::new(WebSearchTool::with_store(Arc::clone(store))));
    }
    c.add(Arc::new(MemoryTool::new(memory.clone())));
    if gates.web_search {
        c.add(Arc::new(WebFetchTool::with_store(Arc::clone(store))));
    }

    // THE THREE TRIGGER-ONLY ADDON TOOLS, each behind its OWN gate. The pattern
    // is the same for all three and it is the point: the gate is asked by addon
    // constant, and only then is the tool asked whether this machine can carry
    // it. `db` needs a `sqlite3` whose read-only lock has been MEASURED (two real
    // processes — see `DbTool::discover`), `clipboard` needs a helper binary,
    // `http` needs a non-empty host allowlist. A gate that is open and a tool
    // that cannot be built is NOT a silent absence: `addon_diagnoses` below turns
    // it into a sentence the shell prints.
    if gates.db
        && let Some(tool) = DbTool::discover()
    {
        c.add(Arc::new(tool.with_store(Arc::clone(store))));
    }
    if gates.clipboard
        && let Some(tool) = ClipboardTool::discover()
    {
        c.add(Arc::new(tool.with_store(Arc::clone(store))));
    }
    if gates.http
        && let Some(tool) = HttpCallTool::discover()
    {
        c.add(Arc::new(tool.with_store(Arc::clone(store))));
    }

    (c, state, diagnosis)
}

/// WHY AN OPEN ADDON STILL PUT NO TOOL IN THE CATALOG.
///
/// A closed gate is the user's own decision and needs no explanation. An OPEN
/// gate with no tool behind it is the confusing state — "I installed db, where
/// is it" — and every one of these tools has a machine-level reason it can fail
/// (no `sqlite3`, no clipboard helper, an emptied host list). Without this the
/// absence looks like a missing feature rather than a missing package.
///
/// IT TAKES THE CATALOG THAT WAS ACTUALLY BUILT rather than re-running
/// discovery to decide whether a tool is there: `DbTool::discover` starts two
/// processes, and asking the same question twice is how two answers appear. The
/// tool's own `diagnose()` is called only on the failing branch.
pub fn addon_diagnoses(catalog: &ToolCatalog, gates: AddonGates) -> Vec<String> {
    let mut out = Vec::new();
    if gates.db && catalog.find("db").is_none() {
        out.push(DbTool::diagnose());
    }
    if gates.clipboard && catalog.find("clipboard").is_none() {
        out.push(ClipboardTool::diagnose());
    }
    if gates.http && catalog.find("http").is_none() {
        out.push(HttpCallTool::diagnose());
    }
    // `shell` has no `diagnose()` of its own: its only failure mode is an
    // allowlist that holds nothing usable, and the sentence for that is written
    // here rather than in the tool, which would otherwise need a second copy of
    // the name rule to explain itself.
    if gates.shell && catalog.find("shell").is_none() {
        out.push(
            "shell is off: the addon is open but its command list holds no usable program name. \
             `tacet addon install shell` records one BARE name per line (git, ls, rg) — with no \
             usable name the tool is not in the catalog at all."
                .to_string(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A helper that builds the catalog with the gate set explicitly — the tests
    /// DO NOT READ the production gate (the user's real `addons.json`).
    fn catalog(web_enabled: bool) -> (ToolCatalog, Option<Arc<CodeState>>) {
        gated(AddonGates::web(web_enabled))
    }

    fn gated(gates: AddonGates) -> (ToolCatalog, Option<Arc<CodeState>>) {
        let store = Arc::new(SharedStore::new());
        let memory = SharedMemory::in_memory();
        let (c, s, _) = production_catalog_gated(&store, &memory, None, gates);
        (c, s)
    }

    #[test]
    fn the_catalog_contains_the_expected_tools() {
        let (c, _) = catalog(true);
        for name in [
            "calculate",
            "time",
            "read_document",
            "create_document",
            "edit_document",
            "find_file",
            "git",
            "web_search",
            "web_fetch",
            "remember",
        ] {
            assert!(c.find(name).is_some(), "missing from the catalog: {name}");
        }
    }

    /// THE ADDON GATE — the only measurable claim of this feature.
    ///
    /// With the addon off the web tools MUST NOT be in the catalog; a closed gate
    /// MUST NOT TOUCH the rest of the catalog (accidentally dropping another tool
    /// would be a "I turned off the web and the documents went too" kind of
    /// failure).
    #[test]
    fn the_web_tools_are_behind_the_addon_gate() {
        let (closed, _) = catalog(false);
        assert!(
            closed.find("web_search").is_none(),
            "web_search is in the catalog with the addon off"
        );
        assert!(
            closed.find("web_fetch").is_none(),
            "web_fetch is in the catalog with the addon off"
        );

        let (open, _) = catalog(true);
        assert!(
            open.find("web_search").is_some(),
            "web_search missing with the addon on"
        );
        assert!(
            open.find("web_fetch").is_some(),
            "web_fetch missing with the addon on"
        );

        // The difference is EXACTLY two tools.
        assert_eq!(open.names().len(), closed.names().len() + 2);
        for name in closed.names() {
            assert!(
                open.find(name).is_some(),
                "a tool present in the closed catalog and absent in the open one: {name}"
            );
        }
    }

    /// The tools that exist ONLY because an addon was installed and opened.
    ///
    /// The names are the tool layer's own (`Tool::name`), and they are repeated
    /// here ON PURPOSE: this test's whole job is to fail when a tool starts
    /// answering to a name the gate does not know about.
    const ADDON_TOOLS: [&str; 6] = [
        "web_search",
        "web_fetch",
        "shell",
        "db",
        "clipboard",
        "http",
    ];

    /// THE DEFAULT INSTALL CARRIES NO ADDON TOOL.
    ///
    /// This is the product's headline promise stated as a measurement: with
    /// nothing installed, the model is not merely refused these tools — it is
    /// never shown them, so there is no runtime check anyone can forget.
    #[test]
    fn a_closed_registry_puts_no_addon_tool_in_the_catalog() {
        let (c, _) = gated(AddonGates::closed());
        for name in ADDON_TOOLS {
            assert!(
                c.find(name).is_none(),
                "{name} is in the catalog with every gate closed"
            );
        }
    }

    /// EACH TOOL IS BEHIND ITS OWN GATE, not a shared one.
    ///
    /// Opening one addon must not bring another's tool along. The check is
    /// deliberately one-directional: it asserts that the OTHER tools are ABSENT,
    /// never that the opened one is present. Presence also depends on the
    /// machine (`db` wants a `sqlite3` whose lock measures clean, `clipboard` a
    /// helper binary, `shell`/`http` a list in the user's registry), so an
    /// assertion on presence would pass or fail with the machine and tell us
    /// nothing about the gate. The direction that matters for a gate is the one
    /// tested here anyway: a tool must never appear ungated.
    #[test]
    fn opening_one_gate_does_not_open_another() {
        let cases: [(&str, AddonGates); 4] = [
            (
                "shell",
                AddonGates {
                    shell: true,
                    ..AddonGates::closed()
                },
            ),
            (
                "db",
                AddonGates {
                    db: true,
                    ..AddonGates::closed()
                },
            ),
            (
                "clipboard",
                AddonGates {
                    clipboard: true,
                    ..AddonGates::closed()
                },
            ),
            (
                "http",
                AddonGates {
                    http: true,
                    ..AddonGates::closed()
                },
            ),
        ];
        for (opened, gates) in cases {
            let (c, _) = gated(gates);
            for name in ADDON_TOOLS {
                if name == opened {
                    continue;
                }
                assert!(
                    c.find(name).is_none(),
                    "opening `{opened}` also brought `{name}` into the catalog"
                );
            }
        }
    }

    /// AN OPEN GATE ONLY EVER ADDS.
    ///
    /// The failure this guards against is "I opened the shell addon and my
    /// documents disappeared": every tool present with all gates closed must
    /// still be present with all gates open.
    #[test]
    fn an_open_gate_never_removes_a_tool() {
        let (closed, _) = gated(AddonGates::closed());
        let (open, _) = gated(AddonGates::all_open());
        for name in closed.names() {
            assert!(
                open.find(name).is_some(),
                "a tool present with the gates closed and absent with them open: {name}"
            );
        }
        assert!(open.names().len() >= closed.names().len());
    }

    /// `production_catalog_with` NAMES ONLY THE WEB GATE, so it must leave every
    /// other one closed. Without this the old four-argument signature would
    /// quietly become "the web gate plus whatever the machine happens to have".
    #[test]
    fn the_web_only_helper_leaves_the_other_gates_closed() {
        let store = Arc::new(SharedStore::new());
        let memory = SharedMemory::in_memory();
        let (c, _, _) = production_catalog_with(&store, &memory, None, true);
        for name in ["shell", "db", "clipboard", "http"] {
            assert!(
                c.find(name).is_none(),
                "{name} appeared through the web-only helper"
            );
        }
    }

    /// A DIAGNOSIS IS PRODUCED EXACTLY WHEN A GATE IS OPEN AND THE TOOL IS
    /// MISSING — never for a gate the user closed themselves.
    #[test]
    fn only_an_open_gate_with_no_tool_is_explained() {
        let (c, _) = gated(AddonGates::closed());
        assert!(
            addon_diagnoses(&c, AddonGates::closed()).is_empty(),
            "a closed gate produced an explanation nobody asked for"
        );

        // Every gate open, on THIS catalog (built with them closed, so every one
        // of the four tools is missing): four sentences, one per tool.
        let all = addon_diagnoses(&c, AddonGates::all_open());
        assert_eq!(all.len(), 4, "one sentence per missing tool: {all:?}");
        for text in &all {
            assert!(!text.is_empty());
        }
    }

    /// THE PRODUCTION BRANCH REALLY READS THE GATE.
    ///
    /// If the gate were only measured in `production_catalog_with`, the fact that
    /// `production_catalog` (the branch the shell and eval call) never asks the
    /// gate would ESCAPE these tests — exactly the "the mechanism was built but
    /// never wired to production" failure. The claim here is: the catalog produced
    /// by the production branch has THE SAME tool set as one built with the gate's
    /// current value.
    #[test]
    fn the_production_branch_reads_the_addon_gate() {
        let store = Arc::new(SharedStore::new());
        let memory = SharedMemory::in_memory();
        // THE PRODUCTION PATH = `fixed_epoch: None` (that is how the shell's
        // `session_catalog` calls it). Whatever the gate's current value is, the
        // catalog must match it.
        let (production, _, _) = production_catalog(&store, &memory, None);
        let enabled = tacet_web::addon::web_search_is_open();
        assert_eq!(production.find("web_search").is_some(), enabled);
        assert_eq!(production.find("web_fetch").is_some(), enabled);

        // THE SAME CLAIM FOR THE OTHER FOUR, in the only direction that can be
        // measured without knowing the machine: a CLOSED gate must mean an
        // absent tool. (An open gate does not guarantee presence — the tool
        // still has to find its `sqlite3`, its helper or its allowlist.)
        let gates = AddonGates::read();
        for (open, name) in [
            (gates.shell, "shell"),
            (gates.db, "db"),
            (gates.clipboard, "clipboard"),
            (gates.http, "http"),
        ] {
            assert!(
                open || production.find(name).is_none(),
                "{name} is in the production catalog with its gate closed"
            );
        }
    }

    /// MEASUREMENT MODE IS MACHINE-INDEPENDENT: given a `fixed_epoch`, the catalog
    /// contains the same tools WHETHER OR NOT the addon is installed on this
    /// machine.
    ///
    /// This test is itself a BOUNDARY GUARD: if someone wires measurement mode to
    /// the gate as well, the eval set loses four cases on machines without the
    /// addon and the score drop gets read as a "regression" — when the cause is
    /// configuration.
    #[test]
    fn measurement_mode_is_unaffected_by_machine_state() {
        let store = Arc::new(SharedStore::new());
        let memory = SharedMemory::in_memory();
        let (c, _, _) = production_catalog(&store, &memory, Some(0));
        assert!(
            c.find("web_search").is_some(),
            "web_search must be in the catalog in measurement mode"
        );
        assert!(
            c.find("web_fetch").is_some(),
            "web_fetch must be in the catalog in measurement mode"
        );
        // AND THE OTHER FOUR ARE NOT THERE, whatever this machine has installed.
        // They cannot be dried out the way the web pair can (see the note on
        // `production_catalog`): `shell`'s description carries the user's own
        // command list, and `db`/`clipboard` do not exist without a binary on
        // the machine. Including them would make the same eval set measure two
        // different catalogs on two laptops.
        for name in ["shell", "db", "clipboard", "http"] {
            assert!(
                c.find(name).is_none(),
                "{name} must not be in the catalog in measurement mode — the score stops being comparable"
            );
        }
    }

    /// THE CATALOG SIZE IS COMPARED AGAINST THE ROUTER BUDGET.
    ///
    /// This test exists not to pin a number but to catch a SILENT FAILURE: with
    /// `MAX_TOOLS = 8` and a catalog larger than that, on messages where no trigger
    /// matches the tools AT THE END are never shown to the model. How many tools
    /// dropped was written only in a comment, and when `write_code` was added it
    /// silently went from two to three — the comment went stale and nobody noticed.
    ///
    /// The claim is NOT an equality: `run_code`/`write_code` are not added if the
    /// shield measurement fails, so it is 9 or 11 depending on the machine. What is
    /// measured is THE RANGE and the number of dropped tools being known.
    #[test]
    fn the_catalog_is_larger_than_the_router_budget() {
        // 8 → 9 with the calendar bridge (the 13th tool) — see MAX_TOOLS.
        const BUDGET: usize = crate::router::MAX_TOOLS;
        // Measured with the gate OPEN: with the web tools added the catalog is at
        // its largest. The counts are one higher on macOS, where the calendar
        // bridge exists in the catalog at all.
        let mac = cfg!(target_os = "macos");
        let (c, state) = catalog(true);
        let n = c.names().len();
        // The two shielded tools either both arrive or neither does.
        let expected = (if state.is_some() { 12 } else { 10 }) + usize::from(mac);
        assert_eq!(
            n, expected,
            "the catalog size changed; the number in the comment and the dropped tool count must be updated"
        );
        assert!(
            n > BUDGET,
            "the budget is no longer binding — the reasoning in the comment has collapsed"
        );
        // The number of tools dropped on a hintless message. The comment states
        // this; the test catches the comment going stale.
        assert_eq!(
            n - BUDGET,
            (if state.is_some() { 3 } else { 1 }) + usize::from(mac)
        );

        // WITH THE ADDON OFF (the default install) the catalog is TWO tools shorter
        // and the dropped count falls by two. That is the MEASURED side effect of
        // the gate: on a shielded machine 10 tools / budget 8 → `git` and
        // `remember` drop (and without an explicit trigger neither is the right
        // answer anyway, see the order note); on an unshielded machine 8 tools →
        // nothing drops.
        let (closed, closed_state) = catalog(false);
        let m = closed.names().len();
        assert_eq!(
            m,
            (if closed_state.is_some() { 10 } else { 8 }) + usize::from(mac)
        );
        assert_eq!(
            m.saturating_sub(BUDGET),
            (if closed_state.is_some() { 1 } else { 0 }) + usize::from(mac)
        );
    }

    /// Names must be unique CASE-INSENSITIVELY too.
    ///
    /// `ToolCatalog::find` forgives case (the model may write `Time(...)`). The
    /// condition for that forgiveness to be safe is exactly this: if two tools
    /// differed only in case, the forgiving lookup would confuse them and run the
    /// wrong tool.
    #[test]
    fn tool_names_are_unique_case_insensitively() {
        // Gate OPEN: tested on the widest catalog, because that is where a clash is
        // most likely.
        let (c, _) = catalog(true);
        let mut names: Vec<String> = c.names().iter().map(|n| n.to_lowercase()).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(
            before,
            names.len(),
            "names are not unique case-insensitively"
        );
    }
}
