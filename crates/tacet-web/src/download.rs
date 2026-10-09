//! Large file download — the ONE principle for model packages.
//!
//! WHY HERE: THE NETWORK MONOPOLY. In Tacet a network call exists only in this
//! crate and in `tacet-mcp` (see the top of `lib.rs`). Downloading a model
//! package looks like shell work but it OPENS A SOCKET; written inside
//! `tacet-cli` the monopoly would stop being auditable by eye — the answer to
//! "where does traffic leave from" would go from two crates to four. The shell
//! CALLS `download` here; it does NOT BUILD its own `ureq` agent.
//!
//! A SEPARATE FILE FROM `client.rs`, because the two jobs have opposite
//! requirements:
//! * search — small response, a STRICT global timeout (10 s), all into memory.
//! * download — gigabyte-sized response, a global timeout is FORBIDDEN (a
//!   legitimate download takes minutes), streaming to disk.
//!
//! Putting both in one function meant loosening one's timeout to fit the
//! other's need.
//!
//! THE APPROVAL GATE IS STRUCTURAL. `download` produces NOT ONE BYTE of network
//! traffic before `DownloadApproval::approve` returns `true` — even the agent is
//! built AFTER that line. Leaving the gate to the caller (`if approved {
//! download(...) }`) looks like it writes the same thing but does not: it gets
//! forgotten at the next call site and nobody notices.
//!
//! PRODUCTION CALLER: `tacet-cli` → `model_download` (`tacet models download <name>`).
//! For one turn this was a TESTED BUT UNWIRED mechanism — the `tacet-cli`
//! manifest had no `tacet-web` dependency, so although the build was green
//! nothing called this file in production. The dependency was added and the
//! branch wired up; before changing anything, the place to look is that
//! function in the shell (the terminal end of the approval gate and the
//! progress line lives there).
//!
//! WHAT IS MEASURED, AND AGAINST WHAT: resuming with `Range` / `If-Range`, the
//! 200/206 distinction, the size cap and the stall watchdog run against a
//! plain-HTTP listener on 127.0.0.1 inside the unit tests (`tests::loopback`)
//! — the same agent configuration with `https_only` off, and nothing else
//! changed. NOT MEASURED: a real TLS mirror, a real CDN's `If-Range` handling,
//! a real multi-gigabyte transfer; CI must not depend on the network. Also
//! measured: the approval gate, the scheme gate, SHA-256's official test
//! vectors, chunked feeding producing the same digest as whole feeding, the
//! temp-name rule, and an existing target being verified without going online.
//!
//! NO PLAIN `http://` — AND UNLIKE SEARCH, NO LOCAL NETWORK EXCEPTION EITHER.
//! `client.rs` allows plain http to a local SearXNG; what is carried there is a
//! query. What is carried here is a MODEL WEIGHT and that file gets loaded and
//! executed on the machine. On an unencrypted first download anyone in between
//! can change the weight; the TOFU digest cannot catch it either, because TOFU
//! accepts the first thing downloaded as CORRECT. That is why the scheme gate
//! is strict here.

use crate::error::WebError;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use ureq::typestate::AgentScope;

/// The extension of a half-downloaded file. Completion is ATOMIC: the byte
/// stream is written to `<target>.downloading` and only moves to the target via
/// `rename` AFTER the digest is verified. That way `<target>` either does not
/// exist or is COMPLETE; a half-written weight file is never seen by the engine.
pub const TEMP_EXTENSION: &str = "downloading";

/// The timeout for the connection and the first response. It is NOT applied to
/// the body STREAM — a legitimate 2.5 GB download takes minutes and a global
/// cap would cut it exactly where it was about to finish.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// The cap for the response HEADERS to arrive. The body streams after that.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the body may deliver NOTHING before the run gives up (audit L6).
/// The same order as `RESPONSE_TIMEOUT`: a source that has gone a minute
/// without a byte is not slow, it is gone — and the leftover is kept, so
/// giving up costs nothing that the next run does not get back.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The floor of the whole-body deadline — see `body_budget`.
const BODY_FLOOR: Duration = Duration::from_secs(15 * 60);
/// The slowest rate the deadline is scaled by: 128 KiB/s, about 1 Mbit/s. A
/// 2.5 GB weight gets roughly five and a half hours on top of the floor.
const MIN_RATE: u64 = 128 * 1024;
/// The deadline when no size is declared, where no scaling is possible.
const UNKNOWN_SIZE_BUDGET: Duration = Duration::from_secs(6 * 60 * 60);

/// The wall for a body with NO declared size. With a declared size the cap is
/// that size plus one byte (see `download_via`).
const BODY_WALL: u64 = 32 * 1024 * 1024 * 1024;

/// The disk write chunk. 64 KiB: a reasonable amount of work per syscall,
/// negligible space in memory.
const CHUNK: usize = 64 * 1024;

/// The shortest interval between progress notifications. Notifying on every
/// chunk would print thousands of lines a second to the terminal.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

// ---------------------------------------------------------------------------
// Contract types
// ---------------------------------------------------------------------------

/// The ONE file to download.
///
/// THE URL AND THE SIZE COME FROM THE CALLER, they are NOT BAKED INTO this
/// file. Reasoning: in this repository nothing unverified may be written down
/// as "works", and the address and SHA-256 of a model mirror are verifiable
/// facts; baking in either without knowing it would mean sending the user to an
/// address we invented. The catalog is read from the user's `packages.json`
/// (see tacet-cli).
#[derive(Debug, Clone)]
pub struct DownloadPlan {
    /// The name shown to the user (`model.gguf`).
    pub name: String,
    /// The source address. `https://` MANDATORY.
    pub url: String,
    /// The final path. Its parent directory is created if missing.
    pub target: PathBuf,
    /// The size the catalog declares. It is SHOWN on the approval screen; the
    /// user knows what they are accepting before the download starts.
    pub expected_bytes: Option<u64>,
    /// The expected SHA-256 (from the catalog or from the previous download's
    /// TOFU record). If `None` the digest is still COMPUTED and handed back to
    /// the caller — the user sees it and records it (TOFU).
    pub expected_sha256: Option<String>,
}

/// The outcome of a successful download.
#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    pub target: PathBuf,
    pub bytes: u64,
    /// The file's REAL digest — always computed, even when no expected value
    /// was given. The caller stores this as the TOFU record.
    pub sha256: String,
    /// Whether it continued from a half-finished file using Range.
    pub resumed: bool,
    /// The target was already in place; NOTHING WENT ONLINE.
    pub already_present: bool,
    /// `expected_sha256` was given and it MATCHED.
    pub digest_verified: bool,
}

/// The gate asked BEFORE the network call.
///
/// A trait, because `tacet-web` does not know about the terminal: the side that
/// asks the question is the shell (the `[y/N]` pattern), the side that goes
/// online is here. Without that split, either the network layer would read
/// stdin or the shell would open a socket; both break the architecture.
pub trait DownloadApproval {
    /// If it returns `false` NO network call is made.
    ///
    /// `existing_bytes`: the bytes already sitting in a half-finished file (a
    /// local `stat`, not the network). Given so the user can be told "how much
    /// is left".
    fn approve(&self, plan: &DownloadPlan, existing_bytes: u64) -> bool;
}

/// Progress notification. The default bodies are empty: a quiet caller (a test,
/// a script) should not have to implement anything.
pub trait Progress {
    fn started(&self, _name: &str, _downloaded: u64, _total: Option<u64>) {}
    fn advanced(&self, _downloaded: u64, _total: Option<u64>) {}
    /// Digest computation started. On a GB-sized file this takes seconds and,
    /// unreported, looks like "the download finished but the program hung".
    fn digesting(&self, _bytes: u64) {}
    fn finished(&self, _outcome: &DownloadOutcome) {}
}

/// Progress that prints nothing — for tests and scripts.
pub struct SilentProgress;
impl Progress for SilentProgress {}

