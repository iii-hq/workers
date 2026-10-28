//! Metadata operations: getattr, setattr, access.
//!
//! ## No Stat Virtualization (D-02)
//!
//! iii-filesystem does not use xattr-based stat overrides. All stat results
//! return raw host data directly. UID/GID changes use a real chown, mode changes use
//! a real chmod. If chown fails with EPERM (host process lacks CAP_CHOWN), the
//! error is silently swallowed per RESEARCH.md guidance: "no-op that returns success".

use std::{
    ffi::CString,
    fs::File,
    io,
    os::fd::{AsRawFd, FromRawFd, RawFd},
    time::Duration,
};

use super::{PassthroughFs, inode};
use crate::{
    Context, SetattrValid,
    backends::shared::{init_binary, platform},
    stat64,
};

//--------------------------------------------------------------------------------------------------
// Types
//--------------------------------------------------------------------------------------------------

/// What setattr changes mode, owner and times through.
enum Target {
    /// An open fd: the `f*` calls.
    Fd(RawFd),
    /// `(dirfd, path, flags)` naming the inode itself: the `*at` calls.
    At(RawFd, CString, libc::c_int),
}

impl Target {
    fn chmod(&self, mode: libc::mode_t) -> libc::c_int {
        match self {
            Self::Fd(fd) => unsafe { libc::fchmod(*fd, mode) },
            Self::At(dir, path, flags) => unsafe {
                libc::fchmodat(*dir, path.as_ptr(), mode, *flags)
            },
        }
    }

    fn chown(&self, uid: libc::uid_t, gid: libc::gid_t) -> libc::c_int {
        match self {
            Self::Fd(fd) => unsafe { libc::fchown(*fd, uid, gid) },
            Self::At(dir, path, flags) => unsafe {
                libc::fchownat(*dir, path.as_ptr(), uid, gid, *flags)
            },
        }
    }

    fn utimens(&self, times: &[libc::timespec; 2]) -> libc::c_int {
        match self {
            Self::Fd(fd) => unsafe { libc::futimens(*fd, times.as_ptr()) },
            Self::At(dir, path, flags) => unsafe {
                libc::utimensat(*dir, path.as_ptr(), times.as_ptr(), *flags)
            },
        }
    }
}

//--------------------------------------------------------------------------------------------------
// Functions
//--------------------------------------------------------------------------------------------------

/// Get attributes for an inode.
///
/// Returns raw stat data (no xattr patching per D-02).
pub(crate) fn do_getattr(
    fs: &PassthroughFs,
    _ctx: Context,
    ino: u64,
    handle: Option<u64>,
) -> io::Result<(stat64, Duration)> {
    let st = inode::stat_inode(fs, ino, handle)?;
    Ok((st, fs.cfg.attr_timeout))
}

/// Set attributes on an inode.
///
/// Processes each SetattrValid flag: SIZE via ftruncate, MODE via chmod,
/// UID/GID via chown (silently succeeds on EPERM), timestamps via utimens,
/// each on the handle's fd when there is one. No xattr/stat_override (per D-02).
pub(crate) fn do_setattr(
    fs: &PassthroughFs,
    _ctx: Context,
    ino: u64,
    attr: stat64,
    handle: Option<u64>,
    valid: SetattrValid,
) -> io::Result<(stat64, Duration)> {
    if init_binary::has_init() && ino == init_binary::INIT_INODE {
        return Err(platform::eperm());
    }

    // Keep the DashMap Ref and RwLockReadGuard alive so the raw fd stays valid
    // for the lifetime of all syscalls below. A concurrent do_release cannot
    // close the underlying File while these guards exist.
    let hdata_guard = handle.and_then(|h| fs.handles.get(&h));
    let file_guard = hdata_guard.as_ref().map(|hd| hd.file.read().unwrap());
    let mut opened = None;
    let fd = match &file_guard {
        Some(fg) => Some(fg.as_raw_fd()),
        // truncate(2) needs write access, not read access. O_NONBLOCK: a FIFO
        // with no reader would block the fs worker thread.
        None if valid.contains(SetattrValid::SIZE) => {
            let fd = inode::open_inode_fd(fs, ino, libc::O_WRONLY | libc::O_NONBLOCK)?;
            Some(opened.insert(unsafe { File::from_raw_fd(fd) }).as_raw_fd())
        }
        None => None,
    };

    // Mode, owner and times need ownership, not access: without an fd they
    // go through a path to the inode, which also changes a symlink itself.
    let target = match fd {
        Some(fd) => Target::Fd(fd),
        None => match inode_path(fs, ino)? {
            Some(path) => path,
            // macOS, once unlinked: open_inode_fd dups the fd kept at unlink.
            None => {
                let fd = inode::open_inode_fd(fs, ino, libc::O_RDONLY)?;
                Target::Fd(opened.insert(unsafe { File::from_raw_fd(fd) }).as_raw_fd())
            }
        },
    };

    // Handle size changes via ftruncate.
    if valid.contains(SetattrValid::SIZE)
        && let Some(fd) = fd
    {
        #[cfg(target_os = "linux")]
        let ret = unsafe { libc::ftruncate64(fd, attr.st_size) };
        #[cfg(target_os = "macos")]
        let ret = unsafe { libc::ftruncate(fd, attr.st_size) };

        if ret < 0 {
            return Err(platform::linux_error(io::Error::last_os_error()));
        }
    }

    // Handle mode changes via chmod (no xattr per D-02).
    if valid.contains(SetattrValid::MODE) {
        let new_mode = platform::mode_u32(attr.st_mode) & !platform::MODE_TYPE_MASK;
        if target.chmod(new_mode as libc::mode_t) < 0 {
            return Err(platform::linux_error(io::Error::last_os_error()));
        }
    }

    // Handle UID/GID changes via chown.
    // If chown fails with EPERM, silently succeed (host lacks CAP_CHOWN).
    if valid.intersects(SetattrValid::UID | SetattrValid::GID) {
        let uid = if valid.contains(SetattrValid::UID) {
            attr.st_uid
        } else {
            u32::MAX // -1 = no change
        };
        let gid = if valid.contains(SetattrValid::GID) {
            attr.st_gid
        } else {
            u32::MAX // -1 = no change
        };
        if target.chown(uid, gid) < 0 {
            let err = io::Error::last_os_error();
            // Silently succeed on EPERM (per RESEARCH.md: "no-op that returns success").
            if err.raw_os_error() != Some(libc::EPERM) {
                return Err(platform::linux_error(err));
            }
        }
    }

    // Handle timestamp changes via utimens.
    if valid.intersects(SetattrValid::ATIME | SetattrValid::MTIME) {
        let times = platform::build_timespecs(attr, valid);
        if target.utimens(&times) < 0 {
            return Err(platform::linux_error(io::Error::last_os_error()));
        }
    }

    // Return updated attributes.
    let st = inode::stat_inode(fs, ino, None)?;
    Ok((st, fs.cfg.attr_timeout))
}

