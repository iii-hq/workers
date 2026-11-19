// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Native64 Linux metadata helpers, copied as an explicit narrow UAPI contract,
//! not authority from compatibility custody or the unintegrated clone branch.
use super::storage::*;
use std::{
    collections::BTreeMap,
    ffi::{CString, OsStr, OsString},
    fs::File,
    io,
    os::{
        fd::AsRawFd,
        unix::ffi::{OsStrExt, OsStringExt},
    },
};
pub(super) type Attrs = BTreeMap<Vec<u8>, Vec<u8>>;
#[repr(C, align(8))]
struct XattrArgs {
    value: u64,
    size: u32,
    flags: u32,
}
const _: [(); 16] = [(); std::mem::size_of::<XattrArgs>()];
// Linux v6.13 native64 x86_64/asm-generic xattrat UAPI, already verified for E.
#[cfg(all(
    target_os = "linux",
    target_pointer_width = "64",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
const SET: libc::c_long = 463;
#[cfg(all(
    target_os = "linux",
    target_pointer_width = "64",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
const GET: libc::c_long = 464;
#[cfg(all(
    target_os = "linux",
    target_pointer_width = "64",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
const LIST: libc::c_long = 465;
#[cfg(all(
    target_os = "linux",
    target_pointer_width = "64",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
const REMOVE: libc::c_long = 466;
pub(super) fn attrs(parent: &File, leaf: &OsStr) -> io::Result<Attrs> {
    #[cfg(all(
        target_os = "linux",
        target_pointer_width = "64",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        let leaf = name(leaf)?;
        let n = unsafe {
            libc::syscall(
                LIST,
                parent.as_raw_fd(),
                leaf.as_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
                std::ptr::null_mut::<u8>(),
                0usize,
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n > 1024 * 1024 {
            return Err(limit());
        }
        let mut list = vec![0; n as usize];
        if n > 0
            && unsafe {
                libc::syscall(
                    LIST,
                    parent.as_raw_fd(),
                    leaf.as_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                    list.as_mut_ptr(),
                    list.len(),
                )
            } != n
        {
            return Err(invalid("xattr list changed"));
        }
        let mut result = Attrs::new();
        let mut total = 0usize;
        for key in list.split(|b| *b == 0).filter(|k| !k.is_empty()) {
            if result.len() >= 256 {
                return Err(limit());
            }
            let c = CString::new(key).map_err(|_| invalid("xattr key"))?;
            let mut args = XattrArgs {
                value: 0,
                size: 0,
                flags: 0,
            };
            let n = unsafe {
                libc::syscall(
                    GET,
                    parent.as_raw_fd(),
                    leaf.as_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                    c.as_ptr(),
                    &mut args,
                    std::mem::size_of::<XattrArgs>(),
                )
            };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            total = total
                .checked_add(n as usize + key.len())
                .ok_or_else(limit)?;
            if total > 1024 * 1024 {
                return Err(limit());
            }
            let mut value = vec![0; n as usize];
            args.value = value.as_mut_ptr() as usize as u64;
            args.size = n as u32;
            if unsafe {
                libc::syscall(
                    GET,
                    parent.as_raw_fd(),
                    leaf.as_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                    c.as_ptr(),
                    &mut args,
                    std::mem::size_of::<XattrArgs>(),
                )
            } != n
            {
                return Err(invalid("xattr value changed"));
            }
            result.insert(key.to_vec(), value);
        }
        Ok(result)
    }
    #[cfg(not(all(
        target_os = "linux",
        target_pointer_width = "64",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        let _ = (parent, leaf);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "payload xattrs need verified native64 xattrat",
        ))
    }
}
pub(super) fn apply(parent: &File, leaf: &OsStr, expected: &Attrs) -> io::Result<()> {
    #[cfg(all(
        target_os = "linux",
        target_pointer_width = "64",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        let current = attrs(parent, leaf)?;
        let leaf = name(leaf)?;
        for key in current.keys().filter(|k| !expected.contains_key(*k)) {
            let key = CString::new(key.as_slice()).map_err(|_| invalid("xattr key"))?;
            if unsafe {
                libc::syscall(
                    REMOVE,
                    parent.as_raw_fd(),
                    leaf.as_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                    key.as_ptr(),
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        for (key, value) in expected {
            let key = CString::new(key.as_slice()).map_err(|_| invalid("xattr key"))?;
            let args = XattrArgs {
                value: value.as_ptr() as usize as u64,
                size: value.len() as u32,
                flags: 0,
            };
            if unsafe {
                libc::syscall(
                    SET,
                    parent.as_raw_fd(),
                    leaf.as_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                    key.as_ptr(),
                    &args,
                    std::mem::size_of::<XattrArgs>(),
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
    #[cfg(not(all(
        target_os = "linux",
        target_pointer_width = "64",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        let _ = (parent, leaf, expected);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "payload metadata requires native64 Linux",
        ))
    }
}
pub(super) fn names(file: &File, max: usize) -> io::Result<Vec<(OsString, u64)>> {
    let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let raw = unsafe { libc::fdopendir(fd) };
    if raw.is_null() {
        unsafe { libc::close(fd) };
        return Err(io::Error::last_os_error());
    }
    struct Dir(*mut libc::DIR);
    impl Drop for Dir {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    let dir = Dir(raw);
    unsafe { libc::rewinddir(dir.0) };
    let mut result = Vec::new();
    loop {
        #[cfg(target_os = "linux")]
        unsafe {
            *libc::__errno_location() = 0;
        }
        let e = unsafe { libc::readdir(dir.0) };
        if e.is_null() {
            #[cfg(target_os = "linux")]
            if unsafe { *libc::__errno_location() } != 0 {
                return Err(io::Error::last_os_error());
            }
            break;
        }
        let bytes = unsafe { std::ffi::CStr::from_ptr((*e).d_name.as_ptr()) }.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        if result.len() >= max {
            return Err(limit());
        }
        let inode = unsafe { (*e).d_ino as u64 };
        if inode == 0 {
            return Err(invalid("filesystem cannot prove member inode"));
        }
        result.push((OsString::from_vec(bytes.to_vec()), inode));
    }
    result.sort();
    Ok(result)
}
/// The target of the symlink inode `pin` itself (an `O_PATH | O_NOFOLLOW`
/// descriptor), read through the descriptor with an empty name (Linux >=
/// 2.6.39), never through a name a concurrent rename could rebind.
/// readlink(2) always touches the inode's atime (O_NOATIME cannot apply to it);
/// callers compare symlink pins without atime for exactly that reason.
pub(super) fn read_link(pin: &File) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; 4097];
    let n = unsafe {
        libc::readlinkat(
            pin.as_raw_fd(),
            c"".as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if n as usize >= bytes.len() {
        return Err(limit());
    }
    bytes.truncate(n as usize);
    Ok(bytes)
}
