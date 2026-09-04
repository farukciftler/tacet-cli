//! FINDING THE WEIGHTS AND OPENING THEM — everything between "the user typed a
//! model name" and "an `EngineProvider` exists".
//!
//! WHY IT IS ITS OWN FILE: none of it runs during a turn, and all of it is
//! about the filesystem and one GGUF header. It is also where the shell's two
//! most confusing failures live — "my folder is right there but it doesn't see
//! it" and "it started, it answered, and the answers were canned" — so the code
//! that has to explain them deserves to be findable.
//!
//! THE FALLBACK IS THE DANGEROUS PART. A binary built without an inference
//! feature still starts and still answers; the answers come from `FakeEngine`.
//! `setup_engine` is the one place that decides whether that is acceptable, and
//! it says so on screen every time it happens.

use crate::ui::{Color, DIM, Screen, TurnIndicator, YELLOW};
use crate::{CANCEL, EngineChoice, host_memory, ui};
use std::sync::Arc;
use tacet_engine::{CONTEXT_BUDGET, EngineProvider, FakeEngine};

// ---------------------------------------------------------------------------
// Engine selection
// ---------------------------------------------------------------------------

/// The variables that point at the model weights.
///
/// The names are NOT written out HERE; they live in two constants — so that the
/// DISCOVERY path (`model_package::pair_from_env`) and the WARNING paths
/// (`model_not_found_report`, `model_list`) are forced to use the same string.
/// When they were written separately, the user could be advised to set a
/// variable name that did not exist.
pub const MODEL_VARIABLE: &str = "TACET_MODEL";
pub const TOKENIZER_VARIABLE: &str = "TACET_TOKENIZER";

// ---------------------------------------------------------------------------
// The model package catalog — discovery, ordering, the remote catalog file
// ---------------------------------------------------------------------------

/// Local model PACKAGES: where they are looked for, which one is picked, what
/// information is shown.
///
/// WHY A SEPARATE MODULE: this whole job used to be a single function called
/// `model_paths` and it had THREE SEPARATE failures. (1) It only looked at
/// `$HOME/models`; since `HOME` does not resolve on Windows it NEVER worked
/// there. (2) It took the "first" `.gguf` in the folder — `read_dir` order
/// depends on the file system, so in a folder holding two weights WHICH ONE got
/// loaded was unpredictable. (3) When it found nothing it printed a one-line
/// guess to the user; it DID NOT SAY which roots it searched, so the answer to
/// "my file is right there but it doesn't see it" existed nowhere.
///
/// NO NETWORK. This whole module is the local file system and environment
/// variables; not one line opens a socket. Downloading is a SEPARATE layer
/// (`tacet_web::download`) and passes the approval gate.
pub mod model_package {
    use super::{MODEL_VARIABLE, TOKENIZER_VARIABLE};
    use std::path::{Path, PathBuf};

    /// The remote package catalog's name inside the config directory.
    pub const CATALOG_FILE: &str = "packages.json";

