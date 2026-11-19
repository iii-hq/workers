// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Owner-only temporary access. Requires Linux fchmodat2(AT_EMPTY_PATH), no procfd
//! pathname fallback. Exact original access/default ACLs precede every grant.
#[cfg(test)]
#[path = "owner_read_tests.rs"]
mod tests;
use super::{
    Lease, PathRole,
    catalog::{Hold, HoldKind, Journal, PermissionIntent, overlaps},
    storage::*,
};
use std::{
    collections::HashMap,
    ffi::OsString,
    fs::{File, Metadata},
    io,
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
pub(crate) struct OwnerReadEntry {
    pub name: OsString,
    pub metadata: Metadata,
}
struct Pinned {
    path: PathBuf,
    fd: File,
    parent: Directory,
    metadata: Metadata,
    identity: Identity,
    granted: u32,
    acl: Option<Vec<u8>>,
    default_acl: Option<Vec<u8>>,
    changed: bool,
}
pub(crate) struct OwnerReadGuard {
    lease: Lease,
    id: String,
    root: PathBuf,
    objects: Vec<Pinned>,
    finished: bool,
    failed: AtomicBool,
    #[cfg(test)]
    fault: Option<GrantFault>,
}
#[cfg(target_os = "linux")]
fn open_path(parent: &File, name: &std::ffi::OsStr) -> io::Result<File> {
    open_at(parent, name, libc::O_PATH, 0)
}
#[cfg(not(target_os = "linux"))]
fn open_path(_parent: &File, _name: &std::ffi::OsStr) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "owner-read requires Linux O_PATH",
    ))
}
fn chmod_pinned(fd: &File, mode: u32) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let empty = std::ffi::CString::new("").map_err(|_| invalid("empty name"))?;
        // fchmodat2 uses a retained O_PATH description. No /proc/self/fd path escape.
        let result = unsafe {
            libc::syscall(
                libc::SYS_fchmodat2,
                fd.as_raw_fd(),
                empty.as_ptr(),
                mode as libc::mode_t,
                libc::AT_EMPTY_PATH,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (fd, mode);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "owner-read requires fchmodat2",
        ))
    }
}
// Verified Linux v6.13 UAPI: __aligned_u64 value; __u32 size; __u32 flags.
#[repr(C, align(8))]
struct XattrArgs {
    value: u64,
    size: u32,
    flags: u32,
}
const _: [(); 16] = [(); std::mem::size_of::<XattrArgs>()];
const _: [(); 8] = [(); std::mem::align_of::<XattrArgs>()];
// libc 0.2.191 does not expose SYS_getxattrat. Linux v6.13 UAPI assigns 464
// to native x86_64 and asm-generic (AArch64). Verified against installed UAPI
// headers and linux-raw-sys 0.12.1/src/{x86_64,aarch64}/general.rs.
// Deliberately exclude x32/ILP32: x32 adds __X32_SYSCALL_BIT. Runtime syscall
// or filesystem failure remains an error, never evidence of an absent ACL.
#[cfg(all(
    target_os = "linux",
    target_pointer_width = "64",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
const SYS_GETXATTRAT: libc::c_long = 464;

