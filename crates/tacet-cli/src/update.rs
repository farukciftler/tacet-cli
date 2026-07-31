//! `tacet update` — replaces this binary with the newest release, after asking
//! replaces this binary with it.
//!
//! NOTHING HERE RUNS UNTIL THE USER SAYS SO. There is no check at start-up and
//! no background ping. A daily check exists, and it is off until the shell has
//! asked — once, after a few turns — and the answer has been written to
//! `config.update.check`.
//!
//! This paragraph used to read "no daily timer", and that sentence was kept true
//! by there being no timer at all. The timer now exists, so the claim narrowed
//! rather than stayed put: what protects the promise is no longer the absence of
//! the feature but the fact that a person turned it on. A silent version check
//! would still be the first thing to make the promise false, and it would be
//! invisible in exactly the way that matters.
//!
//! The network call itself lives in `tacet_web::release` — the monopoly rule.
//! This module asks the question and writes the file; it opens no socket.

use crate::ui::Color;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The repository releases are read from.
pub const REPO: &str = "farukciftler/tacet-cli";

const TIMEOUT: Duration = Duration::from_secs(15);

/// The target triple of the running binary.
///
/// Built from `std::env::consts` rather than a build script: the table is short,
/// and an explicit list makes an unsupported platform say so instead of guessing
/// a triple that has no asset and reporting it as "not published yet".
pub fn host_triple() -> Option<&'static str> {
    Some(match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "macos") => "aarch64-apple-darwin",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        ("x86_64", "windows") => "x86_64-pc-windows-msvc",
        _ => return None,
    })
}

/// The name of the bare-binary asset for this platform.
///
/// The release also carries `.tar.gz`/`.zip` archives for people downloading by
/// hand. The updater deliberately takes the UNCOMPRESSED asset instead: pulling
/// in a tar reader, or shelling out to `tar`, would both be a new dependency —
/// one on a crate, one on whatever happens to be installed on the host.
pub fn asset_name(triple: &str) -> String {
    let suffix = if triple.contains("windows") {
        ".exe"
    } else {
        ""
    };
    format!("tacet-{triple}{suffix}")
}

/// Which inference features this binary was compiled with.
///
/// THE REASON THIS EXISTS IS A REAL LOSS. `cargo install tacet-cli --features
/// metal` produces a binary that runs the model; `tacet update`
/// replaced it with a release asset built WITHOUT those features, and the next
/// launch quietly said "fell back to FakeEngine". Nothing errored, nothing was
/// corrupted — the user simply had a different program with the same name, and
/// the only clue was one grey line above the prompt.
///
/// Printed by `--print-features` so the freshly downloaded binary can be ASKED
/// what it is before it takes over.
pub fn compiled_features() -> &'static str {
    if cfg!(feature = "metal") {
        "metal"
    } else if cfg!(feature = "cuda") {
        "cuda"
    } else if cfg!(feature = "candle") {
        "candle"
    } else {
        "none"
    }
}

/// Ranks a feature string so two binaries can be compared.
///
/// `metal`, `cuda` and `candle` all mean "this one can run a model" — the difference
/// between them is speed/device, and swapping a GPU build for a CPU build is a
/// judgement call rather than a loss. Losing inference entirely is the loss.
fn inference_rank(features: &str) -> u8 {
    match features {
        "metal" | "cuda" => 2,
        "candle" => 2,
        _ => 0,
    }
}

fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Retells a `WebError` in the language of an update check.
///
/// THE SAME DEFECT HAS BEEN SHIPPED HERE BEFORE. `WebError`'s own `Display` was
/// written for web SEARCH, so a 404 from the releases API came out as "the
/// search server did not respond" — a correct type, a green build, and a
/// sentence about a component the user never configured. The error is carried
/// as it is; only the sentence is rebuilt, at the boundary that knows what was
/// being asked.
fn network_text(error: &tacet_web::WebError) -> String {
    use tacet_web::WebError as E;
    match error {
        // 404 is the ordinary case, not a fault: a repository with no published
        // release answers exactly this way.
        E::ServerCode(404) => format!("{REPO} has no published release yet"),
        // GitHub rate-limits anonymous callers per IP. Saying so beats
        // "unreachable", because the fix is to wait rather than to debug.
        E::ServerCode(403) => {
            "GitHub refused the request (403) — likely the anonymous rate limit; try again later"
                .to_string()
        }
        E::ServerCode(code) => format!("the release service answered {code}"),
        E::Timeout => "the release check timed out".to_string(),
        E::Unreachable(detail) => format!("could not reach GitHub: {detail}"),
        E::InvalidJson(detail) => format!("the release payload could not be read: {detail}"),
        other => other.to_string(),
    }
}

/// `tacet update --check` — report only. Returns true when something newer
/// exists. The bare `tacet update` installs; this is the look-only half, kept
/// for scripts and for anyone who wants to read before they write.
pub fn check(color: &Color, quiet: bool) -> Result<bool, String> {
    let release = tacet_web::release::latest(REPO, TIMEOUT).map_err(|e| network_text(&e))?;
    let newer = tacet_web::release::is_newer(&release.tag, current_version());

    if quiet {
        return Ok(newer);
    }

    eprintln!();
    eprintln!(
        "  installed: {}",
        color.paint(crate::ui::BOLD, current_version())
    );
    eprintln!(
        "  latest:    {}",
        color.paint(crate::ui::BOLD, release.tag.trim_start_matches('v'))
    );

    if !newer {
        eprintln!();
        eprintln!("  {}", color.paint(crate::ui::DIM, "up to date."));
        return Ok(false);
    }

    eprintln!();
    if !release.page_url.is_empty() {
        eprintln!("  notes: {}", release.page_url);
    }
    match host_triple() {
        Some(triple) if release.asset_for(&asset_name(triple)).is_some() => {
            eprintln!(
                "  install: {}",
                color.paint(crate::ui::BOLD, "tacet update")
            );
        }
        Some(triple) => {
            // Being precise here matters: "no build for your platform" and
            // "the check failed" are different situations and the user can act
            // on only one of them.
            eprintln!(
                "  {}",
                color.paint(
                    crate::ui::YELLOW,
                    &format!("this release has no binary for {triple} — build from source, or use cargo install tacet-cli")
                )
            );
        }
        None => eprintln!(
            "  {}",
            color.paint(
                crate::ui::YELLOW,
                "no prebuilt binary is published for this platform"
            )
        ),
    }
    Ok(true)
}

/// `tacet update`.
/// What an update run did. BEING UP TO DATE IS NOT AN ERROR — it used to be
/// returned as one, which was harmless while installing needed its own flag and
/// is not now that `tacet update` is the whole command: the most common run of
/// all would have printed a yellow failure and exited non-zero.
pub enum Outcome {
    Updated,
    AlreadyCurrent,
}

