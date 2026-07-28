//! THE SECOND SANDBOX GATE: containment for paths that ALREADY EXIST.
//!
//! `ToolContext::resolve_path` is LEXICAL — it walks the components of the
//! string and rejects `..`, an absolute root and a Windows prefix. That is the
//! right gate on the WRITE path, where the leaf does not exist yet and
//! `canonicalize` would simply fail.
//!
//! It is NOT ENOUGH on the READ path. `run_code` may legitimately create a
//! symbolic link inside the sandbox (the shield profile allows writing there,
//! and creating a link IS a write), so a poisoned prompt can make the model
//! plant `gate -> /Users/<victim>` and then ask a document tool for
//! `gate/Library/.../credentials.json`. Lexically `gate` is an ordinary
//! `Component::Normal`; every later `metadata()`/`read()` FOLLOWS the link and
//! the bytes land in the model's window, from where a later web/mcp call can
//! carry them off the device.
//!
//! CHECKING ONLY THE LEAF WITH `symlink_metadata` DOES NOT CLOSE THIS. In
//! `gate/Library/credentials.json` the leaf is a real file; the escape happens
//! at an INTERMEDIATE component. `canonicalize` resolves EVERY component, so the
//! containment test is redone on the REAL path — that is the only form that
//! holds.
//!
//! This lives in one module ON PURPOSE: patching each of the four call sites
//! separately is how the fifth one gets forgotten.
//!
//! THE SCOPE IS A LIST NOW (see `crate::workspace`): the working directory plus
//! the directories the user named at install time. What changed is only WHICH
//! set the real path is tested against — resolution, the link-free retest and
//! the refusal of a dangling path are untouched. With no extra root registered
//! this file behaves exactly as it did before.

use std::path::{Component, Path, PathBuf};

use tacet_kernel::{ToolContext, ToolError, ToolResult};

use crate::workspace;

/// Resolves an EXISTING path and proves the REAL (link-free) result is still
/// inside one of the workspace roots.
///
/// The error split is deliberate: a path that cannot be resolved is
/// `FileNotFound` (an honest "there is no such file"), while a path that
/// resolves OUTSIDE every root is `SandboxViolation` — the trace must show the
/// escape attempt as a refusal, not as a missing file.
pub fn resolve_existing(ctx: &ToolContext, path: impl AsRef<Path>) -> ToolResult<PathBuf> {
    let raw = path.as_ref();
    let roots = roots(ctx);
    match ctx.resolve_path(raw) {
        // The ordinary case, unchanged: a relative path lives under the working
        // directory. It may still LEAVE it through a planted link, which is why
        // the containment test below runs on the canonical result.
        Ok(lexical) => {
            let real = lexical
                .canonicalize()
                .map_err(|_| ToolError::FileNotFound(lexical.clone()))?;
            if !inside_any(&real, &roots) {
                return Err(ToolError::SandboxViolation(raw.to_path_buf()));
            }
            Ok(real)
        }
        // `resolve_path` is lexical and knows one root, so it refuses every
        // absolute path outside the working directory — including a legitimate
        // one inside a registered root. That is the ONLY case reconsidered here.
        Err(err) => {
            if workspace::has_no_extra_roots() || !raw.is_absolute() {
                return Err(err);
            }
            resolve_absolute_in_roots(raw, &roots)
        }
    }
}

/// The second chance for an ABSOLUTE path, granted only when the user
/// registered extra roots.
///
/// A relative path never gets here: it is resolved against the working
/// directory, and a relative path that escaped it lexically (`../..`) escaped
/// the way an attack escapes. Keeping that refusal means the behaviour of every
/// existing caller is unchanged.
fn resolve_absolute_in_roots(raw: &Path, roots: &[PathBuf]) -> ToolResult<PathBuf> {
    // `..` IS REFUSED OUTRIGHT, not normalised. A path with parent components
    // has two readings — before and after link resolution — and the whole point
    // of this module is that those two must never be allowed to disagree.
    if raw.components().any(|c| c == Component::ParentDir) {
        return Err(ToolError::SandboxViolation(raw.to_path_buf()));
    }
    match raw.canonicalize() {
        Ok(real) => {
            if !inside_any(&real, roots) {
                return Err(ToolError::SandboxViolation(raw.to_path_buf()));
            }
            Ok(real)
        }
        // NO EXISTENCE ORACLE: a path we would refuse anyway must not answer
        // "is there a file here". Only a path that is lexically in scope — and
        // is therefore `..`-free by the check above — earns "not found".
        Err(_) if inside_any(raw, roots) => Err(ToolError::FileNotFound(raw.to_path_buf())),
        Err(_) => Err(ToolError::SandboxViolation(raw.to_path_buf())),
    }
}

