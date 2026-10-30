//! Directory operations: opendir, readdir, readdirplus, releasedir.
//!
//! Entries stream into the FUSE reply through `add_entry`, which answers 0 once
//! the reply is full, and listing stops there. Names are borrowed from a
//! per-request buffer (Linux) or from the handle's snapshot (macOS), so a
//! listing leaves no host memory behind, and READDIRPLUS looks up only the
//! entries the reply carries.

use std::{
    ffi::CString,
    io,
    os::fd::{AsRawFd, FromRawFd},
    sync::{Arc, RwLock, atomic::Ordering},
};

use super::{PassthroughFs, inode};
use crate::{
    AddDirEntry, AddDirEntryPlus, Context, DirEntry, OpenOptions,
    backends::shared::{handle_table::HandleData, init_binary, platform},
};

//--------------------------------------------------------------------------------------------------
// Constants
//--------------------------------------------------------------------------------------------------

/// d_off cookie of the synthetic init.krun entry. Real d_off values stay below it, so a resume
/// from any real offset sees init.krun exactly once and a resume from the cookie itself returns an
/// empty page. It is i64::MAX - 1, not i64::MAX: ext4 reports i64::MAX as the end-of-directory
/// cookie of the last real entry, and a resume from that cookie must still list init.krun.
const INIT_DIR_OFFSET: u64 = i64::MAX as u64 - 1;

//--------------------------------------------------------------------------------------------------
// Functions
//--------------------------------------------------------------------------------------------------

/// Open a directory and return a handle.
pub(crate) fn do_opendir(
    fs: &PassthroughFs,
    _ctx: Context,
    ino: u64,
    _flags: u32,
) -> io::Result<(Option<u64>, OpenOptions)> {
    let fd = inode::open_inode_fd(fs, ino, libc::O_RDONLY | libc::O_DIRECTORY)?;
    let file = unsafe { std::fs::File::from_raw_fd(fd) };

    let handle = fs.next_handle.fetch_add(1, Ordering::Relaxed);
    let data = Arc::new(HandleData {
        file: RwLock::new(file),
    });

    fs.handles.insert(handle, data);
    Ok((Some(handle), fs.cache_dir_options()))
}

/// Read directory entries into `add_entry` until the reply is full.
///
/// The root lists the virtual init.krun last, in place of any real file of
/// that name, as LOOKUP resolves the name to it.
pub(crate) fn do_readdir(
    fs: &PassthroughFs,
    _ctx: Context,
    ino: u64,
    handle: u64,
    size: u32,
    offset: u64,
    add_entry: &mut AddDirEntry<'_>,
) -> io::Result<()> {
    let with_init = ino == 1 && init_binary::has_init();
    // Resuming from init.krun's cookie means the listing is finished.
    if with_init && offset == INIT_DIR_OFFSET {
        return Ok(());
    }

    let reached_end = read_dir_entries(fs, handle, size, offset, &mut |de| {
        if with_init && de.name == init_binary::INIT_FILENAME {
            return Ok(1); // skipped, and the reply is not full
        }
        add_entry(de)
    })?;

    if with_init && reached_end {
        add_entry(DirEntry {
            ino: init_binary::INIT_INODE,
            offset: INIT_DIR_OFFSET,
            type_: platform::DIRENT_REG,
            name: init_binary::INIT_FILENAME,
        })?;
    }
    Ok(())
}

/// Read directory entries with attributes (readdirplus).
///
/// Each entry gets a full `do_lookup` just before it is sent, and gives that
/// reference back if the reply turns it away: the kernel forgets only the
/// entries it received. The root's virtual init.krun gets the synthetic entry.
pub(crate) fn do_readdirplus(
    fs: &PassthroughFs,
    ctx: Context,
    ino: u64,
    handle: u64,
    size: u32,
    offset: u64,
    add_entry: &mut AddDirEntryPlus<'_>,
) -> io::Result<()> {
    let mut sent = Vec::new();
    let res = do_readdir(fs, ctx, ino, handle, size, offset, &mut |mut de| {
        // Skip . and .. -- the kernel handles these itself.
        if de.name == b"." || de.name == b".." {
            return Ok(1);
        }

        // Only the root lists init.krun as the virtual init (do_readdir hides a real one there).
        if ino == 1 && init_binary::has_init() && de.name == init_binary::INIT_FILENAME {
            let entry = init_binary::init_entry(fs.cfg.entry_timeout, fs.cfg.attr_timeout);
            return add_entry(de, entry);
        }

        // An entry that is gone, or cannot be looked up, is left out.
        let Ok(name) = CString::new(de.name) else {
            return Ok(1);
        };
        let Ok(entry) = inode::do_lookup(fs, ino, &name) else {
            return Ok(1);
        };

        // Correct d_type from the lookup's stat (free: no extra syscalls).
        de.type_ = platform::dirent_type_from_mode(platform::mode_file_type(entry.attr.st_mode));
        let looked_up = entry.inode;
        let added = add_entry(de, entry);
        match added {
            Ok(n) if n > 0 => sent.push(looked_up),
            _ => inode::forget_one(fs, looked_up, 1),
        }
        added
    });
    // An error reply carries no entries, so the kernel forgets none of them.
    if res.is_err() {
        for looked_up in sent {
            inode::forget_one(fs, looked_up, 1);
        }
    }
    res
}

