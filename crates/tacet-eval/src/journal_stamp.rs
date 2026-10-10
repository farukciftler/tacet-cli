//! WHAT A JOURNAL'S CASES WERE MEASURED WITH — everything that has to be the
//! same for two of them to belong in one report.
//!
//! WHY THIS IS MORE THAN THE MODEL AND THE CATALOG (audit M24). The stamp was
//! the engine identity, the catalog's tool names and `CARGO_PKG_VERSION`. The
//! version only moves at a release, so a prompt edited, a router rule changed or
//! a case rewritten on the same version left the stamp identical, and a resumed
//! run silently joined cases from the old code to cases from the new — the
//! mixture the journal exists to refuse. `nightly-eval.sh` said "the journal's
//! own stamp would refuse them anyway"; it would not have.
//!
//! So the stamp now also carries:
//!
//! * the COMMIT the source tree was at, and a HASH OF ITS UNCOMMITTED CHANGES.
//!   `-dirty` is a flag, not a hash: two different sets of edits on one commit
//!   are both `-dirty`. The hash covers the diff against `HEAD` and every
//!   untracked, non-ignored file, because a new module is an edit too;
//! * a hash of the TEXT THE MODEL IS SHOWN that no commit pins down for an
//!   installed binary — the system instructions, the last-pass instruction, the
//!   web nudge and the rendered tool descriptions;
//! * the ROUTER OPTIONS (`--budget`, `--force-tool-name`), which change which
//!   tools a case is shown;
//! * one hash PER CASE, of the case as written. Per case rather than one for
//!   the whole list, because `--only` legitimately journals a subset and a later
//!   run of the full list must be able to continue it; what must not happen is a
//!   case file being reused after its case changed.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde::Serialize;
use serde_json::Value;
use tacet_engine::{EngineIdentity, Prompt};
use tacet_kernel::ToolCatalog;
use tacet_kernel::hash::{Sha256, hex};

use crate::tool_selection::SelectionCase;

/// The source tree the measuring binary was built from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BuildProvenance {
    /// `None` when there is no git, or no repository: an installed binary.
    pub commit: Option<String>,
    /// `None` on a clean tree. Otherwise a sha256 over `git diff HEAD` and the
    /// path and contents of every untracked file git does not ignore.
    pub uncommitted_sha256: Option<String>,
}

impl BuildProvenance {
    /// The tree this crate was compiled from, leaving out `journal`.
    ///
    /// `CARGO_MANIFEST_DIR` RATHER THAN THE WORKING DIRECTORY: the question is
    /// which code produced the number, and `tacet eval` can be started from
    /// anywhere. `nightly-eval.sh` rebuilds before it runs, so the tree and the
    /// binary agree; a tree edited after the build makes the stamp differ from
    /// the last run, which refuses — the safe direction.
    ///
    /// THE JOURNAL IS NOT SOURCE. A journal directory inside the checkout —
    /// `tacet eval --journal ./night` run from the repository, a path no
    /// `.gitignore` here covers — is untracked, so its own `identity.json` and
    /// case files went into the hash: the first open stamped the tree without
    /// them, every later open saw a DIFFERENT build and refused to resume.
    /// Measured in a temporary repository before this fix: one journalled run,
    /// reopen, "cannot be resumed: it holds cases measured with a DIFFERENT
    /// build". A clean tree also turned dirty when the first case file landed.
    pub fn of_source_tree(journal: &Path) -> Self {
        Self::of_tree_excluding(Path::new(env!("CARGO_MANIFEST_DIR")), Some(journal))
    }

    /// The provenance of whatever repository contains `dir`.
    pub fn of_tree(dir: &Path) -> Self {
        Self::of_tree_excluding(dir, None)
    }

