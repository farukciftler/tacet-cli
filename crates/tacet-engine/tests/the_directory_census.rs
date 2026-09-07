//! The working-directory census that rides in the system block.
//!
//! THESE TESTS MOVED WITH THE CODE. `dir_context` and `system_text` lived in
//! `tacet-cli`, which is why the eval never had them and asked thirty-odd
//! file questions of a model that had not been told what files exist. The
//! functions are in `tacet-engine::session` now, beside `SYSTEM_INSTRUCTIONS`,
//! and so are the assertions that bound them.

use tacet_engine::{SYSTEM_INSTRUCTIONS, TokenCounter, dir_context, system_text};

/// HIDDEN FILES STAY OUT. This block goes into a prompt on every turn, and
/// `.env` / `.git` / `.ssh` is where the things a user did not mean to
/// recite live.
#[test]
fn the_directory_block_skips_hidden_names_and_marks_folders() {
    let dir = std::env::temp_dir().join(format!(
        "tacet-dir-context-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("notes.md"), b"x").unwrap();
    std::fs::write(dir.join(".env"), b"SECRET=1").unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();

    let block = dir_context(&dir.display().to_string()).expect("no block");
    assert!(block.contains("notes.md"), "{block}");
    assert!(block.contains("src/"), "the folder is not marked: {block}");
    assert!(!block.contains(".env"), "a dotfile leaked: {block}");
    assert!(!block.contains(".git"), "a dotfile leaked: {block}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// THE MEASURED CEILING. This block is a FIXED COST ON EVERY PROMPT, so the
/// thing that must not drift is its worst case — the table in `dir_context`
/// is only honest while this holds. A directory of two hundred long names
/// must not quietly become a thousand-token tax.
#[test]
fn the_directory_block_cannot_grow_past_its_measured_ceiling() {
    let dir = std::env::temp_dir().join(format!(
        "tacet-dir-cap-{}-{:?}",
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
    let block = dir_context(&dir.display().to_string()).expect("no block");
    let tokens = TokenCounter::estimate(&block);
    assert!(
        tokens <= 250,
        "the directory block costs {tokens} tokens — the comment in `dir_context` promises ~228 at the cap"
    );
    // AND IT DOES NOT LIE ABOUT THE REST. A list that silently stops teaches
    // the model that the directory holds only what it can see.
    assert!(block.contains("more not listed"), "{block}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The block is glued to the instructions, not to the question: it must be
/// in the SYSTEM text, the one piece truncation never touches.
#[test]
fn the_directory_block_rides_in_the_system_instructions() {
    let plain = system_text(None);
    assert_eq!(plain, SYSTEM_INSTRUCTIONS);
    let with = system_text(Some(&"<cwd>\n.\na, b/\n</cwd>".to_string()));
    assert!(with.starts_with(SYSTEM_INSTRUCTIONS), "{with}");
    assert!(with.contains("<cwd>"), "{with}");
}