/// Release an open directory handle.
pub(crate) fn do_releasedir(
    fs: &PassthroughFs,
    _ctx: Context,
    _ino: u64,
    _flags: u32,
    handle: u64,
) -> io::Result<()> {
    fs.handles.remove(&handle);
    #[cfg(target_os = "macos")]
    fs.dir_snapshots.remove(&handle);
    Ok(())
}

//--------------------------------------------------------------------------------------------------
// Functions: Helpers
//--------------------------------------------------------------------------------------------------

/// Stream the host entries after `offset` into `add_entry` with `getdents64`,
/// one buffer (sized by the FUSE `size`, clamped to 1KB--64KB) at a time.
/// Returns whether the listing reached its end.
#[cfg(target_os = "linux")]
fn read_dir_entries(
    fs: &PassthroughFs,
    handle: u64,
    size: u32,
    offset: u64,
    add_entry: &mut AddDirEntry<'_>,
) -> io::Result<bool> {
    let data = fs.handles.get(&handle).ok_or_else(platform::ebadf)?.clone();
    // Write lock: the lseek moves the handle's shared position.
    #[allow(clippy::readonly_write_lock)]
    let f = data.file.write().unwrap();
    let fd = f.as_raw_fd();

    // Seek to 0 too: the previous READDIR left the position where it stopped.
    let ret = unsafe { libc::lseek64(fd, offset as i64, libc::SEEK_SET) };
    if ret < 0 {
        return Err(platform::linux_error(io::Error::last_os_error()));
    }

    let mut buf = vec![0u8; (size as usize).clamp(1024, 65536)];
    loop {
        let nread = unsafe { libc::syscall(libc::SYS_getdents64, fd, buf.as_mut_ptr(), buf.len()) };
        if nread < 0 {
            return Err(platform::linux_error(io::Error::last_os_error()));
        }
        if nread == 0 {
            return Ok(true);
        }

        let mut pos = 0usize;
        while pos < nread as usize {
            // linux_dirent64: d_ino, d_off, d_reclen, d_type, then the NUL-terminated name.
            let d_ino = u64::from_ne_bytes(buf[pos..pos + 8].try_into().unwrap());
            let d_off = u64::from_ne_bytes(buf[pos + 8..pos + 16].try_into().unwrap());
            let d_reclen = u16::from_ne_bytes(buf[pos + 16..pos + 18].try_into().unwrap()) as usize;
            let name = &buf[pos + 19..pos + d_reclen];
            let name_len = name.iter().position(|&b| b == 0).unwrap_or(name.len());

            let de = DirEntry {
                ino: d_ino,
                offset: d_off,
                type_: buf[pos + 18] as u32,
                name: &name[..name_len],
            };
            if add_entry(de)? == 0 {
                return Ok(false);
            }
            pos += d_reclen;
        }
    }
}