    /// An installed model package: one `.gguf` and (if present) its tokenizer.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ModelPackage {
        /// The folder name — what the user types with `--model`.
        pub name: String,
        pub dir: PathBuf,
        pub gguf: PathBuf,
        pub gguf_bytes: u64,
        /// `tokenizer.json` SITTING NEXT TO THE WEIGHTS, if the user has one.
        /// `None` no longer means the package is half — see `gguf_tokenizer`.
        pub tokenizer: Option<PathBuf>,
        /// Does the `.gguf` carry its own vocabulary in a shape we can rebuild.
        ///
        /// MEASURED ONCE, AT DISCOVERY, and only when it can change the answer.
        /// `gguf_has_tokenizer` walks the metadata header and stops before the
        /// tensor section (4-6 ms on a 2.5 GB file), but `models list` scans
        /// every package, so a needless read per package would be a visible
        /// pause on a machine holding several weights. When a `tokenizer.json`
        /// is already there the field is left `false` WITHOUT asking: the file
        /// wins anyway, so the answer could not change the outcome.
        pub gguf_tokenizer: bool,
        /// The root this package was found in — the same name can sit in two
        /// roots and the user needs to see WHICH ONE wins.
        pub root: PathBuf,
    }

    impl ModelPackage {
        /// Is it ENOUGH to set up an engine.
        ///
        /// THIS USED TO BE `self.tokenizer.is_some()` AND THAT WAS A BUG, not a
        /// policy: a `.gguf` already carries its vocabulary, its merges and its
        /// special tokens, so a user who downloaded one file by hand was told
        /// their package was half and could not be selected — while the engine
        /// sitting behind this check could have loaded it. The two sides now ask
        /// the same question (`CandleEngine::files_exist` runs exactly this
        /// `gguf_has_tokenizer` call when no `tokenizer.json` is given); if they
        /// disagreed, discovery would refuse packages the loader can handle.
        pub fn is_complete(&self) -> bool {
            self.tokenizer.is_some() || self.gguf_tokenizer
        }

        /// What to print next to the package, in one phrase.
        pub fn tokenizer_note(&self) -> &'static str {
            if self.tokenizer.is_some() {
                "tokenizer: tokenizer.json"
            } else if self.gguf_tokenizer {
                "tokenizer: inside the .gguf"
            } else {
                "tokenizer: MISSING — this package cannot be selected"
            }
        }
    }

    /// The model roots, IN PRIORITY ORDER.
    ///
    /// THIS IS NOT THE CONFIG DIRECTORY and it DELIBERATELY does not tie into
    /// `tacet_kernel::env::config_dir`. That is where SETTINGS live (`mcp.json`,
    /// `memory.json` — kilobytes of text, right to travel with a roaming
    /// profile). Model weights are not a setting, they are GIGABYTES OF DATA;
    /// putting them in `%APPDATA%` (the roaming profile) or `$XDG_CONFIG_HOME`
    /// would bloat the user's network profile or their backup. Hence a separate
    /// notion of a root; not duplication.
    ///
    /// On Unix the second root is `$XDG_DATA_HOME/tacet/models`: in XDG, large
    /// reproducible DATA goes there. XDG's `~/.local/share` default WAS NOT
    /// ADDED — if the user has not set the variable, `~/models` is already first
    /// in line, and creating an unmeasured third directory would make the
    /// question of where packages land even murkier.
    ///
    /// THE WINDOWS ROOTS ARE UNMEASURED: this machine has no rustup and
    /// `cargo check` could not be run for another target. The paths look like
    /// they compile on Windows (they only use `std::env`) but WERE NEVER RUN.
    /// Contrary to the old code this is not a regression but progress: the
    /// previous state searched no root at all on Windows.
    pub fn model_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let mut add = |p: PathBuf| {
            if !roots.contains(&p) {
                roots.push(p);
            }
        };
        if cfg!(windows) {
            if let Some(p) = absolute_env("USERPROFILE") {
                add(p.join("models"));
            }
            if let Some(p) = absolute_env("LOCALAPPDATA") {
                add(p.join("Tacet").join("models"));
            }
        } else {
            if let Some(p) = absolute_env("HOME") {
                add(p.join("models"));
            }
            if let Some(p) = absolute_env("XDG_DATA_HOME") {
                add(p.join("tacet").join("models"));
            }
        }
        roots
    }

    /// An environment variable that is non-empty and carries an ABSOLUTE path.
    ///
    /// A relative value IS IGNORED (the XDG rule): a relative root would tie the
    /// model search to the user's current working directory — opening `tacet`
    /// from another folder would make the model "disappear".
    ///
    /// THE LOCAL COPY WAS DELETED. The same rule lived here and in
    /// `tacet_kernel::env`, and the copies had already drifted: the version
    /// there applied the rule to `XDG_CONFIG_HOME` but not to `TACET_HOME`, the
    /// one variable a user actually types by hand. One home, one rule.
    use tacet_kernel::env::absolute_env;

    /// Scans the packages in the given roots.
    ///
    /// DETERMINISTIC — not the tests' need but THE USER'S:
    /// * packages are sorted BY NAME (`read_dir` order depends on the file
    ///   system),
    /// * if a folder holds several `.gguf` files the first BY FILE NAME is
    ///   picked,
    /// * if the same name exists in two roots the EARLIER root wins and the
    ///   other is dropped.
    ///
    /// All three fix the old "take the first one you find" behaviour: that state
    /// could run the same command with a different weight on two machines (or on
    /// the same machine after adding a file) and silently made measurement
    /// results incomparable.
    pub fn scan(roots: &[PathBuf]) -> Vec<ModelPackage> {
        let mut packages: Vec<ModelPackage> = Vec::new();
        for root in roots {
            let Ok(entries) = std::fs::read_dir(root) else {
                continue; // root missing or unreadable: not an error, "empty"
            };
            let mut candidates: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            candidates.sort();
            for dir in candidates {
                let Some(package) = package_from_dir(root, &dir) else {
                    continue;
                };
                // THE FIRST ROOT WINS: `~/models` is where the user put things by
                // hand, it comes before what was downloaded into the XDG root.
                if packages.iter().any(|p| p.name == package.name) {
                    continue;
                }
                packages.push(package);
            }
        }
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        packages
    }

    /// Turns a single folder into a package. With NO `.gguf` there is no package.
    fn package_from_dir(root: &Path, dir: &Path) -> Option<ModelPackage> {
        let name = dir.file_name()?.to_string_lossy().into_owned();
        let mut ggufs: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file())
            // The extension comparison is CASE INSENSITIVE: downloaded files
            // sometimes arrive as `.GGUF` and the user cannot be expected to know
            // that.
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("gguf"))
            })
            .collect();
        ggufs.sort();
        let gguf = ggufs.into_iter().next()?;
        let gguf_bytes = gguf.metadata().map(|m| m.len()).unwrap_or(0);
        let tok = dir.join("tokenizer.json");
        let tokenizer = tok.is_file().then_some(tok);
        // Asked ONLY when the answer can change the outcome (see the field's
        // note): with a `tokenizer.json` present the file wins either way.
        let gguf_tokenizer = tokenizer.is_none() && tacet_engine::gguf_has_tokenizer(&gguf);
        Some(ModelPackage {
            name,
            dir: dir.to_path_buf(),
            gguf,
            gguf_bytes,
            tokenizer,
            gguf_tokenizer,
            root: root.to_path_buf(),
        })
    }

    /// All installed packages (the default roots).
    pub fn catalog() -> Vec<ModelPackage> {
        scan(&model_roots())
    }

    /// What the engine needs: the weights, and a tokenizer file ONLY IF one was
    /// named. `None` on the second field means "read it out of the GGUF".
    pub type Weights = (String, Option<String>);

    /// The weights given DIRECTLY through environment variables.
    ///
    /// `TACET_TOKENIZER` USED TO BE MANDATORY HERE and it no longer is. The old
    /// rule ("both or neither") existed because half a pair could not load; now
    /// it can, because the vocabulary is in the `.gguf`. What has NOT changed is
    /// the direction of the override: a named `tokenizer.json` still wins, and
    /// if the named file does not exist the load FAILS instead of quietly using
    /// the one inside the weights (`ModelSetting::new`'s own rule — a typo must
    /// not turn into an unexplainable difference in output).
    ///
    /// `TACET_TOKENIZER` ALONE, with no `TACET_MODEL`, is still nothing: there
    /// are no weights to attach it to.
    ///
    /// This branch comes BEFORE the catalog — an explicit request is ahead of
    /// discovery.
    pub fn pair_from_env() -> Option<Weights> {
        let m = tacet_kernel::env_var(MODEL_VARIABLE)?;
        let t = tacet_kernel::env_var(TOKENIZER_VARIABLE);
        Some((
            m.to_string_lossy().into_owned(),
            t.map(|t| t.to_string_lossy().into_owned()),
        ))
    }

    /// The weights for `name` from the given package list.
    ///
    /// SEPARATE AND PURE: so the discovery logic can be tested without touching
    /// environment variables. Environment variables are PROCESS-WIDE and tests
    /// running in parallel step on each other.
    ///
    /// A package with NEITHER tokenizer is still refused (`is_complete`) — it
    /// is refused HERE rather than at load time so the user gets the catalog
    /// report instead of a 2.5 GB wait ending in an error.
    pub fn to_pair(packages: &[ModelPackage], name: &str) -> Option<Weights> {
        let p = packages.iter().find(|p| p.name == name)?;
        if !p.is_complete() {
            return None;
        }
        Some((
            p.gguf.to_string_lossy().into_owned(),
            p.tokenizer
                .as_ref()
                .map(|t| t.to_string_lossy().into_owned()),
        ))
    }

    /// PRODUCTION DISCOVERY: environment first, then the catalog.
    ///
    /// THE ARCHITECTURE IS NOT GUESSED HERE. The folder name ("qwen3-4b") is
    /// only a label; which module gets loaded is told by the GGUF metadata (see
    /// `Architecture::resolve`). If the name and the content diverge — if the
    /// user puts another weight in the folder — the right thing is to follow the
    /// content.
    pub fn resolve_pair(name: &str) -> Option<Weights> {
        if let Some(p) = pair_from_env() {
            return Some(p);
        }
        to_pair(&catalog(), name)
    }

    // -----------------------------------------------------------------------
    // The remote catalog (packages.json)
    // -----------------------------------------------------------------------

    /// The description of a downloadable package.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RemotePackage {
        pub name: String,
        pub files: Vec<RemoteFile>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RemoteFile {
        /// The name it will take on disk (`model.gguf`, `tokenizer.json`).
        pub name: String,
        pub url: String,
        /// The size declared by the catalog. Shown on the approval screen.
        pub bytes: Option<u64>,
        /// The SHA-256 declared by the publisher. If `None`, the digest
        /// COMPUTED on the first download is shown to the user and written next
        /// to the package (TOFU).
        pub sha256: Option<String>,
    }

    /// The catalog shipped with the binary, so a fresh install can fetch a
    /// working model without writing a JSON file first.
    ///
    /// THIS USED TO BE EMPTY, and the reason it was empty still stands as the bar
    /// every entry here had to clear: an invented address sends the user to a
    /// mirror nobody chose, and an invented digest fails verification on the
    /// first download and teaches the user to switch verification off. So none of
    /// the values below were written from memory. Each URL was requested and
    /// answered 200 without credentials, and each `content-length` matched the
    /// size recorded here. The digests are the registry's own `lfs.oid`, which
    /// for a Hugging Face LFS object IS the SHA-256 of the content — a fact that
    /// can be checked without downloading gigabytes, and which the first real
    /// download then confirms.
    ///
    /// `sha256: None` on the Qwen2.5 tokenizer is NOT an oversight: that file is
    /// stored inline rather than through LFS, so the registry publishes no
    /// digest for it. Rather than invent one, the download path falls back to
    /// trust-on-first-use — it computes the digest, shows it, and records it.
    ///
    /// A user's own `packages.json` still wins by name: this is a default, not a
    /// lock. And nothing here downloads on its own — the approval gate prints the
    /// address and the size and waits for a keypress.
    pub fn embedded_catalog() -> Vec<RemotePackage> {
        fn package(name: &str, files: [(&str, &str, u64, Option<&str>); 2]) -> RemotePackage {
            RemotePackage {
                name: name.to_string(),
                files: files
                    .into_iter()
                    .map(|(file, url, bytes, sha)| RemoteFile {
                        name: file.to_string(),
                        url: url.to_string(),
                        bytes: Some(bytes),
                        sha256: sha.map(str::to_string),
                    })
                    .collect(),
            }
        }

        vec![
            // The default (`DEFAULT_MODEL`). Q4_K_M: the smallest quantisation
            // that still answers well enough to be worth shipping as the one a
            // first-time user gets.
            package(
                "qwen3-4b",
                [
                    // WHICH Qwen3-4B, because there are two and this entry named
                    // the wrong one for months.
                    //
                    // `Qwen/Qwen3-4B-GGUF` is the ORIGINAL hybrid model — the one
                    // that reasons out loud before it answers. Everything this
                    // repository has ever MEASURED was the 2507 instruct
                    // refresh: the checked-in baseline carries its fingerprint,
                    // the README's 133/160 was produced on it, and the skill
                    // guides were written against what it does. So
                    // `tacet models download qwen3-4b` handed every new user a
                    // DIFFERENT MODEL from the one the numbers on the page
                    // describe, under the same name.
                    //
                    // MEASURED 5 SEP 2026, both on one rented RTX 3090, same
                    // suite, same build, 184 cases: the hybrid model spends a
                    // MEDIAN OF 247 GENERATED TOKENS PER TURN against 20 for the
                    // instruct model — twelve times the work for the same
                    // answer, and it is not `thinking`, which comes back empty.
                    // It is prose in front of a call.
                    //
                    // THE SOURCE IS `unsloth` AND NOT `Qwen`, which is a change
                    // worth naming: there is no official Qwen GGUF repository for
                    // 2507 (the obvious address answers 401, not a file). What is
                    // trusted here is
                    // not the uploader but the DIGEST — the download is rejected
                    // unless sha256 matches, and this is the same file, byte for
                    // byte, that produced the baseline.
                    (
                        "model.gguf",
                        "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
                        2_497_281_120,
                        Some("3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597"),
                    ),
                    // The tokenizer lives in the base repository, not the GGUF
                    // one: a GGUF carries its vocabulary internally, but this
                    // engine wants a `tokenizer.json` on disk.
                    //
                    // It moved to the 2507 repository with the weights, and that
                    // is a RENAME AND NOT A DIFFERENT FILE: `Qwen/Qwen3-4B` and
                    // `Qwen/Qwen3-4B-Instruct-2507` publish the same 11 422 654
                    // bytes under the same digest, which is the one already
                    // pinned below. The address now names the model it belongs
                    // to instead of that model's predecessor.
                    (
                        "tokenizer.json",
                        "https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507/resolve/main/tokenizer.json",
                        11_422_654,
                        Some("aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4"),
                    ),
                ],
            ),
            // A smaller second option for machines where 2.5 GB of weights is
            // the constraint rather than the quality.
            package(
                "qwen2.5-3b",
                [
                    (
                        "model.gguf",
                        "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
                        2_104_932_768,
                        Some("626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d"),
                    ),
                    (
                        "tokenizer.json",
                        "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct/resolve/main/tokenizer.json",
                        7_031_645,
                        None,
                    ),
                ],
            ),
        ]
    }

    /// The full path of `packages.json` (in the config directory — this is a
    /// SETTING, not the weight itself; for the root distinction see
    /// `model_roots`).
    pub fn remote_catalog_path() -> Option<PathBuf> {
        tacet_kernel::env::config_path(CATALOG_FILE)
    }

    /// The example shown to the user. There is NO real URL: the field names and
    /// the shape are shown, the address belongs to the user.
    pub const EXAMPLE_CATALOG: &str = r#"{
  "packages": [
    {
      "name": "qwen3-4b",
      "files": [
        { "name": "model.gguf",     "url": "https://<your-own-mirror>/qwen3-4b.gguf", "bytes": 2497281120 },
        { "name": "tokenizer.json", "url": "https://<your-own-mirror>/tokenizer.json" }
      ]
    }
  ]
}"#;

    /// Reads the remote catalog: the user's `packages.json` MERGED over the
    /// embedded defaults. `Err` = the file EXISTS but is broken — not silently
    /// swallowed.
    ///
    /// Merged rather than replaced, and by NAME. Writing one entry of your own
    /// used to hide every default, so a user who added a private mirror silently
    /// lost `qwen3-4b` and had no way to tell that their file was the cause.
    /// Same name = yours wins; that is the override anyone writing the file
    /// actually means.
    pub fn read_remote_catalog() -> Result<Vec<RemotePackage>, String> {
        let Some(path) = remote_catalog_path() else {
            return Ok(embedded_catalog());
        };
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(embedded_catalog()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut merged =
            parse_remote_catalog(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
        let taken: std::collections::HashSet<String> =
            merged.iter().map(|p| p.name.clone()).collect();
        merged.extend(
            embedded_catalog()
                .into_iter()
                .filter(|p| !taken.contains(&p.name)),
        );
        Ok(merged)
    }

    /// A catalog name reduced to a SINGLE plain path component, or an error.
    ///
    /// WHY THIS GATE EXISTS: a catalog name BECOMES A PATH at download time —
    /// `root.join(package.name).join(file.name)`. `PathBuf::join` DISCARDS
    /// everything to its left when the joined component is absolute, so a name
    /// of `/Users/u/.zshenv` makes the download target exactly that file, and a
    /// name of `../../x` walks out of the model root (the downloader creates the
    /// target's parent, so the escape does not even need the directory to
    /// exist). `packages.json` is written by hand and gets pasted around, which
    /// makes it the lowest-privilege-looking file that can write anywhere on the
    /// disk. This is the same rule `ToolContext::resolve_path` enforces for
    /// tools; it has to hold here too.
    ///
    /// A BROKEN CATALOG IS AN ERROR, NOT A SKIPPED ENTRY: this file already
    /// refuses to swallow a malformed catalog silently, and a name that was
    /// quietly rewritten would be worse — the user would not learn that their
    /// override does not do what it says.
    fn plain_name(field: &str, value: &str) -> Result<String, String> {
        use std::path::{Component, Path};
        let mut components = Path::new(value).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(p)), None) if !value.is_empty() => {
                Ok(p.to_string_lossy().into_owned())
            }
            _ => Err(format!(
                "'{value}': the {field} must be a plain name — no '/', no '\\', no '..' and no absolute path"
            )),
        }
    }

    /// SEPARATE AND PUBLIC: so it can be tested without touching the file system.
    pub fn parse_remote_catalog(raw: &str) -> Result<Vec<RemotePackage>, String> {
        let root: serde_json::Value =
            serde_json::from_str(raw).map_err(|e| format!("JSON could note be read: {e}"))?;
        let array = root
            .get("packages")
            .and_then(|p| p.as_array())
            .ok_or_else(|| "no `packages` array".to_string())?;
        let mut output = Vec::new();
        for p in array {
            let name = p
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or_else(|| "the package has no `name` field".to_string())?;
            // THE PACKAGE NAME IS A DIRECTORY NAME (`root.join(name)`), so it
            // goes through the same gate as the file names below.
            let name = &plain_name("package name", name)?;
            let file_array = p
                .get("files")
                .and_then(|f| f.as_array())
                .ok_or_else(|| format!("'{name}': no `files` array"))?;
            let mut files = Vec::new();
            for f in file_array {
                let fname = f
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| format!("'{name}': the file has no `name` field"))?;
                // THE FILE NAME IS THE DOWNLOAD TARGET (`dir.join(&f.name)`):
                // an absolute name would make `join` throw away the model root
                // entirely and write anywhere the user can write.
                let fname =
                    &plain_name("file name", fname).map_err(|e| format!("'{name}': {e}"))?;
                let url = f
                    .get("url")
                    .and_then(|u| u.as_str())
                    .ok_or_else(|| format!("'{name}/{fname}': no `url` field"))?;
                files.push(RemoteFile {
                    name: fname.to_string(),
                    url: url.to_string(),
                    bytes: f.get("bytes").and_then(serde_json::Value::as_u64),
                    sha256: f
                        .get("sha256")
                        .and_then(|s| s.as_str())
                        .map(str::to_string)
                        .filter(|s| !s.is_empty()),
                });
            }
            output.push(RemotePackage {
                name: name.to_string(),
                files,
            });
        }
        Ok(output)
    }
}