    /// `of_tree`, with everything under `exclude` left out of the status, the
    /// diff and the untracked files. An `exclude` outside the repository, or
    /// the repository's own top, excludes nothing.
    pub fn of_tree_excluding(dir: &Path, exclude: Option<&Path>) -> Self {
        let git = |args: &[&str]| -> Option<Vec<u8>> {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .ok()?;
            out.status.success().then_some(out.stdout)
        };
        let Some(commit) = git(&["rev-parse", "HEAD"]) else {
            return Self::default();
        };
        let commit = String::from_utf8_lossy(&commit).trim().to_string();
        let top = git(&["rev-parse", "--show-toplevel"])
            .map(|t| String::from_utf8_lossy(&t).trim().to_string())
            .unwrap_or_else(|| dir.to_string_lossy().into_owned());
        // A PATHSPEC, not a filter over the output: git applies it to the
        // status, the diff and the untracked listing alike, so a journal that
        // happens to sit in a tracked directory is left out of all three.
        // `literal` because a directory name is not a glob.
        let mut pathspec: Vec<String> = Vec::new();
        if let Some(rel) = exclude.and_then(|e| inside(Path::new(&top), e)) {
            pathspec.push("--".into());
            pathspec.push(":/".into());
            pathspec.push(format!(":(top,exclude,literal){rel}"));
        }
        let with_spec = |args: &[&str]| -> Vec<String> {
            args.iter()
                .map(|a| (*a).to_string())
                .chain(pathspec.iter().cloned())
                .collect()
        };
        let status_args = with_spec(&["status", "--porcelain", "--untracked-files=all"]);
        let status_args: Vec<&str> = status_args.iter().map(String::as_str).collect();
        let status = git(&status_args).unwrap_or_default();
        if status.is_empty() {
            return Self {
                commit: Some(commit),
                uncommitted_sha256: None,
            };
        }
        let at_top = |args: &[&str]| -> Vec<u8> {
            let args = with_spec(args);
            Command::new("git")
                .arg("-C")
                .arg(&top)
                .args(&args)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| o.stdout)
                .unwrap_or_default()
        };
        let mut h = Sha256::new();
        h.feed(&at_top(&[
            "diff",
            "HEAD",
            "--binary",
            "--no-ext-diff",
            "--no-color",
        ]));
        let untracked = at_top(&["ls-files", "--others", "--exclude-standard", "-z"]);
        for path in untracked.split(|b| *b == 0).filter(|p| !p.is_empty()) {
            // The path and a separator before the contents, so moving bytes
            // from one file's name into another's body cannot collide.
            h.feed(b"\0untracked\0");
            h.feed(path);
            h.feed(b"\0");
            let full = Path::new(&top).join(String::from_utf8_lossy(path).as_ref());
            if let Ok(bytes) = std::fs::read(&full) {
                h.feed(&bytes);
            }
        }
        Self {
            commit: Some(commit),
            uncommitted_sha256: Some(hex(&h.finish())),
        }
    }
}

/// `path` relative to the repository at `top`, with `/` separators, or `None`
/// when it is not strictly inside it.
///
/// BOTH SIDES ARE RESOLVED, because git prints the top with symlinks resolved
/// (`/private/var/...` on macOS) and the caller passes the journal as typed
/// (`/var/...`, or `./night`). The journal need not exist yet — a first open
/// stamps before it creates the directory — so the longest existing ancestor
/// is resolved and the rest appended.
fn inside(top: &Path, path: &Path) -> Option<String> {
    let top = top.canonicalize().ok()?;
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut existing = abs.as_path();
    let mut rest = Vec::new();
    let resolved = loop {
        if let Ok(c) = existing.canonicalize() {
            break rest
                .iter()
                .rev()
                .fold(c, |p: std::path::PathBuf, r| p.join(r));
        }
        rest.push(existing.file_name()?.to_os_string());
        existing = existing.parent()?;
    };
    let rel = resolved.strip_prefix(&top).ok()?;
    let rel: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!rel.is_empty()).then(|| rel.join("/"))
}

/// Everything the cases in one journal must share. See the module header.
pub struct JournalStamp {
    /// Every field but the per-case hashes; compared whole.
    shared: Value,
    /// Case name to the hash of that case as written.
    cases: BTreeMap<String, String>,
    budget: Option<usize>,
    force_tool_name: bool,
    build: BuildProvenance,
}

impl JournalStamp {
    pub fn new(
        identity: &EngineIdentity,
        catalog: &ToolCatalog,
        build: &BuildProvenance,
        cases: &[SelectionCase],
        budget: Option<usize>,
        force_tool_name: bool,
    ) -> Self {
        let names: Vec<&str> = catalog.names();
        let shared = serde_json::json!({
            "identity": identity,
            "catalog": names,
            "case_shape": env!("CARGO_PKG_VERSION"),
            "build": build,
            "prompt_sha256": prompt_sha256(catalog),
            "options": { "budget": budget, "force_tool_name": force_tool_name },
        });
        let cases = cases
            .iter()
            .map(|c| (c.name.clone(), case_sha256(c)))
            .collect();
        Self {
            shared,
            cases,
            budget,
            force_tool_name,
            build: build.clone(),
        }
    }

