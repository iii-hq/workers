//! The mirror: `<root>/.clm/<safe(session_id)>.ctx`, the file the model edits.
//!
//! The root is writable by the session's own execs, so every path here may
//! have been tampered with: the mirror directory must be a real directory,
//! writes go through a fresh `O_EXCL` temp file and a rename (a planted
//! symlink is replaced, never followed), and reads refuse symlinks, FIFOs
//! and oversized files.
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub const MIRROR_DIR: &str = ".clm";
/// A mirror holds at most a budget's worth of text; anything bigger is not ours.
const MAX_MIRROR_BYTES: u64 = 16 << 20;
/// Longest encoded id used verbatim; file names stay far below 255 bytes.
const MAX_SAFE_LEN: usize = 128;

/// Injective, file-name-safe form of a session id: `[A-Za-z0-9_.-]` kept,
/// every other byte as `%XX`. An id whose encoding would exceed 128 bytes
/// becomes `%h<sha256>`; the encoder never emits `%h`, so the forms cannot
/// collide.
pub fn safe_id(sid: &str) -> String {
    let mut out = String::with_capacity(sid.len());
    for b in sid.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-') {
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

/// The mirror path relative to the session root (exec cwd), as the prompt names it.
pub fn mirror_rel(sid: &str) -> String {
    format!("{MIRROR_DIR}/{}.ctx", safe_id(sid))
}

/// Atomically replace the mirror (0600) in `<root>/.clm` (0700, self-ignoring).
/// Fails with `NotFound` when the root itself is gone; the root is never created.
pub fn write_mirror(root: &Path, sid: &str, text: &str) -> io::Result<()> {
    let dir = root.join(MIRROR_DIR);
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a directory", dir.display()),
            ))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => create_private_dir(&dir)?,
        Err(e) => return Err(e),
    }
    match new_private_file(&dir.join(".gitignore")) {
        Ok(mut ignore) => ignore.write_all(b"*\n")?,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    write_private(&root.join(mirror_rel(sid)), text.as_bytes())
}

/// The mirror's current text; `None` when it does not exist. Invalid UTF-8 is
/// replaced, not rejected.
pub fn read_mirror(root: &Path, sid: &str) -> io::Result<Option<String>> {
    let path = root.join(mirror_rel(sid));
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a regular file", path.display()),
            ))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    // Closes the check-then-open gap: no symlink, and a FIFO swapped in
    // cannot block the open.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(&path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not a regular file", path.display()),
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_MIRROR_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MIRROR_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is larger than {MAX_MIRROR_BYTES} bytes", path.display()),
        ));
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Delete the mirror (and a leftover temp file); missing files are fine.
pub fn remove_mirror(root: &Path, sid: &str) -> io::Result<()> {
    let path = root.join(mirror_rel(sid));
    remove_if_exists(&tmp_path(&path))?;
    remove_if_exists(&path)
}

/// Atomic replace with a 0600 file: a fresh `O_EXCL` temp next to `path`,
/// fsync, rename. A stale or planted temp is unlinked first (unlinking a
/// symlink never touches its target).
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

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(dir) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => Err(e),
        _ => Ok(()),
    }
}