/// Serve the entries after `offset` from the handle's snapshot of the
/// directory, taken at offset 0 (open or rewind). Returns whether the listing
/// reached its end.
///
/// Offsets are 1-based positions in the snapshot, so they stay valid while the
/// guest deletes entries it already listed (`rm -r` does), which a fresh
/// enumeration per page would shift. macOS `telldir` cookies cannot serve
/// instead: they do not carry across `fdopendir` streams.
#[cfg(target_os = "macos")]
fn read_dir_entries(
    fs: &PassthroughFs,
    handle: u64,
    _size: u32,
    offset: u64,
    add_entry: &mut AddDirEntry<'_>,
) -> io::Result<bool> {
    // Take the snapshot out of the map first: inserting while holding its guard would deadlock.
    let current = fs
        .dir_snapshots
        .get(&handle)
        .filter(|_| offset > 0)
        .map(|s| Arc::clone(&s));
    let snapshot = match current {
        Some(s) => s,
        None => {
            let data = fs.handles.get(&handle).ok_or_else(platform::ebadf)?.clone();
            let s = Arc::new(snapshot_dir(&data)?);
            fs.dir_snapshots.insert(handle, Arc::clone(&s));
            s
        }
    };

    for (i, (ino, type_, name)) in snapshot.iter().enumerate().skip(offset as usize) {
        let de = DirEntry {
            ino: *ino,
            offset: i as u64 + 1,
            type_: *type_,
            name,
        };
        if add_entry(de)? == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Every entry of the directory open as `data`.
#[cfg(target_os = "macos")]
fn snapshot_dir(data: &HandleData) -> io::Result<super::DirSnapshot> {
    // Write lock: the lseek moves the handle's shared position.
    #[allow(clippy::readonly_write_lock)]
    let f = data.file.write().unwrap();
    let fd = f.as_raw_fd();

    // Rewind first: the stream below reads through a dup, which shares the position.
    let ret = unsafe { libc::lseek(fd, 0, libc::SEEK_SET) };
    if ret < 0 {
        return Err(platform::linux_error(io::Error::last_os_error()));
    }

    let dup_fd = unsafe { libc::dup(fd) };
    if dup_fd < 0 {
        return Err(platform::linux_error(io::Error::last_os_error()));
    }

    let dirp = unsafe { libc::fdopendir(dup_fd) };
    if dirp.is_null() {
        unsafe { libc::close(dup_fd) };
        return Err(platform::linux_error(io::Error::last_os_error()));
    }

    let mut entries = Vec::new();
    loop {
        unsafe { *libc::__error() = 0 };

        let ent = unsafe { libc::readdir(dirp) };
        if ent.is_null() {
            let errno = unsafe { *libc::__error() };
            unsafe { libc::closedir(dirp) };
            if errno != 0 {
                return Err(platform::linux_error(io::Error::from_raw_os_error(errno)));
            }
            return Ok(entries);
        }

        let d = unsafe { &*ent };
        let name = unsafe {
            std::slice::from_raw_parts(d.d_name.as_ptr() as *const u8, d.d_namlen as usize)
        };
        entries.push((d.d_ino, d.d_type as u32, name.into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DynFileSystem, FsOptions};

    const CTX: Context = Context {
        uid: 0,
        gid: 0,
        pid: 1,
    };

    /// A filesystem over a directory of files `f0..f5`, and a handle open on its root.
    fn six_files() -> (tempfile::TempDir, PassthroughFs, u64) {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..6 {
            std::fs::write(dir.path().join(format!("f{i}")), b"x").unwrap();
        }
        let fs = PassthroughFs::builder()
            .root_dir(dir.path())
            .build()
            .unwrap();
        fs.init(FsOptions::empty()).unwrap();
        let (dh, _) = fs.opendir(CTX, 1, 0).unwrap();
        (dir, fs, dh.unwrap())
    }

    /// Whether `name` holds a lookup reference besides the one this call takes.
    fn pinned(fs: &PassthroughFs, name: &[u8]) -> bool {
        let entry = fs.lookup(CTX, 1, &CString::new(name).unwrap()).unwrap();
        fs.forget(CTX, entry.inode, 1);
        fs.inodes.read().unwrap().get(&entry.inode).is_some()
    }

    /// READDIRPLUS keeps a lookup reference only for the entries that fit in
    /// the reply: the kernel forgets just those, so any other stays pinned.
    /// Listing stops at the first entry the reply turns away.
    #[test]
    fn readdirplus_pins_only_the_entries_in_the_reply() {
        let (_dir, fs, dh) = six_files();
        let mut sent = Vec::new();
        let mut calls = 0;
        fs.readdirplus_for_each(CTX, 1, dh, 64 * 1024, 0, &mut |de, _| {
            calls += 1;
            if sent.len() == 2 {
                return Ok(0);
            }
            sent.push(de.name.to_vec());
            Ok(1)
        })
        .unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(calls, 3, "kept listing after the reply was full");

        for i in 0..6 {
            let name = format!("f{i}").into_bytes();
            assert_eq!(pinned(&fs, &name), sent.contains(&name), "f{i}");
        }
    }

    /// An error reply carries no entries, so READDIRPLUS gives back the
    /// references of the entries it had already added.
    #[test]
    fn readdirplus_error_pins_nothing() {
        let (_dir, fs, dh) = six_files();
        let mut added = 0;
        let res = fs.readdirplus_for_each(CTX, 1, dh, 64 * 1024, 0, &mut |_, _| {
            if added == 2 {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            added += 1;
            Ok(1)
        });
        assert!(res.is_err());

        for i in 0..6 {
            assert!(!pinned(&fs, format!("f{i}").as_bytes()), "f{i}");
        }
    }
}
