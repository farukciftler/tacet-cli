//! No shipped benchmark step may expect a tool the router does not show.
//!
//! WHY THIS EXISTS. A tool that is not in the prompt CANNOT be called, however
//! well the model reasons. A benchmark step whose expected tool falls outside
//! the nine is therefore not a hard case — it is an impossible one, and every
//! run books its failure against the MODEL. `tacet bench check` has been able to
//! say this since it was written, and nothing ran it: on the corpus as shipped
//! it found NINE such steps on this machine and FOUR on the default catalog,
//! spread over four files, each one silently subtracting from every score the
//! project has published from them.
//!
//! THE PORTABLE CATALOG, NOT THIS MACHINE'S. The router shows nine of however
//! many exist, so the same file checks differently once MCP servers are
//! attached — that is the question a benchmark asks, not a flaw, but it makes
//! the host answer un-reproducible. Nine failing steps here and four on a clean
//! install is exactly that difference. A gate has to be the same on every
//! machine, so this one is the default catalog.
//!
//! RANK IS NOT ASSERTED. `bench check` warns at rank 6+ because the model takes
//! the first plausible tool on the list, but a case is allowed to be hard. Only
//! "cannot possibly pass" is a failure here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tacet_tools::data_store::SharedStore;
use tacet_tools::memory::SharedMemory;
use tacet_tools::router::Router;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

/// Every `.json` under `benchmarks/`, except the recorded web responses — those
/// are cassettes, not benchmark files.
fn benchmark_files() -> Vec<PathBuf> {
    let root = repo_root().join("benchmarks");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "cassettes") {
                    continue;
                }
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "json") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn no_shipped_step_expects_a_tool_the_router_will_not_show() {
    let files = benchmark_files();
    assert!(
        files.len() >= 15,
        "found only {} benchmark files; the walk is looking in the wrong place",
        files.len()
    );

    let store = Arc::new(SharedStore::new());
    let memory = SharedMemory::in_memory();
    // The DEFAULT catalog with the web addon open — what a fresh install sees,
    // and the same one `tacet bench check --portable` builds.
    let catalog = tacet_tools::catalog::production_catalog_with(&store, &memory, Some(0), true).0;
    let router = Router::new();

    let mut impossible: Vec<String> = Vec::new();
    let mut steps = 0usize;
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let file = match tacet_eval::bench::BenchFile::parse(&text) {
            Ok(f) => f,
            // A file that does not parse is a different test's subject; this one
            // must not turn a parse error into a routing verdict.
            Err(_) => continue,
        };
        let short = path
            .strip_prefix(repo_root())
            .unwrap_or(path)
            .display()
            .to_string();
        for case in &file.cases {
            for step in &case.steps {
                let Some(want) = step.expect.as_deref() else {
                    continue;
                };
                steps += 1;
                let selected = router.select(&step.message, &catalog);
                let shown: Vec<&str> = selected.iter().map(|t| t.name()).collect();
                if !shown.contains(&want) {
                    impossible.push(format!(
                        "\n  {short} · {} · expects {want}, which is not in the nine\n      \
                         {:?}\n      shown: {shown:?}",
                        case.name, step.message
                    ));
                }
            }
        }
    }

    assert!(
        steps > 200,
        "only {steps} steps were checked across {} files; the corpus did not load",
        files.len()
    );
    assert!(
        impossible.is_empty(),
        "{} of {steps} benchmark steps expect a tool the router does not show, so they \
         cannot pass and every run books the failure against the model:{}\n\n\
         Either the router needs the trigger, or the step needs a different expectation.",
        impossible.len(),
        impossible.concat()
    );
}