/// Human-readable bytes. Packages are gigabytes: a raw number tells the user
/// nothing.
pub fn byte_text(b: u64) -> String {
    const UNIT: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut d = b as f64;
    let mut i = 0;
    while d >= 1024.0 && i + 1 < UNIT.len() {
        d /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{d:.1} {}", UNIT[i])
    }
}

/// The window to run in, derived from the weights that were ACTUALLY loaded.
///
/// TWO NUMBERS OUT OF THE SAME FILE and neither is optional: what the model
/// DECLARES it can take (`<arch>.context_length`) and what one token of context
/// COSTS in KV cache. A window is a memory decision as much as a model one —
/// qwen3-4b declares 262144, and honouring that literally would ask for tens of
/// gigabytes of cache. `context_budget` takes the smaller of "what the model
/// allows" and "what the cache budget affords", and never goes below the 4096
/// floor every path in this shell was measured against.
///
/// THE ENGINE IS ASKED FIRST. `EngineProvider::context_length` is the window read
/// by the thing that really opened the weights; the file is only re-read when the
/// engine does not declare one. Both are the same file today, but the engine's
/// answer is the one that describes the model in memory, and that is the one the
/// counter must match.
///
/// THERE IS NO PATH FROM HERE TO A FIXED 4096 for a real model: if the metadata
/// is missing, `context_budget` returns the floor, and that is a MEASURED absence
/// rather than a constant nobody re-examined. A guessed window is worse than a
/// small one — positions past the model's rope table produce plausible nonsense,
/// not an error.
pub fn engine_window(engine: &Arc<dyn EngineProvider>, gguf: &str) -> usize {
    let path = std::path::Path::new(gguf);
    let declared = engine
        .context_length()
        .or_else(|| tacet_engine::gguf_context_length(path));
    let per_token = tacet_engine::gguf_kv_bytes_per_token(path);
    let device = match engine.identity().device.as_str() {
        "metal" => tacet_engine::Device::Metal,
        "cuda" => tacet_engine::Device::Cuda,
        _ => tacet_engine::Device::Cpu,
    };
    tacet_engine::context_budget(declared, per_token, device)
}