pub fn install(color: &Color, no_approval: bool) -> Result<Outcome, String> {
    let release = tacet_web::release::latest(REPO, TIMEOUT).map_err(|e| network_text(&e))?;
    if !tacet_web::release::is_newer(&release.tag, current_version()) {
        return Ok(Outcome::AlreadyCurrent);
    }

    let triple = host_triple().ok_or_else(|| {
        format!(
            "no build is published for this platform ({} {})",
            std::env::consts::ARCH,
            std::env::consts::OS
        )
    })?;
    let wanted = asset_name(triple);
    let asset = release
        .asset_for(&wanted)
        .ok_or_else(|| format!("release {} carries no `{wanted}`", release.tag))?;

    let current = std::env::current_exe().map_err(|e| format!("cannot locate this binary: {e}"))?;
    let staged = staging_path(&current);

    let plan = tacet_web::DownloadPlan {
        name: asset.name.clone(),
        url: asset.url.clone(),
        target: staged.clone(),
        expected_bytes: Some(asset.bytes),
        // THE DIGEST THE RELEASE API PUBLISHES — see `ReleaseAsset::digest`.
        //
        // This line used to say `None`, and the comment above it said GitHub
        // published no per-asset digest. That was wrong: the payload already
        // parsed for the URL and the size carries `"digest": "sha256:…"` for
        // every asset, and a user's transcript showed the updater printing the
        // very same hex string as "computed, NOT verified".
        //
        // WHY IT IS WORTH COMPARING, given that the earlier reasoning about
        // SHA256SUMS still holds: the digest comes from `api.github.com` and the
        // bytes come from the redirect target that serves the asset. Two
        // endpoints, two connections. That is not an attestation against a
        // compromised publisher — nothing shipped alongside an artefact ever is
        // — but it does catch a truncated transfer, a corrupted mirror and a
        // swapped object at the download host, none of which the size check
        // catches on its own.
        //
        // WHEN IT IS `None` (an older release, another forge) the
        // trust-on-first-use path below runs exactly as it did.
        expected_sha256: asset.digest.clone(),
    };

    // A LEFTOVER `.new` FROM AN EARLIER RUN MUST NOT BE INSTALLED. The
    // downloader treats a file that is already the expected size as "already
    // present" and returns WITHOUT asking the approval gate and WITHOUT
    // touching the network — which would silently swap in bytes nobody in this
    // run approved (a failed or interrupted earlier update, or a file another
    // local account dropped next to the binary).
    let _ = std::fs::remove_file(&staged);

    let approval = crate::models::TerminalDownloadApproval {
        color: Color::setup(),
        no_approval,
        // NOT the catalog sentence: see `TOFU_NOTE_NO_PUBLISHER`. There is no
        // catalog on this path and no digest is ever compared.
        no_digest_note: crate::models::TOFU_NOTE_NO_PUBLISHER,
    };
    let progress = crate::models::TerminalDownloadProgress {
        color: Color::setup(),
    };
    let outcome = tacet_web::download(&plan, &approval, &progress)
        .map_err(|e| format!("download failed: {e}"))?;

    make_executable(&staged)?;

    // ASK THE NEW BINARY WHAT IT IS, BEFORE IT TAKES OVER.
    //
    // The release assets are built from the same source but not necessarily
    // with the same features, and a binary that cannot run a model is not an
    // upgrade for someone who installed one that can. This is not guessed from
    // the file name or the tag — the downloaded file is EXECUTED with
    // `--print-features` and asked. If it answers with less than this build
    // has, the swap does not happen at all.
    let incoming = probe_features(&staged);
    let mine = compiled_features();
    if inference_rank(&incoming) < inference_rank(mine) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!(
            "refusing to install: this build has `{mine}` compiled in, the release binary has `{incoming}` \
             — replacing it would silently drop model inference and fall back to the fake engine.\n  \
             update with the features instead:  cargo install tacet-cli --features {mine}"
        ));
    }

    swap(&current, &staged)?;

    eprintln!();
    eprintln!(
        "  {} {} → {}",
        color.paint(crate::ui::BOLD, "installed:"),
        current_version(),
        release.tag.trim_start_matches('v')
    );
    // THE NUMBER TRAVELS WITH WHAT IT IS WORTH. On its own a hex string is
    // indistinguishable from the output of a verified download, and a user who
    // reads it as one stops looking for a verification that may not have
    // happened — which is why the qualifier is printed here and not only on the
    // approval screen, which has long scrolled off.
    //
    // Both sentences are deliberately narrow. The verified one claims a MATCH
    // against what the release published and nothing more; it does not say the
    // publisher is honest. The unverified one stays for releases that carry no
    // digest at all.
    eprintln!(
        "  sha256: {}  {}",
        outcome.sha256,
        if outcome.digest_verified {
            color.paint(
                crate::ui::DIM,
                "(matches the digest published with the release)",
            )
        } else {
            color.paint(
                crate::ui::YELLOW,
                "(computed, NOT verified — no published digest to compare against)",
            )
        }
    );
    eprintln!("  path:   {}", current.display());
    Ok(Outcome::Updated)
}

