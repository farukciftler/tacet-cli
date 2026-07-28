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
//! NOT MEASURED: downloading from a real server, resuming with Range and the
//! 200/206 distinction WERE NOT RUN ON THIS MACHINE — the module has no network
//! test (deliberately: CI must not depend on the network) and no manual
//! download was done either. What IS measured is the principles: the approval
//! gate, the scheme gate, SHA-256's official test vectors, chunked feeding
//! producing the same digest as whole feeding, the temp-name rule, and an
//! existing target being verified without going online.
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
use std::time::{Duration, Instant};

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
/// `true`. The only place to look in order to verify that by reading is this
/// function's body.
pub fn download(
    plan: &DownloadPlan,
    approval: &dyn DownloadApproval,
    progress: &dyn Progress,
) -> DownloadResult<DownloadOutcome> {
    validate_address(&plan.url)?;

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
    let existing = temp.metadata().map(|m| m.len()).unwrap_or(0);

    // ---- THE NETWORK GATE. ABOVE this line no socket has been opened. ----
    if !approval.approve(plan, existing) {
        return Err(DownloadError::NotApproved);
    }

    let agent = download_agent();

    let mut request = agent.get(&plan.url);
    if existing > 0 {
        // RESUME: ask to continue from the byte where the half file ended.
        request = request.header("Range", format!("bytes={existing}-"));
    }
    // A 416 ON A RESUME MEANS THE HALF FILE IS AT LEAST AS LONG AS THE OBJECT.
    // It can only have come from a stale or an overlong leftover (an older,
    // LARGER release under the same `<exe>.new.downloading` name, or a source
    // that sent more bytes than the catalog declared). Resuming can NEVER
    // succeed from there: every future run asks for the same unsatisfiable
    // range and the user sees nothing but "the source returned an error (416)"
    // forever, with no hint that a file has to be deleted by hand. The
    // leftover is dropped and the download restarts from zero ONCE.
    let (mut response, existing) = match request.call() {
        Ok(response) => (response, existing),
        Err(ureq::Error::StatusCode(416)) if existing > 0 => {
            let _ = fs::remove_file(&temp);
            let response = agent
                .get(&plan.url)
                .call()
                .map_err(|e| DownloadError::Network(network_error(&e)))?;
            (response, 0)
        }
        Err(e) => return Err(DownloadError::Network(network_error(&e))),
    };

    // 206 = the server accepted the Range, the body is the REMAINING part.
    // 200 = the server ignored the Range (or does not know it at all), the body
    //       is the WHOLE thing; the half file is then thrown away and rewritten
    //       from scratch. Not telling those apart meant appending the whole
    //       body AFTER the half file and silently producing a corrupt file.
    //
    // AND THE STATUS CODE ALONE IS NOT ENOUGH — see `resume_verdict`.
    let content_range = response
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let (resumed, start) = match resume_verdict(
        response.status().as_u16(),
        content_range.as_deref(),
        existing,
        plan.expected_bytes,
    ) {
        Resume::From(offset) => (true, offset),
        Resume::Restart => (false, 0),
        Resume::Untrusted(detail) => {
            // THE LEFTOVER IS DROPPED. Keeping it would repeat the same
            // untrusted answer on every run, and this is the one case where
            // the half file is KNOWN not to belong to the object being
            // downloaded.
            let _ = fs::remove_file(&temp);
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

    progress.started(&plan.name, start, total);

    // WRITTEN BY STREAMING, not held in memory: reading 2.5 GB with
    // `read_to_vec` pushes the machine into swap. `limit` is not unlimited but
    // set to a realistic cap (32 GiB) — a malicious server cannot fill the disk
    // forever.
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(32 * 1024 * 1024 * 1024)
        .reader();
    let mut buffer = vec![0u8; CHUNK];
    let mut downloaded = start;
    let mut last_notice = Instant::now();
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| DownloadError::Network(WebError::Unreachable(e.to_string())))?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n])
            .map_err(|e| DownloadError::File(format!("{}: {e}", temp.display())))?;
        downloaded += n as u64;
        if last_notice.elapsed() >= PROGRESS_INTERVAL {
            progress.advanced(downloaded, total);
            last_notice = Instant::now();
        }
    }
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
                let _ = fs::remove_file(&temp);
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
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
    };

    // THE ATOMIC MOVE: `rename` within the same directory, a single operation
    // on POSIX and on Windows.
    fs::rename(&temp, &plan.target)
        .map_err(|e| DownloadError::File(format!("{}: {e}", plan.target.display())))?;

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
fn download_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        // A global cap is DELIBERATELY ABSENT (see the top of the file). The
        // connection and header caps stay: on a dead server we do not wait
        // forever.
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(RESPONSE_TIMEOUT))
        .max_redirects(5)
        // THE SCHEME GATE MUST SURVIVE THE REDIRECT. `validate_address` only
        // sees hop 0; `ureq` enforces the scheme in exactly one place and only
        // when this flag is on, and its default is FALSE. Without it a `302
        // Location: http://…` silently downgrades the weight transfer to plain
        // text — exactly the case the header of this file forbids, and the one
        // TOFU cannot catch, because TOFU accepts whatever arrives first.
        .https_only(true)
        .build()
        .into()
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
/// file).
fn validate_address(url: &str) -> DownloadResult<()> {
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
    let mut h = Sha256::new();
    h.feed(data);
    hex(&h.finish())
}