/// A gate that always refuses. FOR TESTS AND AS THE DEFAULT: if a caller
/// forgets to wire the gate up, the right behaviour is not "download silently"
/// but "do nothing".
pub struct AlwaysDeny;
impl DownloadApproval for AlwaysDeny {
    fn approve(&self, _plan: &DownloadPlan, _existing_bytes: u64) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// The download's failure cases.
///
/// WHY NOT `WebError`: the failure sets of search and download do not overlap.
/// "Not approved", "digest mismatch", "could not write to disk" DO NOT EXIST in
/// search; `EmptyResult` is meaningless here. Cramming both jobs' errors into
/// one enum would force every caller to filter out variants that do not concern
/// it. The VARIANT is carried over from `WebError` verbatim (`Network`) — so the
/// classification of a ureq error stays in one place (see `client::convert`) —
/// but the TEXT is not: `WebError`'s `Display` was written for search and would
/// talk to a user downloading a model about searching. `network_text` rebuilds
/// the sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    /// The user did not approve. NOTHING WENT ONLINE.
    NotApproved,
    /// The address is not `https://`, or is malformed.
    InvalidAddress(String),
    /// The transport layer: the CLASSIFICATION is the same as in `client.rs`,
    /// the TEXT is not (see `network_text`).
    Network(WebError),
    /// The local filesystem: the directory could not be opened, the disk is
    /// full, permission denied.
    File(String),
    /// The downloaded file's digest did not match the expected one.
    DigestMismatch { expected: String, found: String },
    /// The catalog declared a size but the downloaded file has a different one.
    ///
    /// A SEPARATE VARIANT, because for a package whose digest is unknown
    /// (before TOFU) this is the only check that catches a TRUNCATED download.
    SizeMismatch { expected: u64, found: u64 },
    /// The half file could not be continued: the server answered the `Range`
    /// with a `206` we cannot trust.
    ///
    /// A SEPARATE VARIANT FROM `SizeMismatch`, because the diagnosis differs.
    /// "The size did not match" sends the user looking for a broken mirror;
    /// this one says the LEFTOVER is no longer valid, and the half file has
    /// already been dropped so the next run starts clean.
    RangeMismatch { asked: u64, detail: String },
    /// The body stopped arriving: nothing for `silent`, after byte `at`.
    ///
    /// A SEPARATE VARIANT, because it is the one failure where the right next
    /// step is simply to run the command again: the leftover is kept and the
    /// next run resumes it.
    Stalled { at: u64, silent: Duration },
}

impl fmt::Display for DownloadError {
    /// These strings go TO THE USER: they suggest an action wherever possible.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::NotApproved => {
                f.write_str("The download was not approved; nothing was downloaded.")
            }
            DownloadError::InvalidAddress(d) => write!(f, "The download address is invalid: {d}"),
            DownloadError::Network(e) => f.write_str(&network_text(e)),
            DownloadError::File(d) => write!(f, "The file could not be written: {d}"),
            DownloadError::DigestMismatch { expected, found } => write!(
                f,
                "SHA-256 did not match (expected {expected}, found {found}) — the file is corrupt or has been altered."
            ),
            DownloadError::SizeMismatch { expected, found } => {
                write!(
                    f,
                    "The size did not match (expected {expected} bytes, downloaded {found} bytes)."
                )
            }
            DownloadError::RangeMismatch { asked, detail } => write!(
                f,
                "The half-finished file could not be continued from byte {asked} ({detail}); it has been discarded, run the command again."
            ),
            DownloadError::Stalled { at, silent } => write!(
                f,
                "The source stopped sending data at byte {at} ({silent:?} without a byte); the half-finished file is kept, run the command again to continue."
            ),
        }
    }
}

/// The transport error's text IN THE DOWNLOAD CONTEXT.
///
/// WHY `WebError`'s OWN `Display` IS NOT USED: those strings were written FOR
/// SEARCH ("The search server could not be reached.", "The search took too
/// long.") and, because the same variants come back over the wire here too, a
/// user downloading a model was being shown a sentence ABOUT SEARCH. This was
/// found by measurement: `tacet models download` was run with an invalid host
/// name and the output said "The search server could not be reached."
///
/// Generalising `WebError`'s strings (to, say, "The server could not be
/// reached") would have been a smaller patch but in the WRONG place: on the
/// search side the thing the user must fix is the SearXNG address, and it is
/// right for the sentence to say so. The two callers' text differs; the split
/// is made at the CALLER boundary.
///
/// ALSO, THE DETAIL CAME BACK: `Unreachable(String)` carries the transport
/// layer's own explanation (DNS did not resolve / connection refused / TLS) and
/// in the download context that is the first thing the user will look at; on
/// the search side that same detail is deliberately swallowed (the chip text
/// must stay short).
fn network_text(e: &WebError) -> String {
    match e {
        WebError::Unreachable(d) => format!("The download source could not be reached: {d}"),
        WebError::Timeout => {
            "The server did not return the response headers in time; the source is slow or unreachable."
                .to_string()
        }
        WebError::ServerCode(c) => match c {
            // 404 and 403 are the two most frequent cases and what the user
            // does differs: one means the address is wrong, the other means the
            // address is right but access is denied.
            404 => "The file is not at the source (404) — the address in packages.json may be stale.".to_string(),
            403 => "The source refused access (403) — the address may require a session or a key.".to_string(),
            _ => format!("The download source returned an error ({c})."),
        },
        // THESE TWO ARE NOT PRODUCED IN A DOWNLOAD (`InvalidJson` and
        // `EmptyResult` come out of interpreting a search response, and the
        // download does not interpret the body). Because the enum is shared
        // they still have to compile; rather than MAKING UP text for an
        // unmeasured case, the source's own text is written out.
        other => other.to_string(),
    }
}

impl std::error::Error for DownloadError {}

pub type DownloadResult<T> = Result<T, DownloadError>;

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

/// Downloads a single file: approval → (if any) resume → stream → digest →
/// atomic move.
///
/// THE ORDER GUARANTEE: the agent is not even built before `approve` returns
/// `true`. The only place to look in order to verify that by reading is
/// `download_via`'s body.
pub fn download(
    plan: &DownloadPlan,
    approval: &dyn DownloadApproval,
    progress: &dyn Progress,
) -> DownloadResult<DownloadOutcome> {
    download_via(plan, approval, progress, &PRODUCTION)
}

/// How the bytes travel. Production has exactly one value of this
/// (`PRODUCTION`); the unit tests below build the other one.
///
/// WHY A SEAM AT ALL: the resume, size-cap and stall rules (audit L3, L5, L6)
/// are decided by what a REAL server sends over a REAL socket — a `206` and
/// its headers, a body that does not stop, a body that stops moving — and a
/// download is https-only, while a test may only open 127.0.0.1 listeners and
/// has no certificate to serve. The seam swaps the one flag that stands in the
/// way and nothing else: the agent comes from the same `download_builder`.
struct Transport {
    /// A FACTORY, not an agent: `download_via` calls it after the approval
    /// gate, so "no agent before approval" stays true on every path.
    agent: fn(Option<u64>) -> ureq::Agent,
    /// Accept `http://127.0.0.1:<port>/…`. False in production; only the
    /// tests' transport sets it.
    plain_loopback: bool,
    /// How long the body may deliver nothing before the run gives up.
    stall: Duration,
}

const PRODUCTION: Transport = Transport {
    agent: download_agent,
    plain_loopback: false,
    stall: STALL_TIMEOUT,
};

