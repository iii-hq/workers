// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

//! Cooperative worker lifecycle admission, not migration authority. Normal
//! operations hold shared locks on existing ancestor directories; no marker or
//! lockfile is created. An owner transition must exclusively lock the SAME
//! ancestor before enrolling custody and keep it through its generation work.
//! Old/uncooperative processes do not participate and still need retirement.
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const REFUSED: &str = "compatibility custody is present, busy or unsafe; local owner authority, retired writers/credentials, sanitization and an admitted boot profile are required";
#[derive(Clone)]
pub(crate) struct Lease(Arc<Held>);
/// Once a writer was spawned, ordinary Drop cannot prove it stopped. Explicit
/// successful reap releases; cancellation/unknown ownership retains the lease.
pub(crate) struct WriterLease(Option<Vec<Lease>>);
impl WriterLease {
    pub(crate) fn new(leases: Vec<Lease>) -> Self {
        Self(Some(leases))
    }
    pub(crate) fn reaped(mut self) {
        drop(self.0.take());
    }
}
impl Drop for WriterLease {
    fn drop(&mut self) {
        if let Some(leases) = self.0.take() {
            std::mem::forget(leases);
        }
    }
}

struct Held {
    directories: Vec<File>,
    paths: Vec<PathBuf>,
}
impl Lease {
    pub(crate) fn revalidate(&self) -> io::Result<()> {
        for (path, held) in self.0.paths.iter().zip(&self.0.directories) {
            let current = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)?;
            let a = current.metadata()?;
            let b = held.metadata()?;
            if a.dev() != b.dev() || a.ino() != b.ino() {
                return Err(refused());
            }
        }
        Ok(())
    }
}
pub(crate) fn refused() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, REFUSED)
}
fn vm_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn present(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Private immutable address, derived from the same base+vm_id the worker uses.
/// Sibling of base, never inside the guest share or <vm_id> removed by prune.
pub(crate) fn custody(base: &Path, id: &str) -> io::Result<PathBuf> {
    if !vm_id(id) {
        return Err(refused());
    }
    let name = base.file_name().ok_or_else(refused)?;
    let mut folder = std::ffi::OsString::from(".");
    folder.push(name);
    folder.push("-compat");
    Ok(base.parent().ok_or_else(refused)?.join(folder).join(id))
}
fn private_tree(path: &Path, id: &str) -> io::Result<()> {
    // Validation is diagnostic, never authorization. All compatibility states
    // refuse public mutation, including valid active records and incomplete ones.
    #[cfg(target_os = "linux")]
    crate::compat_generation::admission::inspect(path, id)
        .map(|_| ())
        .map_err(|_| refused())?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, id);
        return Err(refused());
    }
    Ok(())
}

/// Hold every existing ancestor of base.parent (NOT base itself, whose existing
/// build-dir claims use their own exclusive flock). This also fences enrollment
/// while base/parents are absent and created later. Root locks are shared; an
/// enrollment on an overly broad parent may conservatively be busy.
fn shared_ancestors(base: &Path) -> io::Result<Lease> {
    ancestor_lease(base, false)
}
fn ancestor_lease(base: &Path, exclusive: bool) -> io::Result<Lease> {
    let parent = base.parent().ok_or_else(refused)?;
    if !base.is_absolute() {
        return Err(refused());
    }
    let mut path = PathBuf::from("/");
    let mut dirs = Vec::new();
    let mut paths = Vec::new();
    let root = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    lock_shared(&root)?;
    dirs.push(root);
    paths.push(path.clone());
    for part in parent.components() {
        let name = match part {
            Component::RootDir => continue,
            Component::Normal(n) => n,
            _ => return Err(refused()),
        };
        let c = CString::new(name.as_bytes()).map_err(|_| refused())?;
        let fd = unsafe {
            libc::openat(
                dirs.last().unwrap().as_raw_fd(),
                c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::NotFound {
                break;
            }
            return Err(e);
        }
        // SAFETY: successful openat returns one new owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        lock_shared(&file)?;
        path.push(name);
        paths.push(path.clone());
        dirs.push(file);
    }
    if exclusive {
        // Upgrade on the final EXISTING owner ancestor. Other shared holders
        // prevent transition; failure is busy, never a warning/fallback.
        if unsafe {
            libc::flock(
                dirs.last().unwrap().as_raw_fd(),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        } != 0
        {
            return Err(refused());
        }
    }
    let lease = Lease(Arc::new(Held {
        directories: dirs,
        paths,
    }));
    lease.revalidate()?;
    Ok(lease)
}
/// Only the private compatibility module can possess TransitionAuthority. This
/// lock is exclusion, not a replacement for old writer/credential retirement.
#[cfg(target_os = "linux")]
pub(crate) fn transition(
    base: &Path,
    id: &str,
    _proof: &crate::compat_generation::admission::TransitionAuthority,
) -> io::Result<Lease> {
    if !vm_id(id) {
        return Err(refused());
    }
    ancestor_lease(base, true)
}
fn lock_shared(file: &File) -> io::Result<()> {
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
        return Err(refused());
    }
    Ok(())
}
fn directory_no_follow(path: &Path) -> io::Result<()> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map(drop)
}
fn inspect_vm(base: &Path, id: &str) -> io::Result<()> {
    // Reject substituted base/registry ancestors even when the per-VM child
    // does not yet exist; never let ENOENT below a symlink imply normal mode.
    if present(base)? {
        directory_no_follow(base)?;
    }
    let registry = custody(base, id)?.parent().unwrap().to_path_buf();
    if present(&registry)? {
        directory_no_follow(&registry)?;
    }
    let runtime = base.join(id);
    if present(&runtime)? {
        directory_no_follow(&runtime)?;
    }
    for name in ["compat", "compat-mode", "compat-generation"] {
        if present(&runtime.join(name))? {
            return Err(refused());
        }
    }
    let path = custody(base, id)?;
    if present(&path)? {
        private_tree(&path, id)?;
        return Err(refused()); // A valid selector is NOT a sanitized boot grant.
    }
    Ok(())
}