    /// The source tree the stamp names. Public so the COMMAND that builds a
    /// stamp can be tested, not only this library: a `tacet eval` that passed
    /// `BuildProvenance::default()` here stamped no commit at all and every
    /// test stayed green (audit M24 review).
    pub fn build(&self) -> &BuildProvenance {
        &self.build
    }

    /// The router options the cases were measured with. A run handed a journal
    /// stamped with different ones is a caller bug, and `run_selection_journalled`
    /// says so.
    pub fn options(&self) -> (Option<usize>, bool) {
        (self.budget, self.force_tool_name)
    }

    pub(crate) fn to_value(&self) -> Value {
        let mut v = self.shared.clone();
        v["cases"] = serde_json::to_value(&self.cases).unwrap_or(Value::Null);
        v
    }

    /// Checks `self` against the stamp already in a journal, given the names of
    /// the cases that journal holds a result for. On success returns the stamp to
    /// write back: the old per-case hashes with this run's merged in.
    pub(crate) fn admit(&self, have: &Value, journalled: &[String]) -> Result<Value, String> {
        let mut have_shared = have.clone();
        let have_cases = have_shared
            .as_object_mut()
            .and_then(|o| o.remove("cases"))
            .unwrap_or(Value::Null);
        if have_shared != self.shared {
            let fields: Vec<&str> = [
                "identity",
                "catalog",
                "case_shape",
                "build",
                "prompt_sha256",
                "options",
            ]
            .into_iter()
            .filter(|k| have_shared.get(k) != self.shared.get(k))
            .collect();
            return Err(format!(
                "it holds cases measured with a DIFFERENT {} — model, catalog, build, \
                 prompt or router options",
                if fields.is_empty() {
                    "stamp".to_string()
                } else {
                    fields.join(", ")
                }
            ));
        }
        let have_cases: BTreeMap<String, String> =
            serde_json::from_value(have_cases).unwrap_or_default();
        let stale: Vec<&str> = journalled
            .iter()
            .filter(|name| match self.cases.get(*name) {
                // A result for a case this run does not include is left alone:
                // that is an `--only` run, not a stale one.
                None => false,
                Some(want) => have_cases.get(*name) != Some(want),
            })
            .map(String::as_str)
            .collect();
        if !stale.is_empty() {
            return Err(format!(
                "it holds results for {} case(s) whose definition has changed since they \
                 were measured: {}",
                stale.len(),
                stale.join(", ")
            ));
        }
        let mut merged = have_cases;
        merged.extend(self.cases.iter().map(|(k, v)| (k.clone(), v.clone())));
        let mut out = self.shared.clone();
        out["cases"] = serde_json::to_value(merged).unwrap_or(Value::Null);
        Ok(out)
    }
}

fn case_sha256(case: &SelectionCase) -> String {
    let text = serde_json::to_string(case).unwrap_or_default();
    tacet_kernel::hash::sha256_hex(text.as_bytes())
}