fn hex(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// SHA-256 (FIPS 180-4).
///
/// NO DEPENDENCY WAS ADDED, and this is NOT THE OPPOSITE of the `ureq`/TLS
/// decision, it is the same one. The criterion there was: "when written by
/// hand, is what comes out a security hole, or a closed subset of a few hundred
/// lines". TLS was the first (chain validation, cipher suites, handshake
/// states). SHA-256 is the second: its input is a byte array, its output is 32
/// bytes, its specification is one page and its CORRECTNESS CAN BE PROVEN WITH
/// THE OFFICIAL TEST VECTORS (see the tests). A wrongly written hash does not
/// silently say "valid"; the test vector blows up on the first run.
struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    filled: usize,
    total_bytes: u64,
}

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// The digest as lowercase hex, for callers OUTSIDE this module. The receipt
/// chain (`tacet log`) hashes its entries with the same hand-written, test
/// vector proven core the download verifier uses — one implementation, both
/// jobs. Pure computation: exporting it does not widen the network monopoly.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.feed(bytes);
    hex(&h.finish())
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: H0,
            buffer: [0u8; 64],
            filled: 0,
            total_bytes: 0,
        }
    }

    fn feed(&mut self, mut data: &[u8]) {
        self.total_bytes = self.total_bytes.wrapping_add(data.len() as u64);
        if self.filled > 0 {
            let n = data.len().min(64 - self.filled);
            self.buffer[self.filled..self.filled + n].copy_from_slice(&data[..n]);
            self.filled += n;
            data = &data[n..];
            if self.filled == 64 {
                let block = self.buffer;
                process_block(&mut self.state, &block);
                self.filled = 0;
            }
        }
        // WHOLE BLOCKS are processed without being copied into the buffer: on a
        // 2.5 GB file an unnecessary copy is a measurable slowdown.
        while data.len() >= 64 {
            let (block, rest) = data.split_at(64);
            let mut b = [0u8; 64];
            b.copy_from_slice(block);
            process_block(&mut self.state, &b);
            data = rest;
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.filled = data.len();
        }
    }

    fn finish(mut self) -> [u8; 32] {
        let bit_length = self.total_bytes.wrapping_mul(8);
        // Padding: 0x80, then zeros until 8 bytes are left for the length field.
        self.feed(&[0x80]);
        while self.filled != 56 {
            self.feed(&[0x00]);
        }
        // The length DOES NOT GO through `feed`: it would corrupt the
        // `total_bytes` counter, and after the padding the buffer is at exactly
        // 56 bytes anyway.
        self.buffer[56..64].copy_from_slice(&bit_length.to_be_bytes());
        let block = self.buffer;
        process_block(&mut self.state, &block);

        let mut output = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            output[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        output
    }
}

fn process_block(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[4 * i],
            block[4 * i + 1],
            block[4 * i + 2],
            block[4 * i + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let e1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choice = (e & f) ^ ((!e) & g);
        let t1 = h
            .wrapping_add(e1)
            .wrapping_add(choice)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let a0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let t2 = a0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NO NETWORK TEST — no test in this module opens a socket. What is measured
    // is the principle: the approval gate, the scheme gate, digest correctness,
    // the temp-name rule.

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
            download_agent().config().https_only(),
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
}