/// Used AFTER existing authentication and BEFORE next_instance/claims/download/
/// archive/delete. None means prepare without a key: inspect ALL enrolled VMs,
/// because that request cannot prove which compatibility scope it would affect.
pub(crate) fn worker(base: &Path, state_key: Option<&str>) -> io::Result<Lease> {
    let lease = shared_ancestors(base)?;
    if let Some(key) = state_key {
        let id = hex::encode(Sha256::digest(key))[..32].to_string();
        inspect_vm(base, &id)?;
    } else {
        let placeholder = "0".repeat(32);
        let registry = custody(base, &placeholder)?.parent().unwrap().to_path_buf();
        if present(&registry)? {
            // Empty enrollment itself is incomplete, not permission to prepare.
            return Err(refused());
        }
        if present(base)? {
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(base)?;
            // Only filenames and fixed markers, no foreign state/token bodies.
            let _held = directory;
            for entry in std::fs::read_dir(base)? {
                let entry = entry?;
                if let Some(id) = entry.file_name().to_str().filter(|id| vm_id(id)) {
                    inspect_vm(base, id)?;
                }
            }
        }
    }
    lease.revalidate()?;
    Ok(lease)
}

/// Direct local VM builders must not bypass admission by using state_dir instead
/// of WorkerVms. Recognize canonical worker roots/build scratch and any retained
/// compatibility generation/backup ancestor, without treating names as authority.
/// Protect any supplied file/directory under retained generation/backup custody.
/// Read-only inspection: does not turn the path into a grant or acquire a writer.
pub(crate) fn reject_retained_path(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(refused()),
            Ok(meta) if meta.is_file() => continue,
            Ok(meta) if !meta.is_dir() => return Err(refused()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
            _ => {}
        }
        if ancestor
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.') && n.ends_with("-compat"))
        {
            return Err(refused());
        }
        for marker in [
            "receipt.json",
            "backup.json",
            "active.json",
            "compat-mode",
            "compat-generation",
        ] {
            if present(&ancestor.join(marker))? {
                return Err(refused());
            }
        }
    }
    Ok(())
}

pub(crate) fn local(state_dir: &Path) -> io::Result<Lease> {
    let worker_parent = state_dir
        .ancestors()
        .find(|path| path.file_name().and_then(|n| n.to_str()).is_some_and(vm_id))
        .and_then(Path::parent);
    let local_lease = shared_ancestors(worker_parent.unwrap_or(state_dir))?;
    for path in state_dir.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if present(path)? {
            directory_no_follow(path)?;
        }
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.') && n.ends_with("-compat"))
        {
            return Err(refused());
        }
    }
    for path in state_dir.ancestors() {
        for name in [
            "compat",
            "compat-mode",
            "compat-generation",
            "receipt.json",
            "backup.json",
            "active.json",
        ] {
            if present(&path.join(name))? {
                return Err(refused());
            }
        }
        if let Some(id) = path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|id| vm_id(id))
        {
            let base = path.parent().ok_or_else(refused)?;
            inspect_vm(base, id)?;
            local_lease.revalidate()?;
            return Ok(local_lease);
        }
    }
    local_lease.revalidate()?;
    Ok(local_lease)
}

#[cfg(test)]
mod tests;