fn download_via(
    plan: &DownloadPlan,
    approval: &dyn DownloadApproval,
    progress: &dyn Progress,
    transport: &Transport,
) -> DownloadResult<DownloadOutcome> {
    validate_address(&plan.url, transport.plain_loopback)?;

    // IF THE TARGET IS ALREADY IN PLACE NOTHING GOES ONLINE. This branch is not
    // merely a convenience, it is APPROVAL ECONOMY: asking a user who runs the
    // same command a second time "2.5 GB will be downloaded, do you approve?"
    // turns the approval question into a meaningless ritual and they start
    // approving everything by reflex.
    if plan.target.is_file() {
        let bytes = file_size(&plan.target)?;
        progress.digesting(bytes);
        let digest = digest_from_file(&plan.target)?;
        let verified = check_digest(plan, &digest)?;
        let outcome = DownloadOutcome {
            target: plan.target.clone(),
            bytes,
            sha256: digest,
            resumed: false,
            already_present: true,
            digest_verified: verified,
        };
        progress.finished(&outcome);
        return Ok(outcome);
    }

    if let Some(parent) = plan.target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| DownloadError::File(format!("{}: {e}", parent.display())))?;
    }
    let temp = temp_path(&plan.target);
    let record_path = resume_record_path(&plan.target);
    let leftover = temp.metadata().map(|m| m.len()).unwrap_or(0);
    let basis = resume_basis(plan, leftover, read_record(&record_path).as_ref());
    // The approval screen is told what will REALLY be kept: a leftover that is
    // about to be thrown away is not "already downloaded".
    let existing = match &basis {
        Basis::Resume { from, .. } => *from,
        Basis::Fresh => 0,
    };

    // ---- THE NETWORK GATE. ABOVE this line no socket has been opened. ----
    if !approval.approve(plan, existing) {
        return Err(DownloadError::NotApproved);
    }

    // A LEFTOVER THAT CANNOT BE CONTINUED IS DELETED NOW, not overwritten
    // later: if this run fails before the first byte, the stale prefix must not
    // still be lying there for the next run to mistake for its own.
    if basis == Basis::Fresh {
        discard(&temp, &record_path);
    }

    let agent = (transport.agent)(plan.expected_bytes);

    let mut request = agent.get(&plan.url);
    let mut asked_tag: Option<String> = None;
    if let Basis::Resume { from, if_range } = &basis {
        // RESUME: ask to continue from the byte where the half file ended.
        request = request.header("Range", format!("bytes={from}-"));
        // AND ONLY IF IT IS STILL THE SAME OBJECT. `If-Range` makes the
        // server itself do the comparison: a matching ETag gets the `206`,
        // anything else gets a `200` with the whole new object — which
        // `resume_verdict` turns into a restart. See `resume_basis`.
        if let Some(tag) = if_range {
            request = request.header("If-Range", tag.as_str());
            asked_tag = Some(tag.clone());
        }
    }
    // A 416 ON A RESUME MEANS THE HALF FILE IS AT LEAST AS LONG AS THE OBJECT.
    // It can only have come from a stale or an overlong leftover (an older,
    // LARGER release under the same `<exe>.new.downloading` name, or a source
    // that sent more bytes than the catalog declared). Resuming can NEVER
    // succeed from there: every future run asks for the same unsatisfiable
    // range and the user sees nothing but "the source returned an error (416)"
    // forever, with no hint that a file has to be deleted by hand. The
    // leftover is dropped and the download restarts from zero ONCE.
    let (response, existing) = match request.call() {
        Ok(response) => (response, existing),
        Err(ureq::Error::StatusCode(416)) if existing > 0 => {
            discard(&temp, &record_path);
            asked_tag = None;
            let response = agent
                .get(&plan.url)
                .call()
                .map_err(|e| DownloadError::Network(network_error(&e)))?;
            (response, 0)
        }
        Err(e) => return Err(DownloadError::Network(network_error(&e))),
    };

    // 206 = the server accepted the Range, the body is the REMAINING part.
    // 200 = the server ignored the Range (or does not know it at all, or the
    //       `If-Range` validator no longer matches), the body is the WHOLE
    //       thing; the half file is then thrown away and rewritten from
    //       scratch. Not telling those apart meant appending the whole body
    //       AFTER the half file and silently producing a corrupt file.
    //
    // AND THE STATUS CODE ALONE IS NOT ENOUGH — see `resume_verdict`.
    let content_range = header(&response, "content-range");
    let served_tag = header(&response, "etag");
    let verdict = match resume_verdict(
        response.status().as_u16(),
        content_range.as_deref(),
        existing,
        plan.expected_bytes,
    ) {
        Resume::From(offset) => match same_object(asked_tag.as_deref(), served_tag.as_deref()) {
            Ok(()) => Resume::From(offset),
            Err(detail) => Resume::Untrusted(detail),
        },
        other => other,
    };
    let (resumed, start) = match verdict {
        Resume::From(offset) => (true, offset),
        Resume::Restart => (false, 0),
        Resume::Untrusted(detail) => {
            // THE LEFTOVER IS DROPPED. Keeping it would repeat the same
            // untrusted answer on every run, and this is the one case where
            // the half file is KNOWN not to belong to the object being
            // downloaded.
            discard(&temp, &record_path);
            return Err(DownloadError::RangeMismatch {
                asked: existing,
                detail,
            });
        }
    };
    let remaining = body_length(&response);
    let total = remaining.map(|r| start + r).or(plan.expected_bytes);

    let mut file = if resumed {
        fs::OpenOptions::new().append(true).open(&temp)
    } else {
        fs::File::create(&temp)
    }
    .map_err(|e| DownloadError::File(format!("{}: {e}", temp.display())))?;

    // THE IDENTITY OF WHAT IS BEING WRITTEN, recorded BEFORE the first byte —
    // an interrupted run is exactly the one that needs it. A failed write is
    // ignored on purpose: with no record the next run starts fresh, which is
    // the safe direction.
    if !resumed {
        let _ = write_record(&record_path, &ResumeRecord::of(plan, served_tag.as_deref()));
    }

    progress.started(&plan.name, start, total);

    // THE BODY CAP IS THE DECLARED SIZE PLUS ONE BYTE (audit L5). Reading one
    // byte past `expected_bytes` is all it takes to prove the source is wrong;
    // the old wall was 32 GiB, so a broken mirror — or a `packages.json` entry
    // pointing at the wrong file — could fill the disk before `SizeMismatch`
    // ever got to speak. The 32 GiB wall stays for the one case with no
    // declared size.
    let cap = plan.expected_bytes.map(|e| (e + 1).saturating_sub(start));
    let reader = response
        .into_body()
        .into_with_config()
        .limit(BODY_WALL)
        .reader();
    let streamed = stream_body(
        Box::new(reader),
        &mut file,
        &temp,
        Stream {
            start,
            cap,
            stall: transport.stall,
            total,
        },
        progress,
    );
    // A STALL OR A DROPPED CONNECTION KEEPS THE LEFTOVER AND ITS RECORD: that
    // is the case resume exists for.
    let downloaded = streamed?;
    file.sync_all()
        .map_err(|e| DownloadError::File(format!("{}: {e}", temp.display())))?;
    drop(file);
    progress.advanced(downloaded, total);

    // THE SIZE CHECK COMES BEFORE the digest: the diagnosis of a truncated
    // download is not "SHA did not match" but "the file is half there";
    // describing both with the same sentence would send the user looking for a
    // broken mirror.
    if let Some(expected) = plan.expected_bytes {
        let verdict = size_verdict(Some(expected), downloaded);
        if verdict != SizeVerdict::Exact {
            // AN OVERSIZED LEFTOVER IS DELETED, A SHORT ONE IS KEPT. A short
            // file is what resume is for. A file LONGER than the object is a
            // dead end: the next run sends `Range: bytes=<len>-`, the server
            // answers 416, and without this the user would be stuck on an
            // error code that says nothing about the real cause. (The 416
            // branch above rescues that case too; this deletion stops it from
            // ever arising.)
            if verdict == SizeVerdict::TooLong {
                discard(&temp, &record_path);
            }
            return Err(DownloadError::SizeMismatch {
                expected,
                found: downloaded,
            });
        }
    }

    // THE DIGEST IS OVER THE WHOLE FILE, not over the stream. In a resumed
    // download the streaming bytes are ONLY THE TAIL of the file; digesting the
    // stream would produce a half digest. Reading the finished file from the
    // start takes a few seconds and makes resuming structurally correct.
    progress.digesting(downloaded);
    let digest = digest_from_file(&temp)?;
    let verified = match check_digest(plan, &digest) {
        Ok(v) => v,
        Err(e) => {
            // A CORRUPT TEMP FILE IS DELETED. Left in place, the next run
            // APPENDS to it with Range and reproduces the same failure; the
            // user stays in a "I keep downloading, it is always corrupt" loop.
            discard(&temp, &record_path);
            return Err(e);
        }
    };

    // THE ATOMIC MOVE: `rename` within the same directory, a single operation
    // on POSIX and on Windows.
    fs::rename(&temp, &plan.target)
        .map_err(|e| DownloadError::File(format!("{}: {e}", plan.target.display())))?;
    let _ = fs::remove_file(&record_path);

    let outcome = DownloadOutcome {
        target: plan.target.clone(),
        bytes: downloaded,
        sha256: digest,
        resumed,
        already_present: false,
        digest_verified: verified,
    };
    progress.finished(&outcome);
    Ok(outcome)
}

/// The parameters of one body stream, grouped so `stream_body` does not take
/// eight positional arguments of which four are numbers.
struct Stream {
    /// The byte the body starts at (the leftover's length on a resume).
    start: u64,
    /// The most bytes THIS body may deliver — `expected - start + 1`.
    cap: Option<u64>,
    stall: Duration,
    total: Option<u64>,
}

