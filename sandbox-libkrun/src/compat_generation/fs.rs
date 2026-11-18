// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
use super::{Error, Faults, MAX_RECORD, Result, Step};
use sha2::{Digest, Sha256};
use std::{
    ffi::{CStr, CString, OsStr},
    fs::File,
    io::{Read, Write},
    mem::zeroed,
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Identity(pub u64, pub u64);
pub(super) struct Dir {
    chain: Vec<File>,
    names: Vec<CString>,
    path: PathBuf,
}
fn c(name: &[u8]) -> Result<CString> {
    if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
        return Err(Error::Custody);
    }
    CString::new(name).map_err(|_| Error::Custody)
}
fn owned(fd: i32) -> Result<File> {
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: the successful syscall returned a new descriptor owned here.
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub(super) fn uid() -> u32 {
    unsafe { libc::geteuid() }
}
impl Dir {
    pub(super) fn open(path: &Path, private: bool) -> Result<Self> {
        if !path.is_absolute() || path.as_os_str().as_bytes().len() > 4096 {
            return Err(Error::Custody);
        }
        let mut chain = vec![owned(unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?];
        let mut names = vec![];
        for component in path.components() {
            let name = match component {
                Component::RootDir => continue,
                Component::Normal(n) => c(n.as_bytes())?,
                _ => return Err(Error::Custody),
            };
            let file = owned(unsafe {
                libc::openat(
                    chain.last().unwrap().as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            })?;
            names.push(name);
            chain.push(file);
        }
        for (i, f) in chain.iter().enumerate() {
            let m = f.metadata()?;
            let leaf = i + 1 == chain.len();
            if private && leaf {
                if m.uid() != uid() || m.mode() & 0o7777 != 0o700 {
                    return Err(Error::Custody);
                }
            } else if i + 1 < chain.len() {
                let sticky = m.uid() == 0 && m.mode() & 0o1000 != 0;
                if (m.uid() != 0 && m.uid() != uid()) || (m.mode() & 0o022 != 0 && !sticky) {
                    return Err(Error::Custody);
                }
            }
        }
        Ok(Self {
            chain,
            names,
            path: path.to_path_buf(),
        })
    }
    pub(super) fn fd(&self) -> RawFd {
        self.chain.last().unwrap().as_raw_fd()
    }
    pub(super) fn identity(&self) -> Result<Identity> {
        let m = self.chain.last().unwrap().metadata()?;
        Ok(Identity(m.dev(), m.ino()))
    }
    pub(super) fn revalidate(&self, private: bool) -> Result<()> {
        let reopened = Self::open(&self.path, private)?;
        if reopened.chain.len() != self.chain.len() {
            return Err(Error::Custody);
        }
        for (a, b) in self.chain.iter().zip(&reopened.chain) {
            let a = a.metadata()?;
            let b = b.metadata()?;
            if a.dev() != b.dev() || a.ino() != b.ino() {
                return Err(Error::Custody);
            }
        }
        Ok(())
    }
    pub(super) fn disjoint(&self, other: &Self) -> Result<bool> {
        let a = self.identity()?;
        let b = other.identity()?;
        for f in &self.chain {
            let m = f.metadata()?;
            if Identity(m.dev(), m.ino()) == b {
                return Ok(false);
            }
        }
        for f in &other.chain {
            let m = f.metadata()?;
            if Identity(m.dev(), m.ino()) == a {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(super) fn child(&self, name: &str) -> Result<Self> {
        let name = c(name.as_bytes())?;
        let mut chain = self
            .chain
            .iter()
            .map(File::try_clone)
            .collect::<std::io::Result<Vec<_>>>()?;
        chain.push(owned(unsafe {
            libc::openat(
                self.fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?);
        let mut names = self.names.clone();
        names.push(name.clone());
        Ok(Self {
            chain,
            names,
            path: self.path.join(OsStr::from_bytes(name.to_bytes())),
        })
    }
    pub(super) fn mkdir(&self, name: &str) -> Result<Self> {
        let name_c = c(name.as_bytes())?;
        if unsafe { libc::mkdirat(self.fd(), name_c.as_ptr(), 0o700) } != 0 {
            let e = std::io::Error::last_os_error();
            return Err(if e.raw_os_error() == Some(libc::EEXIST) {
                Error::Collision
            } else {
                e.into()
            });
        }
        self.child(name)
    }
    pub(super) fn exists(&self, name: &str) -> Result<bool> {
        let name = c(name.as_bytes())?;
        let mut st = unsafe { zeroed::<libc::stat>() };
        if unsafe { libc::fstatat(self.fd(), name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) }
            == 0
        {
            return Ok(true);
        }
        let e = std::io::Error::last_os_error();
        if e.kind() == std::io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(e.into())
        }
    }
    pub(super) fn lock(&self) -> Result<File> {
        // Separate open description so every instance contends; retained until
        // transaction/selection handle drops. No lockfile in removable VM state.
        let f = owned(unsafe {
            libc::openat(
                self.fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(f)
    }
    pub(super) fn sync(&self, fault: &mut Faults) -> Result<()> {
        fault.at(Step::DirectorySync)?;
        self.chain.last().unwrap().sync_all()?;
        Ok(())
    }
    pub(super) fn write_record(&self, name: &str, bytes: &[u8], fault: &mut Faults) -> Result<()> {
        if bytes.len() > MAX_RECORD {
            return Err(Error::Corrupt);
        }
        let name = c(name.as_bytes())?;
        let mut f = owned(unsafe {
            libc::openat(
                self.fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        })?;
        f.write_all(bytes)?;
        fault.at(Step::FileSync)?;
        f.sync_all()?;
        Ok(())
    }
    pub(super) fn read_record(&self, name: &str) -> Result<Vec<u8>> {
        let name = c(name.as_bytes())?;
        let file = owned(unsafe {
            libc::openat(
                self.fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        })?;
        let m = file.metadata()?;
        if !m.is_file()
            || m.uid() != uid()
            || m.mode() & 0o7777 != 0o600
            || m.nlink() != 1
            || m.len() > MAX_RECORD as u64
        {
            return Err(Error::Corrupt);
        }
        let mut bytes = vec![];
        file.take(MAX_RECORD as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_RECORD {
            return Err(Error::Corrupt);
        }
        Ok(bytes)
    }
    pub(super) fn sync_record(&self, name: &str, fault: &mut Faults) -> Result<()> {
        // Validate type/private ownership before reopening the same held-dir leaf.
        self.read_record(name)?;
        let name = c(name.as_bytes())?;
        let f = owned(unsafe {
            libc::openat(
                self.fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        fault.at(Step::FileSync)?;
        f.sync_all()?;
        Ok(())
    }
    pub(super) fn duplicate(&self) -> Result<Self> {
        Ok(Self {
            chain: self
                .chain
                .iter()
                .map(File::try_clone)
                .collect::<std::io::Result<Vec<_>>>()?,
            names: self.names.clone(),
            path: self.path.clone(),
        })
    }
    pub(super) fn rename_new(&self, from: &str, to_dir: &Self, to: &str) -> Result<()> {
        let from = c(from.as_bytes())?;
        let to = c(to.as_bytes())?;
        // Linux no-replace; unsupported filesystem/kernel is an error, never a
        // check-then-rename fallback. Both directory handles remain pinned.
        if unsafe {
            libc::renameat2(
                self.fd(),
                from.as_ptr(),
                to_dir.fd(),
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            let e = std::io::Error::last_os_error();
            return Err(if e.raw_os_error() == Some(libc::EEXIST) {
                Error::Collision
            } else {
                e.into()
            });
        }
        Ok(())
    }
    pub(super) fn replace_selector(&self, from: &str) -> Result<()> {
        let from = c(from.as_bytes())?;
        if unsafe { libc::renameat(self.fd(), from.as_ptr(), self.fd(), c"active.json".as_ptr()) }
            != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
}
pub(super) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub(super) fn name(bytes: &[u8]) -> Result<CString> {
    c(bytes)
}
pub(super) fn file(fd: i32) -> Result<File> {
    owned(fd)
}
pub(super) fn mount_id(fd: RawFd) -> Result<u64> {
    let mut st = unsafe { std::mem::zeroed::<libc::statx>() };
    if unsafe {
        libc::statx(
            fd,
            c"".as_ptr(),
            libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            &mut st,
        )
    } != 0
        || st.stx_mask & libc::STATX_MNT_ID == 0
    {
        return Err(Error::Unsupported("mount identity unavailable"));
    }
    Ok(st.stx_mnt_id)
}
pub(super) fn names(fd: RawFd) -> Result<Vec<Vec<u8>>> {
    // Independent directory offset; no recursive pathname traversal or symlink.
    let dup = unsafe {
        libc::openat(
            fd,
            c".".as_ptr(),
            libc::O_RDONLY
                | libc::O_DIRECTORY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NOATIME,
        )
    };
    if dup < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let dir = unsafe { libc::fdopendir(dup) };
    if dir.is_null() {
        unsafe { libc::close(dup) };
        return Err(std::io::Error::last_os_error().into());
    }
    struct Close(*mut libc::DIR);
    impl Drop for Close {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    let _close = Close(dir);
    let mut result = vec![];
    loop {
        unsafe { *libc::__errno_location() = 0 };
        let entry = unsafe { libc::readdir(dir) };
        if entry.is_null() {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(0) {
                return Err(e.into());
            }
            break;
        }
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes != b"." && bytes != b".." {
            result.push(bytes.to_vec());
            if result.len() > super::MAX_ENTRIES {
                return Err(Error::Unsupported("directory entry limit"));
            }
        }
    }
    result.sort();
    Ok(result)
}