/// Loads the weights WITH THE LOADING INDICATOR UP.
///
/// THE LONGEST WAIT IN THE PRODUCT, and the only one that happens once per
/// process. Before this the screen was blank for it, so the very first thing a
/// new user saw was a shell that looked hung; the whole point of `Stage::Loading`
/// is that this wait is NAMED and that the second turn will not repeat it.
///
/// IT FINISHES BEFORE ANYTHING IS PRINTED. The indicator draws on stderr without
/// a newline, and every caller below writes its result with `eprintln!`; a live
/// indicator would leave its own text sitting in front of the model line. The
/// scope of this function is exactly the silent part.
///
/// WHAT CTRL-C DOES HERE, said plainly because it is a wart: the key thread is
/// running (which is what keeps keys typed during the load out of the first
/// prompt), so ctrl-c sets the cancel flag and prints "cancelling…" — but the
/// GGUF loader has no cancellation point, so the load runs to the end. The flag
/// is reset at the start of the first turn, so nothing is dropped either. Making
/// this honest needs a cancellable loader, not a change here.
pub fn load_weights(
    screen: &Arc<Screen>,
    human: bool,
    model_name: &str,
    gguf: &str,
    tokenizer: Option<&str>,
) -> Result<Arc<dyn EngineProvider>, String> {
    // THE MEMORY QUESTION IS ASKED BEFORE THE ALLOCATION, because after it there
    // is nobody left to ask: the kernel's OOM killer sends SIGKILL, which no
    // handler can catch, and the user is left with the shell's bare `Killed`
    // under a spinner that was still saying "loading". Measured on a Linux VPS
    // running 0.1.11 — four attempts, four one-word failures. See `host_memory`
    // for why this only speaks up on Linux and why the estimate is deliberately
    // small.
    if let Some(refusal) = host_memory::refusal(std::path::Path::new(gguf), model_name) {
        return Err(refusal);
    }
    let mut indicator = if human {
        TurnIndicator::start(Arc::clone(screen), &CANCEL, "loading the model")
    } else {
        TurnIndicator::disabled(Arc::clone(screen))
    };
    indicator.stage(ui::Stage::Loading {
        model: model_name.to_string(),
    });
    let loaded = candle_engine_from_path(gguf, tokenizer);
    indicator.finish();
    loaded
}