/// Streams the body into `file` and returns the file's new length.
///
/// THE STALL WATCHDOG (audit L6). The agent had a cap for connecting and for
/// the headers and NONE for the body, on purpose — a global cap cuts a
/// legitimate multi-gigabyte download — so a source that sent the headers and
/// then nothing kept `tacet models download` waiting forever with a frozen
/// progress line. `ureq` has no idle-read timeout to set, so the reading
/// happens on a thread of its own and this side waits on a channel with
/// `recv_timeout`: a body that delivers nothing for `stall` ends the run with
/// `Stalled`, the leftover stays, and the next run resumes from it.
///
/// THE ABANDONED READER is bounded, not leaked forever: the agent's
/// `timeout_recv_body` (see `body_budget`) puts a deadline on the socket, so
/// the thread wakes up, finds the channel closed and ends. It never touches
/// the file — only this side writes — so a late chunk cannot land in a
/// leftover the next run is already appending to.
fn stream_body(
    reader: Box<dyn Read + Send>,
    file: &mut fs::File,
    temp: &Path,
    stream: Stream,
    progress: &dyn Progress,
) -> DownloadResult<u64> {
    // A few chunks of slack so the network is not idle while the disk writes;
    // bounded so a fast source cannot pile the body up in memory.
    let (sender, receiver) = mpsc::sync_channel::<std::io::Result<Vec<u8>>>(4);
    let cap = stream.cap;
    thread::spawn(move || {
        let mut reader: Box<dyn Read + Send> = match cap {
            Some(c) => Box::new(reader.take(c)),
            None => reader,
        };
        loop {
            let mut chunk = vec![0u8; CHUNK];
            let message = match reader.read(&mut chunk) {
                Ok(n) => {
                    chunk.truncate(n);
                    Ok(chunk)
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => Err(e),
            };
            // An empty chunk is the end marker; an error is final too.
            let last = !matches!(&message, Ok(c) if !c.is_empty());
            if sender.send(message).is_err() || last {
                return;
            }
        }
    });

    let mut downloaded = stream.start;
    let mut last_notice = Instant::now();
    loop {
        match receiver.recv_timeout(stream.stall) {
            Ok(Ok(chunk)) if chunk.is_empty() => return Ok(downloaded),
            Ok(Ok(chunk)) => {
                file.write_all(&chunk)
                    .map_err(|e| DownloadError::File(format!("{}: {e}", temp.display())))?;
                downloaded += chunk.len() as u64;
                if last_notice.elapsed() >= PROGRESS_INTERVAL {
                    progress.advanced(downloaded, stream.total);
                    last_notice = Instant::now();
                }
            }
            Ok(Err(e)) => {
                return Err(DownloadError::Network(WebError::Unreachable(e.to_string())));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(DownloadError::Stalled {
                    at: downloaded,
                    silent: stream.stall,
                });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(DownloadError::Network(WebError::Unreachable(
                    "the download stream ended without an end marker".into(),
                )));
            }
        }
    }
}

/// Checks the expected digest if there is one. `Ok(true)` = verified,
/// `Ok(false)` = there was no expected value (TOFU: the caller records the
/// computed one).
fn check_digest(plan: &DownloadPlan, found: &str) -> DownloadResult<bool> {
    match &plan.expected_sha256 {
        None => Ok(false),
        Some(e) if e.eq_ignore_ascii_case(found) => Ok(true),
        Some(e) => Err(DownloadError::DigestMismatch {
            expected: e.to_ascii_lowercase(),
            found: found.to_string(),
        }),
    }
}

/// The agent the download uses. SPLIT OUT SO IT CAN BE ASSERTED ON: the
/// `https_only` flag below is invisible in behaviour until the day a mirror
/// answers with a redirect, and a silently dropped flag would be found by the
/// user, not by us.
pub(crate) fn download_agent(expected_bytes: Option<u64>) -> ureq::Agent {
    download_builder(expected_bytes).build().into()
}

/// The production configuration, as a builder — the tests' loopback transport
/// starts from this exact value and changes `https_only` alone.
fn download_builder(expected_bytes: Option<u64>) -> ureq::config::ConfigBuilder<AgentScope> {
    crate::net::builder()
        // A global cap is DELIBERATELY ABSENT (see the top of the file). The
        // connection and header caps stay: on a dead server we do not wait
        // forever.
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(RESPONSE_TIMEOUT))
        // The body gets a budget scaled to its size — see `body_budget`.
        .timeout_recv_body(Some(body_budget(expected_bytes)))
        .max_redirects(5)
        // THE SCHEME GATE MUST SURVIVE THE REDIRECT. `validate_address` only
        // sees hop 0; `ureq` enforces the scheme in exactly one place and only
        // when this flag is on, and its default is FALSE. Without it a `302
        // Location: http://…` silently downgrades the weight transfer to plain
        // text — exactly the case the header of this file forbids, and the one
        // TOFU cannot catch, because TOFU accepts whatever arrives first.
        .https_only(true)
}

/// The whole-body deadline: a floor plus the declared size at `MIN_RATE`.
///
/// NOT A SPEED LIMIT, A BOUND ON FOREVER. The stall watchdog in `stream_body`
/// catches a body that stops; it cannot catch one that trickles a byte every
/// few seconds, and it cannot free the reader thread it walks away from. This
/// deadline does both. A legitimately slow link that runs into it loses
/// nothing: the leftover stays, and the next run resumes it.
fn body_budget(expected_bytes: Option<u64>) -> Duration {
    match expected_bytes {
        Some(bytes) => BODY_FLOOR + Duration::from_secs(bytes / MIN_RATE),
        None => UNKNOWN_SIZE_BUDGET,
    }
}

/// What the resume decision is based on, made BEFORE anything goes online.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Basis {
    /// No usable leftover: delete whatever is there and download from zero.
    Fresh,
    /// Continue from `from`. `if_range` is the validator the server must still
    /// match; `None` = no validator, and the expected SHA-256 is what will
    /// catch a splice.
    Resume { from: u64, if_range: Option<String> },
}

/// May the leftover be continued at all — and on what guarantee.
///
/// THE SPLICE (audit L3). `resume_verdict` refuses a `206` whose
/// `Content-Range` does not fit, and that closes the attacker's variant. It
/// does NOT close the honest one: a leftover of v0.2.0, a source that now
/// serves a v0.3.0 OF THE SAME SIZE, and a `206` that is perfectly correct
/// FOR v0.3.0 — every header agrees, the sizes add up, and with no published
/// digest (an older release, a catalog entry not yet TOFU'd) nothing ever
/// compares the bytes. The old prefix and the new tail were installed as one
/// binary. The audit called it out from reading `download.rs:561-593`; the
/// test `a_leftover_from_another_version_is_never_spliced` builds it over a
/// socket.
///
/// So a leftover is only continued when one of two things will notice a
/// different object:
/// * the record written when it was started names the SAME plan and holds a
///   strong ETag — sent back as `If-Range`, so the server itself answers
///   `200` (whole object, restart) when the object changed; or
/// * the plan carries an expected SHA-256 — a splice then fails the digest,
///   is deleted, and the next run starts clean. It costs one wasted transfer;
///   it never installs a splice.
///
/// Anything else is a stale leftover and is deleted.
fn resume_basis(plan: &DownloadPlan, leftover: u64, record: Option<&ResumeRecord>) -> Basis {
    if leftover == 0 {
        return Basis::Fresh;
    }
    // Longer than the object: a dead end (see `SizeVerdict::TooLong`).
    if plan.expected_bytes.is_some_and(|e| leftover > e) {
        return Basis::Fresh;
    }
    match record {
        // A record for ANOTHER plan — another URL, another digest, another
        // size — is the clearest stale case there is.
        Some(r) if !r.describes(plan) => Basis::Fresh,
        Some(ResumeRecord {
            etag: Some(tag), ..
        }) => Basis::Resume {
            from: leftover,
            if_range: Some(tag.clone()),
        },
        _ if plan.expected_sha256.is_some() => Basis::Resume {
            from: leftover,
            if_range: None,
        },
        _ => Basis::Fresh,
    }
}

/// A `206` sent under `If-Range` must carry the ETag that was asked for.
///
/// `If-Range` alone trusts the server to compare; a server — or a cache in
/// front of one — that ignores the header answers `206` for whatever object it
/// now holds. When it names that object, a different name is the proof. A
/// `206` with NO ETag is accepted: the server evaluated the header we sent and
/// gave no reason to doubt it, and a source that never sends one could
/// otherwise never be resumed.
fn same_object(asked: Option<&str>, served: Option<&str>) -> Result<(), String> {
    match (asked, served) {
        (Some(asked), Some(served)) if asked.trim() != served.trim() => Err(format!(
            "the source now serves another version of the file (ETag {served}, the half file was {asked})"
        )),
        _ => Ok(()),
    }
}

/// What the `.downloading` file is a prefix OF, written next to it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResumeRecord {
    url: String,
    /// A STRONG ETag only — a weak one (`W/"…"`) may not be used with
    /// `If-Range` (RFC 9110 §13.1.5), so it is not kept.
    etag: Option<String>,
    sha256: Option<String>,
    bytes: Option<u64>,
}

impl ResumeRecord {
    fn of(plan: &DownloadPlan, served_tag: Option<&str>) -> Self {
        ResumeRecord {
            url: plan.url.clone(),
            etag: served_tag
                .map(str::trim)
                .filter(|t| t.starts_with('"') && t.len() > 2)
                .map(str::to_string),
            sha256: plan
                .expected_sha256
                .as_ref()
                .map(|d| d.to_ascii_lowercase()),
            bytes: plan.expected_bytes,
        }
    }

    fn describes(&self, plan: &DownloadPlan) -> bool {
        self.url == plan.url
            && self.sha256
                == plan
                    .expected_sha256
                    .as_ref()
                    .map(|d| d.to_ascii_lowercase())
            && self.bytes == plan.expected_bytes
    }
}

/// `<target>` -> `<target>.downloading.resume`.
fn resume_record_path(target: &Path) -> PathBuf {
    let mut name = temp_path(target).into_os_string();
    name.push(".resume");
    PathBuf::from(name)
}

/// A record that is missing, unreadable or malformed is `None` — the caller
/// then treats the leftover as having no identity, which is the safe reading.
fn read_record(path: &Path) -> Option<ResumeRecord> {
    let raw = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
    Some(ResumeRecord {
        url: text("url")?,
        etag: text("etag"),
        sha256: text("sha256"),
        bytes: value.get("bytes").and_then(|v| v.as_u64()),
    })
}

fn write_record(path: &Path, record: &ResumeRecord) -> std::io::Result<()> {
    let value = serde_json::json!({
        "url": record.url,
        "etag": record.etag,
        "sha256": record.sha256,
        "bytes": record.bytes,
    });
    fs::write(path, value.to_string())
}

/// Removes a leftover together with its record. Errors are ignored: a file
/// that is already gone is the outcome being asked for.
fn discard(temp: &Path, record: &Path) {
    let _ = fs::remove_file(temp);
    let _ = fs::remove_file(record);
}

