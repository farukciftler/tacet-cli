//! RG-28: the three session readers over one corpus, per agent and per version.
//!
//! WHY A CORPUS AND NOT MORE UNIT TESTS. The readers' own tests each hold one
//! shape at a time against one reader. What the version table in
//! `sessions::version` needs is different: a place where "this version of
//! this agent parses with no unparsed line" is a fact the build checks, so a
//! read range cannot be widened by editing a number alone — a fixture for the
//! new end has to be added here and read cleanly. So the tests below tie the
//! corpus to the table in both directions: every measured version has a
//! directory, and every directory's files say the version the directory is
//! named after.
//!
//! WHAT THAT DOES NOT PROVE. ADR 0001 asks for a measurement ("a range is
//! widened by a measurement, never by an edit"); the build can only ask for a
//! fixture. It cannot tell a session captured from that version from a copy
//! of another fixture with the version string edited — and today four Claude
//! Code cases are exactly that (`VERSION_EDITED_COPIES`). Such a case shows
//! the gate and the reader accept that version's string, nothing about that
//! version's format; the measurement is the `measured` text in the table, and
//! it is checked in review, not here.
//!
//! WHAT EVERY CASE IS HELD TO. The WHOLE context the reader rebuilds —
//! role, kind, tool and text of every entry, in order — plus the title, the
//! plan, the number of lines stepped over and each warning the user would be
//! shown. A partial check ("the summary is in there somewhere") is what let
//! a reader that followed `logicalParentUuid` look correct: the right entries
//! were present, with the pre-compaction history in front of them.
//!
//! THE SHAPES are the ones the 9 Oct 2026 measurement counted on disk and the
//! R&D brief lists: a corrupt or torn line, a compaction whose
//! `logicalParentUuid` loops, a parent outside the file, timestamps going
//! backwards, a uuid written twice, sidechains, legacy and paginated Codex
//! rollouts, a `.zst` that must be refused, and Antigravity steps that repeat,
//! arrive out of order and trail the database. `the_corpus_holds_every_shape_
//! rg28_names` fails if a case carrying one of them is taken out.
//!
//! CAPTURED CASES. Six Claude Code cases are real transcripts, masked by the
//! anonymiser (`CAPTURED`); their text is its pseudo-words, and what they
//! are is the corpus README's to say, which a test below holds it to.
//!
//! Nothing here reads the user's real `~/.claude`, `~/.codex` or `~/.gemini`:
//! Claude and Codex read the fixture directories in place, Antigravity a
//! scratch copy (its lag notice is computed from file times, which a checkout
//! does not keep).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tacet_tools::sessions::antigravity::{
    self, AntigravityRead, AntigravitySource, TranscriptFile,
};
use tacet_tools::sessions::claude::{ClaudeRead, ClaudeSource};
use tacet_tools::sessions::codex::{COMPRESSED_TITLE, CodexSource, HistoryMode, RolloutFile};
use tacet_tools::sessions::model::parse_iso_utc;
use tacet_tools::sessions::secrets::Refusal;
use tacet_tools::sessions::version::{self, KNOWN};
use tacet_tools::sessions::{
    AgentKind, AgentSession, EntryKind, Role, SessionError, SessionSource, Status,
};

use AgentKind::{Antigravity as AG, ClaudeCode as CC, Codex as CX};
use EntryKind::{Summary, Text, Thinking, ToolCall, ToolResult};
use Role::{Assistant, System, Tool, User};

// ----------------------------------------------------------------- shapes --

/// One thing a session file does that a naive reader gets wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Shape {
    /// A version inside the measured read range, nothing odd in it.
    Clean,
    /// A version outside the measured range: read, with a warning.
    UnknownVersion,
    /// Not JSON, not an object, a chain entry without a uuid, a torn last line.
    CorruptLine,
    /// A `compact_boundary` and the summary after it.
    CompactBoundary,
    /// Two boundaries whose `logicalParentUuid`s name each other.
    LogicalParentCycle,
    /// `parentUuid` loops on the main chain.
    ParentLoop,
    /// The first entry's parent is in another file.
    OrphanParent,
    /// Timestamps that go backwards along the chain.
    OutOfOrderTimestamps,
    /// A uuid written twice: an abandoned branch re-appended after a rewind,
    /// and an entry re-appended with new content.
    DuplicateUuid,
    /// Subagent entries interleaved with, and after, the main chain.
    Sidechain,
    /// `history_mode: legacy`, no ordinals.
    LegacyRollout,
    /// `history_mode: paginated`, every line numbered.
    PaginatedRollout,
    /// Ordinals out of file order, one repeated, one missing, one line
    /// without, one at `u64::MAX`.
    OrdinalDisorder,
    /// A Codex `compacted` line.
    CompactedRollout,
    /// `.jsonl.zst`: listed, never opened.
    CompressedRollout,
    /// A `step_index` logged twice (RUNNING, then DONE).
    DuplicateStep,
    /// Steps logged out of index order, with a gap.
    OutOfOrderStep,
    /// The database written after the log: the newest steps are not in it.
    LaggingTranscript,
    /// Only `transcript.jsonl`, no `transcript_full.jsonl`.
    FallbackTranscript,
    /// A nested `task.md`.
    NestedTasks,
    /// A `--fork-session` copy: the origin's entries, uuids unchanged, under
    /// a new session id, in the same project directory as the origin.
    Fork,
}

use Shape::*;

/// What RG-28 and the R&D brief ask the corpus to hold, per agent. A case
/// carrying the shape must exist; which case is the corpus's business.
const REQUIRED: &[(AgentKind, Shape)] = &[
    (CC, Clean),
    (CC, UnknownVersion),
    (CC, CorruptLine),
    (CC, CompactBoundary),
    (CC, LogicalParentCycle),
    (CC, ParentLoop),
    (CC, OrphanParent),
    (CC, OutOfOrderTimestamps),
    (CC, DuplicateUuid),
    (CC, Sidechain),
    (CC, Fork),
    (CX, LegacyRollout),
    (CX, PaginatedRollout),
    (CX, CorruptLine),
    (CX, OutOfOrderTimestamps),
    (CX, OrdinalDisorder),
    (CX, CompactedRollout),
    (CX, CompressedRollout),
    (AG, Clean),
    (AG, CorruptLine),
    (AG, DuplicateStep),
    (AG, OutOfOrderStep),
    (AG, LaggingTranscript),
    (AG, FallbackTranscript),
    (AG, NestedTasks),
];

// ------------------------------------------------------------------ cases --

type Row = (Role, EntryKind, Option<&'static str>, &'static str);

/// What one reader returned, kept whole so a case's own check can look at
/// what `AgentSession` has no field for.
enum Read {
    Claude(Box<ClaudeRead>),
    Codex(Box<RolloutFile>, Box<AgentSession>),
    Antigravity(Box<AntigravityRead>),
}

impl Read {
    fn session(&self) -> &AgentSession {
        match self {
            Read::Claude(r) => &r.session,
            Read::Codex(_, s) => s,
            Read::Antigravity(r) => &r.session,
        }
    }

    fn skipped(&self) -> usize {
        match self {
            Read::Claude(r) => r.skipped_lines,
            Read::Codex(r, _) => r.rollout.skipped.len(),
            Read::Antigravity(r) => r.skipped_lines,
        }
    }

    /// What the user is told beside the transcript.
    fn warnings(&self) -> Vec<String> {
        match self {
            Read::Claude(r) => r.warnings.clone(),
            Read::Codex(r, _) => r.rollout.missing_prefix.iter().cloned().collect(),
            Read::Antigravity(r) => r.notices.clone(),
        }
    }

    fn claude(&self) -> &ClaudeRead {
        match self {
            Read::Claude(r) => r,
            _ => panic!("not a Claude read"),
        }
    }

    fn codex(&self) -> &RolloutFile {
        match self {
            Read::Codex(r, _) => r,
            _ => panic!("not a Codex read"),
        }
    }

    fn antigravity(&self) -> &AntigravityRead {
        match self {
            Read::Antigravity(r) => r,
            _ => panic!("not an Antigravity read"),
        }
    }
}

