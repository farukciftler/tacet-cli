//! Is there enough memory to load these weights — asked BEFORE the load, not
//! after the kernel answers it.
//!
//! ===========================================================================
//! WHY THIS FILE EXISTS: A ONE-WORD FAILURE
//! ===========================================================================
//!
//! Measured on a user's Linux VPS, tacet 0.1.11 (candle), qwen3-4b:
//!
//! ```text
//! $ tacet
//! ◞ loading qwen3-4b… 9s · ctrl-c to stopKilled
//! $ tacet chat
//! ◜ loading qwen3-4b… 15s · ctrl-c to stopKilled
//! ```
//!
//! `Killed` is the SHELL's word, printed after the fact for a process that
//! received SIGKILL from the kernel's OOM killer. It is not our message, it
//! carries no cause, and it cannot be caught: SIGKILL is not deliverable to a
//! handler, so there is no error path in which this crate gets to say anything.
//! The user tried four times, which is exactly what a person does when a failure
//! names nothing they could act on.
//!
//! So the only place this can be said is BEFORE the allocation. That is the
//! whole design: read what the machine can spare, compare it against what the
//! file about to be read will cost, and if it does not fit, refuse in words.
//!
//! ===========================================================================
//! IT ONLY ANSWERS WHERE THE ANSWER IS MEASURABLE
//! ===========================================================================
//!
//! `available()` returns `None` on every platform but Linux, and a `None` skips
//! the check entirely. This is deliberate:
//!
//! · LINUX has `MemAvailable` in `/proc/meminfo` — the kernel's OWN estimate of
//!   what can be handed out without swapping. It is the right question (unlike
//!   `MemFree`, which counts cache as unavailable and would refuse on a healthy
//!   machine), it is a plain file read with no dependency, and Linux is where
//!   the OOM killer was actually observed.
//!
//! · macOS would need `vm_stat` or a `sysctl` call, and the number it produces
//!   is much weaker: memory compression and a swap file that grows on demand
//!   mean a Mac routinely runs models "larger than free memory" without dying.
//!   A check there would refuse loads that work. Nobody has measured an OOM kill
//!   on macOS in this project.
//!
//! · WINDOWS would need a Win32 call, which is a dependency question, and no
//!   failure has been observed there either.
//!
//! A wrong "no" is worse than no check: it takes a working install away from
//! someone and leaves them with a message they cannot argue with. That is why
//! silence is the default on anything not measured.
//!
//! ===========================================================================
//! THE ESTIMATE, AND WHY IT IS DELIBERATELY SMALL
//! ===========================================================================
//!
//! `needed_bytes` = the size of the GGUF file + `HEADROOM_BYTES`.
//!
//! The weights are read into memory, so the file size is a FLOOR that needs no
//! guessing. The headroom stands in for the KV cache, the process itself and the
//! allocator's slack. It is set low (512 MB) on purpose: this gate exists to
//! catch the case that is NOT CLOSE — 1.2 GB free against a 2.3 GB file — and to
//! stay out of the way of the case that is merely tight. A generous estimate
//! would start refusing borderline machines that do in fact work, and the cost
//! of a false refusal is higher than the cost of a rare kill we did not predict.
//!
//! `TACET_SKIP_MEMORY_CHECK=1` turns the gate off for the person who knows their
//! machine better than this arithmetic does — someone with a large swap file,
//! for instance. It exists so that a wrong "no" is never the end of the road.

use std::path::Path;

/// What is added to the weights file's size to stand in for the KV cache, the
/// process and allocator slack. See the file header for why it is small.
const HEADROOM_BYTES: u64 = 512 * 1024 * 1024;

/// The environment variable that switches the gate off.
pub const SKIP_VAR: &str = "TACET_SKIP_MEMORY_CHECK";