/// Sets up the engine according to the choice. `Auto`: candle if a model exists,
/// fake otherwise (with a message).
///
/// IT RETURNS THE WINDOW ALONGSIDE THE ENGINE, because this is the only place
/// that knows both which weights were chosen AND whether they were really
/// loaded. Computing it again at the call site would mean resolving the model
/// package a second time and, on the fallback path, deriving a window from a
/// file no engine ever opened.
///
/// THE FAKE ENGINE KEEPS THE FLOOR. It answers from a script, so a window read
/// out of a GGUF would describe a model that is not running — a true number
/// about the wrong thing, which is how a status line starts lying.
///
/// `screen`/`human` ARE HERE ONLY FOR THE LOADING INDICATOR (see `load_weights`).
/// Reading 2.5 GB off disk is the longest wait in the whole product and the one
/// the user meets first; before this, the screen simply sat blank for it.
pub fn setup_engine(
    choice: EngineChoice,
    script: Vec<String>,
    model_name: &str,
    color: &Color,
    screen: &Arc<Screen>,
    human: bool,
) -> Result<(Arc<dyn EngineProvider>, usize), String> {
    let fake = |s: Vec<String>| -> (Arc<dyn EngineProvider>, usize) {
        (
            Arc::new(FakeEngine::script(s).with_default("Understood. (fake engine)")),
            CONTEXT_BUDGET,
        )
    };
    match choice {
        EngineChoice::Fake => Ok(fake(script)),
        EngineChoice::Candle => match model_package::resolve_pair(model_name) {
            Some((m, t)) => load_weights(screen, human, model_name, &m, t.as_deref()).map(|e| {
                let window = engine_window(&e, &m);
                (e, window)
            }),
            // `--engine candle` is an EXPLICIT request: with no model, erroring
            // out is right, not falling back to fake (see the `Auto` branch,
            // which does the opposite).
            //
            // THE CATALOG IS STILL PRINTED: the error message is one line, while
            // what the user needs is "which roots were searched, what was found".
            // This used to print a SINGLE guess, `~/models/<name>`.
            None => {
                model_not_found_report(model_name, color);
                Err(format!("local model note found: {model_name}"))
            }
        },
        EngineChoice::Auto => match model_package::resolve_pair(model_name) {
            Some((m, t)) => match load_weights(screen, human, model_name, &m, t.as_deref()) {
                Ok(engine) => {
                    let window = engine_window(&engine, &m);
                    // THE WINDOW IS SAID OUT LOUD, next to the model it came
                    // from. It is no longer a constant, so a user comparing two
                    // models has to be able to see that it changed.
                    eprintln!(
                        "{}",
                        color.paint(DIM, &format!("(model: {m} · context {window})"))
                    );
                    Ok((engine, window))
                }
                // NO FALLBACK. This used to warn and hand back `FakeEngine`, and
                // that was the product's worst failure mode — see `no_engine`.
                Err(e) => {
                    eprintln!(
                        "{}",
                        color.paint(YELLOW, &format!("(the real model could not be used: {e})"))
                    );
                    Err(no_engine(model_name))
                }
            },
            None => {
                model_not_found_report(model_name, color);
                Err(no_engine(model_name))
            }
        },
    }
}