/// The fixed text the model is shown, whatever the case. The per-step parts —
/// the directory census, the guide a message matches — come from the code the
/// commit already pins.
fn prompt_sha256(catalog: &ToolCatalog) -> String {
    let mut h = Sha256::new();
    for part in [
        tacet_engine::SYSTEM_INSTRUCTIONS,
        tacet_engine::FINAL_PASS_INSTRUCTION,
        tacet_engine::WEB_NUDGE,
    ] {
        h.feed(part.as_bytes());
        h.feed(b"\0");
    }
    h.feed(Prompt::new("", "").with_tools(catalog).text().as_bytes());
    hex(&h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary repository with one commit. `GIT_CONFIG_GLOBAL` and
    /// `GIT_CONFIG_NOSYSTEM` keep the reader's own git configuration — a signing
    /// key, a hook path — out of the fixture.
    fn repo(tag: &str) -> Option<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!(
            "tacet-provenance-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let run = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .ok()
                .filter(|o| o.status.success())
        };
        run(&["init", "-q"])?;
        std::fs::write(dir.join("a.rs"), "fn a() {}\n").ok()?;
        run(&["add", "a.rs"])?;
        run(&["commit", "-q", "-m", "one"])?;
        Some(dir)
    }

    #[test]
    fn a_clean_tree_has_a_commit_and_no_uncommitted_hash() {
        let Some(dir) = repo("clean") else {
            eprintln!("skipped: no git on this machine");
            return;
        };
        let p = BuildProvenance::of_tree(&dir);
        assert_eq!(p.commit.as_deref().map(str::len), Some(40));
        assert_eq!(p.uncommitted_sha256, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `-dirty` is a flag; this is a hash. Two different edits on one commit
    /// must not stamp alike, and an untracked file is an edit.
    #[test]
    fn two_different_edits_on_one_commit_stamp_differently() {
        let Some(dir) = repo("dirty") else {
            eprintln!("skipped: no git on this machine");
            return;
        };
        std::fs::write(dir.join("a.rs"), "fn a() { 1; }\n").unwrap();
        let first = BuildProvenance::of_tree(&dir);
        std::fs::write(dir.join("a.rs"), "fn a() { 2; }\n").unwrap();
        let second = BuildProvenance::of_tree(&dir);
        assert_eq!(first.commit, second.commit);
        assert!(first.uncommitted_sha256.is_some());
        assert_ne!(first.uncommitted_sha256, second.uncommitted_sha256);

        std::fs::write(dir.join("new.rs"), "fn b() {}\n").unwrap();
        let third = BuildProvenance::of_tree(&dir);
        assert_ne!(
            second.uncommitted_sha256, third.uncommitted_sha256,
            "a new untracked module changed nothing in the stamp"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A JOURNAL INSIDE THE CHECKOUT CAN BE RESUMED. Before the exclusion its
    /// own `identity.json` and case files were untracked edits, so the second
    /// open always saw a DIFFERENT build. The journal path is given as typed
    /// (through the temp dir's unresolved name) and does not exist at the first
    /// stamp, which is how `tacet eval --journal ./night` meets it.
    #[test]
    fn a_journal_inside_the_checkout_can_be_resumed() {
        use tacet_engine::EngineProvider as _;
        let Some(dir) = repo("journal-inside") else {
            eprintln!("skipped: no git on this machine");
            return;
        };
        let journal = dir.join("eval-journal");
        let identity = tacet_engine::fake::FakeEngine::script(Vec::<String>::new()).identity();
        let catalog = ToolCatalog::new();
        let stamp = |build: &BuildProvenance| {
            JournalStamp::new(&identity, &catalog, build, &[], None, false)
        };

        let first = BuildProvenance::of_tree_excluding(&dir, Some(&journal));
        assert_eq!(first.uncommitted_sha256, None, "the fixture tree is clean");
        crate::tool_selection::CaseJournal::open(&journal, &stamp(&first))
            .expect("a new journal opens");
        // A finished case, the way the run leaves one.
        std::fs::write(journal.join("some-case.json"), "{}").unwrap();

        let second = BuildProvenance::of_tree_excluding(&dir, Some(&journal));
        assert_eq!(
            second, first,
            "the journal's own files changed the build it is stamped with"
        );
        if let Err(e) = crate::tool_selection::CaseJournal::open(&journal, &stamp(&second)) {
            panic!("a journal inside the checkout could not be resumed: {e}");
        }

        // NOT VACUOUS: an edit OUTSIDE the journal still moves the stamp.
        std::fs::write(dir.join("a.rs"), "fn a() { 3; }\n").unwrap();
        let edited = BuildProvenance::of_tree_excluding(&dir, Some(&journal));
        assert!(
            edited.uncommitted_sha256.is_some(),
            "a source edit was excluded too"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_repository_is_no_provenance_rather_than_an_error() {
        let dir =
            std::env::temp_dir().join(format!("tacet-provenance-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A temporary directory inside somebody's checkout would find that
        // checkout; the assertion is only meaningful outside one.
        if Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .is_ok_and(|o| o.status.success())
        {
            eprintln!("skipped: the temporary directory is inside a repository");
            return;
        }
        assert_eq!(BuildProvenance::of_tree(&dir), BuildProvenance::default());
    }
}
