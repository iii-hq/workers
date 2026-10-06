//! Owner-only files and directories, written atomically.
//!
//! On unix: directories 0700, files 0600, every write a temp file in the
//! same directory + fsync + rename (+ directory fsync). Elsewhere the same
//! write sequence runs and permissions are left to the platform's ACLs.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

pub const DIR_MODE: u32 = 0o700;
pub const FILE_MODE: u32 = 0o600;

/// Create `dir` (parents with default permissions, the leaf owner-only) and
/// tighten an existing leaf that is group- or world-accessible.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(DIR_MODE);
    match builder.create(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => {}
        Err(error) => return Err(error),
    }
    tighten(dir, DIR_MODE)
}

/// Replace `path` with `bytes` atomically, owner-only.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = write_temp(path, bytes)?;
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    sync_parent(path);
    Ok(())
}

/// Create `path` with `bytes` atomically unless it already exists. Returns
/// `false` (and leaves the existing file untouched) when it does, so a
/// concurrent creator's key is never overwritten.
pub fn create_private_exclusive(path: &Path, bytes: &[u8]) -> io::Result<bool> {
    let tmp = write_temp(path, bytes)?;
    let outcome = match fs::hard_link(&tmp, path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        // Filesystems without hard links: check-then-rename. Only another
        // secrets worker on the same vault could race this window.
        Err(_) if !path.exists() => fs::rename(&tmp, path).map(|()| true),
        Err(_) => Ok(false),
    };
    let _ = fs::remove_file(&tmp);
    if matches!(outcome, Ok(true)) {
        sync_parent(path);
    }
    outcome
}

/// Narrow an existing file or directory to `mode` when it grants anything
/// to group or others. No-op off unix.
pub fn tighten(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        let current = fs::metadata(path)?.permissions().mode() & 0o777;
        if current & 0o077 != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

/// The permission bits of `path` (unix only).
#[cfg(unix)]
pub fn mode_of(path: &Path) -> io::Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o777)
}

fn write_temp(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    ensure_private_dir(dir)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?
        .to_string_lossy();
    let tmp = dir.join(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(FILE_MODE);
    let result = options.open(&tmp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    if let Err(error) = result {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    Ok(tmp)
}

/// Persist the rename itself. Best effort: not every platform can open a
/// directory for syncing.
fn sync_parent(path: &Path) {
    if let Some(dir) = path.parent() {
        if let Ok(handle) = File::open(dir) {
            let _ = handle.sync_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/private/file.json");
        write_private_atomic(&path, b"one").unwrap();
        write_private_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("file.json")]);
    }

    #[test]
    fn exclusive_create_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("k/key");
        assert!(create_private_exclusive(&path, b"first").unwrap());
        assert!(!create_private_exclusive(&path, b"second").unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn files_are_0600_and_directories_0700() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("secrets");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.join("vault.json");
        write_private_atomic(&path, b"{}").unwrap();
        assert_eq!(mode_of(&dir).unwrap(), DIR_MODE);
        assert_eq!(mode_of(&path).unwrap(), FILE_MODE);
    }
}
