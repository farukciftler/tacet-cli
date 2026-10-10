//! A 4096-token model must still answer an ordinary question.
//!
//! WHY THIS EXISTS (review of audit L21). `TokenCounter::fit` used to refuse a
//! turn whenever the part truncation may not touch was over the prompt cap —
//! the window minus the 1024 tokens reserved for the answer. Measured on the
//! shell's real prompt on a 4096 window (every llama-family model since the
//! rope-table cut, SmolLM2 included): the system instructions, a full
//! directory census and the nine tools the router picks come to 2658-3113
//! tokens against a cap of 3072. In a crowded directory "hi" and "write a
//! python script that sums a csv" were refused at 3099 and 3113 tokens, with
//! about 990 tokens of the window still free for the answer — turns the same
//! model answered before the change. In a quieter directory the python request
//! went through with the front of the question cut.
//!
//! The prompt is built here the way the turn loop builds it — `system_text`
//! over `dir_context`, `Router::select` over the production catalog, the
//! memory block at its injection limit — and pushed through the same `fit`.
//! No weights: the decision is made on the estimate, before generation.
//!
//! Measured here, 10 Oct 2026 (macOS arm64), nine tools each: "hi" 3438
//! tokens, the python request 3452, "list the files here" 3100 — every one
//! over the 3072 cap and under the 3840 the window holds with `MIN_ANSWER`
//! left. `--nocapture` prints the numbers again.

use std::sync::Arc;

use tacet_engine::{
    GENERATION_SHARE, MIN_ANSWER, Prompt, TokenCounter, context_budget, dir_context, system_text,
};
use tacet_tools::router::Router;

/// A directory whose census runs to its ceiling: two hundred long names, so
/// the block is capped and carries its "(N more not listed)" tail.
fn crowded_directory() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tacet-small-window-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..200 {
        std::fs::write(
            dir.join(format!("a-quite-long-file-name-number-{i:03}.txt")),
            b"x",
        )
        .unwrap();
    }
    dir
}

#[test]
fn the_router_selected_prompt_fits_a_4096_window_with_a_full_directory_block() {
    let dir = crowded_directory();
    let block = dir_context(&dir.display().to_string()).expect("a census");
    let system = system_text(Some(&block));

    let store = Arc::new(tacet_tools::data_store::SharedStore::new());
    let memory = tacet_tools::memory::SharedMemory::in_memory();
    let (catalog, _, _) =
        tacet_tools::catalog::production_catalog_with(&store, &memory, Some(0), true);
    let router = Router::new();

    // A llama-family model on candle: whatever the file declares, the runnable
    // window is the 4096 of candle's rope table.
    let window = context_budget(Some(4096), Some(1_000), tacet_engine::Device::Cpu);
    assert_eq!(window, 4096);
    let counter = TokenCounter::new(window, GENERATION_SHARE);

    // The memory block at its worst: the store never injects more than this.
    let memory_block = "m".repeat(600);

    for message in [
        "hi",
        "write a python script that sums a csv",
        "list the files here",
    ] {
        let selected: tacet_kernel::ToolCatalog =
            router.select(message, &catalog).into_iter().collect();
        let mut prompt = Prompt::new(&system, message)
            .with_tools(&selected)
            .with_memory(&memory_block);
        let before = counter.prompt_estimate(&prompt);
        eprintln!(
            "{message:?}: {} tools, {before} tokens, cap {} ceiling {}",
            selected.names().len(),
            counter.prompt_cap(),
            counter.prompt_ceiling()
        );

        let report = match counter.fit(&mut prompt) {
            Ok(report) => report,
            Err(e) => panic!(
                "{message:?} ({} tools, {before} tokens) was refused on a 4096 \
                 window with {} tokens free: {e}",
                selected.names().len(),
                window.saturating_sub(before)
            ),
        };
        // The user's words reach the model whole.
        assert_eq!(
            prompt.question(),
            message,
            "{message:?}: the question was cut ({before} tokens)"
        );
        assert!(!report.question_truncated && !report.fixed_part_too_large);
        // And the answer has real room, inside the window.
        let estimate = counter.prompt_estimate(&prompt);
        let cap = counter.generation_cap(&prompt);
        assert!(
            cap >= MIN_ANSWER,
            "{message:?}: {cap} tokens left for the answer"
        );
        assert!(estimate + cap <= window);
    }

    let _ = std::fs::remove_dir_all(&dir);
}
