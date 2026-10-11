//! The mirror: `<root>/.clm/<safe(session_id)>.ctx`, the file the model edits.
//!
//! The root is writable by the session's own execs, so everything under it
//! may have been tampered with. On unix every operation therefore works
//! relative to one directory fd for `.clm`, opened without following
//! symlinks and checked to be owned by this process: a planted or raced
//! `.clm` link is refused instead of steering reads, writes or unlinks out of
//! the root. Writes go through a fresh `O_EXCL` temp file and a rename (a
//! planted symlink is replaced, never followed); reads refuse symlinks,
//! FIFOs, hard-linked files and oversized files.
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub const MIRROR_DIR: &str = ".clm";
/// A mirror holds at most a budget's worth of text; anything bigger is not ours.
const MAX_MIRROR_BYTES: u64 = 16 << 20;
/// Longest encoded id used verbatim; file names stay far below 255 bytes.
const MAX_SAFE_LEN: usize = 128;
const GITIGNORE: &str = ".gitignore";

/// Injective, file-name-safe form of a session id: `[A-Za-z0-9_.-]` kept,
/// every other byte as `%XX` (uppercase hex), and so is a leading `.`, so the
/// result is never a dot-name (`.`, `..`, hidden files). The empty id is the
/// fixed token `%e`. An id whose encoding would exceed 128 bytes becomes
/// `%h<sha256>`. The encoder only ever emits `%` followed by two hex digits,
/// so `%e` and `%h...` cannot collide with any encoded id.
pub fn safe_id(sid: &str) -> String {
    if sid.is_empty() {
        return "%e".to_owned();
    }
    let mut out = String::with_capacity(sid.len());
    for (i, b) in sid.bytes().enumerate() {
        let plain = b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-') || (b == b'.' && i > 0);
        if plain {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    if out.len() > MAX_SAFE_LEN {
        return format!("%h{:x}", Sha256::digest(sid.as_bytes()));
    }
    out
}

fn file_name(sid: &str) -> String {
    format!("{}.ctx", safe_id(sid))
}

/// The mirror path relative to the session root (exec cwd), as the prompt names it.
pub fn mirror_rel(sid: &str) -> String {
    format!("{MIRROR_DIR}/{}", file_name(sid))
}

/// Atomically replace the mirror (0600) in `<root>/.clm` (0700, self-ignoring).
/// Fails with `NotFound` when the root itself is gone; the root is never created.
pub fn write_mirror(root: &Path, sid: &str, text: &str) -> io::Result<()> {
    imp::write(root, &file_name(sid), text.as_bytes())
}

/// The mirror's current text; `None` when it does not exist. Invalid UTF-8 is
/// replaced, not rejected.
pub fn read_mirror(root: &Path, sid: &str) -> io::Result<Option<String>> {
    imp::read(root, &file_name(sid))
}

/// Delete the mirror (and a leftover temp file); missing files are fine.
pub fn remove_mirror(root: &Path, sid: &str) -> io::Result<()> {
    imp::remove(root, &file_name(sid))
}

fn invalid_data(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::fs::{File, OpenOptions, Permissions};
    use std::io::{self, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::Path;

    use super::{invalid_data, GITIGNORE, MAX_MIRROR_BYTES, MIRROR_DIR};

    fn cstr(name: &str) -> io::Result<CString> {
        CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "name contains a NUL byte"))
    }

    /// Run a libc call that reports failure as a negative return.
    fn check(ret: libc::c_int) -> io::Result<()> {
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// `openat`; the fd is wrapped in a `File` so it closes on every path.
    fn open_at(dir: &File, name: &str, flags: libc::c_int, mode: libc::c_uint) -> io::Result<File> {
        let name = cstr(name)?;
        // SAFETY: `dir` is a live fd and `name` is NUL-terminated; on success
        // the new fd is owned by nobody else, so `File` may take it over.
        unsafe {
            let fd = libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC,
                mode,
            );
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(File::from_raw_fd(fd))
        }
    }

    /// `unlinkat` (never follows a symlink); a missing entry is fine.
    fn unlink_at(dir: &File, name: &str) -> io::Result<()> {
        let name = cstr(name)?;
        // SAFETY: live fd, NUL-terminated name.
        match check(unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) }) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Whether `name` exists in `dir` as anything, a symlink included.
    fn exists_at(dir: &File, name: &str) -> io::Result<bool> {
        let name = cstr(name)?;
        // SAFETY: an all-zero `stat` is a valid out-parameter.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: live fd, NUL-terminated name, `st` is writable.
        let ret = unsafe {
            libc::fstatat(
                dir.as_raw_fd(),
                name.as_ptr(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        match check(ret) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// The root is the operator-set trust anchor: it is followed like any
    /// path, but only as a directory and never created.
    fn open_root(root: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(root)
    }

    /// The `.clm` directory fd, optionally creating it (0700). A symlink or
    /// other non-directory in its place, or a directory owned by someone
    /// else, is refused.
    fn open_clm(root: &File, root_path: &Path, create: bool) -> io::Result<File> {
        if create {
            let name = cstr(MIRROR_DIR)?;
            // SAFETY: live fd, NUL-terminated name.
            match check(unsafe { libc::mkdirat(root.as_raw_fd(), name.as_ptr(), 0o700) }) {
                Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
                _ => {}
            }
        }
        let shown = || root_path.join(MIRROR_DIR).display().to_string();
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW;
        let dir = open_at(root, MIRROR_DIR, flags, 0).map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP | libc::ENOTDIR) => {
                invalid_data(format!("{} is not a real directory", shown()))
            }
            _ => e,
        })?;
        let meta = dir.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if meta.uid() != unsafe { libc::geteuid() } {
            return Err(invalid_data(format!(
                "{} is not owned by this process",
                shown()
            )));
        }
        if create && meta.mode() & 0o777 != 0o700 {
            // On the fd we hold: cannot be redirected, and undoes a umask.
            dir.set_permissions(Permissions::from_mode(0o700))?;
        }
        Ok(dir)
    }

    /// Atomic replace with a 0600 file inside `dir`: a fresh `O_EXCL` temp,
    /// fsync, rename. A stale or planted temp is unlinked first.
    fn write_atomic(dir: &File, name: &str, bytes: &[u8]) -> io::Result<()> {
        let tmp = format!("{name}.tmp");
        unlink_at(dir, &tmp)?;
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW;
        let mut file = open_at(dir, &tmp, flags, 0o600)?;
        // Explicit, so the mode does not depend on the umask.
        file.set_permissions(Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let (from, to) = (cstr(&tmp)?, cstr(name)?);
        let fd = dir.as_raw_fd();
        // SAFETY: live fd, NUL-terminated names.
        check(unsafe { libc::renameat(fd, from.as_ptr(), fd, to.as_ptr()) })
    }

    pub fn write(root: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
        let root_fd = open_root(root)?;
        let dir = open_clm(&root_fd, root, true)?;
        if !exists_at(&dir, GITIGNORE)? {
            write_atomic(&dir, GITIGNORE, b"*\n")?;
        }
        write_atomic(&dir, name, bytes)
    }

    pub fn read(root: &Path, name: &str) -> io::Result<Option<String>> {
        let shown = || root.join(MIRROR_DIR).join(name).display().to_string();
        let root_fd = match open_root(root) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            other => other?,
        };
        let dir = match open_clm(&root_fd, root, false) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            other => other?,
        };
        // No symlink, and a FIFO swapped in cannot block the open.
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK;
        let file = match open_at(&dir, name, flags, 0) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
                return Err(invalid_data(format!("{} is a symlink", shown())))
            }
            other => other?,
        };
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(invalid_data(format!("{} is not a regular file", shown())));
        }
        if meta.nlink() != 1 {
            return Err(invalid_data(format!("{} has other hard links", shown())));
        }
        if meta.len() > MAX_MIRROR_BYTES {
            return Err(invalid_data(format!(
                "{} is larger than {MAX_MIRROR_BYTES} bytes",
                shown()
            )));
        }
        let mut bytes = Vec::new();
        file.take(MAX_MIRROR_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_MIRROR_BYTES {
            return Err(invalid_data(format!(
                "{} is larger than {MAX_MIRROR_BYTES} bytes",
                shown()
            )));
        }
        Ok(Some(String::from_utf8(bytes).unwrap_or_else(|e| {
            String::from_utf8_lossy(e.as_bytes()).into_owned()
        })))
    }

    pub fn remove(root: &Path, name: &str) -> io::Result<()> {
        let root_fd = match open_root(root) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            other => other?,
        };
        let dir = match open_clm(&root_fd, root, false) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            other => other?,
        };
        unlink_at(&dir, &format!("{name}.tmp"))?;
        unlink_at(&dir, name)
    }
}

