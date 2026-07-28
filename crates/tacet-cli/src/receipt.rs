//! The receipt chain — the storage behind `tacet log`.
//!
//! Every tool execution is recorded by PURE CODE: the model is never asked to
//! log anything and cannot reach this file through any tool (the sandbox sees
//! no files, the document tools live in the working directory, this lives in
//! the config directory). Each JSONL line carries the SHA-256 of
//! `previous hash + canonical payload`, so editing or deleting ANY line breaks
//! every hash after it — `tacet log` verifies the whole chain on each run and
//! says so out loud.
//!
//! WRITE FAILURES ARE NON-FATAL. A read-only config directory must not take
//! the assistant down; the receipt is a witness, not a lock. The failure is
//! whispered once per process (the same restraint as the truncation notice).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ui::{BOLD, Color, DIM, YELLOW};

const FILE: &str = "receipts.jsonl";
/// The chain's anchor: the `prev` of the very first entry.
const GENESIS: &str = "genesis";

static WRITE_FAILED_REPORTED: AtomicBool = AtomicBool::new(false);

fn path() -> Option<PathBuf> {
    tacet_kernel::config_path(FILE)
}

/// The canonical payload text the hash covers. serde_json's default map is
/// ordered (BTree), so serialising the same fields always yields the same
/// bytes — the property the verifier depends on.
fn canonical(at: u64, tool: &str, text: &str, state: &str) -> String {
    serde_json::json!({ "at": at, "state": state, "text": text, "tool": tool }).to_string()
}

/// Appends one receipt. Called from the shell after a turn, once per tool
/// trace; the model is not in the loop.
pub fn append(at: u64, tool: &str, text: &str, state: &str) {
    let Some(p) = path() else { return };
    let prev = last_hash(&p).unwrap_or_else(|| GENESIS.to_string());
    let payload = canonical(at, tool, text, state);
    let hash = tacet_web::download::sha256_hex(format!("{prev}{payload}").as_bytes());
    let line = serde_json::json!({
        "at": at, "state": state, "text": text, "tool": tool,
        "prev": prev, "hash": hash,
    })
    .to_string();

    let write = || -> std::io::Result<()> {
        if let Some(parent) = p.parent() {
            tacet_kernel::fs::create_private_dir(parent)?;
        }
        let fresh = !p.exists();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)?;
        if fresh {
            tacet_kernel::fs::narrow_file(&p);
        }
        writeln!(f, "{line}")
    };
    if write().is_err() && !WRITE_FAILED_REPORTED.swap(true, Ordering::Relaxed) {
        eprintln!("(the receipt log could not be written — receipts are off for this session)");
    }
}

fn last_hash(p: &PathBuf) -> Option<String> {
    let text = std::fs::read_to_string(p).ok()?;
    let last = text.lines().rev().find(|l| !l.trim().is_empty())?;
    serde_json::from_str::<serde_json::Value>(last)
        .ok()?
        .get("hash")?
        .as_str()
        .map(str::to_string)
}

/// One verified entry, parsed for display.
struct Entry {
    at: u64,
    tool: String,
    text: String,
    state: String,
}

/// Walks the file front to back, recomputing every hash. `Err(k)` = the chain
/// is broken at 1-based entry `k` (edited, truncated in the middle, or written
/// by something that is not this code).
fn verify(raw: &str) -> Result<Vec<Entry>, usize> {
    let mut prev = GENESIS.to_string();
    let mut entries = Vec::new();
    for (i, line) in raw.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let v: serde_json::Value = serde_json::from_str(line).map_err(|_| i + 1)?;
        let field = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let at = v.get("at").and_then(|x| x.as_u64()).ok_or(i + 1)?;
        let (tool, text, state, line_prev, line_hash) = match (
            field("tool"),
            field("text"),
            field("state"),
            field("prev"),
            field("hash"),
        ) {
            (Some(a), Some(b), Some(c), Some(d), Some(e)) => (a, b, c, d, e),
            _ => return Err(i + 1),
        };
        let expected = tacet_web::download::sha256_hex(
            format!("{prev}{}", canonical(at, &tool, &text, &state)).as_bytes(),
        );
        if line_prev != prev || line_hash != expected {
            return Err(i + 1);
        }
        prev = line_hash;
        entries.push(Entry {
            at,
            tool,
            text,
            state,
        });
    }
    Ok(entries)
}

/// `tacet log` — the tail of the chain plus the verification verdict.
pub fn log(json: bool, limit: usize) -> ExitCode {
    let color = Color::setup();
    let Some(p) = path() else {
        eprintln!("error: the config directory cannot be resolved");
        return ExitCode::FAILURE;
    };
    let raw = std::fs::read_to_string(&p).unwrap_or_default();
    if raw.trim().is_empty() {
        println!(
            "{}",
            color.paint(DIM, "no receipts yet — they appear as tools run.")
        );
        return ExitCode::SUCCESS;
    }

    match verify(&raw) {
        Err(k) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "intact": false, "broken_at": k, "path": p.display().to_string() })
                );
            } else {
                eprintln!(
                    "{}",
                    color.paint(
                        YELLOW,
                        &format!(
                            "CHAIN BROKEN at entry {k} — the file was edited or damaged ({})",
                            p.display()
                        )
                    )
                );
            }
            ExitCode::FAILURE
        }
        Ok(entries) => {
            if json {
                let rows: Vec<serde_json::Value> = entries
                    .iter()
                    .rev()
                    .take(limit)
                    .map(|e| {
                        serde_json::json!({ "at": e.at, "tool": e.tool, "text": e.text, "state": e.state })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({ "intact": true, "total": entries.len(), "receipts": rows })
                );
                return ExitCode::SUCCESS;
            }
            let shown: Vec<&Entry> = entries.iter().rev().take(limit).collect();
            for e in shown.iter().rev() {
                println!(
                    "  {} {}  {}",
                    color.paint(DIM, &format!("{}", e.at)),
                    color.paint(BOLD, &e.tool),
                    color.paint(DIM, &format!("{} · {}", e.text, e.state))
                );
            }
            println!(
                "{}",
                color.paint(
                    DIM,
                    &format!(
                        "  chain intact · {} receipts · {}",
                        entries.len(),
                        p.display()
                    )
                )
            );
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_chain_verifies_and_a_tampered_one_breaks() {
        let prev = GENESIS.to_string();
        let payload = canonical(7, "calculate", "12*34 = 408", "Read");
        let hash = tacet_web::download::sha256_hex(format!("{prev}{payload}").as_bytes());
        let line1 = serde_json::json!({
            "at": 7, "state": "Read", "text": "12*34 = 408", "tool": "calculate",
            "prev": prev, "hash": hash,
        })
        .to_string();

        let payload2 = canonical(9, "time", "clock", "Read");
        let hash2 = tacet_web::download::sha256_hex(format!("{hash}{payload2}").as_bytes());
        let line2 = serde_json::json!({
            "at": 9, "state": "Read", "text": "clock", "tool": "time",
            "prev": hash, "hash": hash2,
        })
        .to_string();

        let good = format!("{line1}\n{line2}\n");
        assert_eq!(verify(&good).map(|e| e.len()), Ok(2));

        // Editing entry 1's text breaks the chain AT entry 1.
        let tampered = good.replace("408", "409");
        assert!(matches!(verify(&tampered), Err(1)));
    }
}
