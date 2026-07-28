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

use std::path::{Path, PathBuf};

use tacet_kernel::{ToolContext, ToolError, ToolResult};

/// Resolves an EXISTING path and proves the REAL (link-free) result is still
/// inside the working directory.
///
/// The error split is deliberate: a path that cannot be resolved is
/// `FileNotFound` (an honest "there is no such file"), while a path that
/// resolves OUTSIDE the sandbox is `SandboxViolation` — the trace must show the
/// escape attempt as a refusal, not as a missing file.
pub fn resolve_existing(ctx: &ToolContext, path: impl AsRef<Path>) -> ToolResult<PathBuf> {
    let raw = path.as_ref();
    let lexical = ctx.resolve_path(raw)?;
    let real = lexical
        .canonicalize()
        .map_err(|_| ToolError::FileNotFound(lexical.clone()))?;
    // THE ROOT IS RESOLVED TOO: on macOS `/tmp` is a link to `/private/tmp`, so
    // comparing a canonical path against an unresolved root would fail for every
    // legitimate call and the gate would look like a bug instead of a gate.
    let root = ctx
        .working_dir
        .canonicalize()
        .unwrap_or_else(|_| ctx.working_dir.clone());
    if !real.starts_with(&root) {
        return Err(ToolError::SandboxViolation(raw.to_path_buf()));
    }
    Ok(real)
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

    /// A legitimate file inside the sandbox still passes — the gate must not
    /// break the normal flow (that is how a security check gets deleted later).
    #[test]
    fn a_file_inside_the_sandbox_passes() {
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
}
