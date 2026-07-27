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

use crate::edit_document::EditDocumentTool;
use crate::read_document::ReadDocumentTool;
use crate::create_document::CreateDocumentTool;
use crate::find_file::FindFileTool;
use crate::memory::{MemoryTool, SharedMemory};
use crate::calc::CalcTool;
use crate::run_code::{CodeState, RunCodeTool};
use crate::data_store::SharedStore;
use crate::web_search::{WebFetchTool, WebSearchTool};
use crate::time::TimeTool;
use tacet_core::ToolCatalog;
use std::sync::Arc;

/// The diagnostic text returned when code execution could not be discovered —
/// given as a second value rather than a `Result` so the call site can inform the
/// user.
pub struct CodeDiagnosis(pub String);

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
pub fn production_catalog(
    store: &Arc<SharedStore>,
    memory: &SharedMemory,
    fixed_epoch: Option<i64>,
) -> (ToolCatalog, Option<Arc<CodeState>>, Option<CodeDiagnosis>) {
    let web_enabled = fixed_epoch.is_some() || tacet_web::addon::web_search_is_open();
    production_catalog_with(store, memory, fixed_epoch, web_enabled)
}

/// The variant with the gate supplied FROM OUTSIDE — for tests, not the
/// production path.
///
/// WHY A SEPARATE BRANCH: if the only way to measure the gate were
/// `production_catalog`, the test would either read the real user's `addons.json`
/// (a machine-dependent result) or move the process-wide `TACET_HOME` variable —
/// a class of failure that steps on other tests running in parallel and has
/// already happened in this repo.
pub fn production_catalog_with(
    store: &Arc<SharedStore>,
    memory: &SharedMemory,
    fixed_epoch: Option<i64>,
    web_enabled: bool,
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
    // The catalog holds AT MOST 11 tools (counted with `tacet tools`: calculate,
    // time, read_document, create_document, edit_document, find_file, run_code,
    // write_code, web_search, remember, web_fetch), and the router's budget is 8
    // (see `MAX_TOOLS`). When no trigger matches, the selection is ENTIRELY down
    // to this order and the tools at the end are NEVER SHOWN to the model.
    //
    // THE NUMBER IS NOT SINGLE, IT DEPENDS ON TWO CONDITIONS — whoever changes
    // this line must account for both:
    //   * `web_search`/`web_fetch` are added only while the addon gate is OPEN
    //     (see the head of the function). The default install has NO ADDON.
    //   * `run_code`/`write_code` are added only if shield discovery succeeds.
    // Four measured states: with addon + shield 11 tools / the last THREE drop,
    // without addon + shield 9 tools / the last ONE drops; on a machine with no
    // shield, both lose two more tools.
    //
    // A NUMBER LIVING IN A COMMENT GOES STALE: when `write_code` was added this
    // still said "10 tools / the last two" and the number of dropped tools had
    // silently gone from two to three; when the addon gate arrived, "11 tools" was
    // for a while taken as the only correct number. What actually protects is not
    // the comment but the `the_catalog_is_larger_than_the_router_budget` test: it
    // measures both states of the gate.
    //
    // The order is therefore arranged by the question "which tool COULD BE the
    // right answer when the message carries no hint at all":
    //
    //   * calculate / time — the most frequent right answer to short, hintless
    //     questions.
    //   * the document trio and find_file — the body of work of an on-device
    //     assistant.
    //   * run_code — the GENERAL PURPOSE escape hatch; it has a high chance of
    //     being the right answer to a hintless request. It used to be LAST and in
    //     measurement it fell off the budget in cases like "list the primes".
    //   * web_search — a request needing the internet usually says so in words.
    //   * remember / web_fetch — LAST, because neither is ever the right answer
    //     without an explicit trigger: remember needs "remember/forget", web_fetch
    //     needs a URL. With no hint, dropping them is correct.
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

    // THE ADDON GATE. If the addon is not installed (the default state) these two
    // tools DO NOT APPEAR in the catalog at all: the model cannot call them, no
    // grammar is generated for them, they do not enter the router budget. The
    // "data never leaves the device" default is thus enforced not as a RUNTIME
    // check but as THE ABSENCE of the tool — if there is nothing to check there is
    // no check to forget.
    //
    // THE ORDER IS PRESERVED: web_search stays BEFORE `remember`, web_fetch stays
    // LAST (the reasoning is in the order note above). With the gate closed the
    // catalog is two tools shorter and the number of tools dropping off the router
    // budget (8) changes; the test below measures that.
    if web_enabled {
        c.add(Arc::new(WebSearchTool::with_store(Arc::clone(store))));
    }
    c.add(Arc::new(MemoryTool::new(memory.clone())));
    if web_enabled {
        c.add(Arc::new(WebFetchTool::with_store(Arc::clone(store))));
    }

    (c, state, diagnosis)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A helper that builds the catalog with the gate set explicitly — the tests
    /// DO NOT READ the production gate (the user's real `addons.json`).
    fn catalog(web_enabled: bool) -> (ToolCatalog, Option<Arc<CodeState>>) {
        let store = Arc::new(SharedStore::new());
        let memory = SharedMemory::in_memory();
        let (c, s, _) = production_catalog_with(&store, &memory, None, web_enabled);
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
        assert!(open.find("web_search").is_some(), "web_search missing with the addon on");
        assert!(open.find("web_fetch").is_some(), "web_fetch missing with the addon on");

        // The difference is EXACTLY two tools.
        assert_eq!(open.names().len(), closed.names().len() + 2);
        for name in closed.names() {
            assert!(
                open.find(name).is_some(),
                "a tool present in the closed catalog and absent in the open one: {name}"
            );
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
        const BUDGET: usize = 8;
        // Measured with the gate OPEN: with the web tools added the catalog is at
        // its largest.
        let (c, state) = catalog(true);
        let n = c.names().len();
        // The two shielded tools either both arrive or neither does.
        let expected = if state.is_some() { 11 } else { 9 };
        assert_eq!(
            n, expected,
            "the catalog size changed; the number in the comment and the dropped tool count must be updated"
        );
        assert!(n > BUDGET, "the budget is no longer binding — the reasoning in the comment has collapsed");
        // The number of tools dropped on a hintless message. The comment states
        // this; the test catches the comment going stale.
        assert_eq!(n - BUDGET, if state.is_some() { 3 } else { 1 });

        // WITH THE ADDON OFF (the default install) the catalog is TWO tools shorter
        // and the dropped count falls to one. That is the MEASURED side effect of
        // the gate: on a shielded machine 9 tools / budget 8 → only `remember`
        // drops (and without an explicit trigger it is not the right answer anyway,
        // see the order note); on an unshielded machine 7 tools → nothing drops.
        let (closed, closed_state) = catalog(false);
        let m = closed.names().len();
        assert_eq!(m, if closed_state.is_some() { 9 } else { 7 });
        assert_eq!(m.saturating_sub(BUDGET), if closed_state.is_some() { 1 } else { 0 });
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
        assert_eq!(before, names.len(), "names are not unique case-insensitively");
    }
}