/// What this machine can hand out without swapping, in bytes.
///
/// `None` = "not measured on this platform", which every caller must read as
/// PERMISSION TO PROCEED, never as zero.
pub fn available() -> Option<u64> {
    available_from(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// The parsing half, split out so it can be measured on a machine that has no
/// `/proc` — otherwise this function could only be tested on Linux, which is the
/// one platform where a mistake in it would be found the slow way.
///
/// It reads `MemAvailable` and NOTHING ELSE. `MemFree` is the tempting
/// neighbour and it is the wrong number: a healthy Linux box keeps most of RAM
/// in page cache, so `MemFree` on a 16 GB server can read 300 MB while 14 GB is
/// available on demand. Refusing on that would be a false alarm on nearly every
/// machine.
fn available_from(meminfo: &str) -> Option<u64> {
    for line in meminfo.lines() {
        let Some(rest) = line.strip_prefix("MemAvailable:") else {
            continue;
        };
        // The format is `MemAvailable:   1234567 kB` — a count and a unit.
        let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
        return Some(kb * 1024);
    }
    // An older kernel (pre-3.14) has no MemAvailable line. Rather than fall back
    // to a number that means something else, the check goes quiet.
    None
}

/// What a load of this file is expected to cost.
pub fn needed_bytes(gguf: &Path) -> Option<u64> {
    Some(std::fs::metadata(gguf).ok()?.len() + HEADROOM_BYTES)
}

/// `Some(message)` = do not load, and here is what to tell the user.
/// `None` = nothing measurable stands in the way.
///
/// THE MESSAGE NAMES THE WAY OUT, and does it with a smaller model rather than
/// "buy more memory": `qwen2.5-3b` is in the download catalog and is the reason
/// it is there.
pub fn refusal(gguf: &Path, model_name: &str) -> Option<String> {
    if std::env::var_os(SKIP_VAR).is_some() {
        return None;
    }
    let (Some(have), Some(need)) = (available(), needed_bytes(gguf)) else {
        return None;
    };
    if have >= need {
        return None;
    }
    Some(message(have, need, model_name))
}

/// The wording, separated from the decision so the text can be measured without
/// a machine that is actually short on memory.
fn message(have: u64, need: u64, model_name: &str) -> String {
    let mb = |b: u64| b / (1024 * 1024);
    format!(
        "not enough memory to load {model_name}: this machine can spare about {} MB and the \
         weights need roughly {} MB.\n  \
         Loading anyway is what produces a bare `Killed` from the kernel with no explanation.\n  \
         A smaller model is the usual way out:\n    \
         tacet models download qwen2.5-3b && tacet chat --model qwen2.5-3b\n  \
         Adding swap also works. To load regardless of this check: {SKIP_VAR}=1 tacet chat",
        mb(have),
        mb(need)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE REAL FORMAT, taken from a Linux box — including the fields that come
    /// before the one being looked for, because a parser that reads the FIRST
    /// number it sees would answer `MemTotal` here and never refuse anything.
    const MEMINFO: &str = "MemTotal:        4025024 kB\n\
                           MemFree:          148504 kB\n\
                           MemAvailable:    1210408 kB\n\
                           Buffers:           23120 kB\n";

    #[test]
    fn the_kernels_own_estimate_is_the_one_that_is_read() {
        assert_eq!(available_from(MEMINFO), Some(1_210_408 * 1024));
        // Not MemTotal, and not MemFree — both are in the input above.
        assert_ne!(available_from(MEMINFO), Some(4_025_024 * 1024));
        assert_ne!(available_from(MEMINFO), Some(148_504 * 1024));
    }

    /// A kernel too old to have the line, and a file that is not meminfo at all,
    /// both mean "unknown" — which every caller reads as permission to proceed.
    /// Answering 0 here would refuse every load on those machines.
    #[test]
    fn an_unreadable_answer_is_none_and_never_zero() {
        assert_eq!(
            available_from("MemTotal: 4025024 kB\nMemFree: 148504 kB\n"),
            None
        );
        assert_eq!(available_from(""), None);
        assert_eq!(available_from("MemAvailable: not-a-number kB"), None);
    }

    /// THE VPS THIS WAS WRITTEN FOR. 1.15 GB available against qwen3-4b's
    /// 2.3 GiB of weights: the machine that printed `Killed` four times.
    #[test]
    fn the_message_names_both_numbers_and_a_way_out() {
        let have = 1_210_408 * 1024;
        let need = 2_469_606_195 + HEADROOM_BYTES;
        let m = message(have, need, "qwen3-4b");
        assert!(m.contains("qwen3-4b"), "{m}");
        assert!(
            m.contains("1182 MB"),
            "the number it has must be shown: {m}"
        );
        assert!(
            m.contains("2867 MB"),
            "the number it needs must be shown: {m}"
        );
        // The way out is a command that can be pasted, not advice.
        assert!(m.contains("tacet models download qwen2.5-3b"), "{m}");
        assert!(
            m.contains(SKIP_VAR),
            "the override must be discoverable: {m}"
        );
        // And it explains the word the user actually saw.
        assert!(m.contains("Killed"), "{m}");
    }

    /// THE GATE IS ONE-SIDED. It refuses only when the numbers are known AND the
    /// machine is short; every other combination proceeds. A test for this
    /// belongs here because the failure mode — refusing on an unknown — would
    /// look like "tacet stopped working" on macOS and Windows, where `available`
    /// is always `None`.
    #[test]
    fn nothing_measurable_means_nothing_refused() {
        let missing = Path::new("/nonexistent/model.gguf");
        assert!(needed_bytes(missing).is_none());
        assert!(refusal(missing, "qwen3-4b").is_none());
    }
}