/// Runs the downloaded binary once, just to ask what it was built with.
///
/// It is already on disk and about to become the program the user runs, so
/// executing it here adds no trust that installing it would not. The call is
/// given a short leash and a failure reads as "unknown", which ranks as no
/// inference — the safe direction, because the cost of a false alarm is a
/// message and the cost of a false pass is a silently degraded install.
fn probe_features(path: &Path) -> String {
    match std::process::Command::new(path)
        .arg("--print-features")
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        // An older release predates the flag and exits with a usage error. That
        // is exactly the build that has no features to report, so "unknown" is
        // the honest reading rather than a special case.
        _ => "unknown".to_string(),
    }
}

/// Where the new binary is written before it takes over.
///
/// Next to the current binary, not in the temp directory: the swap below is a
/// rename, and a rename is only atomic within one filesystem. `/tmp` is very
/// often a different one, which would silently turn the swap into a copy —
/// and a half-copied binary is a broken install rather than a failed one.
fn staging_path(current: &Path) -> PathBuf {
    let mut name = current.file_name().unwrap_or_default().to_os_string();
    name.push(".new");
    current.with_file_name(name)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| format!("cannot read the downloaded file: {e}"))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| format!("cannot set the execute bit: {e}"))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    // Windows decides by extension, not by a permission bit. NOT MEASURED: no
    // part of this file has ever been run on Windows.
    Ok(())
}

/// Puts the new binary in place of the running one.
#[cfg(unix)]
fn swap(current: &Path, staged: &Path) -> Result<(), String> {
    // Renaming over a running binary is legal on Unix: the running process
    // keeps the old inode open, and the next launch gets the new file.
    std::fs::rename(staged, current).map_err(|e| format!("could not replace the binary: {e}"))
}

#[cfg(not(unix))]
fn swap(current: &Path, staged: &Path) -> Result<(), String> {
    // Windows refuses to overwrite a running executable, so the running one is
    // moved aside first. The leftover `.old` is removed on the next run — it
    // cannot be deleted now, because it is the file currently executing.
    // NOT MEASURED on Windows.
    let old = current.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(current, &old)
        .map_err(|e| format!("could not move the running binary aside: {e}"))?;
    if let Err(e) = std::fs::rename(staged, current) {
        let _ = std::fs::rename(&old, current);
        return Err(format!("could not replace the binary: {e}"));
    }
    Ok(())
}

/// Removes the `.old` left behind by a previous Windows update. Called at
/// start-up; a no-op everywhere else.
pub fn sweep_previous() {
    if cfg!(unix) {
        return;
    }
    if let Ok(current) = std::env::current_exe() {
        let _ = std::fs::remove_file(current.with_extension("old"));
    }
}

// ── The daily check and the one-time offer ────────────────────────────────
//
// Everything below is off unless the user turned it on. `config.update.check`
// starts as `ask`, the shell offers once after a few turns, and the answer is
// written down so the question is never repeated.

/// The turn the offer appears on.
///
/// Not the first: someone who just typed `tacet` for the first time is trying to
/// see whether this thing works, and a question about update policy is noise
/// before the answer to that. By the third turn they have used it and can decide
/// with context.
const OFFER_AFTER_TURNS: usize = 3;

const CACHE_FILE: &str = "update.json";
/// A day. Short enough to be useful, long enough that a shell opened twenty
/// times in a morning still makes one request.
const CHECK_EVERY_SECS: u64 = 24 * 60 * 60;