#[derive(Clone, PartialEq, Eq, Debug)]
struct Acls {
    access: Option<Vec<u8>>,
    default: Option<Vec<u8>>,
}
fn child_matches(
    parent: &Directory,
    leaf: &std::ffi::OsStr,
    child: &File,
    expected: &Identity,
) -> io::Result<Metadata> {
    if leaf.as_bytes().is_empty() || leaf.as_bytes().contains(&b'/') || leaf == "." || leaf == ".."
    {
        return Err(invalid("ACL lookup requires one exact component"));
    }
    parent.revalidate()?;
    let current = open_path(&parent.file, leaf)?;
    let held = child.metadata()?;
    let named = current.metadata()?;
    if held.file_type().is_symlink()
        || !expected.same_inode(&Identity::metadata(&held))
        || expected.links != held.nlink()
        || Identity::metadata(&held) != Identity::metadata(&named)
        || held.gid() != named.gid()
        || held.ctime() != named.ctime()
        || held.ctime_nsec() != named.ctime_nsec()
    {
        return Err(invalid("ACL inode/metadata changed"));
    }
    Ok(held)
}
fn xattr_read(
    parent: &File,
    leaf: &std::ffi::OsStr,
    attribute: &str,
) -> io::Result<Option<Vec<u8>>> {
    #[cfg(all(
        target_os = "linux",
        target_pointer_width = "64",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        let component = name(leaf)?;
        let attribute = std::ffi::CString::new(attribute).map_err(|_| invalid("ACL name"))?;
        let mut query = XattrArgs {
            value: 0,
            size: 0,
            flags: 0,
        };
        // Nonempty single component under the retained directory FD. Never
        // getxattrat(O_PATH,"",...) or /proc/self/fd; no guessed syscall number.
        let size = unsafe {
            libc::syscall(
                SYS_GETXATTRAT,
                parent.as_raw_fd(),
                component.as_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
                attribute.as_ptr(),
                &mut query as *mut XattrArgs,
                std::mem::size_of::<XattrArgs>(),
            )
        };
        if size < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENODATA) {
                return Ok(None);
            }
            return Err(error);
        }
        if size > 65536 {
            return Err(limit());
        }
        let mut bytes = vec![0u8; size as usize];
        let mut args = XattrArgs {
            value: bytes.as_mut_ptr() as usize as u64,
            size: size as u32,
            flags: 0,
        };
        let read = unsafe {
            libc::syscall(
                SYS_GETXATTRAT,
                parent.as_raw_fd(),
                component.as_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
                attribute.as_ptr(),
                &mut args as *mut XattrArgs,
                std::mem::size_of::<XattrArgs>(),
            )
        };
        if read < 0 {
            return Err(io::Error::last_os_error());
        }
        if read != size {
            return Err(invalid("ACL size changed during capture"));
        }
        Ok(Some(bytes))
    }
    #[cfg(not(all(
        target_os = "linux",
        target_pointer_width = "64",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        let _ = (parent, leaf, attribute);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "owner-read needs verified getxattrat UAPI on native 64-bit Linux x86_64/aarch64",
        ))
    }
}
fn capture_acls(
    parent: &Directory,
    leaf: &std::ffi::OsStr,
    child: &File,
    expected: &Identity,
) -> io::Result<Acls> {
    let before = child_matches(parent, leaf, child, expected)?;
    let access = xattr_read(&parent.file, leaf, "system.posix_acl_access")?;
    let default = if before.is_dir() {
        xattr_read(&parent.file, leaf, "system.posix_acl_default")?
    } else {
        None
    };
    let after = child_matches(parent, leaf, child, expected)?;
    if before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
        || before.gid() != after.gid()
        || before.len() != after.len()
    {
        return Err(invalid("ACL capture raced with metadata change"));
    }
    Ok(Acls { access, default })
}
// Compare the known effect of an owner-only grant against the SAVED original
// bytes. Never reconstruct an alleged original ACL from post-grant contents.
fn granted_access_matches(
    original: &Option<Vec<u8>>,
    current: &Option<Vec<u8>>,
    mode: u32,
) -> bool {
    match (original, current) {
        (None, None) => true,
        (Some(old), Some(now)) => {
            if old.len() != now.len()
                || old.len() < 4
                || (old.len() - 4) % 8 != 0
                || old[..4] != 2u32.to_le_bytes()
                || old[..4] != now[..4]
            {
                return false;
            }
            let mut owner = 0;
            for (a, b) in old[4..].chunks_exact(8).zip(now[4..].chunks_exact(8)) {
                if u16::from_le_bytes([a[0], a[1]]) == 1 {
                    owner += 1;
                    if a[..2] != b[..2]
                        || a[4..] != b[4..]
                        || b[2..4] != (((mode >> 6) & 7) as u16).to_le_bytes()
                    {
                        return false;
                    }
                } else if a != b {
                    return false;
                }
            }
            owner == 1
        }
        _ => false,
    }
}
#[cfg(test)]
#[derive(Clone, Copy)]
enum GrantFault {
    AccessSyscall(i32),
    DefaultRead,
    Journal,
    AfterIntent,
}
impl OwnerReadGuard {
    pub(super) fn begin(lease: Lease, root: &Path) -> io::Result<Self> {
        Self::begin_inner(
            lease,
            root,
            #[cfg(test)]
            None,
        )
    }
    fn begin_inner(
        lease: Lease,
        root: &Path,
        #[cfg(test)] fault: Option<GrantFault>,
    ) -> io::Result<Self> {
        let root = absolute(root)?;
        lease.revalidate()?;
        let mut tx = lease.0.namespace.transaction()?;
        lease.validate_continuation(&tx)?;
        let op = tx.operation_mut(&lease.0.id)?;
        if !op.entries.iter().any(|e| {
            (e.role == PathRole::ManagedCacheRead
                || (e.role == PathRole::MutableTree
                    && super::shared_cache::managed_root(op, &root)))
                && e.path.get().is_ok_and(|p| p == root)
        }) || op.closed
            || op.uncertain
        {
            return Err(invalid(
                "managed cache provenance not explicitly registered",
            ));
        }
        if op.entries.iter().any(|e| {
            e.role != PathRole::ManagedCacheRead
                && !(e.role == PathRole::MutableTree
                    && e.path.get().is_ok_and(|p| p == root)
                    && super::shared_cache::managed_root(op, &root))
                && e.path.get().is_ok_and(|p| overlaps(&p, &root))
        }) || op
            .holds
            .iter()
            .any(|h| h.path.get().is_ok_and(|p| overlaps(&p, &root)))
        {
            return Err(busy());
        }
        if op.holds.len() >= 256 || op.journals.len() >= 256 {
            return Err(limit());
        }
        let id = new_id();
        op.epoch = op.epoch.checked_add(1).ok_or_else(limit)?;
        let epoch = op.epoch;
        op.holds.push(Hold {
            id: id.clone(),
            path: HostPath::new(&root)?,
            kind: HoldKind::OwnerRead,
            epoch,
            owner: Some(super::catalog::ResourceOwner {
                holder: lease.0.holder.clone(),
                runtime: None,
            }),
        });
        op.journals.push(Journal {
            id: id.clone(),
            plan: "owner-read".into(),
            root: HostPath::new(&root)?,
            footprint: vec![HostPath::new(&root)?],
            intents: Vec::new(),
            permissions: Vec::new(),
            payload: None,
            epoch,
            committed: false,
            changes: Vec::new(),
            artifacts: Vec::new(),
            disposals: Vec::new(),
        });
        tx.save()?;
        drop(tx);
        let mut guard = Self {
            lease,
            id,
            root: root.clone(),
            objects: Vec::new(),
            finished: false,
            failed: AtomicBool::new(false),
            #[cfg(test)]
            fault,
        };
        guard.scan(&root, 0)?;
        // All regular links must be contained in this verified tree BEFORE any
        // file permission grant. Directory grants were necessary for enumeration.
        let mut counts = HashMap::new();
        for p in &guard.objects {
            if p.metadata.is_file() {
                *counts
                    .entry((p.identity.dev, p.identity.ino))
                    .or_insert(0u64) += 1;
            }
        }
        for p in &guard.objects {
            if p.metadata.is_file()
                && counts.get(&(p.identity.dev, p.identity.ino)) != Some(&p.identity.links)
            {
                return Err(invalid("managed cache file has external hardlinks"));
            }
        }
        Ok(guard)
    }
    fn scan(&mut self, path: &Path, depth: usize) -> io::Result<()> {
        if depth > 64 || self.objects.len() >= 4096 {
            return Err(limit());
        }
        let parent = Directory::open(path.parent().ok_or_else(|| invalid("cache parent"))?)?;
        let fd = open_path(
            &parent.file,
            path.file_name().ok_or_else(|| invalid("cache name"))?,
        )?;
        let metadata = fd.metadata()?;
        let identity = Identity::metadata(&metadata);
        if identity.uid != uid()
            || (!metadata.file_type().is_symlink() && identity.mode & 0o022 != 0)
            || !(metadata.is_file() || metadata.is_dir() || metadata.file_type().is_symlink())
        {
            return Err(invalid("unowned or unsafe managed cache object"));
        }
        if self
            .objects
            .first()
            .is_some_and(|p| p.identity.dev != identity.dev)
        {
            return Err(invalid("managed cache crosses filesystem"));
        }
        let is_dir = metadata.is_dir();
        self.objects.push(Pinned {
            path: path.to_path_buf(),
            fd,
            parent,
            metadata,
            identity,
            granted: 0,
            acl: None,
            default_acl: None,
            changed: false,
        });
        let index = self.objects.len() - 1;
        if is_dir {
            self.grant(index, true)?;
            let directory = Directory::open(path)?;
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(path)? {
                if entries.len() >= 4096 {
                    return Err(limit());
                }
                entries.push(entry?);
            }
            if entries.len() > 4096 {
                return Err(limit());
            }
            entries.sort_by_key(|e| e.file_name());
            for child in entries {
                directory.revalidate()?;
                self.scan(&child.path(), depth + 1)?;
            }
        }
        Ok(())
    }
    fn grant(&mut self, index: usize, directory: bool) -> io::Result<()> {
        self.checkpoint()?;
        let original = self.objects[index].identity.clone();
        let path = self.objects[index].path.clone();
        let need = if directory { 0o500 } else { 0o400 };
        let mode = original.mode & 0o7777;
        if mode & need == need || self.objects[index].changed {
            return Ok(());
        }
        self.objects[index].parent.revalidate()?;
        let parent = Directory::open(&self.objects[index].parent.path)?;
        let leaf = path.file_name().ok_or_else(|| invalid("cache name"))?;
        #[cfg(test)]
        if let Some(GrantFault::AccessSyscall(code)) = self.fault {
            return Err(io::Error::from_raw_os_error(code));
        }
        let original_acls = capture_acls(&parent, leaf, &self.objects[index].fd, &original)?;
        #[cfg(test)]
        if matches!(self.fault, Some(GrantFault::DefaultRead)) {
            return Err(io::Error::from_raw_os_error(libc::EACCES));
        }
        let mut tx = self.lease.0.namespace.transaction()?;
        let journal = tx
            .operation_mut(&self.lease.0.id)?
            .journals
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| invalid("permission journal missing"))?;
        if journal.permissions.len() >= 4096 {
            return Err(limit());
        }
        journal.permissions.push(PermissionIntent {
            path: HostPath::new(&path)?,
            parent: Identity::of(&parent.file)?,
            original: original.clone(),
            granted_mode: mode | need,
            acl: original_acls.access.clone().map(hex::encode),
            default_acl: original_acls.default.clone().map(hex::encode),
            acl_captured: true,
            acl_before_grant: true,
            restored: false,
            ctime_changed: true,
        });
        #[cfg(test)]
        if matches!(self.fault, Some(GrantFault::Journal)) {
            return Err(io::Error::from_raw_os_error(libc::EIO));
        }
        tx.save()?;
        drop(tx);
        #[cfg(test)]
        if matches!(self.fault, Some(GrantFault::AfterIntent)) {
            return Err(io::Error::from_raw_os_error(libc::EIO));
        }
        // Both original ACL tuples are durable BEFORE the first mode write.
        // Recheck bytes as well as identity after journal I/O; an ACL-only change
        // can retain the same inode/mode and must not be blessed by the grant.
        if capture_acls(&parent, leaf, &self.objects[index].fd, &original)? != original_acls {
            return Err(invalid("original ACL changed after permission intent"));
        }
        chmod_pinned(&self.objects[index].fd, mode | need)?;
        for p in &mut self.objects {
            if p.identity.dev == original.dev && p.identity.ino == original.ino {
                p.changed = true;
                p.granted = mode | need;
                p.acl = original_acls.access.clone();
                p.default_acl = original_acls.default.clone();
            }
        }
        let mut granted = original.clone();
        granted.mode = (granted.mode & !0o7777) | (mode | need);
        let current = capture_acls(&parent, leaf, &self.objects[index].fd, &granted)?;
        if current.default != original_acls.default
            || !granted_access_matches(&original_acls.access, &current.access, mode | need)
        {
            return Err(invalid("ACL changed outside owner grant"));
        }
        Ok(())
    }
    fn checkpoint_inner(&self) -> io::Result<()> {
        let tx = self.lease.0.namespace.transaction()?;
        self.lease.validate_continuation(&tx)?;
        let op = tx.operation(&self.lease.0.id)?;
        if op.uncertain
            || op.closed
            || !op
                .holds
                .iter()
                .any(|h| h.id == self.id && h.kind == HoldKind::OwnerRead)
        {
            return Err(invalid("owner-read scope unavailable"));
        }
        for p in &self.objects {
            p.parent.revalidate()?;
            let now = Identity::of(&p.fd)?;
            let current = std::fs::symlink_metadata(&p.path)?;
            let current = Identity::metadata(&current);
            let mode = if p.changed {
                p.granted
            } else {
                p.identity.mode & 0o7777
            };
            if now.dev != p.identity.dev
                || now.ino != p.identity.ino
                || current.dev != now.dev
                || current.ino != now.ino
                || p.fd.metadata()?.gid() != p.metadata.gid()
                || now.uid != p.identity.uid
                || now.links != p.identity.links
                || current.links != p.identity.links
                || now.mode & 0o7777 != mode
                || current.mode != now.mode
            {
                return Err(invalid(
                    "managed cache inode replaced or permissions changed",
                ));
            }
        }
        Ok(())
    }
    fn locate(&self, path: &Path) -> io::Result<usize> {
        let path = absolute(path)?;
        if !path.starts_with(&self.root) {
            return Err(invalid("owner-read outside registered tree"));
        }
        self.objects
            .iter()
            .position(|p| p.path == path)
            .ok_or_else(|| invalid("cache entry was not inventoried"))
    }
    fn read_dir_inner(&mut self, path: &Path) -> io::Result<Vec<OwnerReadEntry>> {
        self.checkpoint()?;
        let index = self.locate(path)?;
        if !self.objects[index].metadata.is_dir() {
            return Err(invalid("cache path is not a directory"));
        }
        Ok(self
            .objects
            .iter()
            .filter(|p| p.path.parent() == Some(self.objects[index].path.as_path()))
            .map(|p| OwnerReadEntry {
                name: p.path.file_name().unwrap_or_default().to_os_string(),
                metadata: p.metadata.clone(),
            })
            .collect())
    }
    fn open_file_inner(&mut self, path: &Path) -> io::Result<File> {
        self.checkpoint()?;
        let index = self.locate(path)?;
        if !self.objects[index].metadata.is_file() {
            return Err(invalid("cache entry is not a regular file"));
        }
        // Grant/restoration applies once per inode shared by verified internal links.
        if let Some(other) = self.objects.iter().position(|p| {
            p.changed
                && p.identity.dev == self.objects[index].identity.dev
                && p.identity.ino == self.objects[index].identity.ino
        }) {
            self.objects[index].changed = true;
            self.objects[index].granted = self.objects[other].granted;
            self.objects[index].acl = self.objects[other].acl.clone();
            self.objects[index].default_acl = self.objects[other].default_acl.clone();
        } else {
            self.grant(index, false)?;
        }
        self.checkpoint()?;
        let path = &self.objects[index].path;
        let parent = Directory::open(path.parent().ok_or_else(|| invalid("cache parent"))?)?;
        let file = open_at(
            &parent.file,
            path.file_name().ok_or_else(|| invalid("cache file"))?,
            libc::O_RDONLY | libc::O_NONBLOCK,
            0,
        )?;
        if file.metadata()?.ino() != self.objects[index].identity.ino
            || file.metadata()?.dev() != self.objects[index].identity.dev
        {
            return Err(invalid("cache file substituted"));
        }
        Ok(file)
    }
    fn original_metadata_inner(&self, path: &Path) -> io::Result<Metadata> {
        self.checkpoint()?;
        Ok(self.objects[self.locate(path)?].metadata.clone())
    }
    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.checkpoint()?;
        let mut restored = std::collections::HashSet::new();
        for index in (0..self.objects.len()).rev() {
            let p = &self.objects[index];
            if !p.changed || !restored.insert((p.identity.dev, p.identity.ino)) {
                continue;
            }
            let parent = &p.parent;
            parent.revalidate()?;
            let readable = open_at(
                &parent.file,
                p.path.file_name().ok_or_else(|| invalid("cache name"))?,
                libc::O_RDONLY
                    | if p.metadata.is_dir() {
                        libc::O_DIRECTORY
                    } else {
                        libc::O_NONBLOCK
                    },
                0,
            )?;
            let mut granted = p.identity.clone();
            granted.mode = (granted.mode & !0o7777) | p.granted;
            let current = capture_acls(
                &parent,
                p.path.file_name().ok_or_else(|| invalid("cache name"))?,
                &p.fd,
                &granted,
            )?;
            if current.default != p.default_acl
                || !granted_access_matches(&p.acl, &current.access, p.granted)
            {
                return Err(invalid("cache ACL changed externally"));
            }
            chmod_pinned(&p.fd, p.identity.mode & 0o7777)?;
            let restored = capture_acls(
                &parent,
                p.path.file_name().ok_or_else(|| invalid("cache name"))?,
                &p.fd,
                &p.identity,
            )?;
            if restored.access != p.acl || restored.default != p.default_acl {
                return Err(invalid("cache ACL restoration failed"));
            }
            readable.sync_all()?;
            let mut tx = self.lease.0.namespace.transaction()?;
            let journal = tx
                .operation_mut(&self.lease.0.id)?
                .journals
                .iter_mut()
                .find(|j| j.id == self.id)
                .ok_or_else(|| invalid("permission journal missing"))?;
            for intent in &mut journal.permissions {
                if intent.original.dev == p.identity.dev && intent.original.ino == p.identity.ino {
                    intent.restored = true;
                }
            }
            tx.save()?;
        }
        let mut tx = self.lease.0.namespace.transaction()?;
        let op = tx.operation_mut(&self.lease.0.id)?;
        let journal = op
            .journals
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| invalid("permission journal missing"))?;
        if journal.permissions.iter().any(|i| !i.restored) {
            return Err(invalid("unrestored permission intent"));
        }
        journal.committed = true;
        op.holds.retain(|h| h.id != self.id);
        tx.save()?;
        self.finished = true;
        Ok(())
    }
}
impl OwnerReadGuard {
    fn terminal<T>(&self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() {
            self.failed.store(true, Ordering::Release);
            let _ = self.lease.finish_hold(&self.id, true);
        }
        result
    }
    pub(crate) fn checkpoint(&self) -> io::Result<()> {
        if self.failed.load(Ordering::Acquire) {
            return Err(invalid("owner-read failure is terminal"));
        }
        self.terminal(self.checkpoint_inner())
    }
    pub(crate) fn read_dir(&mut self, path: &Path) -> io::Result<Vec<OwnerReadEntry>> {
        let r = self.read_dir_inner(path);
        self.terminal(r)
    }
    pub(crate) fn open_file(&mut self, path: &Path) -> io::Result<File> {
        let r = self.open_file_inner(path);
        self.terminal(r)
    }
    pub(crate) fn original_metadata(&self, path: &Path) -> io::Result<Metadata> {
        self.terminal(self.original_metadata_inner(path))
    }
}
pub(super) fn restore_verified(
    journal: &mut super::catalog::Journal,
    expected_uid: u32,
) -> io::Result<bool> {
    for intent in journal.permissions.iter_mut().rev() {
        let path = intent.path.get()?;
        let parent = Directory::open(
            path.parent()
                .ok_or_else(|| invalid("permission recovery parent"))?,
        )?;
        if !intent.parent.same_inode(&Identity::of(&parent.file)?) {
            return Ok(false);
        }
        let fd = open_path(
            &parent.file,
            path.file_name()
                .ok_or_else(|| invalid("permission recovery name"))?,
        )?;
        let id = Identity::of(&fd)?;
        if id.dev != intent.original.dev
            || id.ino != intent.original.ino
            || id.uid != expected_uid
            || id.links != intent.original.links
        {
            return Ok(false);
        }
        // Old after-grant/incomplete records are never upgraded by inference.
        if !intent.acl_captured || !intent.acl_before_grant {
            return Ok(false);
        }
        let original = Acls {
            access: intent
                .acl
                .as_ref()
                .map(|v| hex::decode(v).map_err(|_| invalid("invalid recorded access ACL")))
                .transpose()?,
            default: intent
                .default_acl
                .as_ref()
                .map(|v| hex::decode(v).map_err(|_| invalid("invalid recorded default ACL")))
                .transpose()?,
        };
        let leaf = path.file_name().ok_or_else(|| invalid("recovery name"))?;
        if intent.restored || id.mode == intent.original.mode {
            if id.mode != intent.original.mode
                || capture_acls(&parent, leaf, &fd, &intent.original)? != original
            {
                return Ok(false);
            }
            parent.file.sync_all()?;
            intent.restored = true;
            continue;
        }
        if id.mode & 0o7777 != intent.granted_mode {
            return Ok(false);
        }
        let current = capture_acls(&parent, leaf, &fd, &id)?;
        if current.default != original.default
            || !granted_access_matches(&original.access, &current.access, intent.granted_mode)
        {
            return Ok(false);
        }
        let readable = open_at(
            &parent.file,
            leaf,
            libc::O_RDONLY
                | if id.mode & libc::S_IFMT == libc::S_IFDIR {
                    libc::O_DIRECTORY
                } else {
                    libc::O_NONBLOCK
                },
            0,
        )?;
        chmod_pinned(&fd, intent.original.mode & 0o7777)?;
        if capture_acls(&parent, leaf, &fd, &intent.original)? != original {
            return Err(invalid("permission recovery ACL mismatch"));
        }
        readable.sync_all()?;
        parent.file.sync_all()?;
        intent.restored = true;
    }
    Ok(true)
}
impl Drop for OwnerReadGuard {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.lease.finish_hold(&self.id, true);
        }
    }
}
