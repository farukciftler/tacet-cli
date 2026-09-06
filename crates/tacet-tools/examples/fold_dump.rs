//! Prints the folded form of each line on stdin, for `esp32/check.py`.
//!
//! WHY A BINARY. The trainer is Python, the device is C, and the fold every
//! Tacet user runs is Rust. The cross-check compared the first two and left the
//! third to seven fixed strings — under a test named
//! `the_fold_matches_the_trainer` that never consulted the trainer. So the one
//! implementation that ships to users was the one nothing verified. This lets
//! check.py drive all three over one corpus.
//!
//! Lines arrive escaped, because a message under test may contain the very
//! separators being tested — the same escaping check.py already uses for the C.

use std::io::{BufRead, Write};

fn unescape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('v') => out.push('\u{0b}'),
            Some('f') => out.push('\u{0c}'),
            Some('\\') => out.push('\\'),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => out.push_str(&format!("\\x{hex}")),
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\\' => "\\\\".to_string(),
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            c if (c as u32) < 0x20 => format!("\\x{:02x}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let folded = tacet_tools::slot_gate::fold(&unescape(&line));
        writeln!(out, "{}", escape(&folded)).expect("stdout is writable");
    }
}
