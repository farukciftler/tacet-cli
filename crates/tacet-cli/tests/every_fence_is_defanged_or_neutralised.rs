//! Every closing tag this workspace writes is either a fence the defang knows,
//! or named below as not a fence (audit H4-a, review of 10 Oct 2026).
//!
//! WHY A SCAN AND NOT A LIST. `FENCE_NAMES` held seven names while the tools
//! wrote four more fences around foreign text — `<search_result>`,
//! `<api_response>`, `<clipboard>` and the shell's `<stdin>` — and nothing
//! said so. Each of the four was closed from inside by the text it fenced
//! (measured: a fake SearXNG result, an API body, a piped page). A list of
//! fences kept by hand is the thing that drifted; the source is what writes
//! them, so the source is what is read.
//!
//! WHAT IS READ: every `.rs` file under `crates/*/src`, up to its first
//! `#[cfg(test)]` or `#[cfg(all(test, …))]` (tests write tags on purpose), skipping comment lines. A
//! `</name>` there must be on `FENCE_NAMES` (the prompt defangs it), on
//! `DATA_FENCE_NAMES` (its writer neutralises it, and has a test that says
//! so), a model token on `NAMED_MARKERS`, or in `NOT_A_FENCE` with the reason
//! it never reaches a prompt as a fence.

use std::path::{Path, PathBuf};

/// (file suffix, tag, why it is not a fence of ours).
const NOT_A_FENCE: &[(&str, &str, &str)] = &[
    // OOXML written into the .xlsx/.docx a user asked for; never a prompt.
    ("tacet-tools/src/create_document.rs", "*", "OOXML in a file"),
    // The same OOXML read back; the model gets cell text, not the markup.
    (
        "tacet-tools/src/read_document.rs",
        "*",
        "OOXML being parsed",
    ),
    // Antigravity's own envelope around a user request in its transcript.
    // The reader STRIPS it; Tacet never writes it into a prompt.
    (
        "tacet-tools/src/sessions/antigravity.rs",
        "USER_REQUEST",
        "Antigravity's tag, stripped on read",
    ),
    (
        "tacet-tools/src/sessions/antigravity.rs",
        "ADDITIONAL_METADATA",
        "Antigravity's tag, stripped on read",
    ),
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `(file, tag)` for every `</tag>` in the non-test, non-comment source.
fn closing_tags() -> Vec<(String, String)> {
    let root = workspace();
    let mut files = Vec::new();
    for c in std::fs::read_dir(&root).unwrap().flatten() {
        rust_files(&c.path().join("src"), &mut files);
    }
    assert!(files.len() > 50, "the scan found {} files", files.len());
    let mut found = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for line in text.lines() {
            if line.starts_with("#[cfg(test)]") || line.starts_with("#[cfg(all(test") {
                break;
            }
            if line.trim_start().starts_with("//") {
                continue;
            }
            let mut rest = line;
            while let Some(at) = rest.find("</") {
                let after = &rest[at + 2..];
                let name: String = after
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() && after[name.len()..].starts_with('>') {
                    found.push((rel.clone(), name));
                }
                rest = after;
            }
        }
    }
    found
}

fn covered(file: &str, tag: &str) -> bool {
    let lower = tag.to_ascii_lowercase();
    tacet_kernel::defang::FENCE_NAMES.contains(&lower.as_str())
        || tacet_kernel::defang::DATA_FENCE_NAMES.contains(&lower.as_str())
        || tacet_kernel::defang::NAMED_MARKERS.contains(&format!("</{tag}>").as_str())
        || NOT_A_FENCE
            .iter()
            .any(|(f, t, _)| file.ends_with(f) && (*t == "*" || *t == tag))
}

#[test]
fn every_closing_tag_in_the_source_is_a_fence_the_defang_covers() {
    let found = closing_tags();
    let loose: Vec<_> = found.iter().filter(|(f, t)| !covered(f, t)).collect();
    assert!(
        loose.is_empty(),
        "a closing tag no defang covers: {loose:?}. A fence Tacet writes around \
         foreign text goes on DATA_FENCE_NAMES and its writer calls \
         neutralise_data_fences; a fence of the prompt's own goes on FENCE_NAMES."
    );
}

/// The scan must see the fences it exists for, or it proves nothing: a
/// regex that matched no line would pass the test above with an empty list.
#[test]
fn the_scan_sees_the_data_fences_it_was_written_for() {
    let found = closing_tags();
    for (file, tag) in [
        ("tacet-tools/src/web_search.rs", "search_result"),
        ("tacet-tools/src/http_call.rs", "api_response"),
        ("tacet-tools/src/clipboard.rs", "clipboard"),
        ("tacet-cli/src/main.rs", "stdin"),
        ("tacet-engine/src/prompt.rs", "tool_response"),
    ] {
        assert!(
            found.iter().any(|(f, t)| f.ends_with(file) && t == tag),
            "the scan did not see </{tag}> in {file}"
        );
    }
}