/// Every root in scope: the working directory first, then the registered ones.
///
/// THE ROOTS ARE RESOLVED: on macOS `/tmp` is a link to `/private/tmp`, so
/// comparing a canonical path against an unresolved root would fail for every
/// legitimate call and the gate would look like a bug instead of a gate.
/// (`workspace` stores its roots canonical already; the working directory
/// arrives raw from `ToolContext`.)
fn roots(ctx: &ToolContext) -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(4);
    roots.push(
        ctx.working_dir
            .canonicalize()
            .unwrap_or_else(|_| ctx.working_dir.clone()),
    );
    roots.extend(workspace::extra_roots());
    roots
}

/// Containment against a LIST. `Path::starts_with` compares whole components,
/// so a sibling named like a root (`/w/root-evil` against `/w/root`) does not
/// pass — the string-prefix version of this test is a known bug class.
fn inside_any(real: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| real.starts_with(root))
}

/// `resolve_existing` + "it really is a file".
pub fn resolve_existing_file(ctx: &ToolContext, path: impl AsRef<Path>) -> ToolResult<PathBuf> {
    let real = resolve_existing(ctx, path)?;
    if !real.is_file() {
        return Err(ToolError::FileNotFound(real));
    }
    Ok(real)
}

/// `resolve_existing` + "it really is a directory".
pub fn resolve_existing_dir(ctx: &ToolContext, path: impl AsRef<Path>) -> ToolResult<PathBuf> {
    let real = resolve_existing(ctx, path)?;
    if !real.is_dir() {
        return Err(ToolError::FileNotFound(real));
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tacet_kernel::{InMemoryDataStore, SilentReporter};

    fn temp_root(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-gate-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path.canonicalize().expect("resolved")
    }

    fn context(root: &Path) -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            root.to_path_buf(),
            Arc::new(SilentReporter),
        )
    }

    /// The root list is process-wide, so every test here takes the shared lock
    /// and starts from an empty list. A test that inherited another test's roots
    /// would be testing a scope nobody wrote down.
    fn isolated() -> std::sync::MutexGuard<'static, ()> {
        let guard = workspace::test_lock();
        workspace::clear_roots();
        guard
    }

    /// A legitimate file inside the sandbox still passes — the gate must not
    /// break the normal flow (that is how a security check gets deleted later).
    #[test]
    fn a_file_inside_the_sandbox_passes() {
        let _guard = isolated();
        let root = temp_root("inside");
        std::fs::write(root.join("notes.txt"), "hello").expect("write");
        let ctx = context(&root);
        let resolved = resolve_existing_file(&ctx, "notes.txt").expect("must pass");
        assert_eq!(resolved, root.join("notes.txt"));
        assert!(resolve_existing_dir(&ctx, ".").is_ok());
    }

    /// THE ATTACK: a directory link planted inside the sandbox (run_code can do
    /// this) is walked THROUGH by every lexical check. The canonical containment
    /// test must refuse it — both for the link itself and for a path whose
    /// INTERMEDIATE component is the link.
    #[cfg(unix)]
    #[test]
    fn a_planted_directory_link_cannot_be_walked_through() {
        let _guard = isolated();
        let root = temp_root("gate");
        let outside = temp_root("outside");
        std::fs::create_dir_all(outside.join("Library")).expect("outside tree");
        std::fs::write(
            outside.join("Library").join("credentials.json"),
            "{\"k\":1}",
        )
        .expect("victim file");
        std::os::unix::fs::symlink(&outside, root.join("gate")).expect("link");

        let ctx = context(&root);
        assert!(matches!(
            resolve_existing_dir(&ctx, "gate"),
            Err(ToolError::SandboxViolation(_))
        ));
        assert!(matches!(
            resolve_existing_file(&ctx, "gate/Library/credentials.json"),
            Err(ToolError::SandboxViolation(_))
        ));
        // A deeper root is the case a leaf-only check misses.
        assert!(matches!(
            resolve_existing_dir(&ctx, "gate/Library"),
            Err(ToolError::SandboxViolation(_))
        ));
    }

    /// A direct FILE link out of the sandbox is refused as well.
    #[cfg(unix)]
    #[test]
    fn a_planted_file_link_cannot_be_read() {
        let _guard = isolated();
        let root = temp_root("filelink");
        let outside = temp_root("secret");
        let secret = outside.join("id.txt");
        std::fs::write(&secret, "PRIVATE KEY MATERIAL").expect("secret");
        std::os::unix::fs::symlink(&secret, root.join("notes.txt")).expect("link");

        let ctx = context(&root);
        assert!(matches!(
            resolve_existing_file(&ctx, "notes.txt"),
            Err(ToolError::SandboxViolation(_))
        ));
    }

    /// WITH NO EXTRA ROOT NOTHING MOVED. An absolute path inside the working
    /// directory passed before this change (`resolve_path` strips the prefix)
    /// and still passes; an absolute path elsewhere was a violation and still
    /// is — NOT a "file not found", which would leak whether it exists.
    #[test]
    fn an_empty_root_list_behaves_exactly_as_before() {
        let _guard = isolated();
        let root = temp_root("empty-list");
        let elsewhere = temp_root("elsewhere");
        std::fs::write(root.join("notes.txt"), "hello").expect("write");
        std::fs::write(elsewhere.join("other.txt"), "hello").expect("write");

        let ctx = context(&root);
        assert_eq!(
            resolve_existing_file(&ctx, root.join("notes.txt")).expect("absolute, in the sandbox"),
            root.join("notes.txt")
        );
        assert!(matches!(
            resolve_existing_file(&ctx, elsewhere.join("other.txt")),
            Err(ToolError::SandboxViolation(_))
        ));
        assert!(matches!(
            resolve_existing_file(&ctx, elsewhere.join("missing.txt")),
            Err(ToolError::SandboxViolation(_))
        ));
    }

    /// THE POINT OF THE FEATURE: a directory the user named is in scope, and a
    /// relative path still resolves against the working directory only.
    #[test]
    fn a_registered_second_root_is_in_scope() {
        let _guard = isolated();
        let root = temp_root("primary");
        let second = temp_root("secondary");
        std::fs::write(second.join("plan.txt"), "the plan").expect("write");
        workspace::install_roots([&second]).expect("install");

        let ctx = context(&root);
        assert_eq!(
            resolve_existing_file(&ctx, second.join("plan.txt")).expect("second root is in scope"),
            second.join("plan.txt")
        );
        assert!(resolve_existing_dir(&ctx, &second).is_ok());
        // The working directory did not stop being a root.
        std::fs::write(root.join("notes.txt"), "hello").expect("write");
        assert!(resolve_existing_file(&ctx, "notes.txt").is_ok());
        // A name that exists in the second root is NOT reachable relatively:
        // relative paths still mean "under the working directory".
        assert!(matches!(
            resolve_existing_file(&ctx, "plan.txt"),
            Err(ToolError::FileNotFound(_))
        ));
    }

    /// A third directory nobody named stays out — registering one root does not
    /// open the parent that happens to contain both.
    #[test]
    fn a_directory_outside_every_root_is_refused() {
        let _guard = isolated();
        let root = temp_root("scoped");
        let second = temp_root("named");
        let outside = temp_root("unnamed");
        std::fs::write(outside.join("secret.txt"), "PRIVATE").expect("write");
        workspace::install_roots([&second]).expect("install");

        let ctx = context(&root);
        assert!(matches!(
            resolve_existing_file(&ctx, outside.join("secret.txt")),
            Err(ToolError::SandboxViolation(_))
        ));
        // The parent shared by all the temp roots is not in scope either.
        let shared_parent = outside.parent().expect("temp parent").to_path_buf();
        assert!(matches!(
            resolve_existing_dir(&ctx, &shared_parent),
            Err(ToolError::SandboxViolation(_))
        ));
    }

    /// A link BETWEEN two roots is not an escape: both ends are in scope, so
    /// the widened containment test must accept it. This is the case a
    /// single-root check would wrongly refuse once the feature exists.
    #[cfg(unix)]
    #[test]
    fn a_link_from_one_root_into_another_is_allowed() {
        let _guard = isolated();
        let root = temp_root("linked-primary");
        let second = temp_root("linked-secondary");
        std::fs::write(root.join("notes.txt"), "hello").expect("write");
        std::os::unix::fs::symlink(root.join("notes.txt"), second.join("shortcut.txt"))
            .expect("link");
        std::os::unix::fs::symlink(&second, root.join("second")).expect("dir link");
        workspace::install_roots([&second]).expect("install");

        let ctx = context(&root);
        // Second root -> first root, through a file link.
        assert_eq!(
            resolve_existing_file(&ctx, second.join("shortcut.txt")).expect("both ends in scope"),
            root.join("notes.txt")
        );
        // First root -> second root, through a directory link walked relatively.
        std::fs::write(second.join("plan.txt"), "the plan").expect("write");
        assert_eq!(
            resolve_existing_file(&ctx, "second/plan.txt").expect("both ends in scope"),
            second.join("plan.txt")
        );
    }

    /// THE OLD ATTACK, RETESTED WITH A ROOT LIST: a link planted in ANY root
    /// that leaves EVERY root is still refused, whether it is reached
    /// relatively or by its absolute path. Widening the scope must not have
    /// turned the containment test into a formality.
    #[cfg(unix)]
    #[test]
    fn a_link_out_of_every_root_is_still_refused() {
        let _guard = isolated();
        let root = temp_root("multi-primary");
        let second = temp_root("multi-secondary");
        let outside = temp_root("multi-outside");
        std::fs::create_dir_all(outside.join("Library")).expect("outside tree");
        std::fs::write(outside.join("Library").join("credentials.json"), "{}").expect("victim");
        std::os::unix::fs::symlink(&outside, second.join("gate")).expect("link");
        std::os::unix::fs::symlink(&outside, root.join("gate")).expect("link");
        workspace::install_roots([&second]).expect("install");

        let ctx = context(&root);
        // Planted in the SECOND root, reached absolutely.
        assert!(matches!(
            resolve_existing_dir(&ctx, second.join("gate")),
            Err(ToolError::SandboxViolation(_))
        ));
        assert!(matches!(
            resolve_existing_file(&ctx, second.join("gate/Library/credentials.json")),
            Err(ToolError::SandboxViolation(_))
        ));
        // Planted in the working directory, reached relatively — the original
        // attack, unaffected by the extra root.
        assert!(matches!(
            resolve_existing_file(&ctx, "gate/Library/credentials.json"),
            Err(ToolError::SandboxViolation(_))
        ));
    }

    /// `..` in an absolute path is refused before anything is resolved: a path
    /// with two readings must never get one of them accepted.
    #[test]
    fn parent_components_cannot_climb_out_of_a_root() {
        let _guard = isolated();
        let root = temp_root("climb-primary");
        let second = temp_root("climb-secondary");
        let outside = temp_root("climb-outside");
        std::fs::write(outside.join("secret.txt"), "PRIVATE").expect("write");
        workspace::install_roots([&second]).expect("install");

        let ctx = context(&root);
        let climb = second.join("..").join(
            outside
                .file_name()
                .expect("named temp dir")
                .to_string_lossy()
                .to_string(),
        );
        assert!(matches!(
            resolve_existing_dir(&ctx, &climb),
            Err(ToolError::SandboxViolation(_))
        ));
        assert!(matches!(
            resolve_existing_file(&ctx, climb.join("secret.txt")),
            Err(ToolError::SandboxViolation(_))
        ));
        // AND THE NON-EXISTENT CASE READS AS A REFUSAL, NOT AS "NOT FOUND".
        // Without the `..` refusal this path is lexically inside the root, so
        // the not-found branch would answer it — turning the gate into a probe
        // for what does and does not exist outside the scope.
        assert!(matches!(
            resolve_existing_file(&ctx, climb.join("missing.txt")),
            Err(ToolError::SandboxViolation(_))
        ));
    }

    /// A sibling whose name merely STARTS WITH a root's name is outside it.
    /// This is the prefix-matching bug class the SSRF gate already paid for.
    #[test]
    fn a_name_prefixed_sibling_is_not_inside_the_root() {
        let _guard = isolated();
        let root = temp_root("sibling-primary");
        let base = temp_root("sibling-base");
        let named = base.join("acme");
        let lookalike = base.join("acme-evil");
        std::fs::create_dir_all(&named).expect("root dir");
        std::fs::create_dir_all(&lookalike).expect("sibling dir");
        std::fs::write(lookalike.join("secret.txt"), "PRIVATE").expect("write");
        workspace::install_roots([&named]).expect("install");

        let ctx = context(&root);
        assert!(matches!(
            resolve_existing_file(&ctx, lookalike.join("secret.txt")),
            Err(ToolError::SandboxViolation(_))
        ));
    }

    /// A dangling link inside a root is "not found", never a silent pass.
    #[cfg(unix)]
    #[test]
    fn a_dangling_link_inside_a_root_is_not_found() {
        let _guard = isolated();
        let root = temp_root("dangling-primary");
        let second = temp_root("dangling-secondary");
        std::os::unix::fs::symlink(second.join("gone.txt"), second.join("ghost.txt"))
            .expect("link");
        workspace::install_roots([&second]).expect("install");

        let ctx = context(&root);
        assert!(matches!(
            resolve_existing_file(&ctx, second.join("ghost.txt")),
            Err(ToolError::FileNotFound(_))
        ));
    }
}