fn header(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// What to do with the half-finished file, given the server's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resume {
    /// Append to the leftover, continuing from this offset.
    From(u64),
    /// Truncate and download the whole object.
    Restart,
    /// The `206` cannot be trusted; the leftover is dropped and the run fails
    /// with the reason.
    Untrusted(String),
}

/// Whether the server's answer really agrees to continue OUR half file.
///
/// A PURE FUNCTION, so the decision can be measured without a socket — and it
/// is the decision that used to be wrong. The old code asked ONE question,
/// "is the status 206", and that is not the same as "these bytes belong after
/// mine". A concrete failure that needed no attacker: a `--install` is
/// interrupted at 3 MB of a 8 MB v0.2.0 asset, v0.3.0 (9 MB) is published, the
/// next run asks for `bytes=3145728-`, and the source happily serves the tail
/// of the NEW object. 3 MB of v0.2.0 + 5.86 MB of v0.3.0 = 9 MB, the size check
/// passes, `expected_sha256` is `None` so nothing else looks, and a spliced
/// binary is made executable and moved over the running one.
///
/// So the `Content-Range` is read and three things have to hold: the header
/// EXISTS, its start offset is exactly what we asked for, and — when the server
/// declares one — the total length matches what the catalog declared. The
/// attacker's variant (a `206` whose body starts at 0) fails on the second
/// check.
fn resume_verdict(
    status: u16,
    content_range: Option<&str>,
    asked: u64,
    expected_total: Option<u64>,
) -> Resume {
    if asked == 0 || status != 206 {
        // 200 means the server ignored the Range. Anything else that got this
        // far is not a partial answer either (ureq turns 4xx/5xx into errors).
        return Resume::Restart;
    }
    let Some(raw) = content_range else {
        return Resume::Untrusted("the server sent a 206 with no Content-Range header".into());
    };
    let Some((start, _end, total)) = parse_content_range(raw) else {
        return Resume::Untrusted(format!(
            "the Content-Range header was not understood: {raw}"
        ));
    };
    if start != asked {
        return Resume::Untrusted(format!(
            "the server continued from byte {start}, not from {asked}"
        ));
    }
    if let (Some(total), Some(expected)) = (total, expected_total)
        && total != expected
    {
        return Resume::Untrusted(format!(
            "the object at the source is {total} bytes, the catalog declares {expected}"
        ));
    }
    Resume::From(start)
}

/// What the downloaded length says about the file on disk.
///
/// A SEPARATE, PURE FUNCTION because the two failing directions call for
/// OPPOSITE handling of the leftover, and that difference is exactly what the
/// old single `!=` comparison hid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeVerdict {
    /// The size matches, or the catalog declared none to compare against.
    Exact,
    /// Fewer bytes than declared: a truncated transfer, which resume fixes.
    TooShort,
    /// MORE bytes than declared: the leftover can never be continued.
    TooLong,
}

fn size_verdict(expected: Option<u64>, downloaded: u64) -> SizeVerdict {
    match expected {
        None => SizeVerdict::Exact,
        Some(expected) if downloaded > expected => SizeVerdict::TooLong,
        Some(expected) if downloaded < expected => SizeVerdict::TooShort,
        Some(_) => SizeVerdict::Exact,
    }
}

/// `bytes 3145728-9437183/9437184` -> `(3145728, 9437183, Some(9437184))`.
/// A `*` total (the server does not know the length) comes back as `None`.
fn parse_content_range(raw: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = raw.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.trim().split_once('-')?;
    let start = start.trim().parse().ok()?;
    let end = end.trim().parse().ok()?;
    let total = match total.trim() {
        "*" => None,
        value => Some(value.parse().ok()?),
    };
    Some((start, end, total))
}

/// `<target>` -> `<target>.downloading`. The extension is ADDED, not replaced:
/// `model.gguf` -> `model.gguf.downloading`. Had `set_extension` been used it
/// would produce `model.downloading` and two different package files could land
/// on the same temp name.
pub fn temp_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_os_string();
    name.push(".");
    name.push(TEMP_EXTENSION);
    PathBuf::from(name)
}

fn file_size(path: &Path) -> DownloadResult<u64> {
    path.metadata()
        .map(|m| m.len())
        .map_err(|e| DownloadError::File(format!("{}: {e}", path.display())))
}

