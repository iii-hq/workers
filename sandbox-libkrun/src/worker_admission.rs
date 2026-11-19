// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.

//! Cooperative lifecycle exclusion, NOT migration authority or retirement.
//! An owner domain is exactly base.parent(), which must already exist. Never
//! fall back to a shallower inode when a hierarchy is missing. No lockfile or
//! directory is created, and the build-claim flock on base is not borrowed.
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
    pins: Vec<PinnedPath>,
    checks: Vec<Check>,
    retained: Vec<Lease>,
}
enum Check {
    Worker { base: PathBuf, id: Option<String> },
    Input(PathBuf),
}
/// Directory descriptors pin identities, but only the owner-domain descriptor
/// carries flock. Opening ancestors is NOT taking locks on ancestor claim roots.
struct PinnedPath {
    path: PathBuf,
    directory: bool,
    directories: Vec<(PathBuf, File)>,
    file_identity: Option<(u64, u64)>,
}
impl PinnedPath {
    fn revalidate(&self) -> io::Result<()> {
        let current = pin_path(&self.path, self.directory)?;
        if current.directories.len() < self.directories.len() {
            return Err(refused());
        }
        for ((path, held), (now, file)) in self.directories.iter().zip(&current.directories) {
            let a = held.metadata()?;
            let b = file.metadata()?;
            if path != now || a.dev() != b.dev() || a.ino() != b.ino() {
                return Err(refused());
            }
        }
        if self.file_identity.is_some() && self.file_identity != current.file_identity {
            return Err(refused());
        }
        Ok(())
    }
}
impl Lease {
    pub(crate) fn revalidate(&self) -> io::Result<()> {
        for lease in &self.0.retained {
            lease.revalidate()?;
        }
        for pin in &self.0.pins {
            pin.revalidate()?;
        }
        for check in &self.0.checks {
            match check {
                Check::Worker { base, id } => inspect_worker(base, id.as_deref())?,
                Check::Input(path) => inspect_input(path)?,
            }
        }
        Ok(())
    }
    fn worker_bases(&self, bases: &mut Vec<PathBuf>) {
        for check in &self.0.checks {
            match check {
                Check::Worker { base, .. } => bases.push(base.clone()),
                Check::Input(path) => {
                    bases.extend(input_vms(path).into_iter().map(|(base, _)| base))
                }
            }
        }
        for lease in &self.0.retained {
            lease.worker_bases(bases);
        }
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
/// Relative inputs are anchored once, without canonicalize (which would erase
/// symlinks). Reject traversal even in the not-yet-existing tail. A `.` spelling
/// changes no lookup; all surviving components are still opened no-follow.
fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.as_os_str().is_empty() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(refused());
    }
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => result.push(component.as_os_str()),
            Component::CurDir => {}
            _ => return Err(refused()),
        }
    }
    Ok(result)
}
fn open_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}
/// Pin the existing prefix with descriptor-relative, no-follow opens. A missing
/// tail is allowed, but never skips lexical enrollment discovery or validation.
fn pin_path(path: &Path, directory: bool) -> io::Result<PinnedPath> {
    let path = absolute(path)?;
    let mut here = PathBuf::from("/");
    let mut directories = vec![(here.clone(), open_directory(&here)?)];
    let mut file_identity = None;
    let mut parts = path.components().peekable();
    while let Some(part) = parts.next() {
        let name = match part {
            Component::RootDir => continue,
            Component::Normal(name) => name,
            _ => return Err(refused()),
        };
        let c = CString::new(name.as_bytes()).map_err(|_| refused())?;
        let parent = &directories.last().ok_or_else(refused)?.1;
        // SAFETY: parent is an owned directory descriptor; c is NUL terminated.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        here.push(name);
        if fd < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::NotFound {
                break;
            }
            if !directory && parts.peek().is_none() {
                let metadata = std::fs::symlink_metadata(&here)?;
                if metadata.is_file() {
                    file_identity = Some((metadata.dev(), metadata.ino()));
                    break;
                }
            }
            return Err(e);
        }
        // SAFETY: successful openat returns one new owned descriptor.
        directories.push((here.clone(), unsafe { File::from_raw_fd(fd) }));
    }
    Ok(PinnedPath {
        path,
        directory,
        directories,
        file_identity,
    })
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

