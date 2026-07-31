//! `tacet models` — listing what is on disk and fetching what is not.
//!
//! WHY IT IS ITS OWN FILE: everything here is about WEIGHT FILES, and none of
//! it runs during a conversation. It is also the only part of the shell that
//! opens a socket on purpose, which is a good reason for it to have a boundary
//! somebody can see.
//!
//! THE DOWNLOAD IS THE ONLY OUTBOUND PATH IN THIS MODULE, and it is gated three
//! ways before a byte lands: the address is printed first, an approval is asked
//! for, and the file is rejected unless its sha256 matches. The two `TOFU_NOTE`
//! constants below say plainly which of those guarantees is WEAKER when no
//! published digest exists — the honesty is the feature.

use crate::model_package;
use crate::ui::{BOLD, Color, DIM, YELLOW};
use crate::{MODEL_VARIABLE, TOKENIZER_VARIABLE, byte_text};
use std::io::Write;
use std::process::ExitCode;

// ---------------------------------------------------------------------------
// model — model (weight) packages
// ---------------------------------------------------------------------------

/// `tacet models list` — prints the installed model packages.
///
/// WHY IT EXISTS: model discovery was silent. The shell either said
/// "(model: /long/path.gguf)" or "not found"; NOTHING IN BETWEEN was visible —
/// which roots were scanned, what else is there, whether a half package exists,
/// which `.gguf` was picked. This is where the answer to "my folder is right
/// there but it doesn't see it" lives.
///
/// NO NETWORK: this command is entirely local. The remote catalog
/// (`packages.json`) is only READ and which packages have a source is shown; no
/// address is called.
pub fn model_list(json: bool, selected_name: &str) -> ExitCode {
    let color = Color::setup();
    let roots = model_package::model_roots();
    let packages = model_package::scan(&roots);
    let env = model_package::pair_from_env();
    let remote = model_package::read_remote_catalog();

    if json {
        let records: Vec<serde_json::Value> = packages
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "dir": p.dir.display().to_string(),
                    "gguf": p.gguf.display().to_string(),
                    "gguf_bytes": p.gguf_bytes,
                    "tokenizer": p.tokenizer.as_ref().map(|t| t.display().to_string()),
                    // A SEPARATE FIELD rather than a fabricated `tokenizer`
                    // path: there is no file to name, and writing the .gguf's
                    // path into a field called `tokenizer` would make a script
                    // hand that path to `TACET_TOKENIZER`.
                    "gguf_tokenizer": p.gguf_tokenizer,
                    "complete": p.is_complete(),
                    "root": p.root.display().to_string(),
                    // "Selected": if there is an env override NONE is selected —
                    // in that case discovery is not in play at all.
                    "selected": env.is_none() && p.name == selected_name && p.is_complete(),
                })
            })
            .collect();
        let output = serde_json::json!({
            "roots": roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>(),
            "requested": selected_name,
            "env_override": env.as_ref().map(|(m, t)| serde_json::json!({ "gguf": m, "tokenizer": t })),
            "packages": records,
            "remote_catalog": {
                "path": model_package::remote_catalog_path().map(|p| p.display().to_string()),
                "error": remote.as_ref().err(),
                "packages": remote.as_ref().map(|r| r.iter().map(|p| p.name.clone()).collect::<Vec<_>>()).unwrap_or_default(),
                "example": model_package::EXAMPLE_CATALOG,
            },
        });
        println!("{output}");
        return ExitCode::SUCCESS;
    }

    if roots.is_empty() {
        println!("{}", color.paint(DIM, "model root: could not be resolved"));
    } else {
        for r in &roots {
            let note = if r.is_dir() { "" } else { " (missing)" };
            println!(
                "{}",
                color.paint(DIM, &format!("model root: {}{note}", r.display()))
            );
        }
    }
    if let Some((m, t)) = &env {
        // With an override in place the catalog IS NOT SILENCED, but the warning
        // comes first: none of the list below is being used.
        let tokenizer_line = match t {
            Some(t) => format!("  tokenizer : {t}"),
            None => format!("  tokenizer : inside the .gguf ({TOKENIZER_VARIABLE} not set)"),
        };
        println!(
            "{}",
            color.paint(
                YELLOW,
                &format!("{MODEL_VARIABLE} set — discovery disabled:\n  gguf      : {m}\n{tokenizer_line}")
            )
        );
    }
    println!();

    if packages.is_empty() {
        println!("{}", color.paint(DIM, "(no model package installed)"));
    }
    for p in &packages {
        let selected = env.is_none() && p.name == selected_name && p.is_complete();
        let mark = if selected {
            color.paint(BOLD, " ← selected")
        } else {
            String::new()
        };
        println!(
            "{}  {}{}",
            color.paint(BOLD, &p.name),
            color.paint(DIM, &byte_text(p.gguf_bytes)),
            mark
        );
        println!("  {}", color.paint(DIM, &p.gguf.display().to_string()));
        // A HALF PACKAGE IS SAID PLAINLY: the `.gguf` is there but no engine can
        // be set up, and the user would only learn that by trying
        // `--engine candle` and getting an error. A `.gguf` carrying its own
        // vocabulary is NOT half and is no longer described as if it were.
        let note = p.tokenizer_note();
        println!(
            "  {}",
            color.paint(if p.is_complete() { DIM } else { YELLOW }, note)
        );
        println!();
    }

    let complete = packages.iter().filter(|p| p.is_complete()).count();
    println!("{} packages · {complete} usable", packages.len());
    if env.is_none()
        && !packages
            .iter()
            .any(|p| p.name == selected_name && p.is_complete())
    {
        println!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "the requested '{selected_name}' is not usable — chat falls back to FakeEngine"
                )
            )
        );
    }

    // THE REMOTE CATALOG: if the file exists it says what it recognises, if it is
    // broken it DOES NOT STAY SILENT, if it is missing it shows where to write it.
    println!();
    match &remote {
        Err(e) => println!(
            "{}",
            color.paint(YELLOW, &format!("packages.json could not be read: {e}"))
        ),
        Ok(r) if r.is_empty() => match model_package::remote_catalog_path() {
            Some(p) => {
                println!(
                    "{}",
                    color.paint(DIM, &format!("no download catalog: {}", p.display()))
                );
                println!("{}", color.paint(DIM, "shape:"));
                for line in model_package::EXAMPLE_CATALOG.lines() {
                    println!("{}", color.paint(DIM, &format!("  {line}")));
                }
            }
            None => println!(
                "{}",
                color.paint(DIM, "the config directory could not be resolved")
            ),
        },
        Ok(r) => {
            let names: Vec<&str> = r.iter().map(|p| p.name.as_str()).collect();
            println!(
                "{}",
                color.paint(
                    DIM,
                    &format!("in the download catalog: {}", names.join(", "))
                )
            );
            // THE COMMAND NOW EXISTS, which is why it is SUGGESTED. In the
            // previous round this only said "here is what I recognise", because
            // `model download` did not exist and suggesting a nonexistent command
            // would send the user to something that does nothing.
            if let Some(first) = names.first() {
                println!(
                    "{}",
                    color.paint(DIM, &format!("to download: tacet models download {first}"))
                );
            }
        }
    }
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// model download — THE PRODUCTION CALLER of `tacet_web::download`
// ---------------------------------------------------------------------------

