//! An interrupted run must not throw away what it already measured.
//!
//! WHY THIS EXISTS. The suite takes about 48 minutes on a laptop and everything
//! it had done lived in memory until the last case finished. A Ctrl-C at case
//! 180, a closed lid, a rented box reclaimed, or a panic in one case threw away
//! 47 minutes of real model time, and the next attempt started at zero. That is
//! the difference between a measurement somebody runs overnight and one they do
//! not start.
//!
//! WHAT THE JOURNAL MUST GET RIGHT, and it is not the resuming — it is the
//! REFUSING. Cases measured on two different models, catalogs or builds,
//! assembled into one report, would be a mixture presented as a measurement:
//! the single worst artifact this crate could produce, and one nobody reading
//! the JSON could detect. So a stamp mismatch stops the run rather than quietly
//! starting over.

use std::sync::Arc;

use tacet_engine::EngineProvider;
use tacet_engine::fake::FakeEngine;
use tacet_eval::env::Env;
use tacet_eval::tool_selection::{
    CaseJournal, SelectionCase, SelectionStep, run_selection_journalled, suite_catalog,
};
use tacet_tools::memory::SharedMemory;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tacet-journal-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn cases() -> Vec<SelectionCase> {
    vec![
        SelectionCase {
            name: "probe-one".into(),
            category: tacet_eval::Category::Tool,
            steps: vec![SelectionStep::new(
                "What is 125 times 8?",
                Some("calculate"),
            )],
        },
        SelectionCase {
            name: "probe-two".into(),
            category: tacet_eval::Category::Tool,
            steps: vec![SelectionStep::new("What is 2 plus 2?", Some("calculate"))],
        },
    ]
}

/// A script long enough for one run of both cases. `with_default` keeps it from
/// erroring once the script is spent, which is what a second run would do if the
/// journal were not read.
fn engine(default: &str) -> Arc<dyn EngineProvider> {
    Arc::new(
        FakeEngine::script([
            r#"calculate({"expression":"125*8"})"#,
            "125 times 8 is 1000.",
            r#"calculate({"expression":"2+2"})"#,
            "2 plus 2 is 4.",
        ])
        .with_default(default),
    )
}

/// An engine with NO script at all: every generation comes back as text that is
/// not a call, so any case it actually runs fails. A resumed run must not need
/// it for anything.
fn spent_engine() -> Arc<dyn EngineProvider> {
    Arc::new(
        FakeEngine::script(Vec::<String>::new())
            .with_default("this text is not a call and would fail the case"),
    )
}

fn catalog_names() -> Vec<String> {
    let env = Env::setup().expect("an eval environment");
    suite_catalog(&env, &SharedMemory::in_memory())
        .catalog
        .names()
        .into_iter()
        .map(String::from)
        .collect()
}

fn open(dir: &std::path::Path, engine: &Arc<dyn EngineProvider>) -> Result<CaseJournal, String> {
    CaseJournal::open(dir, &engine.identity(), &catalog_names())
}

fn run(dir: &std::path::Path, engine: &Arc<dyn EngineProvider>) -> tacet_eval::SelectionReport {
    let journal = open(dir, engine).expect("the journal opens");
    run_selection_journalled(
        &cases(),
        engine,
        None,
        false,
        &suite_catalog,
        Some(&journal),
    )
}

#[test]
fn a_second_run_reuses_what_the_first_one_measured() {
    let dir = tmp("resume");
    let first = run(&dir, &engine("first-default"));
    assert_eq!(
        first.tool_passed, 2,
        "the fixture must pass on the first run"
    );

    // Every file is on disk, so the second run should need the engine for
    // NOTHING. Its script is spent, so anything it did run would come back as
    // the default text and fail the case — which is the point: this cannot pass
    // by accident.
    let second = run(&dir, &spent_engine());
    assert_eq!(
        second.tool_passed, 2,
        "the second run re-ran the cases instead of reading them back"
    );
    assert_eq!(
        second.cases.len(),
        2,
        "a resumed report must carry every case, not only the new ones"
    );
    assert_eq!(second.cases[0].name, "probe-one");
    assert_eq!(second.cases[1].name, "probe-two", "in the suite's order");
}

/// A directory holding one case must run the other and keep both.
#[test]
fn a_half_finished_run_finishes() {
    let dir = tmp("half");
    // Journal only the first case by running a one-case suite into the dir.
    let one = vec![cases().remove(0)];
    let e = engine("x");
    let journal = open(&dir, &e).expect("the journal opens");
    let report = run_selection_journalled(&one, &e, None, false, &suite_catalog, Some(&journal));
    assert_eq!(report.cases.len(), 1);

    // The second case is genuinely re-run, so this one keeps its script.
    let full = run(&dir, &engine("x"));
    assert_eq!(full.cases.len(), 2);
    assert_eq!(
        full.tool_passed, 2,
        "the resumed case and the newly run one must both count"
    );
}

/// THE ASSERTION THAT MATTERS. A journal written against one model must not be
/// silently reused for another.
#[test]
fn a_journal_from_a_different_run_is_refused_not_reused() {
    let dir = tmp("stamp");
    run(&dir, &engine("x"));

    // Forge a stamp from a different model.
    let stamp = dir.join("identity.json");
    let text = std::fs::read_to_string(&stamp).expect("the stamp is written on the first case");
    assert!(
        text.contains("identity") && text.contains("catalog"),
        "the stamp must name the model AND the catalog — the two things \
         `--compare` refuses to pair across: {text}"
    );
    std::fs::write(&stamp, text.replace("\"engine\"", "\"engine_was\"")).expect("writable");

    // OPENING IT MUST FAIL. The caller — `tacet eval` — turns that into a
    // non-zero exit; what must never happen is a report that mixes cases from
    // two builds and says nothing about it in the JSON.
    let refused = open(&dir, &engine("x"));
    let message = refused
        .err()
        .unwrap_or_else(|| panic!("a journal stamped for a different build was accepted"));
    assert!(
        message.contains("DIFFERENT"),
        "the refusal must say what is wrong, not just that something is: {message}"
    );
}

/// A truncated file — a crash mid-write — must be re-run, not read as a
/// finished case, and not deleted either: it is the record of the crash.
#[test]
fn a_half_written_file_is_run_again_and_kept() {
    let dir = tmp("torn");
    run(&dir, &engine("x"));
    let torn = dir.join("probe-one.json");
    std::fs::write(&torn, "{\"name\":\"probe-one\",\"cate").expect("writable");

    let after = run(&dir, &engine("x"));
    assert_eq!(
        after.cases.len(),
        2,
        "a torn file must not remove the case from the report"
    );
    assert!(
        torn.exists(),
        "the file must still be there after the re-run"
    );
}