fn cache_path() -> Option<PathBuf> {
    tacet_kernel::config_path(CACHE_FILE)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `(checked_at, latest_tag)` from the cache; zeros when absent or unreadable.
fn read_cache() -> (u64, String) {
    let Some(path) = cache_path() else {
        return (0, String::new());
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return (0, String::new());
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (0, String::new());
    };
    (
        value
            .get("checked_at")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        value
            .get("latest")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    )
}

fn write_cache(latest: &str) {
    let Some(path) = cache_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::json!({ "checked_at": now_secs(), "latest": latest });
    let _ = std::fs::write(path, body.to_string() + "\n");
}

/// The line to print at the end of a session, or `None`.
///
/// THE CHECK IS THROTTLED BY THE CACHE, not by the print. Without that, a shell
/// opened twenty times would make twenty requests and the "once a day" in the
/// setting's description would be a lie.
///
/// A failed check returns `None` and says nothing. This is a courtesy, not a
/// task: printing "the update check failed" would send the user to investigate a
/// problem they do not have.
pub fn daily_notice(color: &Color) -> Option<String> {
    if crate::config::update_policy() != crate::config::UpdatePolicy::On {
        return None;
    }
    let (checked_at, cached) = read_cache();
    let latest = if now_secs().saturating_sub(checked_at) < CHECK_EVERY_SECS {
        cached
    } else {
        let fetched = tacet_web::release::latest(REPO, TIMEOUT).ok()?.tag;
        write_cache(&fetched);
        fetched
    };
    if latest.is_empty() || !tacet_web::release::is_newer(&latest, current_version()) {
        return None;
    }
    Some(format!(
        "  {} {} → {} · {}",
        color.paint(crate::ui::DIM, "update available:"),
        current_version(),
        latest.trim_start_matches('v'),
        color.paint(crate::ui::DIM, "tacet update")
    ))
}

/// Offers the check once, on the `OFFER_AFTER_TURNS`th turn. Returns `true` when
/// a question was actually asked, so the caller can redraw its prompt.
///
/// Asked, not assumed, in either direction. Defaulting to on would make the
/// promise that this program stays off the network false in a way nobody would
/// see; defaulting to off silently means the feature does not exist for the
/// people who would want it.
pub fn maybe_offer(color: &Color, turns: usize) -> bool {
    if turns != OFFER_AFTER_TURNS
        || crate::config::update_policy() != crate::config::UpdatePolicy::Ask
        || !std::io::IsTerminal::is_terminal(&std::io::stdin())
    {
        return false;
    }
    eprintln!();
    eprintln!(
        "  {}",
        color.paint(
            crate::ui::DIM,
            "Tacet does not contact any server on its own. It can look for a new\n  \
             version once a day — that is one request to GitHub, and nothing else."
        )
    );
    let yes = crate::ui::ask_yes_no(color, "  Check for updates daily?");
    match crate::config::set_update_policy(yes) {
        Ok(()) => eprintln!(
            "  {}",
            color.paint(
                crate::ui::DIM,
                if yes {
                    "on — change it with: tacet config set update.check off"
                } else {
                    "off — change it with: tacet config set update.check on"
                }
            )
        ),
        // The setting could not be stored, so the question would come back every
        // session. Saying so beats asking again tomorrow with no explanation.
        Err(e) => eprintln!(
            "  {}",
            color.paint(crate::ui::YELLOW, &format!("not saved: {e}"))
        ),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE UPDATE PATH MUST NOT CLAIM A TRUST-ON-FIRST-USE CHAIN IT DOES NOT
    /// HAVE.
    ///
    /// The binary download shares its approval gate with the model download,
    /// and the model sentence ("no sha256 in the catalog … first trust") is
    /// false twice over here: there is no catalog to write a digest into, and
    /// every release has a different digest, so the comparison never happens on
    /// any later run either. The `sha256:` line printed after the install is
    /// indistinguishable from a verified one, so the words have to carry the
    /// difference.
    #[test]
    fn the_update_gate_does_not_promise_a_later_verification() {
        let note = crate::models::TOFU_NOTE_NO_PUBLISHER;
        assert_ne!(
            note,
            crate::models::TOFU_NOTE_CATALOG,
            "the update path reuses the catalog sentence"
        );
        let lowered = note.to_lowercase();
        assert!(
            !lowered.contains("catalog"),
            "there is no catalog on this path: {note}"
        );
        assert!(
            !lowered.contains("first trust"),
            "no later download is verified, so nothing is 'trusted first': {note}"
        );
        assert!(
            lowered.contains("not compared") || lowered.contains("not verified"),
            "the note has to say the digest is not checked: {note}"
        );
    }

    #[test]
    fn the_asset_name_carries_no_version() {
        // The release also holds `tacet-v0.2.0-<triple>.tar.gz`. If the bare
        // asset carried the version too, matching it would mean reconstructing
        // the release's version string here — and the archive would match the
        // same needle.
        let name = asset_name("aarch64-apple-darwin");
        assert_eq!(name, "tacet-aarch64-apple-darwin");
        assert!(!name.contains("0."));
    }

    #[test]
    fn the_windows_asset_keeps_its_extension() {
        assert_eq!(
            asset_name("x86_64-pc-windows-msvc"),
            "tacet-x86_64-pc-windows-msvc.exe"
        );
    }

    #[test]
    fn the_bare_asset_does_not_match_the_archive_of_the_same_platform() {
        // The property the naming scheme exists for.
        let archive = "tacet-v0.2.0-aarch64-apple-darwin.tar.gz";
        assert!(!archive.contains(&asset_name("aarch64-apple-darwin")));
    }

    #[test]
    fn staging_sits_beside_the_binary_not_in_temp() {
        // A rename is atomic only within one filesystem; `/tmp` is usually a
        // different one and the swap would silently degrade to a copy.
        let current = Path::new("/usr/local/bin/tacet");
        let staged = staging_path(current);
        assert_eq!(staged.parent(), current.parent());
        assert_eq!(staged.file_name().unwrap(), "tacet.new");
    }

    #[test]
    fn this_host_has_a_triple() {
        // If this fails on a platform we build for, `--install` there would
        // report "not published" for a release that does carry the asset.
        assert!(host_triple().is_some(), "no triple mapped for this host");
    }
}

#[cfg(test)]
mod downgrade_guard_tests {
    use super::*;

    /// The loss this guard exists to prevent: a build that can run a model must
    /// not be replaced by one that cannot.
    #[test]
    fn losing_inference_ranks_lower() {
        assert!(inference_rank("none") < inference_rank("metal"));
        assert!(inference_rank("none") < inference_rank("cuda"));
        assert!(inference_rank("none") < inference_rank("candle"));
        assert!(inference_rank("unknown") < inference_rank("candle"));
    }

    /// metal <-> candle is a speed choice, not a loss, so the guard must not
    /// block it — otherwise a Linux user could never take a Linux release.
    #[test]
    fn swapping_gpu_for_cpu_is_not_a_downgrade() {
        assert_eq!(inference_rank("metal"), inference_rank("candle"));
        assert_eq!(inference_rank("cuda"), inference_rank("candle"));
    }

    /// An old release predates `--print-features` and exits with a usage error;
    /// that reads as "unknown", which is exactly the build with nothing to
    /// report. The safe direction: a false alarm costs a message, a false pass
    /// costs the user their inference.
    #[test]
    fn an_unreadable_answer_is_treated_as_no_inference() {
        assert_eq!(inference_rank("unknown"), 0);
        assert_eq!(inference_rank(""), 0);
    }

    /// The flag must report what the crate was actually compiled with, or the
    /// guard compares two strings that mean nothing.
    #[test]
    fn the_reported_feature_matches_the_build() {
        let reported = compiled_features();
        if cfg!(feature = "metal") {
            assert_eq!(reported, "metal");
        } else if cfg!(feature = "cuda") {
            assert_eq!(reported, "cuda");
        } else if cfg!(feature = "candle") {
            assert_eq!(reported, "candle");
        } else {
            assert_eq!(reported, "none");
        }
    }
}