/// WHY `Auto` REFUSES TO CHAT INSTEAD OF ANSWERING FROM A SCRIPT — the product's
/// worst failure mode, and the last one that was still shipping.
///
/// `cargo install` DOES NOT REMEMBER `--features`. A user installs with
/// `--features metal`, upgrades months later without the flag, and gets a binary
/// that cannot run a model. Until this function existed that binary still
/// started, still accepted questions, and still answered — from `FakeEngine`,
/// which replies "Understood. (fake engine)" and a short script. There was a
/// warning line at startup, dimmed, on stderr, printed once. Then every
/// following turn looked exactly like a working product.
///
/// THAT IS THE SHAPE OF A BUG NOBODY REPORTS. The user does not conclude "my
/// build has no inference feature"; they conclude the project does not work, and
/// they leave. No issue is opened, because from where they sit nothing crashed.
///
/// THE RULE IS THE SANDBOX'S RULE, APPLIED TO THE ENGINE. `run_code` does not run
/// unprotected when no sandbox can be verified — the tool leaves the catalog
/// (`RunCodeTool::discover`). The same sentence with the nouns changed: when no
/// model can be loaded, the shell does not converse. Absence is legible; a canned
/// answer dressed as a real one is not.
///
/// THE SCRIPTED ENGINE IS NOT GONE, it is now something you ASK for. `--engine
/// fake` still works and is what eval and the tests use; what it no longer is, is
/// somewhere you arrive by accident. And when it IS asked for, every turn says so
/// — see `FAKE_TURN_NOTICE` in `chat.rs`, because a notice printed once at
/// startup scrolls away and a piped stdout never carried it at all.
///
/// THE MISSING FEATURE IS NAMED FIRST when it is missing, because that is the
/// case the user cannot diagnose. "No model found" they can act on; "this binary
/// cannot load one at all" reads identically from the outside and does not.
fn no_engine(model_name: &str) -> String {
    let mut lines =
        vec!["no model could be loaded, so there is nothing to answer with".to_string()];
    if cfg!(not(feature = "candle")) {
        lines.push(
            "THIS BINARY HAS NO INFERENCE ENGINE: it was built without `--features candle` (or \
             `metal`), so no model file can help. `tacet --version` prints the build. Reinstall \
             with `cargo install tacet-cli --features metal` on Apple silicon, or `--features \
             candle` elsewhere — `cargo install` does not remember the flag from last time."
                .to_string(),
        );
    } else {
        lines.push(format!(
            "get weights with `tacet models download {model_name}`, or point `{MODEL_VARIABLE}` \
             at a .gguf you already have"
        ));
    }
    lines.push(
        "if you actually want the scripted engine, ask for it: `tacet --engine fake` (the answers \
         are fixed, and every turn says so)"
            .to_string(),
    );
    lines.join("\n  ")
}