// NOTE: there is no directory-fd API here, so paths are re-resolved on every
// call and the `.clm` confinement above does not apply to this fallback.
#[cfg(not(unix))]
mod imp {
    use std::fs::{self, OpenOptions};
    use std::io::{self, Read, Write};
    use std::path::Path;

    use super::{
        invalid_data, new_private_file, remove_if_exists, tmp_path, write_private, GITIGNORE,
        MAX_MIRROR_BYTES, MIRROR_DIR,
    };

    pub fn write(root: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
        let dir = root.join(MIRROR_DIR);
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is not a directory", dir.display()),
                ))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => match fs::create_dir(&dir) {
                Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
                _ => {}
            },
            Err(e) => return Err(e),
        }
        match new_private_file(&dir.join(GITIGNORE)) {
            Ok(mut ignore) => ignore.write_all(b"*\n")?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        write_private(&dir.join(name), bytes)
    }

    pub fn read(root: &Path, name: &str) -> io::Result<Option<String>> {
        let path = root.join(MIRROR_DIR).join(name);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => {
                return Err(invalid_data(format!(
                    "{} is not a regular file",
                    path.display()
                )))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
        let file = OpenOptions::new().read(true).open(&path)?;
        let mut bytes = Vec::new();
        file.take(MAX_MIRROR_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_MIRROR_BYTES {
            return Err(invalid_data(format!(
                "{} is larger than {MAX_MIRROR_BYTES} bytes",
                path.display()
            )));
        }
        Ok(Some(String::from_utf8(bytes).unwrap_or_else(|e| {
            String::from_utf8_lossy(e.as_bytes()).into_owned()
        })))
    }

    pub fn remove(root: &Path, name: &str) -> io::Result<()> {
        let path = root.join(MIRROR_DIR).join(name);
        remove_if_exists(&tmp_path(&path))?;
        remove_if_exists(&path)
    }
}

/// Atomic replace with a 0600 file: a fresh `O_EXCL` temp next to `path`,
/// fsync, rename. A stale or planted temp is unlinked first (unlinking a
/// symlink never touches its target). Path-based: for the worker's own
/// private data dir, not for the agent-writable root.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(path);
    remove_if_exists(&tmp)?;
    let mut file = new_private_file(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)
}

pub(crate) fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = OsString::from(path.as_os_str());
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

/// `O_CREAT | O_EXCL` (never follows a symlink), mode 0600.
fn new_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
