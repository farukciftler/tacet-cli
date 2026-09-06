//! Where the eval harness is allowed to differ from the shell, and where it is not.
//!
//! WHY THIS EXISTS. The eval's whole claim is that it measures the program a
//! user runs. It built its catalog with `production_catalog`, which returns
//! three things, and kept only the first. The second is `run_code`'s attempt
//! counter, and the shell resets it at EVERY user turn — the tool's own comment
//! says it is handed out precisely because it cannot be reached once the tool is
//! inside an `Arc` in the catalog.
//!
//! So a two-step case that spent both code attempts in step 1 began step 2 with
//! the budget already gone, and the call came back "two attempts exhausted"
//! without running. The failure is scored against the model, the report says
//! nothing about a budget, and the number is quoted on the front page.
//!
//! This asserts the handle survives. It cannot assert the reset itself without a
//! model, because the counter is `pub(crate)` to `tacet-tools` — but a dropped
//! handle is exactly how it was lost, and that is what a guard is for.

use tacet_eval::env::Env;
use tacet_eval::tool_selection::suite_catalog;
use tacet_tools::memory::SharedMemory;

#[test]
fn the_suite_keeps_the_handle_it_needs_to_reset_the_code_budget() {
    let Ok(env) = Env::setup() else {
        // A sandbox-less or read-only machine cannot build the catalog at all;
        // that is not this test's subject and failing here would say something
        // untrue about the harness.
        eprintln!("skipped: the eval environment could not be set up on this machine");
        return;
    };
    let memory = SharedMemory::in_memory();
    let host = suite_catalog(&env, &memory);

    let has_code_tool = host.catalog.find("run_code").is_some();
    assert_eq!(
        has_code_tool,
        host.code_state.is_some(),
        "the code budget handle and the code tool must appear together.\n\
         run_code in the catalog: {has_code_tool}, code_state carried: {}\n\
         If run_code is offered and the handle is None, every step after the \
         first spends a budget nobody resets. If the handle is Some with no tool \
         behind it, something is constructing state for a tool that is not there.",
        host.code_state.is_some()
    );
}

/// `write_code` shares `run_code`'s counter — one budget per turn for model-run
/// code, not one per tool. If they ever stop arriving together the shared budget
/// has been split and the reset above no longer covers both.
#[test]
fn the_two_code_tools_arrive_together() {
    let Ok(env) = Env::setup() else {
        eprintln!("skipped: the eval environment could not be set up on this machine");
        return;
    };
    let memory = SharedMemory::in_memory();
    let c = suite_catalog(&env, &memory).catalog;
    assert_eq!(
        c.find("run_code").is_some(),
        c.find("write_code").is_some(),
        "run_code and write_code are built from the same sandbox discovery and \
         share one attempt counter; one without the other means that discovery \
         has been split"
    );
}