/// Prints THE CATALOG when no model is found.
///
/// WHY SO MUCH DETAIL: this is the wall the user hits most often and the old
/// state gave them a one-line guess ("put it under ~/models/<name>"). That line
/// left three questions unanswered: which directories WERE SEARCHED, what was
/// found there, and why none of the finds could be selected. All three are
/// written here.
///
/// IF THERE IS AN ENV OVERRIDE IT IS SAID FIRST: showing the catalog while
/// `TACET_MODEL` is set would mislead — in that case discovery never ran at all.
pub fn model_not_found_report(requested: &str, color: &Color) {
    if let Some((m, t)) = model_package::pair_from_env() {
        // The tokenizer line reports what was ACTUALLY asked for. Printing a
        // `TACET_TOKENIZER` value the user never set would send them looking for
        // a variable that is not in their environment.
        let tokenizer_line = match &t {
            Some(t) => format!("\n   tokenizer: {t}"),
            None => format!(
                "\n   tokenizer: not set ({TOKENIZER_VARIABLE}) — the one inside the .gguf would be used"
            ),
        };
        eprintln!(
            "{}",
            color.paint(
                YELLOW,
                &format!(
                    "({MODEL_VARIABLE} is set but the files could note be loaded:\n   gguf : {m}{tokenizer_line})"
                )
            )
        );
        return;
    }

    let roots = model_package::model_roots();
    eprintln!(
        "{}",
        color.paint(
            YELLOW,
            &format!("(model package note found: '{requested}')")
        )
    );
    if roots.is_empty() {
        // Neither HOME/USERPROFILE nor XDG_DATA_HOME/LOCALAPPDATA resolved.
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "  no root to search — point at the files directly with {}/{}",
                    MODEL_VARIABLE, TOKENIZER_VARIABLE
                )
            )
        );
        return;
    }
    for r in &roots {
        eprintln!(
            "{}",
            color.paint(DIM, &format!("  searched: {}", r.display()))
        );
    }

    let packages = model_package::scan(&roots);
    if packages.is_empty() {
        // NO PACKAGES AT ALL. What to suggest depends on THE STATE OF THE
        // CATALOG: `tacet models download` now exists but downloads ONLY from the
        // user's own `packages.json` (the embedded catalog is deliberately
        // empty). So suggesting the command with an empty catalog would send the
        // user to a line that does nothing.
        eprintln!("{}", color.paint(DIM, "  no packages at all."));
        let catalog = model_package::read_remote_catalog();
        match &catalog {
            Ok(c) if !c.is_empty() => {
                let names: Vec<&str> = c.iter().map(|p| p.name.as_str()).collect();
                eprintln!(
                    "{}",
                    color.paint(DIM, &format!("  downloadable (packages.json): {}", names.join(", ")))
                );
                eprintln!(
                    "{}",
                    color.paint(DIM, &format!("  to download: tacet models download {}", names[0]))
                );
            }
            // A BROKEN CATALOG IS NOT PASSED OVER IN SILENCE: the user wrote the
            // file, and the sentence "no packages at all" does not tell them the
            // file was not read.
            Err(e) => eprintln!("{}", color.paint(YELLOW, &format!("  packages.json could note be read: {e}"))),
            Ok(_) => match model_package::remote_catalog_path() {
                Some(p) => eprintln!(
                    "{}",
                    color.paint(
                        DIM,
                        &format!(
                            "  you can write a download source into {}; for the shape: tacet models list --json",
                            p.display()
                        )
                    )
                ),
                None => eprintln!("{}", color.paint(DIM, "  the config directory could note be resolved.")),
            },
        }
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!(
                    "  or put <name>/*.gguf + tokenizer.json in a folder: {}",
                    roots[0].display()
                )
            )
        );
        return;
    }

    eprintln!(
        "{}",
        color.paint(DIM, "  what was found (tacet models list):")
    );
    for p in &packages {
        let note = if p.is_complete() {
            ""
        } else {
            "  [no tokenizer, in the folder or in the .gguf — cannot be selected]"
        };
        eprintln!("{}", color.paint(DIM, &format!("    {}{note}", p.name)));
    }
    let selectable: Vec<&str> = packages
        .iter()
        .filter(|p| p.is_complete())
        .map(|p| p.name.as_str())
        .collect();
    if !selectable.is_empty() {
        eprintln!(
            "{}",
            color.paint(
                DIM,
                &format!("  to select: tacet --model {}", selectable[0])
            )
        );
    }
}
// ---------------------------------------------------------------------------
// candle engine setup
// ---------------------------------------------------------------------------

/// `tokenizer: None` means "the vocabulary is inside the weights". The choice is
/// made ONE LEVEL UP, in discovery (`model_package::to_pair`), so that the two
/// places that answer "is this package usable" — the catalog report the user
/// reads and the loader — cannot drift apart.
#[cfg(feature = "candle")]
pub fn candle_engine_from_path(
    model: &str,
    tokenizer: Option<&str>,
) -> Result<Arc<dyn EngineProvider>, String> {
    let setting = match tokenizer {
        Some(t) => tacet_engine::ModelSetting::new(model, t),
        None => tacet_engine::ModelSetting::from_gguf(model),
    };
    // File existence is checked BEFORE a 2.5 GB load; learning about a missing
    // file at the end of that wait is a pointless delay.
    tacet_engine::CandleEngine::files_exist(&setting).map_err(|e| e.to_string())?;
    let engine = tacet_engine::CandleEngine::load(&setting).map_err(|e| e.to_string())?;
    // WHICH ARCHITECTURE was loaded is printed. Had it stayed silent, a model
    // running with the wrong template would look like "it gives odd answers" and
    // be hard to diagnose.
    //
    // WHICH TOKENIZER is printed for the same reason and it is the sharper of
    // the two: the two sources are indistinguishable from the output — a
    // vocabulary rebuilt from the wrong place does not error, it produces text
    // that reads like broken weights.
    eprintln!(
        "(architecture: {}, template: {:?}, tokenizer: {})",
        engine.architecture().name(),
        engine.architecture().template(),
        engine.tokenizer_source().name()
    );
    Ok(Arc::new(engine) as Arc<dyn EngineProvider>)
}

#[cfg(not(feature = "candle"))]
pub fn candle_engine_from_path(
    _model: &str,
    _tokenizer: Option<&str>,
) -> Result<Arc<dyn EngineProvider>, String> {
    Err("this binary was built without the `candle` feature".into())
}

#[cfg(test)]
mod refusal {
    use super::*;

