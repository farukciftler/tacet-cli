//! Which versions of each agent this module has actually seen, and what that
//! permits.
//!
//! WHY A GATE. Claude Code's own documentation says the transcript format "is
//! internal to Claude Code and changes between versions, so scripts that parse
//! these files directly can break on any release"; 17 versions wrote
//! transcripts on one machine in one month. A reader that meets a version it
//! has never seen may still be right — so reading goes ahead WITH A WARNING the
//! user sees. A writer that is wrong costs the user their history (MEASURED:
//! one bad `parentUuid` and `--resume` loaded nothing before it) — so writing
//! is REFUSED for any version not in the table, and the table only lists a
//! version once a write was round-tripped on it.
//!
//! READ AND WRITE RANGES ARE SEPARATE ON PURPOSE. "We parsed its files" and
//! "we appended to its file and the agent loaded it correctly" are different
//! measurements, made on different sets of versions. Collapsing them into one
//! range would let a version that was only ever read be written to.
//!
//! WIDENING A RANGE IS A MEASUREMENT, NOT AN EDIT. Each row says where its
//! numbers came from. Extend a read range after parsing that version's files
//! without a single unparsed line (the RG-28 fixture corpus); extend a write
//! range only after re-running the round-trip experiment (RG-29) on it. The
//! ADR (docs/adr/0001-agent-sessions.md) carries the same table and a test
//! holds the two together.

use std::cmp::Ordering;
use std::fmt;
use std::path::Path;

use super::model::AgentKind;

/// One measured range. `flavor` separates products that share an agent but
/// not a version line (Antigravity IDE 2.5.5, `agy` CLI 1.1.10).
#[derive(Debug, Clone, Copy)]
pub struct Known {
    pub agent: AgentKind,
    pub flavor: Option<&'static str>,
    pub read: (&'static str, &'static str),
    pub write: Option<(&'static str, &'static str)>,
    pub measured: &'static str,
}

/// THE TABLE. Codex has no row: it is not installed on the machine this was
/// measured on, its format is known from source (openai/codex @ a06545b) and
/// fixtures only, so every Codex version reads with a warning and none is
/// written to until a real install is measured.
pub const KNOWN: &[Known] = &[
    Known {
        agent: AgentKind::ClaudeCode,
        flavor: None,
        read: ("2.1.263", "2.1.293"),
        write: Some(("2.1.285", "2.1.285")),
        measured: "read: the 30 newest transcripts on one Mac (340 MB, 17 versions, no unparsed line), \
                   9 Oct 2026; write: the E2-E5 append/resume experiment on 2.1.285, 9 Oct 2026",
    },
    Known {
        agent: AgentKind::Antigravity,
        flavor: Some("ide"),
        read: ("2.5.5", "2.5.5"),
        write: None,
        measured: "read: brain/ transcripts and task.md of 12 IDE conversations, 9 Oct 2026; \
                   write: never measured (RG-26 needs the GUI to re-read task.md, by hand)",
    },
    // Measured, and kept so the measurement is not lost, but no read reaches
    // it today: `agy` writes no version anywhere a read opens, and its own is
    // only in the binary (`antigravity::installed_version` has no row for its
    // root), so every `agy` read is asked about no version and warns. The ADR
    // row says so, and `antigravity`'s tests hold it to that.
    Known {
        agent: AgentKind::Antigravity,
        flavor: Some("cli"),
        read: ("1.1.10", "1.1.10"),
        write: None,
        measured: "read: brain/ transcripts of 22 agy conversations, 9 Oct 2026; write: never measured",
    },
];

/// A version as dotted numbers. Strict: `2.1.285` and `0.46.0` parse;
/// `2.1.285-beta`, `v2` and `latest` do not, and an unparseable version is an
/// UNKNOWN version — a pre-release of a known version is not that version.
#[derive(Debug, Clone)]
pub struct Version(Vec<u64>);

// Equality is the ordering's, not the derived field-by-field one: a derived
// `PartialEq` would call `2.1` and `2.1.0` different while `cmp` calls them
// equal, and a range check would then depend on which of the two it used.
impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let parts: Option<Vec<u64>> = text
            .split('.')
            .map(|p| {
                if !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()) {
                    p.parse().ok()
                } else {
                    None
                }
            })
            .collect();
        parts.filter(|p| p.len() <= 4).map(Version)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    /// `2.1` equals `2.1.0`: missing components are zero.
    fn cmp(&self, other: &Self) -> Ordering {
        let n = self.0.len().max(other.0.len());
        for i in 0..n {
            let (a, b) = (
                self.0.get(i).copied().unwrap_or(0),
                other.0.get(i).copied().unwrap_or(0),
            );
            match a.cmp(&b) {
                Ordering::Equal => continue,
                unequal => return unequal,
            }
        }
        Ordering::Equal
    }
}