struct Case {
    agent: AgentKind,
    /// The directory under `corpus/<agent>/`, which is the version the files
    /// record.
    version: &'static str,
    id: &'static str,
    shapes: &'static [Shape],
    title: &'static str,
    /// `None`: the reader must refuse the file (a `.zst`).
    entries: Option<&'static [Row]>,
    plan: &'static [(&'static str, Status)],
    skipped: usize,
    /// One substring per warning, in order; the count must match too.
    warnings: &'static [&'static str],
    /// Antigravity only: seconds the database is written after the log
    /// (negative: before). `None`: no database file.
    db_after_log: Option<i64>,
    check: fn(&Read),
}

fn nothing_more(_: &Read) {}

fn at(iso: &str) -> i64 {
    parse_iso_utc(iso).unwrap_or_else(|| panic!("bad timestamp {iso}"))
}

const CLAUDE_CLEAN: &[Row] = &[
    (
        User,
        Text,
        None,
        "Add a --dry-run flag to the export command",
    ),
    (Assistant, Thinking, None, "The flag goes in cli.rs."),
    (
        Assistant,
        ToolCall,
        Some("Read"),
        r#"{"file_path":"/Users/dev/corpus/src/cli.rs"}"#,
    ),
    (Tool, ToolResult, Some("Read"), "fn main() {}"),
    (
        Assistant,
        Text,
        None,
        "Added --dry-run; nothing is written when it is set.",
    ),
];
const CLAUDE_CLEAN_TITLE: &str = "Export dry-run flag";

fn claude_clean_leaf(r: &Read) {
    assert_eq!(
        r.claude().leaf.as_deref(),
        Some("e0000006-0000-4000-8000-000000000000")
    );
    assert!(!r.claude().compacted);
}

fn compacted(r: &Read) {
    assert!(r.claude().compacted, "the context starts at a boundary");
}

fn compacted_at_the_second_boundary(r: &Read) {
    compacted(r);
    assert_eq!(
        r.claude().leaf.as_deref(),
        Some("e0000018-0000-4000-8000-000000000000")
    );
}

fn timestamps_do_not_order_the_chain(r: &Read) {
    let s = r.session();
    // MEASURED: 1,900 backward steps across 30 files. The chain decides the
    // order; the clock only bounds the session.
    assert_eq!(s.started, Some(at("2026-10-09T10:05:00Z")));
    assert_eq!(s.updated, Some(at("2026-10-09T10:05:09Z")));
    assert!(s.entries[1].ts < s.entries[0].ts, "{:?}", s.entries);
}

fn the_newest_branch_is_the_leaf(r: &Read) {
    assert_eq!(
        r.claude().leaf.as_deref(),
        Some("e0000006-0000-4000-8000-000000000000"),
        "a uuid re-appended from an abandoned branch is not the newest entry"
    );
}

fn the_sidechain_is_not_the_leaf(r: &Read) {
    assert_eq!(
        r.claude().leaf.as_deref(),
        Some("e0000004-0000-4000-8000-000000000000")
    );
}

fn legacy_in_file_order(r: &Read) {
    let f = r.codex();
    assert_eq!(f.rollout.history_mode, HistoryMode::Legacy);
    assert_eq!(f.rollout.last_ordinal, None);
    assert_eq!(
        f.rollout.cwd.as_deref(),
        Some(Path::new("/home/dev/corpus"))
    );
    assert!(!f.archived);
}

fn legacy_skips_by_line_and_bounds_by_clock(r: &Read) {
    legacy_in_file_order(r);
    let f = r.codex();
    assert_eq!(f.rollout.skipped, [3, 4, 5], "1-based line numbers");
    assert_eq!(f.rollout.started, Some(at("2026-10-05T10:00:00Z")));
    assert_eq!(f.rollout.updated, Some(at("2026-10-05T10:00:09Z")));
}

fn paginated_by_ordinal(r: &Read) {
    let f = r.codex();
    assert_eq!(f.rollout.history_mode, HistoryMode::Paginated);
    assert_eq!(f.rollout.skipped, [5], "the one line without an ordinal");
    assert_eq!(f.rollout.last_ordinal, Some(7));
    assert_eq!(f.rollout.ordinal_gaps, 2, "ordinals 5 and 6 never written");
}

fn paginated_compacted(r: &Read) {
    let f = r.codex();
    assert_eq!(f.rollout.history_mode, HistoryMode::Paginated);
    assert_eq!(f.rollout.last_ordinal, Some(5));
    assert_eq!(f.rollout.ordinal_gaps, 0);
}

/// Hostile, not measured: an ordinal of `u64::MAX` after ordinal 1. The
/// reader's gap count was `hi - lo + 1`, which panicked in a debug build
/// ("attempt to add with overflow") and, with no release profile set, wrapped
/// to 0 in a release one — reporting a file with 2^64-3 missing lines as
/// whole. The gaps are exact; the last ordinal is the one in the file, and
/// that there is no ordinal after it is the appender's to refuse.
fn an_ordinal_at_the_top_of_the_range(r: &Read) {
    let f = r.codex();
    assert_eq!(f.rollout.history_mode, HistoryMode::Paginated);
    assert_eq!(f.rollout.last_ordinal, Some(u64::MAX));
    assert_eq!(
        f.rollout.ordinal_gaps,
        u64::MAX - 2,
        "2^64 ordinals from 0 to u64::MAX, three of them written"
    );
}

fn the_log_trails_the_database(r: &Read) {
    let a = r.antigravity();
    assert_eq!(a.transcript, TranscriptFile::Full);
    assert_eq!(a.lag_seconds, Some(90));
    assert!(!a.session.live, "no -wal file and the last step is DONE");
    assert_eq!(
        a.session.cwd.as_deref(),
        Some(Path::new("/Users/dev/corpus"))
    );
    let depths: Vec<usize> = a.tasks.iter().map(|t| t.depth).collect();
    assert_eq!(depths, [0, 0, 1, 1, 0], "four-space indents are one level");
}

fn a_report_beside_the_log(r: &Read) {
    let a = r.antigravity();
    assert_eq!(a.lag_seconds, None);
    // `list_dir`'s directory is one the agent looked into, not where it was
    // opened; only `run_command`'s `Cwd` names the workspace.
    assert_eq!(a.session.cwd, None);
    let reports: Vec<(&str, Option<&str>, Option<bool>)> = a
        .artifacts
        .iter()
        .map(|x| (x.name.as_str(), x.summary.as_deref(), x.user_facing))
        .collect();
    assert_eq!(
        reports,
        [(
            "walkthrough.md",
            Some("Module layout explained"),
            Some(true)
        )]
    );
}

fn the_log_is_ahead_of_the_database(r: &Read) {
    let a = r.antigravity();
    assert_eq!(a.transcript, TranscriptFile::Full);
    assert_eq!(a.lag_seconds, None);
}

fn the_short_log_still_running(r: &Read) {
    let a = r.antigravity();
    assert_eq!(a.transcript, TranscriptFile::Fallback);
    assert!(a.session.live, "its last step is still RUNNING");
}

const AG_METADATA: &str = "The current local time is: 2026-10-08T17:00:00+03:00.";

// --------------------------------------------- captured from Claude Code --
// REAL FILES, MASKED. Each is a transcript Claude Code 2.1.285 wrote on the
// measuring Mac on 11 Oct 2026 during a throwaway run made for this project
// (no prompt, path or file of the user's), passed through `corpus_capture
// --into` and read before it was committed (README, "What is real"). The
// text below is therefore the anonymiser's pseudo-words: the same word is the
// same pseudo-word within one run, which is why `XEDMND-7` is the codeword in
// the note, in the answer and in the fork.

/// The 11 Oct round trip (`scripts/session-roundtrip.sh`), session a: E2's
/// assistant-role entry Tacet wrote, then E4's note chained to a parent that
/// does not exist — the read shows what Claude Code then showed, the note and
/// nothing before it.
const RT_A: &str = "8efd26aa-bf4f-4660-9712-f11cb03b9613";
/// Session b: E3's note chained to the real leaf, and the answer crediting
/// Tacet with it.
const RT_B: &str = "b35773e3-c3e3-4b5a-94f3-c2d2a449361d";
/// E5: `--resume b --fork-session`.
const RT_FORK: &str = "59905b9b-3af4-4285-b3ee-d573d9b01228";
/// RG-21's runs 1 and 2: a startup, then a resume that ran Tacet's
/// SessionStart hook (its `additionalContext` is an attachment line, which
/// the reader leaves out of the context).
const HOOK_RESUMED: &str = "c46e3daf-1074-4fbc-956c-3a2638be85a1";
/// RG-21's run 3: `--resume --fork-session` of the session above, which ran
/// no SessionStart hook.
const HOOK_FORK: &str = "a75f24d5-c79c-42c4-8417-e7ca4710192b";
/// RG-21's run 4: a startup under a record Tacet had already written.
const HOOK_STARTUP: &str = "2522b459-b16f-4938-b7ca-b01e86dcfc95";

