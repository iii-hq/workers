// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Owner-only temporary access. Requires Linux fchmodat2(AT_EMPTY_PATH), no procfd
//! pathname fallback. Only ACL_USER_OBJ changes; original guest metadata retained.
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
    metadata: Metadata,
    identity: Identity,
    granted: u32,
    acl: Option<Vec<u8>>,
    changed: bool,
}
pub(crate) struct OwnerReadGuard {
    lease: Lease,
    id: String,
    root: PathBuf,
    objects: Vec<Pinned>,
    finished: bool,
    failed: AtomicBool,
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
fn acl(fd: &File) -> io::Result<Option<Vec<u8>>> {
    #[cfg(target_os = "linux")]
    {
        let name =
            std::ffi::CString::new("system.posix_acl_access").map_err(|_| invalid("acl name"))?;
        let size =
            unsafe { libc::fgetxattr(fd.as_raw_fd(), name.as_ptr(), std::ptr::null_mut(), 0) };
        if size < 0 {
            let e = io::Error::last_os_error();
            if matches!(e.raw_os_error(), Some(libc::ENODATA) | Some(libc::ENOTSUP)) {
                return Ok(None);
            }
            return Err(e);
        }
        if size > 65536 {
            return Err(limit());
        }
        let mut bytes = vec![0; size as usize];
        let read = unsafe {
            libc::fgetxattr(
                fd.as_raw_fd(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if read != size {
            return Err(invalid("ACL changed during read"));
        }
        Ok(Some(bytes))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = fd;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "owner-read ACL validation requires Linux",
        ))
    }
}
fn original_acl(mut bytes: Vec<u8>, mode: u32) -> io::Result<Vec<u8>> {
    if bytes.len() < 4
        || (bytes.len() - 4) % 8 != 0
        || u32::from_le_bytes(bytes[..4].try_into().map_err(|_| invalid("ACL header"))?) != 2
    {
        return Err(invalid("unsupported ACL encoding"));
    }
    let mut found = 0;
    for entry in bytes[4..].chunks_exact_mut(8) {
        let tag = u16::from_le_bytes([entry[0], entry[1]]);
        if tag == 1 {
            entry[2..4].copy_from_slice(&(((mode >> 6) & 7) as u16).to_le_bytes());
            found += 1;
        }
    }
    if found != 1 {
        return Err(invalid("ACL owner entry missing"));
    }
    Ok(bytes)
}
impl OwnerReadGuard {
    pub(super) fn begin(lease: Lease, root: &Path) -> io::Result<Self> {
        let root = absolute(root)?;
        lease.revalidate()?;
        let mut tx = lease.0.namespace.transaction()?;
        let op = tx.operation_mut(&lease.0.id)?;
        if !op
            .entries
            .iter()
            .any(|e| e.role == PathRole::ManagedCacheRead && e.path.get().is_ok_and(|p| p == root))
            || op.closed
            || op.uncertain
        {
            return Err(invalid(
                "managed cache provenance not explicitly registered",
            ));
        }
        if op.entries.iter().any(|e| {
            e.role != PathRole::ManagedCacheRead && e.path.get().is_ok_and(|p| overlaps(&p, &root))
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
        });
        op.journals.push(Journal {
            id: id.clone(),
            plan: "owner-read".into(),
            root: HostPath::new(&root)?,
            footprint: vec![HostPath::new(&root)?],
            intents: Vec::new(),
            permissions: Vec::new(),
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
            metadata,
            identity,
            granted: 0,
            acl: None,
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
        let parent = Directory::open(path.parent().ok_or_else(|| invalid("cache parent"))?)?;
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
            acl: None,
            acl_captured: false,
            restored: false,
            ctime_changed: true,
        });
        tx.save()?;
        drop(tx);
        chmod_pinned(&self.objects[index].fd, mode | need)?;
        self.objects[index].changed = true;
        self.objects[index].granted = mode | need;
        let readable = open_at(
            &parent.file,
            path.file_name().ok_or_else(|| invalid("cache name"))?,
            libc::O_RDONLY
                | if directory {
                    libc::O_DIRECTORY
                } else {
                    libc::O_NONBLOCK
                },
            0,
        )?;
        let original_acl = acl(&readable)?
            .map(|bytes| original_acl(bytes, mode))
            .transpose()?;
        self.objects[index].acl = original_acl.clone();
        let mut tx = self.lease.0.namespace.transaction()?;
        let journal = tx
            .operation_mut(&self.lease.0.id)?
            .journals
            .iter_mut()
            .find(|j| j.id == self.id)
            .ok_or_else(|| invalid("permission journal missing"))?;
        let intent = journal
            .permissions
            .last_mut()
            .ok_or_else(|| invalid("permission intent missing"))?;
        intent.acl = original_acl.clone().map(hex::encode);
        intent.acl_captured = true;
        tx.save()?;
        for p in &mut self.objects {
            if p.identity.dev == original.dev && p.identity.ino == original.ino {
                p.changed = true;
                p.granted = mode | need;
                p.acl = original_acl.clone();
            }
        }
        Ok(())
    }
    fn checkpoint_inner(&self) -> io::Result<()> {
        let tx = self.lease.0.namespace.transaction()?;
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
            let parent = Directory::open(p.path.parent().ok_or_else(|| invalid("cache parent"))?)?;
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
            if acl(&readable)?
                .map(|a| original_acl(a, p.identity.mode))
                .transpose()?
                != p.acl
            {
                return Err(invalid("cache ACL changed externally"));
            }
            let current = Identity::metadata(&std::fs::symlink_metadata(&p.path)?);
            if current.dev != p.identity.dev
                || current.ino != p.identity.ino
                || current.uid != p.identity.uid
            {
                return Err(invalid("cache replaced during restoration"));
            }
            chmod_pinned(&p.fd, p.identity.mode & 0o7777)?;
            if Identity::of(&p.fd)?.mode != p.identity.mode || acl(&readable)? != p.acl {
                return Err(invalid("cache permission restoration failed"));
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
        if intent.restored {
            if id.mode != intent.original.mode {
                return Ok(false);
            }
            continue;
        }
        if !intent.acl_captured || id.mode & 0o7777 != intent.granted_mode {
            return Ok(false);
        }
        let readable = open_at(
            &parent.file,
            path.file_name()
                .ok_or_else(|| invalid("permission recovery name"))?,
            libc::O_RDONLY
                | if id.mode & libc::S_IFMT == libc::S_IFDIR {
                    libc::O_DIRECTORY
                } else {
                    libc::O_NONBLOCK
                },
            0,
        )?;
        let expected_acl = intent
            .acl
            .as_ref()
            .map(|v| hex::decode(v).map_err(|_| invalid("invalid recorded ACL")))
            .transpose()?;
        if acl(&readable)?
            .map(|bytes| original_acl(bytes, intent.original.mode))
            .transpose()?
            != expected_acl
        {
            return Ok(false);
        }
        chmod_pinned(&fd, intent.original.mode & 0o7777)?;
        if Identity::of(&fd)?.mode != intent.original.mode || acl(&readable)? != expected_acl {
            return Err(invalid("permission recovery restoration failed"));
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