fn within(version: &Version, (lo, hi): (&str, &str)) -> bool {
    match (Version::parse(lo), Version::parse(hi)) {
        (Some(lo), Some(hi)) => &lo <= version && version <= &hi,
        // A malformed row in the table never admits anything.
        _ => false,
    }
}

fn row(agent: AgentKind, flavor: Option<&str>) -> Option<&'static Known> {
    KNOWN
        .iter()
        .find(|k| k.agent == agent && k.flavor == flavor)
}

fn label(agent: AgentKind, flavor: Option<&str>, version: Option<&str>) -> String {
    let mut s = agent.display_name().to_string();
    if let Some(f) = flavor {
        s.push_str(&format!(" ({f})"));
    }
    match version {
        Some(v) => s.push_str(&format!(" {v}")),
        None => s.push_str(" of unrecorded version"),
    }
    s
}

/// What reading a session of this version means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadVerdict {
    Known,
    /// Read anyway, and show the user this line: the result may be incomplete.
    Unknown {
        warning: String,
    },
}

impl ReadVerdict {
    pub fn warning(&self) -> Option<&str> {
        match self {
            ReadVerdict::Known => None,
            ReadVerdict::Unknown { warning } => Some(warning),
        }
    }
}

pub fn check_read(agent: AgentKind, flavor: Option<&str>, version: Option<&str>) -> ReadVerdict {
    let parsed = version.and_then(Version::parse);
    if let (Some(v), Some(k)) = (&parsed, row(agent, flavor))
        && within(v, k.read)
    {
        return ReadVerdict::Known;
    }
    let seen = match row(agent, flavor) {
        Some(k) => format!("versions {}–{} were measured", k.read.0, k.read.1),
        None => "no version of it was measured on a real install".to_string(),
    };
    ReadVerdict::Unknown {
        warning: format!(
            "{} has not been read by this Tacet before ({seen}); the session may be shown incompletely",
            label(agent, flavor, version)
        ),
    }
}

/// Why a write was not attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRefused {
    pub reason: String,
}

impl fmt::Display for WriteRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not written: {}", self.reason)
    }
}

impl std::error::Error for WriteRefused {}

/// May Tacet write into a session of this version? Only inside a measured
/// write range; a missing or unparseable version is refused like an unknown
/// one, because "we could not tell" is not "it is safe".
pub fn check_write(
    agent: AgentKind,
    flavor: Option<&str>,
    version: Option<&str>,
) -> Result<(), WriteRefused> {
    let refuse = |why: String| {
        Err(WriteRefused {
            reason: format!("{} — {why}", label(agent, flavor, version)),
        })
    };
    let Some(v) = version.and_then(Version::parse) else {
        return refuse("the session does not record a version Tacet can compare".into());
    };
    match row(agent, flavor).and_then(|k| k.write) {
        Some(range) if within(&v, range) => Ok(()),
        Some((lo, hi)) => refuse(format!("writing was only verified on {lo}–{hi}")),
        None => refuse("writing into its files has never been verified".into()),
    }
}

/// Which file of the agent's own storage may be written at all.
///
/// ANTIGRAVITY: its terms (§6) forbid third-party access to the product, and
/// writing into its storage is unsupported; ADR 0001 therefore allows exactly
/// one write — checkbox updates to a conversation's `task.md` — and only when
/// the user has passed the explicit experimental flag. Everything else
/// (`.db`, `messages/`, metadata) is refused unconditionally; the supported way
/// to reach an Antigravity session is its PreInvocation hook, which reads
/// Tacet's handoff file and writes nothing of Antigravity's.
///
/// CLAUDE CODE AND CODEX: their transcripts (`.jsonl`) only, never a compressed
/// `.jsonl.zst` rollout (no decoder; an append to one corrupts it).
pub fn check_write_target(
    agent: AgentKind,
    target: &Path,
    experimental: bool,
) -> Result<(), WriteRefused> {
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let refuse = |why: &str| {
        Err(WriteRefused {
            reason: format!("{}: {why}", target.display()),
        })
    };
    match agent {
        AgentKind::Antigravity if name != "task.md" => refuse(
            "Antigravity storage is read-only to Tacet; only task.md may be written, behind the experimental flag",
        ),
        AgentKind::Antigravity if !experimental => refuse(
            "writing Antigravity's task.md is experimental and off by default (its terms forbid third-party access)",
        ),
        AgentKind::Antigravity => Ok(()),
        AgentKind::ClaudeCode | AgentKind::Codex if name.ends_with(".jsonl") => Ok(()),
        AgentKind::ClaudeCode | AgentKind::Codex => {
            refuse("only a .jsonl transcript may be appended to")
        }
    }
}