/// The captured cases, and the forks among them with the session each was
/// forked from.
const CAPTURED: &[&str] = &[RT_A, RT_B, RT_FORK, HOOK_RESUMED, HOOK_FORK, HOOK_STARTUP];
const CAPTURED_FORKS: &[(&str, &str)] = &[(RT_B, RT_FORK), (HOOK_RESUMED, HOOK_FORK)];

const RT_A_TITLE: &str = "Ajkx bwote dscgqtas fdswwzeuj sf gqqz htwlboobnnjx.";
const RT_A_ROWS: &[Row] = &[
    (
        User,
        Text,
        None,
        "[Note from Tacet, a local assistant, not from the user] Dbaozwht: WUAKR-9.",
    ),
    (Assistant, Text, None, "Rc sppgrska jxmyohspx."),
    (
        User,
        Text,
        None,
        "Ajkx bwote dscgqtas fdswwzeuj sf gqqz htwlboobnnjx.",
    ),
    (
        Assistant,
        Text,
        None,
        "Gsh cpah dscgqtas sf gqqz htwlboobnnjx gr **WUAKR-9**. Yl urkw sf nan qldkfif jgve uxdjifr ek kxiss tcwy Mxkeq, a rldyc gbrmdvzkp.",
    ),
];
const RT_B_TITLE: &str = "Knt scvn a jgve fnexb I etq tajx, zze emry etq nan dscgqtas?";
const RT_B_ROWS: &[Row] = &[
    (User, Text, None, "Pamss bkja tfiegmq: XSCHB"),
    (Assistant, Text, None, "XSCHB"),
    (
        User,
        Text,
        None,
        "[Note from Tacet, a local assistant, not from the user] Msdcw adc rtgo tajx Mxkeq tiayl nan fwzuigwl cpdrt piukqlxc jee (ubkqc U85). Dbaozwht: XEDMND-7.",
    ),
    (Assistant, Text, None, "Rc sppgrska jxmyohspx."),
    (
        User,
        Text,
        None,
        "Knt scvn a jgve fnexb I etq tajx, zze emry etq nan dscgqtas?",
    ),
    (
        Assistant,
        Text,
        None,
        "Gsh jgve etq tcwy **Mxkeq**, a rldyc gbrmdvzkp. Yl owhn Mxkeq sir tiayl nan fwzuigwl cpdrt piukqlxc jee (ubkqc U85) fnexb adc rtgo tajx, zze nn zdzj nan dscgqtas **XEDMND-7**.",
    ),
];
const RT_FORK_TITLE: &str = "Pamss bkja tfiegmq: XSCHB";
const RT_FORK_ROWS: &[Row] = &[
    (User, Text, None, "Pamss bkja tfiegmq: XSCHB"),
    (Assistant, Text, None, "XSCHB"),
    (
        User,
        Text,
        None,
        "[Note from Tacet, a local assistant, not from the user] Msdcw adc rtgo tajx Mxkeq tiayl nan fwzuigwl cpdrt piukqlxc jee (ubkqc U85). Dbaozwht: XEDMND-7.",
    ),
    (Assistant, Text, None, "Rc sppgrska jxmyohspx."),
    (
        User,
        Text,
        None,
        "Knt scvn a jgve fnexb I etq tajx, zze emry etq nan dscgqtas?",
    ),
    (
        Assistant,
        Text,
        None,
        "Gsh jgve etq tcwy **Mxkeq**, a rldyc gbrmdvzkp. Yl owhn Mxkeq sir tiayl nan fwzuigwl cpdrt piukqlxc jee (ubkqc U85) fnexb adc rtgo tajx, zze nn zdzj nan dscgqtas **XEDMND-7**.",
    ),
    (User, Text, None, "Zizd nan dscgqtas tcwy Mxkeq's jgve."),
    (Assistant, Text, None, "**XEDMND-7**"),
];
const HOOK_RESUMED_TITLE: &str = "Fyovru pjvh rfpk xdga kd yeuqunb ql nkcy drcdisbplsxx, ramdrxu qttun llq kkgii: kd yefjn a cura ztxc ls ektwhskn sxejtv…";
const HOOK_RESUMED_ROWS: &[Row] = &[
    (User, Text, None, "Uorpo otnr zenovyd: VYDTE"),
    (Assistant, Text, None, "VYDTE"),
    (
        User,
        Text,
        None,
        "Fyovru pjvh rfpk xdga kd yeuqunb ql nkcy drcdisbplsxx, ramdrxu qttun llq kkgii: kd yefjn a cura ztxc ls ektwhskn sxejtv tuh nkcy tylwbrc ql lvqk obsrdvj? Mb cfv, uwbtk vqr gwtdi htpli oy cura 2 vbx rqd ihb vzkh vqr sxejtv. Mb gyr, mioho JXSP.",
    ),
    (
        Assistant,
        Text,
        None,
        "Tqo. Lwxg 2 kd otxhll **\"Emwnsw vqr zmqung xkhfzx cd OMZUBO-588\"**, vbx ez's klvsvi ql ektwhskn.\n\nKqy sxejtv uev vzkh ny **Tczsu**, a qjdlt jhctacrox st nkcy kozemyi. Wt hyfn ql qafeqjv nkcy tylwbrc's slcwnol kkji dc a akgeqce sxejtv, uiwy wdixzvq 9220-68-93X88:89:52Y ny riwkj.",
    ),
];
const HOOK_FORK_TITLE: &str = "Uorpo otnr zenovyd: VYDTE";
const HOOK_FORK_ROWS: &[Row] = &[
    (User, Text, None, "Uorpo otnr zenovyd: VYDTE"),
    (Assistant, Text, None, "VYDTE"),
    (
        User,
        Text,
        None,
        "Fyovru pjvh rfpk xdga kd yeuqunb ql nkcy drcdisbplsxx, ramdrxu qttun llq kkgii: kd yefjn a cura ztxc ls ektwhskn sxejtv tuh nkcy tylwbrc ql lvqk obsrdvj? Mb cfv, uwbtk vqr gwtdi htpli oy cura 2 vbx rqd ihb vzkh vqr sxejtv. Mb gyr, mioho JXSP.",
    ),
    (
        Assistant,
        Text,
        None,
        "Tqo. Lwxg 2 kd otxhll **\"Emwnsw vqr zmqung xkhfzx cd OMZUBO-588\"**, vbx ez's klvsvi ql ektwhskn.\n\nKqy sxejtv uev vzkh ny **Tczsu**, a qjdlt jhctacrox st nkcy kozemyi. Wt hyfn ql qafeqjv nkcy tylwbrc's slcwnol kkji dc a akgeqce sxejtv, uiwy wdixzvq 9220-68-93X88:89:52Y ny riwkj.",
    ),
    (
        User,
        Text,
        None,
        "Fyovru pjvh rfpk xdga kd yeuqunb ql nkcy drcdisbplsxx, ramdrxu qttun llq kkgii: tog zzyf lzqbt ukit vqr ektwhskn sxejtv tuh nkcy tylwbrc ztxc ugo, vbx xdga kd vqr gwtdi htpli oy cura 3? Mb yefjn kd oi cura 3, mioho QE QUTL 3.",
    ),
    (
        Assistant,
        Text,
        None,
        "Kqy sxejtv qjrpu 2 lzqbt. QE QUTL 3.",
    ),
];
const HOOK_STARTUP_TITLE: &str = "Fyovru pjvh rfpk xdga kd yeuqunb ql nkcy drcdisbplsxx, ramdrxu qttun llq kkgii: kd yefjn a cura ztxc ls ektwhskn sxejtv…";
const HOOK_STARTUP_ROWS: &[Row] = &[
    (
        User,
        Text,
        None,
        "Fyovru pjvh rfpk xdga kd yeuqunb ql nkcy drcdisbplsxx, ramdrxu qttun llq kkgii: kd yefjn a cura ztxc ls ektwhskn sxejtv tuh nkcy tylwbrc ql lvqk obsrdvj? Mb cfv, uwbtk vqr gwtdi htpli oy cura 1, pci jogwse, vbx ihb vzkh vqr sxejtv. Mb gyr, mioho JXSP.",
    ),
    (
        Assistant,
        Text,
        None,
        "Tqo. A CpzfaifJybfc kkji onczaq a akgeqce sxejtv tuh nkcy tylwbrc.\n\n- **Lwxg 1 htpli:** \"Meos vqr wlxbzztyb cd JCJVPZ-947\"\n- **Comqnr:** blocked\n- **Dyer ny:** Tczsu, a qjdlt jhctacrox st nkcy kozemyi. Kqy sxejtv kkch ez uev \"Czwfomb 9220-68-93X88:07:41J ny riwkj\".",
    ),
];

