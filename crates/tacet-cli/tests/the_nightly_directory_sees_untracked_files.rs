//! The overnight run's directory says "dirty" whenever its journal stamp does.
//!
//! WHY. `scripts/nightly-eval.sh` names the run's directory after the date and
//! the commit, with `-dirty` for uncommitted edits, and the journal inside it is
//! stamped by `BuildProvenance`, which counts UNTRACKED files as edits — a new
//! module nobody has `git add`ed is code the binary was built from. The script
//! decided "dirty" from `git diff` alone, so a tree whose only edit was a new
//! file got a clean-looking directory and a dirty stamp: the name on disk
//! claimed the run measured the commit, and it did not (audit M24 review).
//!
//! THE SCRIPT ITSELF IS RUN, in a throwaway repository with `HOME` pointed into
//! it. `cargo` is kept off `PATH`, so the script stops at its own "cargo is not
//! on PATH" guard (exit 127) — after it has made the run directory and before it
//! builds or measures anything. Nothing of the user's is read or written.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives at <repo>/crates/tacet-cli")
        .to_path_buf()
}

/// The directory `git` lives in on this machine, so the script can find it on
/// a `PATH` that does not also hold `cargo`.
fn git_dir() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).find(|d| d.join("git").is_file())
}

struct Fixture {
    top: PathBuf,
    path: String,
}

impl Fixture {
    fn new() -> Option<Self> {
        let git = git_dir()?;
        let path = std::env::join_paths([git, "/usr/bin".into(), "/bin".into()])
            .ok()?
            .into_string()
            .ok()?;
        if std::env::split_paths(&path).any(|d| d.join("cargo").is_file()) {
            // The script would get past its guard and start a release build.
            return None;
        }
        let top = std::env::temp_dir().join(format!("tacet-nightly-dirty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        std::fs::create_dir_all(top.join("scripts")).ok()?;
        std::fs::create_dir_all(top.join("home")).ok()?;
        std::fs::copy(
            repo_root().join("scripts/nightly-eval.sh"),
            top.join("scripts/nightly-eval.sh"),
        )
        .ok()?;
        std::fs::write(top.join("a.rs"), "fn a() {}\n").ok()?;
        // `home/` is where the script writes; it must not count as an edit of
        // the fixture's own tree.
        std::fs::write(top.join(".gitignore"), "home/\n").ok()?;
        let f = Fixture { top, path };
        f.git(&["init", "-q"])?;
        f.git(&["add", "."])?;
        f.git(&["commit", "-q", "-m", "one"])?;
        Some(f)
    }

    /// `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_NOSYSTEM` keep the reader's own git
    /// configuration — a signing key, a hook path — out of the fixture.
    fn git(&self, args: &[&str]) -> Option<()> {
        Command::new("git")
            .arg("-C")
            .arg(&self.top)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|_| ())
    }

    /// Runs the script and returns the names of the run directories it has made.
    fn run(&self) -> Vec<String> {
        let home = self.top.join("home");
        let out = Command::new("/bin/bash")
            .arg(self.top.join("scripts/nightly-eval.sh"))
            .env_clear()
            .env("PATH", &self.path)
            .env("HOME", &home)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("bash runs");
        assert_eq!(
            out.status.code(),
            Some(127),
            "the script must stop at its missing-cargo guard here, before building \
             anything: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut names: Vec<String> = std::fs::read_dir(home.join(".tacet/nightly"))
            .expect("the script made its output directory")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.top);
    }
}

#[test]
fn an_untracked_file_makes_the_run_directory_dirty() {
    let Some(f) = Fixture::new() else {
        eprintln!("skipped: no git, or git shares a directory with cargo on this machine");
        return;
    };

    // NOT VACUOUS: a clean tree is not called dirty.
    let clean = f.run();
    assert!(
        clean.len() == 1 && !clean[0].ends_with("-dirty"),
        "a clean checkout was named {clean:?}"
    );

    std::fs::write(f.top.join("new.rs"), "fn b() {}\n").expect("writable");
    let after = f.run();
    assert!(
        after.iter().any(|n| n.ends_with("-dirty")),
        "a tree whose only edit is an untracked file was named {after:?} — the \
         journal inside is stamped dirty, and the directory says it measured the \
         commit"
    );
}