/// Both gates. Every write path calls this and nothing narrower.
pub fn may_write(
    agent: AgentKind,
    flavor: Option<&str>,
    version: Option<&str>,
    target: &Path,
    experimental: bool,
) -> Result<(), WriteRefused> {
    check_write_target(agent, target, experimental)?;
    check_write(agent, flavor, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically_not_as_text() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("2.1.285") > v("2.1.29"));
        assert!(v("2.1.293") > v("2.1.285"));
        assert_eq!(v("2.1"), v("2.1.0"));
        for bad in [
            "",
            "v2.1.285",
            "2.1.285-beta",
            "2..1",
            "latest",
            "2.1.285 (Claude Code)",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_measured_version_reads_quietly_and_an_unknown_one_warns() {
        assert_eq!(
            check_read(AgentKind::ClaudeCode, None, Some("2.1.285")),
            ReadVerdict::Known
        );
        assert_eq!(
            check_read(AgentKind::ClaudeCode, None, Some("2.1.263")),
            ReadVerdict::Known
        );
        for unknown in [Some("2.1.294"), Some("2.0.0"), Some("2.1.285-beta"), None] {
            let verdict = check_read(AgentKind::ClaudeCode, None, unknown);
            assert!(
                verdict.warning().unwrap().contains("2.1.263–2.1.293"),
                "{verdict:?}"
            );
        }
        let codex = check_read(AgentKind::Codex, None, Some("0.46.0"));
        assert!(
            codex
                .warning()
                .unwrap()
                .contains("no version of it was measured"),
            "{codex:?}"
        );
        assert_eq!(
            check_read(AgentKind::Antigravity, Some("ide"), Some("2.5.5")),
            ReadVerdict::Known
        );
        // The CLI's version line is not the IDE's: 2.5.5 is not a known agy.
        assert!(
            check_read(AgentKind::Antigravity, Some("cli"), Some("2.5.5"))
                .warning()
                .is_some()
        );
    }

    /// The rule the whole table exists for: a version that was READ but never
    /// round-tripped is not written to, and neither is anything unrecorded.
    #[test]
    fn a_write_is_refused_for_every_version_that_was_not_round_tripped() {
        assert_eq!(
            check_write(AgentKind::ClaudeCode, None, Some("2.1.285")),
            Ok(())
        );
        for v in [
            Some("2.1.284"),
            Some("2.1.293"),
            Some("2.1.286"),
            Some("nightly"),
            None,
        ] {
            assert!(
                check_write(AgentKind::ClaudeCode, None, v).is_err(),
                "{v:?} was writable"
            );
        }
        assert!(check_write(AgentKind::Codex, None, Some("0.46.0")).is_err());
        assert!(check_write(AgentKind::Antigravity, Some("ide"), Some("2.5.5")).is_err());
    }

    #[test]
    fn antigravity_is_written_only_to_task_md_and_only_behind_the_flag() {
        let task = Path::new("/x/brain/c1/task.md");
        assert!(check_write_target(AgentKind::Antigravity, task, false).is_err());
        assert_eq!(
            check_write_target(AgentKind::Antigravity, task, true),
            Ok(())
        );
        for other in [
            "/x/conversations/c1.db",
            "/x/brain/c1/messages/m.json",
            "/x/brain/c1/task.md.metadata.json",
        ] {
            assert!(
                check_write_target(AgentKind::Antigravity, Path::new(other), true).is_err(),
                "{other}"
            );
        }
        // Even with the flag, the version gate still has the last word.
        assert!(
            may_write(
                AgentKind::Antigravity,
                Some("ide"),
                Some("2.5.5"),
                task,
                true
            )
            .is_err()
        );
        assert!(
            check_write_target(AgentKind::Codex, Path::new("/x/rollout-a.jsonl.zst"), true)
                .is_err()
        );
        assert_eq!(
            may_write(
                AgentKind::ClaudeCode,
                None,
                Some("2.1.285"),
                Path::new("/x/a.jsonl"),
                false
            ),
            Ok(())
        );
    }

    #[test]
    fn every_row_of_the_table_parses() {
        for k in KNOWN {
            let (lo, hi) = k.read;
            assert!(
                Version::parse(lo).is_some() && Version::parse(hi).is_some(),
                "{k:?}"
            );
            assert!(Version::parse(lo) <= Version::parse(hi), "{k:?}");
            if let Some((lo, hi)) = k.write {
                let v = Version::parse(lo).unwrap();
                assert!(
                    within(&v, k.read) && within(&Version::parse(hi).unwrap(), k.read),
                    "write outside read: {k:?}"
                );
            }
        }
    }
}