    /// THE CLAIM: a build with no inference engine says so FIRST, and names the
    /// flag `cargo install` forgot.
    ///
    /// NOT VACUOUS, and this is the part worth reading: the assertion is
    /// `cfg`-split rather than written once, because the two builds must say
    /// DIFFERENT things and a single test that accepted either would pass on a
    /// binary that said neither. Under `--features candle` the missing piece is
    /// the weights, and telling that user to reinstall would be a wrong
    /// instruction; without the feature the weights are irrelevant and telling
    /// that user to download 2 GB would waste their afternoon and not fix it.
    #[test]
    fn the_refusal_names_the_thing_the_user_cannot_see() {
        let text = no_engine("qwen3-4b");

        assert!(
            text.contains("nothing to answer with"),
            "the refusal must say why there is no answer: {text}"
        );
        assert!(
            text.contains("--engine fake"),
            "the refusal must name the way to ASK for the scripted engine: {text}"
        );

        if cfg!(feature = "candle") {
            assert!(
                text.contains("tacet models download qwen3-4b") && text.contains(MODEL_VARIABLE),
                "with an inference engine present the missing piece is the WEIGHTS, and both \
                 routes to them must be named: {text}"
            );
            assert!(
                !text.contains("cargo install"),
                "this build CAN load a model, so telling the user to reinstall is a wrong \
                 instruction: {text}"
            );
        } else {
            assert!(
                text.contains("NO INFERENCE ENGINE") && text.contains("--features metal"),
                "a featureless build must name the feature, because that is the one thing the \
                 user cannot diagnose from the outside: {text}"
            );
            assert!(
                !text.contains("models download"),
                "no weight file can help this build, so sending the user after one is a wrong \
                 instruction: {text}"
            );
        }
    }

    /// The failing half of the old behaviour, pinned so it cannot come back:
    /// `Auto` with no weights must be an `Err`, not an engine.
    ///
    /// WHY IT PASSES A NAME THAT CANNOT RESOLVE rather than mocking discovery:
    /// `resolve_pair` reads the real model roots, and a name no root can hold is
    /// the only input that means "nothing found" on every machine — including one
    /// with a full `~/models` directory, where a plausible name would find
    /// weights and measure nothing.
    #[test]
    fn auto_with_no_weights_refuses_instead_of_answering_from_a_script() {
        let screen = Screen::setup();
        let color = Color::setup();
        let result = setup_engine(
            EngineChoice::Auto,
            Vec::new(),
            "no-such-model-\u{1f6ab}-in-any-root",
            &color,
            &screen,
            false,
        );

        let Err(message) = result else {
            panic!(
                "Auto fell back to an engine with no weights — this is the canned-answer bug \
                 that shipped for months"
            );
        };
        assert!(
            message.contains("nothing to answer with"),
            "the error must be the diagnosis, not a bare string: {message}"
        );
    }

    /// `--engine fake` still works. The refusal above must not have taken the
    /// scripted engine away — it only stopped it being somewhere you ARRIVE.
    #[test]
    fn asking_for_the_scripted_engine_still_gets_one() {
        let screen = Screen::setup();
        let color = Color::setup();
        let (engine, _) = setup_engine(
            EngineChoice::Fake,
            vec!["Hello.".to_string()],
            "irrelevant",
            &color,
            &screen,
            false,
        )
        .expect("an explicit --engine fake is a request, not an accident");
        assert_eq!(
            engine.name(),
            "fake",
            "the name is what `chat` reads to decide whether to sign the answer as scripted"
        );
    }
}

/// THE MODEL THIS BINARY DOWNLOADS IS THE MODEL THE NUMBERS WERE MEASURED ON.
///
/// The defect this exists to make impossible, because it was real: the built-in
/// catalog pointed `qwen3-4b` at `Qwen/Qwen3-4B-GGUF` — the original hybrid
/// model — while every measurement in this repository was made on the 2507
/// instruct refresh. The checked-in baseline carried the right fingerprint the
/// whole time and nothing compared the two, so `tacet models download qwen3-4b`
/// and "133/160, qwen3-4b Q4_K_M" quietly meant different weights.
///
/// IT COMPARES BYTE COUNTS AND NOT DIGESTS, deliberately. The catalog pins the
/// sha256 of the WHOLE file; the report records `EngineIdentity`'s fingerprint,
/// which is a hash of the size and the two 1 MB edges (see `file_fingerprint` —
/// it is cheap enough to run on every load, which is why it exists in that
/// shape). The two are not comparable, and inventing a third hash so that they
/// were would mean reading 2.5 GB in a unit test. The size is the field both
/// sides already record, and it separates these two models — 2 497 281 120
/// against 2 497 280 256, which is close enough that a glance does not catch it
/// and a comparison always does.
///
/// IT READS THE BASELINE OUT OF THE SIBLING CRATE. `tacet-eval` cannot host this
/// test, because the catalog lives here and `tacet-cli` is what depends on
/// `tacet-eval`, not the other way round.
#[cfg(test)]
mod shipped_weights {
    /// The baseline the README quotes, read from disk rather than through
    /// `tacet-eval`: what is being checked is the FILE that is committed.
    fn baseline() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tacet-eval/baselines/qwen3-4b-both.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
        serde_json::from_str(&text).expect("the baseline is JSON")
    }

    #[test]
    fn the_default_package_is_the_weights_the_baseline_was_measured_on() {
        let measured = baseline()["identity"]["model_bytes"]
            .as_u64()
            .expect("the baseline records the size of the weights it ran on");

        let catalog = super::model_package::embedded_catalog();
        let default = catalog
            .iter()
            .find(|p| p.name == crate::DEFAULT_MODEL)
            .unwrap_or_else(|| panic!("the catalog offers {}", crate::DEFAULT_MODEL));
        let gguf = default
            .files
            .iter()
            .find(|f| f.name == "model.gguf")
            .expect("the default package ships weights");

        assert_eq!(
            gguf.bytes,
            Some(measured),
            "the catalog would download {:?} ({:?} bytes) but the baseline in \
crates/tacet-eval/baselines/qwen3-4b-both.json was measured on {measured} bytes. \
These are two different models under one name — the exact defect this test \
exists for. Either point the catalog at the weights that were measured, or \
re-measure and replace the baseline; do not leave them disagreeing.",
            gguf.url,
            gguf.bytes,
        );
    }
}