fn captured_leaf(r: &Read, leaf: &str) {
    assert_eq!(r.claude().leaf.as_deref(), Some(leaf));
    assert!(!r.claude().compacted);
}

fn rt_a_leaf(r: &Read) {
    captured_leaf(r, "06128eff-bcbc-4288-981f-dfd3caf44c2e");
}

fn rt_b_leaf(r: &Read) {
    captured_leaf(r, "75df7e35-67ae-4460-9586-fb7c9128b4ba");
}

fn rt_fork_leaf(r: &Read) {
    captured_leaf(r, "1813c99d-3709-4f40-81aa-4507b6150aab");
}

fn hook_resumed_leaf(r: &Read) {
    captured_leaf(r, "d1ef5f3d-5fb7-4e53-b7e2-41757fc59212");
}

fn hook_fork_leaf(r: &Read) {
    captured_leaf(r, "7585ab46-470a-4b77-af84-e063b379a3e5");
}

fn hook_startup_leaf(r: &Read) {
    captured_leaf(r, "012043c3-a83d-4a58-8b32-cc921d343cce");
}

const CASES: &[Case] = &[
    // ---------------------------------------------------- Claude Code --
    // The two ends of the measured read range, the one measured write
    // version, and one version past the range.
    Case {
        agent: CC,
        version: "2.1.263",
        id: "c0000263-0000-4000-8000-000000000001",
        shapes: &[Clean],
        title: CLAUDE_CLEAN_TITLE,
        entries: Some(CLAUDE_CLEAN),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: claude_clean_leaf,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: "c0000285-0000-4000-8000-000000000001",
        shapes: &[Clean],
        title: CLAUDE_CLEAN_TITLE,
        entries: Some(CLAUDE_CLEAN),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: claude_clean_leaf,
    },
    Case {
        agent: CC,
        version: "2.1.293",
        id: "c0000293-0000-4000-8000-000000000001",
        shapes: &[Clean],
        title: CLAUDE_CLEAN_TITLE,
        entries: Some(CLAUDE_CLEAN),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: claude_clean_leaf,
    },
    Case {
        agent: CC,
        version: "2.2.0",
        id: "c0000220-0000-4000-8000-000000000001",
        shapes: &[UnknownVersion],
        title: CLAUDE_CLEAN_TITLE,
        entries: Some(CLAUDE_CLEAN),
        plan: &[],
        skipped: 0,
        warnings: &["has not been read by this Tacet before"],
        db_after_log: None,
        check: claude_clean_leaf,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: "c0000285-0000-4000-8000-000000000011",
        shapes: &[CorruptLine],
        title: "Rename the config key",
        entries: Some(&[
            (User, Text, None, "Rename the config key"),
            (Assistant, Text, None, "Renamed it."),
            (User, Text, None, "Now update the docs"),
            (Assistant, Text, None, "Docs updated."),
        ]),
        plan: &[],
        // Not JSON; a JSON array; a user entry with no uuid; the torn last line.
        skipped: 4,
        warnings: &["4 line(s) of the transcript were not understood"],
        db_after_log: None,
        check: nothing_more,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: "c0000285-0000-4000-8000-000000000012",
        shapes: &[CompactBoundary, LogicalParentCycle],
        title: "Write the release note",
        entries: Some(&[
            (
                System,
                Summary,
                None,
                "Summary: both halves are done; only the release note is left.",
            ),
            (User, Text, None, "Write the release note"),
            (Assistant, Text, None, "Release note written."),
        ]),
        plan: &[],
        skipped: 0,
        // No loop warning: `logicalParentUuid` is never followed, so its loop
        // is never met.
        warnings: &[],
        db_after_log: None,
        check: compacted_at_the_second_boundary,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: "c0000285-0000-4000-8000-000000000013",
        shapes: &[CompactBoundary],
        title: "Add tests for it",
        entries: Some(&[
            (System, Summary, None, "Summary: the parser was rewritten."),
            (User, Text, None, "Add tests for it"),
            (Assistant, Text, None, "Tests added."),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: compacted,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: "c0000285-0000-4000-8000-000000000014",
        shapes: &[OrphanParent],
        title: "Pick up where the other file stopped",
        entries: Some(&[
            (User, Text, None, "Pick up where the other file stopped"),
            (Assistant, Text, None, "Picked up."),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &["f0000000-0000-4000-8000-00000000dead, which is not in this file"],
        db_after_log: None,
        check: nothing_more,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: "c0000285-0000-4000-8000-000000000015",
        shapes: &[OutOfOrderTimestamps],
        title: "First question",
        entries: Some(&[
            (User, Text, None, "First question"),
            (Assistant, Text, None, "First answer"),
            (User, Text, None, "Second question"),
            (Assistant, Text, None, "Second answer"),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: timestamps_do_not_order_the_chain,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: "c0000285-0000-4000-8000-000000000016",
        shapes: &[DuplicateUuid],
        title: "Write the migration",
        entries: Some(&[
            (User, Text, None, "Write the migration"),
            // Last write wins for content...
            (Assistant, Text, None, "Plan: two steps (revised)"),
            // ...and first write for position: the re-appended abandoned
            // answer is not where the session stopped.
            (User, Text, None, "second attempt"),
            (Assistant, Text, None, "kept answer"),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: the_newest_branch_is_the_leaf,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: "c0000285-0000-4000-8000-000000000017",
        shapes: &[Sidechain],
        title: "Survey the repository",
        entries: Some(&[
            (User, Text, None, "Survey the repository"),
            (
                Assistant,
                ToolCall,
                Some("Task"),
                r#"{"description":"count crates"}"#,
            ),
            (Tool, ToolResult, Some("Task"), "12 crates"),
            (Assistant, Text, None, "The repository has 12 crates."),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: the_sidechain_is_not_the_leaf,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: "c0000285-0000-4000-8000-000000000018",
        shapes: &[ParentLoop],
        title: "loop start",
        entries: Some(&[
            (User, Text, None, "loop start"),
            (Assistant, Text, None, "loop middle"),
            (User, Text, None, "loop end"),
            (Assistant, Text, None, "after the loop"),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &["parentUuid loops back to e0000003-0000-4000-8000-000000000000"],
        db_after_log: None,
        check: nothing_more,
    },
    // Captured (see `CAPTURED`). No `ai-title` in a `claude -p` session, so
    // each is named by its `last-prompt` line; a fork's carries its origin's
    // first prompt, not the one asked in the fork (as Claude Code wrote it).
    Case {
        agent: CC,
        version: "2.1.285",
        id: RT_A,
        shapes: &[OrphanParent],
        title: RT_A_TITLE,
        entries: Some(RT_A_ROWS),
        plan: &[],
        skipped: 0,
        warnings: &["00000000-0000-4000-8000-000000000000, which is not in this file"],
        db_after_log: None,
        check: rt_a_leaf,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: RT_B,
        shapes: &[Clean],
        title: RT_B_TITLE,
        entries: Some(RT_B_ROWS),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: rt_b_leaf,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: RT_FORK,
        shapes: &[Clean, Fork],
        title: RT_FORK_TITLE,
        entries: Some(RT_FORK_ROWS),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: rt_fork_leaf,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: HOOK_RESUMED,
        shapes: &[Clean],
        title: HOOK_RESUMED_TITLE,
        entries: Some(HOOK_RESUMED_ROWS),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: hook_resumed_leaf,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: HOOK_FORK,
        shapes: &[Clean, Fork],
        title: HOOK_FORK_TITLE,
        entries: Some(HOOK_FORK_ROWS),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: hook_fork_leaf,
    },
    Case {
        agent: CC,
        version: "2.1.285",
        id: HOOK_STARTUP,
        shapes: &[Clean],
        title: HOOK_STARTUP_TITLE,
        entries: Some(HOOK_STARTUP_ROWS),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: hook_startup_leaf,
    },
    // ----------------------------------------------------------- Codex --
    // Written from source (openai/codex @ a06545b), not collected: Codex was
    // not installed on the machine the corpus was made on.
    Case {
        agent: CX,
        version: "0.46.0",
        id: "0199e000-0046-7000-8000-000000000001",
        shapes: &[LegacyRollout],
        title: "Fix the off-by-one in pager.rs",
        entries: Some(&[
            (
                System,
                Text,
                None,
                "<environment_context>\n  <cwd>/home/dev/corpus</cwd>\n</environment_context>",
            ),
            (User, Text, None, "Fix the off-by-one in pager.rs"),
            (
                Assistant,
                ToolCall,
                Some("update_plan"),
                r#"{"plan":[{"step":"Find the bug","status":"in_progress"},{"step":"Add a test","status":"pending"}]}"#,
            ),
            (Tool, ToolResult, Some("update_plan"), "Plan updated"),
            (
                Assistant,
                ToolCall,
                Some("shell"),
                r#"{"command":["cargo","test"]}"#,
            ),
            (Tool, ToolResult, Some("shell"), "test result: ok"),
            (
                Assistant,
                ToolCall,
                Some("update_plan"),
                r#"{"plan":[{"step":"Find the bug","status":"completed"},{"step":"Add a test","status":"completed"}]}"#,
            ),
            (Tool, ToolResult, Some("update_plan"), "Plan updated"),
            (
                Assistant,
                Text,
                None,
                "Fixed: the loop now stops at len - 1.",
            ),
        ]),
        plan: &[
            ("Find the bug", Status::Completed),
            ("Add a test", Status::Completed),
        ],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: legacy_in_file_order,
    },
    Case {
        agent: CX,
        version: "0.46.0",
        id: "0199e000-0046-7000-8000-000000000002",
        shapes: &[LegacyRollout, CorruptLine, OutOfOrderTimestamps],
        title: "Bump the version",
        entries: Some(&[
            (User, Text, None, "Bump the version"),
            // Logged three seconds BEFORE the question it answers.
            (Assistant, Text, None, "Bumped to 1.2.0."),
        ]),
        plan: &[],
        // A torn line, a line with no payload, a line with no type.
        skipped: 3,
        warnings: &[],
        db_after_log: None,
        check: legacy_skips_by_line_and_bounds_by_clock,
    },
    Case {
        agent: CX,
        version: "0.46.0",
        id: "0199e000-0046-7000-8000-000000000003",
        shapes: &[CompressedRollout],
        title: COMPRESSED_TITLE,
        entries: None,
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: nothing_more,
    },
    Case {
        agent: CX,
        version: "0.47.0",
        id: "0199e000-0047-7000-8000-000000000001",
        shapes: &[PaginatedRollout, OrdinalDisorder],
        title: "Split the parser module",
        entries: Some(&[
            (User, Text, None, "Split the parser module"),
            (Assistant, Text, None, "Plan first, then code."),
            // Ordinal 4 written twice: the later line is the one kept.
            (
                Assistant,
                Text,
                None,
                "Split into lexer.rs, parser.rs and ast.rs.",
            ),
            (User, Text, None, "Thanks"),
        ]),
        plan: &[],
        skipped: 1,
        warnings: &[],
        db_after_log: None,
        check: paginated_by_ordinal,
    },
    Case {
        agent: CX,
        version: "0.47.0",
        id: "0199e000-0047-7000-8000-000000000002",
        shapes: &[PaginatedRollout, CompactedRollout],
        title: "Port the cache to the new API",
        entries: Some(&[
            (User, Text, None, "Port the cache to the new API"),
            (Assistant, Text, None, "Step one is done."),
            // A remote compaction: `message` empty, the kept history in
            // `replacement_history`.
            (
                System,
                Summary,
                None,
                "Port the cache to the new API\nSummary: step one is done; step two is the eviction policy.",
            ),
            (User, Text, None, "continue"),
            (Assistant, Text, None, "Eviction policy ported."),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: paginated_compacted,
    },
    Case {
        agent: CX,
        version: "0.47.0",
        id: "0199e000-0047-7000-8000-000000000003",
        shapes: &[PaginatedRollout, OrdinalDisorder],
        title: "Check the lockfile",
        // The absurd ordinal is still the last line, so it is still read: a
        // number no writer could reach says the file was edited, not that its
        // text is wrong.
        entries: Some(&[
            (User, Text, None, "Check the lockfile"),
            (Assistant, Text, None, "The lockfile is current."),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: None,
        check: an_ordinal_at_the_top_of_the_range,
    },
    // ----------------------------------------------------- Antigravity --
    Case {
        agent: AG,
        version: "2.5.5",
        id: "a0000255-0000-4000-8000-000000000002",
        shapes: &[Clean],
        title: "Describe the module layout",
        entries: Some(&[
            (User, Text, None, "Describe the module layout"),
            (System, Text, None, AG_METADATA),
            (
                Tool,
                ToolResult,
                Some("LIST_DIRECTORY"),
                "src/: lib.rs, io.rs",
            ),
            (
                Assistant,
                ToolCall,
                Some("list_dir"),
                r#"{"DirectoryPath":"/Users/dev/corpus/src"}"#,
            ),
            (Assistant, Thinking, None, "Two files only."),
            (Assistant, Text, None, "lib.rs re-exports io.rs."),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: Some(-30),
        check: a_report_beside_the_log,
    },
    Case {
        agent: AG,
        version: "2.5.5",
        id: "a0000255-0000-4000-8000-000000000001",
        shapes: &[
            DuplicateStep,
            OutOfOrderStep,
            CorruptLine,
            LaggingTranscript,
            NestedTasks,
        ],
        title: "Add retry to the uploader",
        entries: Some(&[
            (User, Text, None, "Add retry to the uploader"),
            (System, Text, None, AG_METADATA),
            (Assistant, Thinking, None, "Retry belongs in upload.rs."),
            (Assistant, Text, None, "I will add a bounded retry."),
            // Step 2 was logged RUNNING (no content) and then DONE.
            (
                Tool,
                ToolResult,
                Some("RUN_COMMAND"),
                "test result: ok. 4 passed",
            ),
            (
                Assistant,
                ToolCall,
                Some("run_command"),
                r#"{"CommandLine":"cargo test","Cwd":"/Users/dev/corpus"}"#,
            ),
            (Tool, ToolResult, Some("CODE_ACTION"), "Edited upload.rs"),
            (Assistant, Text, None, "Retry added with three attempts."),
        ]),
        plan: &[
            ("Find where uploads fail", Status::Completed),
            ("Add retry", Status::InProgress),
            ("Bounded attempts", Status::Completed),
            ("Backoff", Status::Pending),
            ("Document it", Status::Pending),
        ],
        // Step 5, torn mid-line while it was being written.
        skipped: 1,
        warnings: &[
            "written 90 s after this transcript",
            "1 line(s) of the transcript were not readable steps",
        ],
        // MEASURED: the IDE's log trailed its database by more than 60 s.
        db_after_log: Some(90),
        check: the_log_trails_the_database,
    },
    Case {
        agent: AG,
        version: "1.1.10",
        id: "a0000110-0000-4000-8000-000000000001",
        shapes: &[Clean, DuplicateStep, OutOfOrderStep],
        title: "Explain the build failure",
        entries: Some(&[
            (User, Text, None, "Explain the build failure"),
            (System, Text, None, AG_METADATA),
            (Tool, ToolResult, Some("VIEW_FILE"), "build.rs: 40 lines"),
            // Step 3 failed (ERROR, no content) and was retried; step 2 never
            // reached the log.
            (
                Assistant,
                Text,
                None,
                "The build script reads a variable that is unset.",
            ),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &[],
        db_after_log: Some(-600),
        check: the_log_is_ahead_of_the_database,
    },
    Case {
        agent: AG,
        version: "1.1.10",
        id: "a0000110-0000-4000-8000-000000000002",
        shapes: &[FallbackTranscript],
        title: "Tidy the imports",
        entries: Some(&[
            (User, Text, None, "Tidy the imports"),
            (System, Text, None, AG_METADATA),
            (Assistant, Text, None, "Tidying."),
        ]),
        plan: &[],
        skipped: 0,
        warnings: &["was not there, so this was read from transcript.jsonl"],
        db_after_log: None,
        check: the_short_log_still_running,
    },
];

// ---------------------------------------------------------------- helpers --

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions/corpus")
}

/// `corpus/<agent>/<version>`. Joined from the slug: a string literal whose
/// last component is an agent CLI's name reads to the network-monopoly scan
/// as a program being run.
fn version_dir(agent: AgentKind, version: &str) -> PathBuf {
    corpus().join(agent.slug()).join(version)
}

/// The Antigravity data root under a version directory (`antigravity-ide`,
/// `antigravity-cli`): the one directory there.
fn antigravity_root_name(version: &str) -> String {
    let dir = version_dir(AG, version);
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "{}: {names:?}", dir.display());
    names.into_iter().next().unwrap()
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn touch(path: &Path, at: i64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(UNIX_EPOCH + Duration::from_secs(at as u64))
        .unwrap();
}

/// A scratch home holding every Antigravity version's root at
/// `<home>/.gemini/<root>`, with the file times each case asks for. A
/// checkout leaves file times arbitrary, and the lag notice is made of them.
struct Gemini {
    home: PathBuf,
}

impl Gemini {
    fn new(tag: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "tacet-corpus-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&home);
        let log_time = at("2026-10-09T09:00:00Z");
        for case in CASES.iter().filter(|c| c.agent == AG) {
            let name = antigravity_root_name(case.version);
            let root = home.join(".gemini").join(&name);
            if !root.exists() {
                copy_tree(&version_dir(AG, case.version).join(&name), &root);
            }
        }
        let g = Gemini { home };
        for case in CASES.iter().filter(|c| c.agent == AG) {
            let root = g.root(case.version);
            let logs = root
                .join("brain")
                .join(case.id)
                .join(".system_generated/logs");
            for log in ["transcript_full.jsonl", "transcript.jsonl"] {
                if logs.join(log).exists() {
                    touch(&logs.join(log), log_time);
                }
            }
            let db = root.join("conversations").join(format!("{}.db", case.id));
            match case.db_after_log {
                Some(after) => touch(&db, log_time + after),
                None => assert!(
                    !db.exists(),
                    "{}: a database the case does not expect",
                    case.id
                ),
            }
        }
        g
    }

    fn root(&self, version: &str) -> PathBuf {
        self.home
            .join(".gemini")
            .join(antigravity_root_name(version))
    }
}

impl Drop for Gemini {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn dead(_: u32) -> bool {
    false
}

/// The case's root as its reader is pointed at it.
fn root_of(case: &Case, gemini: &Gemini) -> PathBuf {
    match case.agent {
        AG => gemini.root(case.version),
        _ => version_dir(case.agent, case.version),
    }
}

fn read(case: &Case, gemini: &Gemini) -> Result<Read, SessionError> {
    let root = root_of(case, gemini);
    match case.agent {
        CC => ClaudeSource::with_probe(dead)
            .read_report(&root, case.id)
            .map(|r| Read::Claude(Box::new(r))),
        CX => {
            let file = CodexSource.read_rollout(&root, case.id)?;
            let session = CodexSource.read(&root, case.id)?;
            Ok(Read::Codex(Box::new(file), Box::new(session)))
        }
        AG => AntigravitySource
            .read_full(&root, case.id)
            .map(|r| Read::Antigravity(Box::new(r))),
    }
}

fn source(agent: AgentKind) -> Box<dyn SessionSource> {
    match agent {
        CC => Box::new(ClaudeSource::with_probe(dead)),
        CX => Box::new(CodexSource),
        AG => Box::new(AntigravitySource),
    }
}

type OwnedRow = (Role, EntryKind, Option<String>, String);

fn shape(read: &Read) -> Vec<OwnedRow> {
    read.session()
        .entries
        .iter()
        .map(|e| (e.role, e.kind, e.tool.clone(), e.text.clone()))
        .collect()
}

fn owned(rows: &[Row]) -> Vec<OwnedRow> {
    rows.iter()
        .map(|(role, kind, tool, text)| (*role, *kind, tool.map(str::to_string), text.to_string()))
        .collect()
}

// ------------------------------------------------------------------ tests --

/// Every case through its reader, held to the whole context it rebuilds.
#[test]
fn every_case_reads_as_the_corpus_says() {
    let gemini = Gemini::new("read");
    for case in CASES {
        let id = case.id;
        let result = read(case, &gemini);
        let Some(want) = case.entries else {
            match result {
                Err(SessionError::Refused(Refusal::Compressed { .. })) => continue,
                Err(e) => panic!("{id}: refused for the wrong reason: {e}"),
                Ok(_) => panic!("{id}: a compressed rollout was read"),
            }
        };
        let r = result.unwrap_or_else(|e| panic!("{id}: {e}"));
        let s = r.session();
        assert_eq!(s.agent, case.agent, "{id}");
        assert_eq!(s.id, id);
        assert_eq!(shape(&r), owned(want), "{id}: the rebuilt context");
        assert_eq!(s.title, case.title, "{id}: title");
        let plan: Vec<(&str, Status)> = s
            .plan
            .iter()
            .map(|p| (p.title.as_str(), p.status))
            .collect();
        assert_eq!(plan, case.plan, "{id}: plan");
        assert_eq!(r.skipped(), case.skipped, "{id}: lines stepped over");
        let warnings = r.warnings();
        assert_eq!(
            warnings.len(),
            case.warnings.len(),
            "{id}: warnings {warnings:#?}"
        );
        for (got, want) in warnings.iter().zip(case.warnings) {
            assert!(got.contains(want), "{id}: {got:?} does not say {want:?}");
        }
        (case.check)(&r);
    }
}

/// THE LISTING AND THE READ NAME A SESSION THE SAME WAY. Each listing is
/// built from a head or a tail of the file and each read from all of it; a
/// title that differs between the two is the user picking one session and
/// being shown another's name.
#[test]
fn every_listing_names_exactly_the_corpus_cases_with_the_read_title() {
    let gemini = Gemini::new("list");
    let mut roots: BTreeMap<PathBuf, (AgentKind, Vec<&Case>)> = BTreeMap::new();
    for case in CASES {
        roots
            .entry(root_of(case, &gemini))
            .or_insert_with(|| (case.agent, Vec::new()))
            .1
            .push(case);
    }
    for (root, (agent, cases)) in roots {
        let list = source(agent).discover(&root);
        let listed: BTreeMap<&str, &str> = list
            .iter()
            .map(|s| (s.id.as_str(), s.title.as_str()))
            .collect();
        let want: BTreeMap<&str, &str> = cases.iter().map(|c| (c.id, c.title)).collect();
        assert_eq!(listed, want, "{}", root.display());
        assert!(list.iter().all(|s| s.agent == agent));
    }
}

#[test]
fn the_corpus_holds_every_shape_rg28_names() {
    let held: BTreeSet<(String, Shape)> = CASES
        .iter()
        .flat_map(|c| {
            c.shapes
                .iter()
                .map(move |s| (c.agent.slug().to_string(), *s))
        })
        .collect();
    let missing: Vec<_> = REQUIRED
        .iter()
        .filter(|(agent, shape)| !held.contains(&(agent.slug().to_string(), *shape)))
        .collect();
    assert!(
        missing.is_empty(),
        "no corpus case carries {missing:?}; a reader regression on that shape \
         would now pass the build"
    );
}

/// A FILE NO CASE NAMES IS A FIXTURE NOTHING CHECKS. Every file under the
/// corpus sits in its case's `<agent>/<version>/` and carries the case's id in
/// its path; the README is the one exception.
#[test]
fn every_file_in_the_corpus_belongs_to_a_case() {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), out);
            } else {
                out.push(entry.path());
            }
        }
    }
    let mut files = Vec::new();
    walk(&corpus(), &mut files);
    assert!(files.len() > CASES.len(), "the corpus was not found");
    let orphans: Vec<String> = files
        .iter()
        .filter(|f| *f != &corpus().join("README.md"))
        .filter(|f| {
            !CASES.iter().any(|c| {
                f.starts_with(version_dir(c.agent, c.version)) && f.to_string_lossy().contains(c.id)
            })
        })
        .map(|f| f.display().to_string())
        .collect();
    assert!(orphans.is_empty(), "files no case checks: {orphans:#?}");
}

/// THE TABLE AND THE CORPUS AGREE, BOTH WAYS. Every end of every measured
/// range in `version::KNOWN` has a corpus directory with a case that reads
/// with no warning and no skipped line — so a range cannot be widened without
/// the files that justify it. And every corpus version is classified by the
/// gate as the case says: in range reads quietly, out of range warns.
#[test]
fn every_measured_version_has_a_corpus_that_reads_cleanly() {
    let gemini = Gemini::new("versions");
    let flavor_of = |case: &Case| match case.agent {
        AG => antigravity::flavor(&gemini.root(case.version)),
        _ => None,
    };
    for k in KNOWN {
        let mut ends = vec![k.read.0, k.read.1];
        if let Some((lo, hi)) = k.write {
            ends.extend([lo, hi]);
        }
        for end in ends {
            let clean = CASES.iter().find(|c| {
                c.agent == k.agent
                    && c.version == end
                    && flavor_of(c) == k.flavor
                    && c.shapes.contains(&Clean)
            });
            let Some(case) = clean else {
                panic!(
                    "{} {:?} {end} is in the version table with no Clean case in the corpus \
                     ({}): add a fixture for that version first. A fixture is what the build \
                     can require; the measurement the range rests on is the table's `measured` \
                     text, and a copy with the version edited does not supply it",
                    k.agent.display_name(),
                    k.flavor,
                    k.measured
                );
            };
            let r = read(case, &gemini).unwrap_or_else(|e| panic!("{}: {e}", case.id));
            assert_eq!(r.skipped(), 0, "{}", case.id);
            assert!(r.warnings().is_empty(), "{}: {:?}", case.id, r.warnings());
        }
    }
    for case in CASES {
        let verdict = version::check_read(case.agent, flavor_of(case), Some(case.version));
        // Codex has no row at all (never installed where it was measured), so
        // every Codex version warns until a real install's rollouts are added
        // here and a row is written for them.
        let unknown = case.shapes.contains(&UnknownVersion);
        assert_eq!(
            verdict.warning().is_some(),
            unknown || case.agent == CX,
            "{} {}: {verdict:?}",
            case.id,
            case.version
        );
    }
}

/// THE DIRECTORY IS THE VERSION. A file filed under 2.1.285 that records
/// 2.1.270 would make the corpus vouch for a version nobody looked at.
#[test]
fn every_file_records_the_version_of_its_directory() {
    for case in CASES.iter().filter(|c| c.agent != AG) {
        let dir = version_dir(case.agent, case.version);
        let mut stack = vec![dir.clone()];
        let mut seen = 0;
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if !name.contains(case.id) || !name.ends_with(".jsonl") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for line in text.lines() {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                        continue;
                    };
                    let recorded = match case.agent {
                        CC => v.get("version"),
                        _ if v["type"] == "session_meta" => v["payload"].get("cli_version"),
                        _ => None,
                    };
                    if let Some(recorded) = recorded {
                        seen += 1;
                        assert_eq!(
                            recorded.as_str(),
                            Some(case.version),
                            "{}: a line under {} records another version",
                            path.display(),
                            dir.display()
                        );
                    }
                }
            }
        }
        if case.entries.is_some() {
            assert!(seen > 0, "{}: no line records a version", case.id);
        }
    }
    // ANTIGRAVITY'S VERSION IS THE DIRECTORY NAME ALONE. Nothing under brain/
    // records one (the reader's own doc, written from the 12 IDE and 22 agy
    // conversations of 9 Oct 2026; it returns `version: None`), so there is nothing in its files to
    // hold against the directory — the version comes from where the case is
    // filed, and the gate is handed the flavor and no version. If the reader
    // ever finds one, this fails, and the loop above must learn where it is.
    let gemini = Gemini::new("ag-version");
    for case in CASES.iter().filter(|c| c.agent == AG) {
        let r = read(case, &gemini).unwrap_or_else(|e| panic!("{}: {e}", case.id));
        assert_eq!(
            r.session().version,
            None,
            "{}: an Antigravity file now records a version, and nothing checks it \
             against {}",
            case.id,
            case.version
        );
    }
}

/// The cases whose files are another case's files with only the version
/// string and the session id changed. Each stands in for a version the corpus
/// has no captured session of: it shows the gate and the reader accept that
/// version's string, not that the version's format was looked at. Replacing
/// one with an anonymised real session (README, "Adding a real session")
/// takes it off this list; adding another copy puts it on.
const VERSION_EDITED_COPIES: &[&str] = &[
    "c0000220-0000-4000-8000-000000000001",
    "c0000263-0000-4000-8000-000000000001",
    "c0000285-0000-4000-8000-000000000001",
    "c0000293-0000-4000-8000-000000000001",
];

/// A COPY IS NAMED AS ONE. The corpus guard above is satisfied by copying a
/// fixture and editing one string — the adversarial review of 10 Oct 2026
/// found three of the four measured-range ends were exactly that, with nothing
/// saying so. This keeps the list above equal to what the files are, both
/// ways: an undeclared copy fails, and so does a declared one that has become
/// a real capture.
#[test]
fn version_edited_copies_are_declared_as_copies() {
    let mut by_body: BTreeMap<(&str, String), Vec<&str>> = BTreeMap::new();
    for case in CASES
        .iter()
        .filter(|c| c.agent != AG && c.entries.is_some())
    {
        let mut files: Vec<(String, String)> = Vec::new();
        let mut stack = vec![version_dir(case.agent, case.version)];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if !name.contains(case.id) {
                    continue;
                }
                let neutral = |s: &str| {
                    s.replace(case.version, "<version>")
                        .replace(case.id, "<id>")
                };
                let text = std::fs::read_to_string(&path).unwrap();
                files.push((neutral(&name), neutral(&text)));
            }
        }
        assert!(!files.is_empty(), "{}: no file", case.id);
        files.sort();
        let body = files
            .into_iter()
            .map(|(n, t)| format!("{n}\n{t}"))
            .collect::<Vec<_>>()
            .join("\n");
        by_body
            .entry((case.agent.slug(), body))
            .or_default()
            .push(case.id);
    }
    let mut copies: Vec<&str> = by_body
        .into_values()
        .filter(|ids| ids.len() > 1)
        .flatten()
        .collect();
    copies.sort_unstable();
    assert_eq!(
        copies, VERSION_EDITED_COPIES,
        "the cases whose files differ only in version and id are not the ones \
         VERSION_EDITED_COPIES declares"
    );
}

/// ANONYMISED MEANS CHECKED, NOT PROMISED. A real session copied in for a new
/// version carries the copier's home directory, user name and whatever was
/// pasted into it; these are the leaks a machine can see. The rest is the
/// README's checklist.
#[test]
fn the_corpus_is_anonymised() {
    let mut problems = Vec::new();
    let me: Vec<String> = ["HOME", "USER", "LOGNAME", "USERNAME"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .map(|v| v.trim_end_matches('/').to_string())
        .filter(|v| v.len() >= 4 && v != "/Users/dev" && v != "/home/dev" && v != "/root")
        .collect();
    let mut stack = vec![corpus()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let shown = path.strip_prefix(corpus()).unwrap().display().to_string();
            let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
            problems.extend(
                anonymity_problems(&text, &me)
                    .into_iter()
                    .map(|p| format!("{shown}: {p}")),
            );
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

fn anonymity_problems(text: &str, me: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for who in me {
        // A home directory anywhere; a bare user name only where a path or
        // Claude Code's encoded project directory would put it — a user called
        // `main` must not fail on every `"gitBranch":"main"`.
        let hit = if who.starts_with('/') {
            text.contains(who.as_str())
        } else {
            [
                format!("/{who}/"),
                format!("/{who}\""),
                format!("\\{who}\\"),
                format!("-{who}-"),
            ]
            .iter()
            .any(|p| text.contains(p.as_str()))
        };
        if hit {
            out.push(format!("names this machine's {who:?}"));
        }
    }
    // `-Users-x-` is the same home directory as Claude Code files it.
    for prefix in ["/Users/", "/home/", "C:\\Users\\", "-Users-", "-home-"] {
        for (at, _) in text.match_indices(prefix) {
            let rest = &text[at + prefix.len()..];
            if !["dev/", "dev\\", "dev-"]
                .iter()
                .any(|ok| rest.starts_with(ok))
            {
                let end = rest
                    .find(|c: char| !(c.is_alphanumeric() || c == '.' || c == '_' || c == '-'))
                    .unwrap_or(rest.len());
                out.push(format!(
                    "a home directory other than dev: {prefix}{}",
                    &rest[..end]
                ));
            }
        }
    }
    let b = text.as_bytes();
    for (at, _) in text.match_indices('@') {
        let local = at > 0 && (b[at - 1].is_ascii_alphanumeric() || b"._-+".contains(&b[at - 1]));
        let domain: String = text[at + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
            .collect();
        if local && domain.contains('.') && !domain.ends_with('.') {
            out.push(format!("an e-mail address at {domain}"));
        }
    }
    for secret in [
        "sk-ant-",
        "sk-proj-",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AKIA",
        "ya29.",
        "AIza",
        "-----BEGIN",
    ] {
        if text.contains(secret) {
            out.push(format!("something shaped like a credential ({secret}…)"));
        }
    }
    out
}

/// The scanner above, held to what it must catch: a check that cannot see a
/// leak is a green light on the one thing the README asks a person to trust.
#[test]
fn the_anonymity_scan_sees_what_it_claims_to() {
    let me = ["/Users/someone".to_string(), "someone".to_string()];
    for leak in [
        r#"{"cwd":"/Users/someone/work"}"#,
        "projects/-Users-someone-work/x.jsonl",
        "projects/-home-alice-repo/x.jsonl",
        r#"{"cwd":"/home/alice/repo"}"#,
        "mail me at jane.doe@example.org",
        "ANTHROPIC_API_KEY=sk-ant-api03-xyz",
        "token ya29.a0Af",
    ] {
        assert!(!anonymity_problems(leak, &me).is_empty(), "missed {leak:?}");
    }
    for fine in [
        r#"{"cwd":"/Users/dev/corpus"}"#,
        "/home/dev/corpus",
        "@Override and a@b without a dot",
        "-Users-dev-corpus",
    ] {
        assert!(anonymity_problems(fine, &me).is_empty(), "flagged {fine:?}");
    }
}

/// A FORK IS ITS ORIGIN UNDER A NEW ID. What the two captured forks hold,
/// as Claude Code 2.1.285 wrote them: the origin's every uuid, every line
/// stamped with the fork's own session id, filed beside the origin, and the
/// origin's context as the start of the fork's. A reader that keyed entries
/// by uuid across files, or listed one project directory's sessions by the
/// first `sessionId` it met, would merge the two; this is where it shows.
#[test]
fn a_captured_fork_is_its_origin_under_a_new_id() {
    fn ids(path: &Path) -> (BTreeSet<String>, BTreeSet<String>) {
        let (mut uuids, mut sessions) = (BTreeSet::new(), BTreeSet::new());
        for line in std::fs::read_to_string(path).unwrap().lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            if let Some(u) = v["uuid"].as_str() {
                uuids.insert(u.to_string());
            }
            if let Some(s) = v["sessionId"].as_str() {
                sessions.insert(s.to_string());
            }
        }
        (uuids, sessions)
    }
    let gemini = Gemini::new("fork");
    let case = |id: &str| CASES.iter().find(|c| c.id == id).unwrap();
    assert!(!CAPTURED_FORKS.is_empty());
    for (origin, fork) in CAPTURED_FORKS {
        let (o, f) = (case(origin), case(fork));
        assert!(f.shapes.contains(&Fork) && !o.shapes.contains(&Fork));
        let (o, f) = (read(o, &gemini).unwrap(), read(f, &gemini).unwrap());
        let (op, fp) = (&o.claude().path, &f.claude().path);
        assert_eq!(op.parent(), fp.parent(), "{fork}: not beside {origin}");
        let ((ou, os), (fu, fs)) = (ids(op), ids(fp));
        assert!(ou.is_subset(&fu), "{fork}: lost uuids of {origin}");
        assert_eq!(os, BTreeSet::from([origin.to_string()]));
        assert_eq!(fs, BTreeSet::from([fork.to_string()]));
        let (oc, fc) = (shape(&o), shape(&f));
        assert!(fc.len() > oc.len(), "{fork}: nothing asked in the fork");
        assert_eq!(fc[..oc.len()], oc[..], "{fork}: does not start as {origin}");
    }
}

/// WHAT IS CAPTURED IS SAID, AND ONLY THAT. The corpus README is where a
/// reader learns which fixtures are real; it said "no real session is in the
/// corpus yet" for as long as that was true, and a capture added without
/// changing it would leave the page claiming less than the corpus holds — or,
/// the other way round, a session id left in the README after its files went
/// would claim more. Both directions, by id.
#[test]
fn the_captured_cases_are_the_ones_the_corpus_readme_names() {
    let readme = std::fs::read_to_string(corpus().join("README.md")).unwrap();
    assert!(
        !readme.contains("No real session is in the corpus yet"),
        "the README still says no real session is in the corpus"
    );
    for id in CAPTURED {
        let case = CASES
            .iter()
            .find(|c| c.id == *id)
            .unwrap_or_else(|| panic!("{id}: captured, but no case reads it"));
        assert!(case.entries.is_some(), "{id}");
        assert!(
            !VERSION_EDITED_COPIES.contains(id),
            "{id}: declared a capture and a copy"
        );
        assert!(readme.contains(id), "the corpus README does not name {id}");
    }
    for (origin, fork) in CAPTURED_FORKS {
        assert!(CAPTURED.contains(origin) && CAPTURED.contains(fork));
    }
    let named: BTreeSet<&str> = readme
        .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
        .filter(|w| w.len() == 36 && w.matches('-').count() == 4)
        .collect();
    let captured: BTreeSet<&str> = CAPTURED.iter().copied().collect();
    assert_eq!(named, captured, "session ids the README names vs CAPTURED");
}

/// THE CORPUS IS PUBLIC, AND A VENDOR ID NAMES A CALL ON A REAL ACCOUNT. The
/// first real captures (11 Oct 2026) kept 21 `req_…` and 21 `msg_…` ids
/// verbatim, because the readers group by them and the anonymiser counted
/// them as structure. Every vendor-shaped id anywhere under the corpus must
/// now be a pseudonym (`Scrub::vendor_id`, body starting with `anon`); a
/// raw one fails here, whichever agent or version folder it sits in.
#[test]
fn the_real_captures_carry_no_raw_vendor_id() {
    use tacet_tools::sessions::anonymise::VENDOR_ID_MARK;
    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                files(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut all = Vec::new();
    files(&corpus(), &mut all);
    let mut raw = Vec::new();
    for path in &all {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for prefix in ["req_", "msg_", "toolu_", "srvtoolu_"] {
            for (at, _) in text.match_indices(prefix) {
                // A prefix inside a longer word (`xmsg_`) is not an id.
                if at > 0 && text.as_bytes()[at - 1].is_ascii_alphanumeric() {
                    continue;
                }
                let tail: String = text[at + prefix.len()..]
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect();
                if tail.len() >= 12 && !tail.starts_with(VENDOR_ID_MARK) {
                    raw.push(format!("{}: {prefix}{tail}", path.display()));
                }
            }
        }
    }
    assert!(
        all.len() > 10,
        "the corpus walk found only {} files",
        all.len()
    );
    assert!(
        raw.is_empty(),
        "{} raw vendor id(s) in the corpus:\n{}",
        raw.len(),
        raw.join("\n")
    );
}