fn shared_ancestors(base: &Path) -> io::Result<Lease> {
    ancestor_lease(base, false)
}
/// The domain must exist in full BEFORE admission. Refusing a missing owner
/// parent is intentional: locking the deepest existing prefix is not stable as
/// normal preparation creates the rest. Never flock `/`, nor flock `base`.
fn ancestor_lease(base: &Path, exclusive: bool) -> io::Result<Lease> {
    if !base.is_absolute() {
        return Err(refused());
    }
    let base = absolute(base)?;
    let parent = base
        .parent()
        .filter(|p| p.parent().is_some())
        .ok_or_else(refused)?;
    let pin = pin_path(parent, true)?;
    let (last_path, file) = pin.directories.last().ok_or_else(refused)?;
    if last_path != parent {
        return Err(refused());
    }
    let mode = if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_SH
    };
    // SAFETY: file remains owned by the returned lease, including every clone.
    if unsafe { libc::flock(file.as_raw_fd(), mode | libc::LOCK_NB) } != 0 {
        return Err(refused());
    }
    let lease = Lease(Arc::new(Held {
        pins: vec![pin],
        checks: Vec::new(),
        retained: Vec::new(),
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
fn directory_no_follow(path: &Path) -> io::Result<()> {
    let pin = pin_path(path, true)?;
    if pin.directories.last().map(|(path, _)| path) != Some(&pin.path) {
        return Err(refused());
    }
    Ok(())
}
fn inspect_vm(base: &Path, id: &str) -> io::Result<()> {
    inspect_vm_address(base, id, true)
}
fn inspect_vm_address(base: &Path, id: &str, legacy: bool) -> io::Result<()> {
    if present(base)? {
        directory_no_follow(base)?;
    }
    let path = custody(base, id)?;
    let registry = path.parent().ok_or_else(refused)?;
    if present(registry)? {
        directory_no_follow(registry)?;
        let metadata = std::fs::symlink_metadata(registry)?;
        // This is the derived private namespace, not an arbitrary application
        // directory. Wrong ownership/permissions are not absence of enrollment.
        // SAFETY: geteuid has no pointer arguments and does not change credentials.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o700 {
            return Err(refused());
        }
        for entry in std::fs::read_dir(registry)? {
            if !entry?.file_name().to_str().is_some_and(vm_id) {
                return Err(refused()); // Unknown registry entries have no safe VM scope.
            }
        }
    }
    let runtime = base.join(id);
    if present(&runtime)? {
        directory_no_follow(&runtime)?;
        if legacy {
            for name in ["compat", "compat-mode", "compat-generation"] {
                if present(&runtime.join(name))? {
                    return Err(refused());
                }
            }
        }
    }
    if present(&path)? {
        private_tree(&path, id)?;
        return Err(refused()); // A valid selector is NOT a sanitized boot grant.
    }
    Ok(())
}
fn inspect_worker(base: &Path, id: Option<&str>) -> io::Result<()> {
    if let Some(id) = id {
        return inspect_vm(base, id);
    }
    let path = custody(base, &"0".repeat(32))?;
    if present(path.parent().ok_or_else(refused)?)? {
        return Err(refused()); // Even empty enrollment is incomplete.
    }
    if present(base)? {
        directory_no_follow(base)?;
        for entry in std::fs::read_dir(base)? {
            let entry = entry?;
            if let Some(id) = entry.file_name().to_str().filter(|id| vm_id(id)) {
                inspect_vm(base, id)?;
            }
        }
    }
    Ok(())
}

/// Used AFTER authentication and BEFORE next_instance/claims/download/archive.
/// None inspects ALL enrolled VMs: a keyless request cannot pick a safe scope.
pub(crate) fn worker(base: &Path, state_key: Option<&str>) -> io::Result<Lease> {
    let fence = shared_ancestors(base)?;
    let base = absolute(base)?;
    let id = state_key.map(|key| hex::encode(Sha256::digest(key))[..32].to_string());
    let lease = Lease(Arc::new(Held {
        pins: vec![pin_path(&base, true)?],
        checks: vec![Check::Worker { base, id }],
        retained: vec![fence],
    }));
    lease.revalidate()?;
    Ok(lease)
}

/// An id-shaped directory is only a candidate address, never ownership proof.
/// Check every such ancestor, including absent ones, against its sibling registry.
fn input_vms(path: &Path) -> Vec<(PathBuf, String)> {
    path.ancestors()
        .filter_map(|runtime| {
            let id = runtime.file_name()?.to_str().filter(|id| vm_id(id))?;
            // A regular input file with a hex filename is not a VM directory.
            if std::fs::symlink_metadata(runtime).is_ok_and(|m| m.is_file()) {
                return None;
            }
            Some((runtime.parent()?.to_path_buf(), id.to_string()))
        })
        .collect()
}
fn inspect_input(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        // The reserved sibling address derives a base name; generic `compat`
        // and application receipt/active/backup filenames carry no such binding.
        if ancestor
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| {
                n.strip_prefix('.')
                    .and_then(|n| n.strip_suffix("-compat"))
                    .is_some_and(|base| !base.is_empty())
            })
        {
            return Err(refused());
        }
    }
    for (base, id) in input_vms(path) {
        let runtime = base.join(&id);
        // A hex-named application directory alone is not a legacy VM. For
        // local inputs use the canonical runtime layout, or a derived registry;
        // worker(base, key) already has an explicitly identified VM address.
        let canonical_runtime = path
            .strip_prefix(&runtime)
            .ok()
            .and_then(|tail| tail.components().next())
            .and_then(|part| part.as_os_str().to_str())
            .is_some_and(|name| {
                name == "root" || name == "upper.ext4" || name.starts_with("build-")
            });
        let address = custody(&base, &id)?;
        let identified = canonical_runtime || present(address.parent().ok_or_else(refused)?)?;
        inspect_vm_address(&base, &id, identified)?;
    }
    // A supplied whole base has no vm_id ancestor. Its own derived sibling
    // registry still forbids repurposing it, even when the base is absent.
    if path.file_name().is_some() {
        let address = custody(path, &"0".repeat(32))?;
        if present(address.parent().ok_or_else(refused)?)? {
            return Err(refused());
        }
    }
    Ok(())
}
/// Read-only rejection helper, NOT a retained admission for later use of a path.
pub(crate) fn reject_retained_path(path: &Path) -> io::Result<()> {
    let pin = pin_path(path, false)?;
    inspect_input(&pin.path)?;
    pin.revalidate()
}
fn input_lease(path: &Path, directory: bool, known_bases: &[PathBuf]) -> io::Result<Lease> {
    let pin = pin_path(path, directory)?;
    let mut bases: Vec<PathBuf> = input_vms(&pin.path)
        .into_iter()
        .map(|(base, _)| base)
        .collect();
    let mut checks = vec![Check::Input(pin.path.clone())];
    // A known whole-base input must not bypass sibling enrollment checks, and
    // an input below that base must not take its existing exclusive claim flock.
    for base in known_bases {
        if (pin.path.starts_with(base) || base.starts_with(&pin.path)) && !bases.contains(base) {
            bases.push(base.clone());
            checks.push(Check::Worker {
                base: base.clone(),
                id: None,
            });
        }
    }
    if bases.is_empty() {
        bases.push(pin.path.clone());
    }
    bases.sort();
    bases.dedup();
    // Ambiguous nested layouts may make an owner domain someone else's claim
    // root. Refuse instead of taking that lock and hanging a later claim.
    if bases.iter().any(|base| {
        base.parent()
            .is_some_and(|parent| bases.iter().chain(known_bases).any(|b| b == parent))
    }) {
        return Err(refused());
    }
    let retained = bases
        .iter()
        .map(|base| shared_ancestors(base))
        .collect::<io::Result<Vec<_>>>()?;
    let lease = Lease(Arc::new(Held {
        pins: vec![pin],
        checks,
        retained,
    }));
    lease.revalidate()?;
    Ok(lease)
}
/// Admit all inputs before effects. Retain the original and every input fence
/// until the last clone dies; revalidation covers every component. No authority
/// is issued, and no path is canonicalized past a symlink or traversal.
pub(crate) fn with_inputs(admission: Lease, paths: &[&Path]) -> io::Result<Lease> {
    if paths.is_empty() {
        return Ok(admission);
    }
    let mut bases = Vec::new();
    admission.worker_bases(&mut bases);
    // Resolve the complete batch before locking: an input can identify the
    // claim root of another input, regardless of their order in the slice.
    let paths = paths
        .iter()
        .map(|path| absolute(path))
        .collect::<io::Result<Vec<_>>>()?;
    for path in &paths {
        bases.extend(input_vms(path).into_iter().map(|(base, _)| base));
    }
    bases.sort();
    bases.dedup();
    let mut retained = Vec::with_capacity(paths.len() + 1);
    retained.push(admission);
    for path in &paths {
        retained.push(input_lease(path, false, &bases)?);
    }
    let lease = Lease(Arc::new(Held {
        pins: Vec::new(),
        checks: Vec::new(),
        retained,
    }));
    lease.revalidate()?;
    Ok(lease)
}
pub(crate) fn local(state_dir: &Path) -> io::Result<Lease> {
    input_lease(state_dir, true, &[])
}

#[cfg(test)]
mod tests;
