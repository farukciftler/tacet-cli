//! WHERE SECRETS ARE ALLOWED TO LAND: the permission contract of the
//! configuration directory.
//!
//! WHY IN THE CORE, NEXT TO `env`. `env` says WHERE the configuration lives;
//! this module says WHO MAY READ IT. That is the same kind of clause and it has
//! to hold for every layer at once — the config directory holds `memory.json`
//! (the user's personal notes), `mcp.json` (a PLAIN-TEXT bearer token) and
//! `addons.json` (an address that may carry a credential), and each of those is
//! written by a different crate. Written as three copies of the same chmod, the
//! copies drift: MEASURED — only `memory.json` ever got the 0600 stamp, the
//! directory around it was born 0755 and every other file in it 0644, readable
//! by any second local account that can traverse the home directory (on macOS
//! every account is in `staff`).
//!
//! This does not break the "no work is done in this crate" rule the way a tool
//! implementation would: nothing here decides anything, reads a setting or
//! parses a format. These are two primitives that exist so the rule has exactly
//! one home, in the same crate as the path it applies to.
//!
//! NOT LEFT TO THE UMASK. A default umask of 022 gives 0755 directories and
//! 0644 files, and the user never chose that for their notes. The mode is set
//! EXPLICITLY, the way `create_document` already does for exported documents.
//!
//! PLATFORM RECORD — THE STAMP EXISTS ONLY ON UNIX. On Windows nothing is done,
//! deliberately: `set_permissions` there only flips the "read-only" flag, it
//! does not restrict access, so stamping would produce the ILLUSION of privacy
//! without any of it. The real equivalent is narrowing an ACL, which needs a
//! `windows-sys` dependency and contradicts the zero-dependency identity. On
//! Windows the protection rests on the file's LOCATION (under the user
//! profile). NOT MEASURED ON A WINDOWS MACHINE.

use std::path::Path;

/// The mode of a directory that holds secrets: owner only, no group, no world.
#[cfg(unix)]
pub const PRIVATE_DIR_MODE: u32 = 0o700;

/// The mode of a file that holds secrets.
#[cfg(unix)]
pub const PRIVATE_FILE_MODE: u32 = 0o600;

/// Creates (recursively) a directory that must not be readable by other local
/// accounts, and NARROWS IT IF IT ALREADY EXISTS.
///
/// THE SECOND HALF MATTERS AS MUCH AS THE FIRST. Every existing install already
/// has a 0755 directory on disk; a fix that only stamps freshly created
/// directories would protect nobody who already runs the tool. `DirBuilder`'s
/// mode applies at CREATION only, so the mode is stamped afterwards as well.
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(PRIVATE_DIR_MODE);
        builder.create(dir)?;
        // Best effort: a directory we do not own cannot be chmodded, and that
        // is not a reason to fail the write the caller came here for.
        let _ = std::fs::set_permissions(
            dir,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(
                PRIVATE_DIR_MODE,
            ),
        );
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Writes a file that must not be world-readable.
///
/// THE MODE IS RIGHT BEFORE THE SECRET BYTES LAND. `fs::write` creates at
/// `0666 & ~umask` (measured: 0644) and a later `chmod` cannot take back a
/// descriptor another user already opened in between, nor a crash between the
/// two calls — which leaves a 0644 file with the full contents on disk. So the
/// file is OPENED with the narrow mode and, because `O_CREAT`'s mode applies
/// only on creation, a LEFTOVER file from an earlier run is narrowed through
/// its own open descriptor (`fchmod`, not a second path lookup, so no new
/// TOCTOU window) before a single byte is written into it.
pub fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(PRIVATE_FILE_MODE);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
    }
    file.write_all(contents)
}

/// Narrows a file that is already on disk without rewriting it.
///
/// FOR EXISTING INSTALLS AND FOR FILES WE DELIBERATELY DO NOT OVERWRITE. It only
/// ever narrows; nothing here widens a mode the user chose themselves.
pub fn narrow_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(PRIVATE_FILE_MODE));
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// THE UMASK MUST NOT DECIDE. This test would be RED against the old code
    /// (`create_dir_all` + `fs::write` under umask 022 give 0755/0644) and the
    /// only way it goes green is an explicit mode.
    #[test]
    fn a_private_directory_and_its_files_exclude_other_local_accounts() {
        let dir = std::env::temp_dir().join(format!("tacet-fs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create_private_dir(&dir).unwrap();
        assert_eq!(mode_of(&dir), 0o700, "the directory is traversable");

        let file = dir.join("secret.json");
        write_private(&file, b"{\"token\":\"s3cret\"}").unwrap();
        assert_eq!(mode_of(&file), 0o600, "the secret is world-readable");
        assert_eq!(std::fs::read(&file).unwrap(), b"{\"token\":\"s3cret\"}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A 0644 FILE LEFT BY AN EARLIER RUN IS THE REAL CASE: `O_CREAT`'s mode
    /// does not apply to a file that already exists, so without the `fchmod`
    /// the secret would be written straight into a world-readable inode.
    #[test]
    fn an_existing_world_readable_file_is_narrowed_before_it_is_written() {
        let dir = std::env::temp_dir().join(format!("tacet-fs-leftover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create_private_dir(&dir).unwrap();

        let file = dir.join("leftover.json");
        std::fs::write(&file, b"old").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(mode_of(&file), 0o644, "the premise");

        write_private(&file, b"new secret").unwrap();
        assert_eq!(mode_of(&file), 0o600, "the leftover kept its old mode");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AN EXISTING 0755 DIRECTORY IS THE UPGRADING USER. If only fresh
    /// directories were stamped, every install that already exists would stay
    /// open.
    #[test]
    fn an_existing_open_directory_is_narrowed_too() {
        let dir = std::env::temp_dir().join(format!("tacet-fs-olddir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(mode_of(&dir), 0o755, "the premise");

        create_private_dir(&dir).unwrap();
        assert_eq!(mode_of(&dir), 0o700, "an upgrading user stays exposed");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