/// `Content-Length` — the length of the body (of the REMAINING part if it is a
/// Range response).
fn body_length(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    response
        .headers()
        .get("content-length")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The scheme gate. There is NO local network exception (see the top of the
/// file); `plain_loopback` is the tests' transport only (see `Transport`).
fn validate_address(url: &str, plain_loopback: bool) -> DownloadResult<()> {
    if plain_loopback && url.starts_with("http://127.0.0.1:") {
        return Ok(());
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(DownloadError::InvalidAddress(format!(
            "the address must start with https://: {url}"
        )));
    };
    if rest.is_empty() || rest.starts_with('/') {
        return Err(DownloadError::InvalidAddress("no host name".into()));
    }
    Ok(())
}

/// Converts a `ureq` error into a `WebError` — it uses the SAME mapping as the
/// translation in `client.rs`. Written separately, the same failure would be
/// described with two different sentences.
fn network_error(error: &ureq::Error) -> WebError {
    match error {
        ureq::Error::StatusCode(code) => WebError::ServerCode(*code),
        ureq::Error::Timeout(_) => WebError::Timeout,
        other => WebError::Unreachable(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// SHA-256
// ---------------------------------------------------------------------------

/// The file's SHA-256 digest (lowercase hexadecimal).
///
/// NO NETWORK, fully local: `model verify` runs this offline.
pub fn digest_from_file(path: &Path) -> DownloadResult<String> {
    let mut f = fs::File::open(path)
        .map_err(|e| DownloadError::File(format!("{}: {e}", path.display())))?;
    let mut h = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        let n = f
            .read(&mut buffer)
            .map_err(|e| DownloadError::File(format!("{}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        h.feed(&buffer[..n]);
    }
    Ok(hex(&h.finish()))
}

/// The SHA-256 digest of in-memory data.
pub fn digest(data: &[u8]) -> String {
    sha256_hex(data)
}

/// The digest as lowercase hex, for callers OUTSIDE this module (the receipt
/// chain hashes its entries with it). RE-EXPORTED, NOT REIMPLEMENTED: the hash
/// itself lives in `tacet_kernel::hash`, where the download verifier, the
/// receipt chain and the MCP client's PKCE challenge all reach the same proven
/// core. Pure computation: re-exporting it does not widen the network monopoly.
pub use tacet_kernel::sha256_hex;

use tacet_kernel::Sha256;
use tacet_kernel::hash::hex;

#[cfg(test)]
mod tests {
    use super::*;

    // NO INTERNET. The tests at the bottom of this module (`loopback`) open a
    // plain-HTTP listener on 127.0.0.1 and drive `download_via` against it —
    // the only way to measure what a real `206`, a body that will not stop and
    // a body that stops moving do to the leftover. Everything above them is
    // measured without a socket: the approval gate, the scheme gate, digest
    // correctness, the temp-name rule.

    /// SHA-256 OFFICIAL TEST VECTORS (FIPS 180-2 appendix B). This test is the
    /// only legitimate justification for a hand-written hash: a wrong constant
    /// or a shifted loop blows up here.
    #[test]
    fn the_official_sha256_vectors_hold() {
        assert_eq!(
            digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // Multi-block (1 000 000 x 'a') — the padding and the length field are
        // measured here.
        let long = vec![b'a'; 1_000_000];
        assert_eq!(
            digest(&long),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    /// Chunked feeding and one-shot feeding must give the SAME digest — a
    /// streaming download is fed chunk by chunk, so this is the real usage path.
    #[test]
    fn chunked_feeding_gives_the_same_digest() {
        let data: Vec<u8> = (0u8..=255).cycle().take(200_000).collect();
        let one_shot = digest(&data);
        let mut h = Sha256::new();
        for chunk in data.chunks(7) {
            h.feed(chunk);
        }
        assert_eq!(hex(&h.finish()), one_shot);
    }

    /// THE APPROVAL GATE: a refused download DOES NOT GO ONLINE and does not
    /// create the target.
    #[test]
    fn without_approval_there_is_no_download() {
        let dir = std::env::temp_dir().join("tacet-download-approval-test");
        let _ = fs::remove_dir_all(&dir);
        let target = dir.join("model.gguf");
        let plan = DownloadPlan {
            name: "model.gguf".into(),
            // An unreachable address ON PURPOSE: if the approval gate did not
            // work the test would fail with a network error, not with
            // `NotApproved`.
            url: "https://download-test.invalid/model.gguf".into(),
            target: target.clone(),
            expected_bytes: Some(10),
            expected_sha256: None,
        };
        let e = download(&plan, &AlwaysDeny, &SilentProgress)
            .expect_err("the download should have been refused");
        assert_eq!(e, DownloadError::NotApproved);
        assert!(!target.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// The scheme gate comes even BEFORE the approval: plain http is refused
    /// without being asked about.
    #[test]
    fn plain_http_and_a_scheme_less_address_are_rejected() {
        struct AlwaysYes;
        impl DownloadApproval for AlwaysYes {
            fn approve(&self, _p: &DownloadPlan, _b: u64) -> bool {
                panic!("the scheme gate should have run before the approval");
            }
        }
        for address in [
            "http://remote.test/m.gguf",
            "http://localhost:8080/m.gguf",
            "m.gguf",
            "https://",
        ] {
            let plan = DownloadPlan {
                name: "m".into(),
                url: address.into(),
                target: std::env::temp_dir().join("tacet-nonexistent-target.gguf"),
                expected_bytes: None,
                expected_sha256: None,
            };
            let e = download(&plan, &AlwaysYes, &SilentProgress).unwrap_err();
            assert!(
                matches!(e, DownloadError::InvalidAddress(_)),
                "{address}: {e:?}"
            );
        }
    }

    /// If the target is already in place NOTHING GOES ONLINE; the digest is
    /// computed and returned.
    #[test]
    fn an_existing_target_is_digested_without_going_online() {
        let dir = std::env::temp_dir().join("tacet-download-existing-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("model.gguf");
        fs::write(&target, b"abc").unwrap();

        let plan = DownloadPlan {
            name: "model.gguf".into(),
            url: "https://download-test.invalid/model.gguf".into(),
            target: target.clone(),
            expected_bytes: None,
            expected_sha256: Some(
                "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD".into(),
            ),
        };
        // The approval gate is `AlwaysDeny`: had it been called the test would
        // have failed with `NotApproved`.
        let o = download(&plan, &AlwaysDeny, &SilentProgress)
            .expect("an existing file must pass without going online");
        assert!(o.already_present);
        assert!(
            o.digest_verified,
            "an uppercase expected digest must match too"
        );
        assert_eq!(o.bytes, 3);

        // A wrong expected digest: the same path must ERROR.
        let broken = DownloadPlan {
            expected_sha256: Some("00".repeat(32)),
            ..plan
        };
        assert!(matches!(
            download(&broken, &AlwaysDeny, &SilentProgress),
            Err(DownloadError::DigestMismatch { .. })
        ));

        assert_eq!(digest_from_file(&target).unwrap(), digest(b"abc"));
        let _ = fs::remove_dir_all(&dir);
    }

    /// A TRANSPORT ERROR IS TOLD IN THE LANGUAGE OF DOWNLOADING — not of
    /// searching.
    ///
    /// The reason for this test is a measured failure: when `tacet model
    /// download` was run with an invalid host name, the user was told "The
    /// search server could not be reached." Because the variants are shared in
    /// `WebError` the failure was silent — the build was green, the type was
    /// right, the sentence was wrong.
    #[test]
    fn a_network_error_is_told_in_the_language_of_downloading() {
        let text =
            DownloadError::Network(WebError::Unreachable("dns did not resolve".into())).to_string();
        assert!(
            !text.to_lowercase().contains("search"),
            "search wording leaked: {text}"
        );
        // The transport layer's own explanation COMES BACK: in a download it is
        // the first thing the user will look at.
        assert!(
            text.contains("dns did not resolve"),
            "the detail was swallowed: {text}"
        );

        for e in [
            DownloadError::Network(WebError::Timeout),
            DownloadError::Network(WebError::ServerCode(404)),
            DownloadError::Network(WebError::ServerCode(403)),
            DownloadError::Network(WebError::ServerCode(500)),
        ] {
            let m = e.to_string();
            assert!(!m.to_lowercase().contains("search"), "{e:?}: {m}");
            assert!(!m.is_empty());
        }
        // 404 and 403 are SEPARATE sentences: what the user does differs.
        assert_ne!(
            DownloadError::Network(WebError::ServerCode(404)).to_string(),
            DownloadError::Network(WebError::ServerCode(403)).to_string()
        );
    }

    /// THE SCHEME GATE SURVIVES THE REDIRECT.
    ///
    /// `validate_address` only ever sees the first address, so the invariant
    /// that closes a `302 Location: http://…` is the agent's `https_only`
    /// flag — and a flag has no behaviour to observe until a mirror actually
    /// redirects. This asserts the configuration itself, which is what a
    /// silent removal would change.
    #[test]
    fn the_download_agent_refuses_to_leave_https() {
        assert!(
            download_agent(Some(1024)).config().https_only(),
            "a redirect could downgrade the weight transfer to plain text"
        );
    }

    /// THE RESUME DECISION. A 206 is not a promise that the bytes belong after
    /// ours; each case here is a way that promise can be false.
    #[test]
    fn a_206_is_only_trusted_when_the_content_range_agrees() {
        // The honest case.
        assert_eq!(
            resume_verdict(
                206,
                Some("bytes 3145728-9437183/9437184"),
                3_145_728,
                Some(9_437_184)
            ),
            Resume::From(3_145_728)
        );
        // The server does not declare a total: the offset alone still decides.
        assert_eq!(
            resume_verdict(206, Some("bytes 100-199/*"), 100, Some(200)),
            Resume::From(100)
        );

        // THE MEASURED FAILURE: a leftover from v0.2.0 (8 MB) and a v0.3.0
        // (9 MB) at the source. The tail arrives from the WRONG object and the
        // total gives it away — without this check the sizes add up to exactly
        // the expected 9 MB and every later check passes.
        assert!(matches!(
            resume_verdict(
                206,
                Some("bytes 3145728-9437183/9437184"),
                3_145_728,
                Some(8_388_608)
            ),
            Resume::Untrusted(_)
        ));
        // The attacker's variant: a 206 whose body starts somewhere else.
        assert!(matches!(
            resume_verdict(
                206,
                Some("bytes 0-9437183/9437184"),
                3_145_728,
                Some(9_437_184)
            ),
            Resume::Untrusted(_)
        ));
        // A 206 that says nothing about what it is sending.
        assert!(matches!(
            resume_verdict(206, None, 3_145_728, Some(9_437_184)),
            Resume::Untrusted(_)
        ));
        assert!(matches!(
            resume_verdict(206, Some("pages 1-2/9"), 3_145_728, None),
            Resume::Untrusted(_)
        ));

        // 200 = the Range was ignored: start over, do NOT append.
        assert_eq!(
            resume_verdict(200, None, 3_145_728, Some(9_437_184)),
            Resume::Restart
        );
        // Nothing to continue: the header is irrelevant.
        assert_eq!(
            resume_verdict(206, Some("bytes 5-9/10"), 0, None),
            Resume::Restart
        );
    }

    #[test]
    fn the_content_range_header_is_read_the_way_the_rfc_writes_it() {
        assert_eq!(
            parse_content_range("bytes 0-499/1234"),
            Some((0, 499, Some(1234)))
        );
        assert_eq!(
            parse_content_range("bytes 500-999/*"),
            Some((500, 999, None))
        );
        assert_eq!(parse_content_range("bytes */1234"), None);
        assert_eq!(parse_content_range(""), None);
        assert_eq!(parse_content_range("bytes 0-1"), None);
    }

    /// A DEAD END MUST NOT BE LEFT ON DISK. The leftover is only useful if the
    /// next run can continue it; when it is LONGER than the object it can only
    /// produce a 416 forever.
    #[test]
    fn an_oversized_leftover_is_a_dead_end_and_a_short_one_is_not() {
        // TOO LONG is the case that locks the user out: the next run asks for
        // a range past the end of the object and every source answers 416.
        assert_eq!(size_verdict(Some(10), 20), SizeVerdict::TooLong);
        // TOO SHORT is the ordinary interrupted download — the leftover is the
        // whole point of resuming.
        assert_eq!(size_verdict(Some(10), 5), SizeVerdict::TooShort);
        assert_eq!(size_verdict(Some(10), 10), SizeVerdict::Exact);
        // No declared size: there is nothing to contradict.
        assert_eq!(size_verdict(None, 10), SizeVerdict::Exact);
    }

    /// The new error says WHICH problem it is: "the leftover is stale" is not
    /// "the size did not match", and the two send the user to different places.
    #[test]
    fn a_range_mismatch_is_told_apart_from_a_size_mismatch() {
        let range = DownloadError::RangeMismatch {
            asked: 3_145_728,
            detail: "the server continued from byte 0, not from 3145728".into(),
        }
        .to_string();
        let size = DownloadError::SizeMismatch {
            expected: 10,
            found: 20,
        }
        .to_string();
        assert_ne!(range, size);
        assert!(range.contains("3145728"), "{range}");
        assert!(
            range.contains("again"),
            "the user must be told the next run will work: {range}"
        );
    }

    /// The temp name ADDS the extension, it does not replace it: two package
    /// files cannot land on the same temp name.
    #[test]
    fn the_temp_name_adds_the_extension() {
        assert_eq!(
            temp_path(Path::new("/a/model.gguf")),
            PathBuf::from("/a/model.gguf.downloading")
        );
        assert_ne!(
            temp_path(Path::new("/a/model.gguf")),
            temp_path(Path::new("/a/model.json"))
        );
    }

    fn plan_for(url: &str, target: &Path, bytes: Option<u64>, sha: Option<&str>) -> DownloadPlan {
        DownloadPlan {
            name: "m.bin".into(),
            url: url.into(),
            target: target.to_path_buf(),
            expected_bytes: bytes,
            expected_sha256: sha.map(str::to_string),
        }
    }

    /// THE RESUME DECISION, without a socket. Each arm is one way a leftover
    /// can be someone else's prefix.
    #[test]
    fn a_leftover_is_only_continued_when_something_will_notice_another_object() {
        let target = Path::new("/nowhere/m.bin");
        let url = "https://mirror.test/m.bin";
        let plain = plan_for(url, target, Some(16), None);
        let tagged = |tag: Option<&str>| ResumeRecord {
            etag: tag.map(str::to_string),
            ..ResumeRecord::of(&plain, None)
        };

        // A record of the same plan with a strong ETag: resume, under If-Range.
        assert_eq!(
            resume_basis(&plain, 6, Some(&tagged(Some("\"v1\"")))),
            Basis::Resume {
                from: 6,
                if_range: Some("\"v1\"".into())
            }
        );
        // No validator and no digest: nothing would notice a splice.
        assert_eq!(resume_basis(&plain, 6, Some(&tagged(None))), Basis::Fresh);
        assert_eq!(resume_basis(&plain, 6, None), Basis::Fresh);
        // A record for another URL, another size or another digest is stale.
        let moved = plan_for("https://mirror.test/m2.bin", target, Some(16), None);
        assert_eq!(
            resume_basis(&moved, 6, Some(&tagged(Some("\"v1\"")))),
            Basis::Fresh
        );
        let resized = plan_for(url, target, Some(17), None);
        assert_eq!(
            resume_basis(&resized, 6, Some(&tagged(Some("\"v1\"")))),
            Basis::Fresh
        );
        // With an expected digest a splice cannot be INSTALLED, so a leftover
        // with no record (one left by an older tacet) may still be continued.
        let pinned = plan_for(url, target, Some(16), Some("ab"));
        assert_eq!(
            resume_basis(&pinned, 6, None),
            Basis::Resume {
                from: 6,
                if_range: None
            }
        );
        // ...but a record that says the leftover belongs to ANOTHER digest
        // still wins: it is known to be the wrong object.
        assert_eq!(
            resume_basis(&pinned, 6, Some(&tagged(Some("\"v1\"")))),
            Basis::Fresh
        );
        // Nothing, or more than the whole object, is never continued.
        assert_eq!(
            resume_basis(&plain, 0, Some(&tagged(Some("\"v1\"")))),
            Basis::Fresh
        );
        assert_eq!(
            resume_basis(&plain, 17, Some(&tagged(Some("\"v1\"")))),
            Basis::Fresh
        );
    }

    #[test]
    fn a_weak_etag_is_never_kept_as_a_resume_validator() {
        let plan = plan_for("https://m.test/a", Path::new("/x"), None, None);
        assert_eq!(ResumeRecord::of(&plan, Some("W/\"v1\"")).etag, None);
        assert_eq!(ResumeRecord::of(&plan, Some("\"\"")).etag, None);
        assert_eq!(
            ResumeRecord::of(&plan, Some(" \"v1\" ")).etag.as_deref(),
            Some("\"v1\"")
        );
        assert!(same_object(Some("\"v1\""), Some("\"v2\"")).is_err());
        assert!(same_object(Some("\"v1\""), Some("\"v1\"")).is_ok());
        assert!(same_object(Some("\"v1\""), None).is_ok());
        assert!(same_object(None, Some("\"v2\"")).is_ok());
    }

    /// THE BODY HAS A DEADLINE (audit L6), and it grows with the declared
    /// size rather than cutting a large download short.
    #[test]
    fn the_download_agent_puts_a_deadline_on_the_body() {
        let small = download_agent(Some(1024)).config().timeouts().recv_body;
        let large = download_agent(Some(5 * 1024 * 1024 * 1024))
            .config()
            .timeouts()
            .recv_body;
        let unknown = download_agent(None).config().timeouts().recv_body;
        assert!(small.is_some() && large.is_some() && unknown.is_some());
        assert!(large > small, "{large:?} vs {small:?}");
        // 5 GiB at the floor rate must not be cut before it could finish.
        assert!(large.unwrap() >= Duration::from_secs(5 * 1024 * 1024 * 1024 / MIN_RATE));
        // And there is still NO global cap — see the top of the file.
        assert!(download_agent(None).config().timeouts().global.is_none());
    }

    /// Driving `download_via` over a real socket. See the note at the top of
    /// this test module.
    mod loopback {
        use super::*;
        use std::net::{Shutdown, TcpListener, TcpStream};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};

        /// What the server does with one request.
        enum Reply {
            /// Send these bytes and close the connection.
            Close(Vec<u8>),
            /// Send these bytes and then keep the connection open, silent.
            Hold(Vec<u8>),
        }

        struct Server {
            base: String,
            heads: Arc<Mutex<Vec<String>>>,
            stop: Arc<AtomicBool>,
        }

        impl Drop for Server {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::Relaxed);
            }
        }

        impl Server {
            /// The request heads, lower-cased.
            fn heads(&self) -> Vec<String> {
                self.heads.lock().unwrap().clone()
            }
        }

        fn serve(handler: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Server {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let heads = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let handler = Arc::new(handler);
            {
                let heads = Arc::clone(&heads);
                let stop = Arc::clone(&stop);
                thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let heads = Arc::clone(&heads);
                                let stop = Arc::clone(&stop);
                                let handler = Arc::clone(&handler);
                                thread::spawn(move || answer(stream, &*handler, &heads, &stop));
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(2));
                            }
                            Err(_) => return,
                        }
                    }
                });
            }
            Server {
                base: format!("http://127.0.0.1:{port}"),
                heads,
                stop,
            }
        }

        fn answer(
            mut stream: TcpStream,
            handler: &(dyn Fn(&str) -> Reply + Send + Sync),
            heads: &Mutex<Vec<String>>,
            stop: &AtomicBool,
        ) {
            stream.set_nonblocking(false).ok();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => return,
                }
            }
            let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
            heads.lock().unwrap().push(head.clone());
            match handler(&head) {
                Reply::Close(bytes) => {
                    let _ = stream.write_all(&bytes);
                    let _ = stream.flush();
                    let _ = stream.shutdown(Shutdown::Both);
                }
                Reply::Hold(bytes) => {
                    let _ = stream.write_all(&bytes);
                    let _ = stream.flush();
                    let until = Instant::now() + Duration::from_secs(30);
                    while !stop.load(Ordering::Relaxed) && Instant::now() < until {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        }

        fn header_in(head: &str, name: &str) -> Option<String> {
            head.lines()
                .find_map(|l| l.strip_prefix(&format!("{name}:")))
                .map(|v| v.trim().to_string())
        }

        fn full(body: &[u8], etag: &str) -> Vec<u8> {
            let mut r = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: {etag}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            r.extend_from_slice(body);
            r
        }

        fn partial(body: &[u8], from: usize, etag: &str) -> Vec<u8> {
            let mut r = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{}\r\nETag: {etag}\r\nConnection: close\r\n\r\n",
                body.len() - from,
                body.len() - 1,
                body.len()
            )
            .into_bytes();
            r.extend_from_slice(&body[from..]);
            r
        }

        fn range_start(head: &str) -> Option<usize> {
            header_in(head, "range")?
                .strip_prefix("bytes=")?
                .trim_end_matches('-')
                .parse()
                .ok()
        }

        /// The production configuration with ONE change: plain http.
        fn loopback_agent(expected: Option<u64>) -> ureq::Agent {
            download_builder(expected).https_only(false).build().into()
        }

        const LOOPBACK: Transport = Transport {
            agent: loopback_agent,
            plain_loopback: true,
            stall: Duration::from_millis(400),
        };

        /// Approves everything and remembers what it was told was kept.
        struct Yes(Mutex<Vec<u64>>);
        impl DownloadApproval for Yes {
            fn approve(&self, _plan: &DownloadPlan, existing: u64) -> bool {
                self.0.lock().unwrap().push(existing);
                true
            }
        }
        fn yes() -> Yes {
            Yes(Mutex::new(Vec::new()))
        }

        fn scratch(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "tacet-download-loopback-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        const V1: &[u8] = b"OLD-RELEASE-v0.2.0-bytes";
        const V2: &[u8] = b"new-release-v0.3.0-BYTES";

        /// A server that honours `If-Range` the way RFC 9110 says: a range
        /// against a validator that no longer matches gets the WHOLE object.
        fn honest_server(current: &'static [u8], tag: &'static str) -> Server {
            serve(move |head| match range_start(head) {
                Some(from) => match header_in(head, "if-range") {
                    Some(asked) if asked != tag => Reply::Close(full(current, tag)),
                    _ => Reply::Close(partial(current, from, tag)),
                },
                None => Reply::Close(full(current, tag)),
            })
        }

        /// THE SPLICE (audit L3), end to end. A leftover of v0.2.0 sits on
        /// disk; the source now serves a v0.3.0 OF THE SAME SIZE, with no
        /// digest published. Before the fix the run asked for the tail with a
        /// bare `Range`, the server sent a `206` that was correct in every
        /// header, and the result was `OLD-RE` + the tail of v0.3.0 — moved
        /// over the target as a finished download.
        #[test]
        fn a_leftover_from_another_version_is_never_spliced() {
            assert_eq!(V1.len(), V2.len());
            let server = honest_server(V2, "\"v0.3.0\"");
            let url = format!("{}/tacet.bin", server.base);

            // (a) The leftover WITH a record of the old version's ETag.
            let dir = scratch("splice-tagged");
            let target = dir.join("tacet.bin");
            let plan = plan_for(&url, &target, Some(V2.len() as u64), None);
            fs::write(temp_path(&target), &V1[..6]).unwrap();
            write_record(
                &resume_record_path(&target),
                &ResumeRecord::of(&plan, Some("\"v0.2.0\"")),
            )
            .unwrap();
            let outcome = download_via(&plan, &yes(), &SilentProgress, &LOOPBACK)
                .expect("the download must complete");
            assert_eq!(
                fs::read(&target).unwrap(),
                V2,
                "the old prefix was spliced onto the new object"
            );
            assert!(!outcome.resumed);
            assert!(
                !resume_record_path(&target).exists(),
                "the record outlived the download"
            );

            // (b) The leftover with NO record and no digest — nothing could
            // notice a splice, so it must not even be asked for.
            let dir = scratch("splice-bare");
            let target = dir.join("tacet.bin");
            let plan = plan_for(&url, &target, Some(V2.len() as u64), None);
            fs::write(temp_path(&target), &V1[..6]).unwrap();
            let approval = yes();
            download_via(&plan, &approval, &SilentProgress, &LOOPBACK)
                .expect("the download must complete");
            assert_eq!(fs::read(&target).unwrap(), V2, "spliced");
            assert_eq!(
                *approval.0.lock().unwrap(),
                vec![0],
                "the user was told a stale leftover would be kept"
            );
            let last = server.heads().pop().unwrap();
            assert!(
                header_in(&last, "range").is_none(),
                "a stale leftover was resumed: {last}"
            );
            let _ = fs::remove_dir_all(&dir);
            let _ = fs::remove_dir_all(scratch("splice-tagged"));
        }

        /// The honest resume still works — a guard that refused every resume
        /// would pass the splice test and quietly delete the feature.
        #[test]
        fn a_leftover_of_the_same_version_resumes_under_if_range() {
            let server = honest_server(V2, "\"v0.3.0\"");
            let dir = scratch("resume-same");
            let target = dir.join("tacet.bin");
            let plan = plan_for(
                &format!("{}/tacet.bin", server.base),
                &target,
                Some(V2.len() as u64),
                None,
            );
            fs::write(temp_path(&target), &V2[..6]).unwrap();
            write_record(
                &resume_record_path(&target),
                &ResumeRecord::of(&plan, Some("\"v0.3.0\"")),
            )
            .unwrap();
            let approval = yes();
            let outcome = download_via(&plan, &approval, &SilentProgress, &LOOPBACK).unwrap();
            assert!(outcome.resumed);
            assert_eq!(fs::read(&target).unwrap(), V2);
            assert_eq!(*approval.0.lock().unwrap(), vec![6]);
            let head = server.heads().pop().unwrap();
            assert_eq!(header_in(&head, "range").as_deref(), Some("bytes=6-"));
            assert_eq!(header_in(&head, "if-range").as_deref(), Some("\"v0.3.0\""));
            let _ = fs::remove_dir_all(&dir);
        }

        /// A server — or a cache in front of it — that ignores `If-Range` and
        /// serves a `206` of whatever it now holds. The ETag it names gives
        /// it away.
        #[test]
        fn a_206_that_names_another_etag_is_refused_and_the_leftover_dropped() {
            let server = serve(|head| {
                Reply::Close(partial(V2, range_start(head).unwrap_or(0), "\"v0.3.0\""))
            });
            let dir = scratch("resume-other-tag");
            let target = dir.join("tacet.bin");
            let plan = plan_for(
                &format!("{}/tacet.bin", server.base),
                &target,
                Some(V2.len() as u64),
                None,
            );
            fs::write(temp_path(&target), &V1[..6]).unwrap();
            write_record(
                &resume_record_path(&target),
                &ResumeRecord::of(&plan, Some("\"v0.2.0\"")),
            )
            .unwrap();
            let e = download_via(&plan, &yes(), &SilentProgress, &LOOPBACK).unwrap_err();
            assert!(matches!(e, DownloadError::RangeMismatch { .. }), "{e:?}");
            assert!(!target.exists());
            assert!(!temp_path(&target).exists());
            assert!(!resume_record_path(&target).exists());
            let _ = fs::remove_dir_all(&dir);
        }

        /// With a published digest a leftover with no record may be resumed —
        /// and if it turns out to be another object's prefix, the digest
        /// refuses it and the leftover is gone. A splice is never installed.
        #[test]
        fn with_a_digest_a_splice_is_caught_and_the_honest_resume_kept() {
            let server = serve(|head| {
                Reply::Close(match range_start(head) {
                    Some(from) => partial(V2, from, "\"v0.3.0\""),
                    None => full(V2, "\"v0.3.0\""),
                })
            });
            let url = format!("{}/tacet.bin", server.base);
            let pinned = digest(V2);

            let dir = scratch("digest-honest");
            let target = dir.join("tacet.bin");
            let plan = plan_for(&url, &target, Some(V2.len() as u64), Some(&pinned));
            fs::write(temp_path(&target), &V2[..6]).unwrap();
            let o = download_via(&plan, &yes(), &SilentProgress, &LOOPBACK).unwrap();
            assert!(o.resumed && o.digest_verified);
            assert_eq!(fs::read(&target).unwrap(), V2);
            let _ = fs::remove_dir_all(&dir);

            let dir = scratch("digest-splice");
            let target = dir.join("tacet.bin");
            let plan = plan_for(&url, &target, Some(V2.len() as u64), Some(&pinned));
            fs::write(temp_path(&target), &V1[..6]).unwrap();
            let e = download_via(&plan, &yes(), &SilentProgress, &LOOPBACK).unwrap_err();
            assert!(matches!(e, DownloadError::DigestMismatch { .. }), "{e:?}");
            assert!(!target.exists());
            assert!(
                !temp_path(&target).exists(),
                "the spliced leftover was kept"
            );
            let _ = fs::remove_dir_all(&dir);
        }

        /// THE BODY STOPS ONE BYTE PAST THE DECLARED SIZE (audit L5). The
        /// source declares no length and does not stop; before the fix the
        /// run read until a 32 GiB wall.
        #[test]
        fn a_body_longer_than_declared_stops_one_byte_past_it() {
            let server = serve(|_| {
                let mut r = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
                r.extend(std::iter::repeat_n(b'x', 1024 * 1024));
                Reply::Close(r)
            });
            let dir = scratch("overlong");
            let target = dir.join("m.bin");
            let plan = plan_for(&format!("{}/m.bin", server.base), &target, Some(10), None);
            let e = download_via(&plan, &yes(), &SilentProgress, &LOOPBACK).unwrap_err();
            assert_eq!(
                e,
                DownloadError::SizeMismatch {
                    expected: 10,
                    found: 11
                }
            );
            assert!(
                !temp_path(&target).exists(),
                "an overlong leftover was kept"
            );
            let _ = fs::remove_dir_all(&dir);
        }

        /// THE STALL WATCHDOG (audit L6). Headers, five bytes, then silence on
        /// an open connection. Before the fix this never returned.
        #[test]
        fn a_body_that_stops_moving_ends_the_run_and_keeps_the_leftover() {
            let server = serve(|_| {
                let mut r =
                    b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nETag: \"s1\"\r\n\r\n".to_vec();
                r.extend_from_slice(b"12345");
                Reply::Hold(r)
            });
            let dir = scratch("stall");
            let target = dir.join("m.bin");
            let plan = plan_for(&format!("{}/m.bin", server.base), &target, Some(100), None);
            let (done, finished) = mpsc::channel();
            {
                let plan = plan.clone();
                thread::spawn(move || {
                    let _ = done.send(download_via(&plan, &yes(), &SilentProgress, &LOOPBACK));
                });
            }
            let outcome = finished
                .recv_timeout(Duration::from_secs(10))
                .expect("the download hung on a silent body");
            match outcome {
                Err(DownloadError::Stalled { at, .. }) => assert_eq!(at, 5),
                other => panic!("expected a stall, got {other:?}"),
            }
            // The leftover AND its identity stay: the next run resumes them.
            assert_eq!(fs::read(temp_path(&target)).unwrap(), b"12345");
            assert_eq!(
                read_record(&resume_record_path(&target)).and_then(|r| r.etag),
                Some("\"s1\"".into())
            );
            drop(server);
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