/// The terminal end of the download approval gate.
///
/// THIS SIDE ASKS THE QUESTION, `tacet-web` is the side that goes on the network.
/// Without the split, either the network layer would read stdin or the shell
/// would open a socket. `tacet_web::download` does not even set up the ureq agent
/// before this gate returns `true`.
pub struct TerminalDownloadApproval {
    pub color: Color,
    /// `--no-approval`: no question is asked. WHAT IS BEING DOWNLOADED IS STILL
    /// PRINTED — even in script mode the record has to land in the log.
    pub no_approval: bool,
    /// What is said when the plan carries NO expected digest.
    ///
    /// WHY THIS IS A FIELD AND NOT ONE FIXED SENTENCE: two callers share this
    /// gate and the truth differs between them. On the MODEL path there is a
    /// catalog with a `sha256` field, so "first trust" is real — the user can
    /// paste the computed digest into `packages.json` and the next download is
    /// verified. On the UPDATE path there is no catalog, no field to fill, and
    /// every release has a different digest, so verification NEVER happens.
    /// Printing the model sentence there told the user a TOFU chain existed
    /// when none did, which is exactly the belief that makes an unverified
    /// binary look checked.
    pub no_digest_note: &'static str,
}

/// The model path: the catalog HAS a digest field, so first trust is real.
pub const TOFU_NOTE_CATALOG: &str = "no sha256 in the catalog — the downloaded file's digest will be COMPUTED and shown (first trust)";