/// A path that names the inode itself and is not followed past it, or `None`
/// when it has none (macOS, once unlinked: `/.vol` no longer resolves it).
fn inode_path(fs: &PassthroughFs, ino: u64) -> io::Result<Option<Target>> {
    #[cfg(target_os = "linux")]
    {
        // The procfd magic link resolves to the pinned inode, a symlink included,
        // and is not followed further.
        let inode_fd = inode::get_inode_fd(fs, ino)?;
        let name = CString::new(inode_fd.raw().to_string()).expect("a number has no nul");
        Ok(Some(Target::At(fs.proc_self_fd.as_raw_fd(), name, 0)))
    }

    #[cfg(target_os = "macos")]
    {
        use std::sync::atomic::Ordering;

        let inodes = fs.inodes.read().unwrap();
        let data = inodes.get(&ino).ok_or_else(platform::ebadf)?;
        if data.unlinked_fd.load(Ordering::Acquire) >= 0 {
            return Ok(None);
        }
        let path = inode::vol_path(data.dev, data.ino);
        Ok(Some(Target::At(
            libc::AT_FDCWD,
            path,
            libc::AT_SYMLINK_NOFOLLOW,
        )))
    }
}

/// Check file access permissions.
///
/// For init.krun, checks against 0o755 (r-x for all, no write).
/// For other inodes, uses `stat_inode` to check real host permissions.
pub(crate) fn do_access(fs: &PassthroughFs, ctx: Context, ino: u64, mask: u32) -> io::Result<()> {
    if init_binary::has_init() && ino == init_binary::INIT_INODE {
        // init.krun is mode 0o755: readable and executable by all, not writable.
        if mask == platform::ACCESS_F_OK {
            return Ok(());
        }
        if mask & platform::ACCESS_W_OK != 0 {
            return Err(platform::eacces());
        }
        return Ok(());
    }

    let st = inode::stat_inode(fs, ino, None)?;

    // F_OK: just check existence.
    if mask == platform::ACCESS_F_OK {
        return Ok(());
    }

    let st_mode = platform::mode_u32(st.st_mode);

    // Root (uid 0) bypasses read/write checks.
    if ctx.uid == 0 {
        if mask & platform::ACCESS_X_OK != 0 && st_mode & 0o111 == 0 {
            return Err(platform::eacces());
        }
        return Ok(());
    }

    let bits = if st.st_uid == ctx.uid {
        (st_mode >> 6) & 0o7
    } else if st.st_gid == ctx.gid {
        (st_mode >> 3) & 0o7
    } else {
        st_mode & 0o7
    };

    if mask & platform::ACCESS_R_OK != 0 && bits & 0o4 == 0 {
        return Err(platform::eacces());
    }
    if mask & platform::ACCESS_W_OK != 0 && bits & 0o2 == 0 {
        return Err(platform::eacces());
    }
    if mask & platform::ACCESS_X_OK != 0 && bits & 0o1 == 0 {
        return Err(platform::eacces());
    }

    Ok(())
}
