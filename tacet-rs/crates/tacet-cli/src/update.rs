//! `tacet update` — asks GitHub whether a newer release exists, and on request
//! replaces this binary with it.
//!
//! IT NEVER RUNS BY ITSELF. No check at start-up, no daily timer, no background
//! ping. This program's entire claim is that it does not reach the network
//! unless asked; a silent version check would be the first thing to make that
//! claim false, and it would be invisible in exactly the way that matters.
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

/// `tacet update` — report only. Returns true when something newer exists.
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
                color.paint(crate::ui::BOLD, "tacet update --install")
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

/// `tacet update --install`.
pub fn install(color: &Color, no_approval: bool) -> Result<(), String> {
    let release = tacet_web::release::latest(REPO, TIMEOUT).map_err(|e| network_text(&e))?;
    if !tacet_web::release::is_newer(&release.tag, current_version()) {
        return Err(format!(
            "already on the latest version ({}) — nothing to install",
            current_version()
        ));
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
        // GitHub publishes no per-asset digest in this payload. The release
        // carries a SHA256SUMS file, but verifying the download against a file
        // fetched from the SAME host over the SAME connection proves only that
        // both arrived intact — it is not an independent check, and printing
        // "verified" for it would overstate what happened. The digest is
        // computed and SHOWN instead.
        expected_sha256: None,
    };

    let approval = crate::TerminalDownloadApproval {
        color: Color::setup(),
        no_approval,
    };
    let progress = crate::TerminalDownloadProgress {
        color: Color::setup(),
    };
    let outcome = tacet_web::download(&plan, &approval, &progress)
        .map_err(|e| format!("download failed: {e}"))?;

    make_executable(&staged)?;
    swap(&current, &staged)?;

    eprintln!();
    eprintln!(
        "  {} {} → {}",
        color.paint(crate::ui::BOLD, "installed:"),
        current_version(),
        release.tag.trim_start_matches('v')
    );
    eprintln!("  sha256: {}", outcome.sha256);
    eprintln!("  path:   {}", current.display());
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

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