/// The update path: there is no catalog and no digest to compare against, now
/// or later. SAID PLAINLY, because the closing `sha256:` line looks identical
/// to the output of a verified download.
pub const TOFU_NOTE_NO_PUBLISHER: &str = "no published digest for this binary — its digest will be COMPUTED and SHOWN, NOT COMPARED. Nothing is remembered for next time either: a new version has a new digest, so this download rests on TLS alone";

impl tacet_web::DownloadApproval for TerminalDownloadApproval {
    fn approve(&self, plan: &tacet_web::DownloadPlan, existing_bytes: u64) -> bool {
        let size = match plan.expected_bytes {
            Some(b) => byte_text(b),
            // If the catalog declared no size, NO FAKE NUMBER IS PRODUCED. The
            // user sees "size unknown" and decides; an estimated figure would make
            // the only quantitative fact their approval rests on a fabrication.
            None => "size unknown".to_string(),
        };
        eprintln!();
        eprintln!(
            "  {} {}  ({size})",
            self.color.paint(BOLD, "to download:"),
            plan.name
        );
        eprintln!("    source: {}", plan.url);
        eprintln!("    target: {}", plan.target.display());
        if existing_bytes > 0 {
            eprintln!(
                "    {}",
                self.color.paint(
                    DIM,
                    &format!(
                        "a half file exists: {} — it will be resumed",
                        byte_text(existing_bytes)
                    )
                )
            );
        }
        if plan.expected_sha256.is_none() {
            // TOFU IS SAID PLAINLY. Giving the impression that "the digest was
            // verified" would hide that the first download is unprotected.
            eprintln!("    {}", self.color.paint(YELLOW, self.no_digest_note));
        }
        if self.no_approval {
            eprintln!(
                "    {}",
                self.color
                    .paint(DIM, "--no-approval: downloading without asking")
            );
            return true;
        }
        eprint!("  Download it? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
    }
}

/// The terminal end of the download progress.
///
/// ONE LINE, overwritten with `\r`: a 2.5 GB download printing five lines a
/// second would make the terminal unusable. A speed readout WAS NOT ADDED —
/// showing an unmeasured time estimate ("3 min left") would break this repo's
/// "do not write what you did not measure" rule at the interface level.
pub struct TerminalDownloadProgress {
    pub color: Color,
}

/// The RIGHT side of the progress line. SEPARATE AND PURE: so it can be tested
/// independently of the drawing — what needs measuring is not the caret's place
/// but that NO PERCENTAGE IS PRODUCED when the total is unknown.
pub fn progress_text(downloaded: u64, total: Option<u64>) -> String {
    match total {
        // The `t > 0` condition is not decoration, it is the answer to dividing by
        // zero: on a server declaring length 0 it would print `%NaN`.
        Some(t) if t > 0 => {
            format!(
                "{} / {}  ({:.0}%)",
                byte_text(downloaded),
                byte_text(t),
                (downloaded as f64 / t as f64) * 100.0
            )
        }
        // If the server gave no `Content-Length`, a percentage IS NOT INVENTED.
        _ => byte_text(downloaded),
    }
}

impl TerminalDownloadProgress {
    fn line(&self, name: &str, downloaded: u64, total: Option<u64>) {
        eprint!(
            "\r  {} {}   ",
            self.color.paint(DIM, name),
            progress_text(downloaded, total)
        );
        let _ = std::io::stderr().flush();
    }
}

impl tacet_web::Progress for TerminalDownloadProgress {
    fn started(&self, name: &str, downloaded: u64, total: Option<u64>) {
        self.line(name, downloaded, total);
    }
    fn advanced(&self, downloaded: u64, total: Option<u64>) {
        self.line("", downloaded, total);
    }
    fn digesting(&self, bytes: u64) {
        // IT HAS TO BE REPORTED: the SHA-256 of a GB-sized file takes seconds and
        // if the line stayed at "download finished" the program would look hung.
        eprint!(
            "\r  {}   ",
            self.color
                .paint(DIM, &format!("computing digest ({})…", byte_text(bytes)))
        );
        let _ = std::io::stderr().flush();
    }
    fn finished(&self, _outcome: &tacet_web::DownloadOutcome) {
        eprintln!();
    }
}

/// The root the download lands in: the FIRST of `model_roots()`.
///
/// WHY THE FIRST, NOT "the first directory that exists": `scan` takes its
/// priority order from the same list and if the same name exists in two roots the
/// EARLIER root wins. Downloading into the second root would leave the downloaded
/// package IN THE SHADOW of a half folder with the same name in the first root —
/// the user would say "I downloaded it but it doesn't show up".
pub fn download_root() -> Option<std::path::PathBuf> {
    model_package::model_roots().into_iter().next()
}

/// `tacet models download <name>` — downloads the package from `packages.json`.
///
/// A PRODUCTION CALL: this function really does call `tacet_web::download`. The
/// module sat "tested but not wired" for a whole round; it is written out step by
/// step so whoever looks here can see the entire chain in one place:
/// catalog → package → root → per-file plan → approval → download → digest report.
pub fn model_download(name: &str, no_approval: bool) -> ExitCode {
    let color = Color::setup();
    let catalog = match model_package::read_remote_catalog() {
        Ok(c) => c,
        // A BROKEN CATALOG IS NOT SILENTLY SWALLOWED: if the file EXISTS but
        // cannot be read, saying "package not found" would send the user looking
        // in the wrong place.
        Err(e) => {
            eprintln!(
                "{}",
                color.paint(YELLOW, &format!("packages.json could not be read: {e}"))
            );
            return ExitCode::FAILURE;
        }
    };

    let Some(package) = catalog.iter().find(|p| p.name == name) else {
        eprintln!(
            "{}",
            color.paint(YELLOW, &format!("'{name}' is not in the download catalog"))
        );
        if catalog.is_empty() {
            match model_package::remote_catalog_path() {
                Some(p) => {
                    eprintln!(
                        "{}",
                        color.paint(
                            DIM,
                            &format!("  the catalog is empty or missing: {}", p.display())
                        )
                    );
                    eprintln!("{}", color.paint(DIM, "  shape:"));
                    for line in model_package::EXAMPLE_CATALOG.lines() {
                        eprintln!("{}", color.paint(DIM, &format!("    {line}")));
                    }
                }
                None => eprintln!(
                    "{}",
                    color.paint(DIM, "  the config directory could not be resolved")
                ),
            }
        } else {
            let names: Vec<&str> = catalog.iter().map(|p| p.name.as_str()).collect();
            eprintln!(
                "{}",
                color.paint(DIM, &format!("  in the catalog: {}", names.join(", ")))
            );
        }
        return ExitCode::FAILURE;
    };

    if package.files.is_empty() {
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!("the `files` list of package '{name}' is empty")
            )
        );
        return ExitCode::FAILURE;
    }

    let Some(root) = download_root() else {
        eprintln!(
            "{}",
            color.paint(YELLOW, "the download root could not be resolved")
        );
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "  neither HOME/USERPROFILE nor XDG_DATA_HOME/LOCALAPPDATA resolved — point at the files directly with {}/{}",
                    MODEL_VARIABLE, TOKENIZER_VARIABLE
                )
            )
        );
        return ExitCode::FAILURE;
    };
    let dir = root.join(name);

    println!(
        "{}",
        color.paint(BOLD, &format!("{name} → {}", dir.display()))
    );
    println!(
        "{}",
        color.paint(DIM, &format!("{} files", package.files.len()))
    );

    let approval = TerminalDownloadApproval {
        color: Color::setup(),
        no_approval,
        no_digest_note: TOFU_NOTE_CATALOG,
    };
    let progress = TerminalDownloadProgress {
        color: Color::setup(),
    };
    // TOFU RECORDS: the computed digest of the files that have NO expected digest.
    // Printed together at the end so the user can paste them into
    // `packages.json` — on the second download the verification becomes real.
    let mut tofu: Vec<(String, String)> = Vec::new();

    for f in &package.files {
        let plan = tacet_web::DownloadPlan {
            name: f.name.clone(),
            url: f.url.clone(),
            target: dir.join(&f.name),
            expected_bytes: f.bytes,
            expected_sha256: f.sha256.clone(),
        };
        // DEFENCE IN DEPTH, AND IT IS THE LAST GATE THAT SEES A REAL PATH.
        // `parse_remote_catalog` already refuses a name that is not a plain
        // component, but THIS is the value that reaches the file system, and a
        // download that escapes the model root writes an executable file
        // wherever the user can write (`~/.zshenv` runs at the next shell). The
        // check is cheap and it survives any future edit that builds `target`
        // some other way.
        if !plan.target.starts_with(&dir) {
            eprintln!();
            eprintln!(
                "{}",
                color.paint(
                    YELLOW,
                    &format!(
                        "'{}': the download target falls outside the model directory — refused",
                        f.name
                    )
                )
            );
            return ExitCode::FAILURE;
        }
        match tacet_web::download(&plan, &approval, &progress) {
            Ok(o) => {
                let note = if o.already_present {
                    // NO NETWORK CALL WAS MADE: the file was in place, it was only
                    // digested. A second run of the command is thus a VERIFICATION
                    // round.
                    "already present — no network call"
                } else if o.resumed {
                    "resumed from a half file"
                } else {
                    "downloaded"
                };
                let digest_note = if o.digest_verified {
                    "sha256 verified"
                } else {
                    "sha256 not in the catalog"
                };
                println!(
                    "  {} {}  ({}, {note}, {digest_note})",
                    color.paint(BOLD, "✓"),
                    f.name,
                    byte_text(o.bytes)
                );
                if !o.digest_verified {
                    tofu.push((f.name.clone(), o.sha256.clone()));
                }
            }
            Err(e) => {
                // WE STOP AT THE FIRST FAILURE: carrying on with a half package
                // would download the remaining files too and give the user the
                // impression that it is "ready".
                eprintln!();
                eprintln!("{}", color.paint(YELLOW, &format!("{}: {e}", f.name)));
                return ExitCode::FAILURE;
            }
        }
    }

    if !tofu.is_empty() {
        println!();
        println!(
            "{}",
            color.paint(YELLOW, "these files had no digest in the catalog — the first download WAS NOT VERIFIED (TOFU).")
        );
        println!(
            "{}",
            color.paint(
                DIM,
                "if you write them into packages.json, later downloads are verified:"
            )
        );
        for (file, digest) in &tofu {
            println!(
                "{}",
                color.paint(DIM, &format!("  \"{file}\": \"sha256\": \"{digest}\""))
            );
        }
    }

    // THE RESULT IS STATED IN TERMS OF USABILITY. "Download finished" is not
    // enough: a package missing its `tokenizer.json` sits on disk but cannot set
    // up an engine, and the user would only learn that by trying `--engine candle`
    // and getting an error. The catalog is RESCANNED; the claim comes from the
    // file system.
    println!();
    let rescan = model_package::scan(&[root]);
    match rescan.iter().find(|p| p.name == name) {
        Some(p) if p.is_complete() => {
            println!(
                "{}",
                color.paint(BOLD, &format!("ready: tacet --model {name}"))
            );
            ExitCode::SUCCESS
        }
        Some(_) => {
            println!(
                "{}",
                color.paint(
                    YELLOW,
                    "the package is HALF: no tokenizer.json — it cannot be selected"
                )
            );
            ExitCode::FAILURE
        }
        None => {
            println!(
                "{}",
                color.paint(
                    YELLOW,
                    "the package does not show up in the scan: no `.gguf` file in the folder"
                )
            );
            ExitCode::FAILURE
        }
    }
}
