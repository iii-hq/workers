// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Core-owned payload production. Callers supply planned content, never a stage
//! pathname, writable FD, PID or alleged writer-finished boolean.
use super::{Lease, MutationGuard, UseGuard, storage::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CString, OsStr, OsString},
    fs::{File, Metadata},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{FileExt, MetadataExt},
        },
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
const CEILING_ENTRIES: usize = 262144;
const CEILING_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const CEILING_METADATA: usize = 64 * 1024 * 1024;
const MAX_DEPTH: usize = 128;
const MAX_ATTR: usize = 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PayloadMetadata {
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub atime_sec: i64,
    pub atime_nsec: i64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub xattrs: BTreeMap<Vec<u8>, Vec<u8>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum PayloadKind {
    Directory,
    File { size: u64 },
    Symlink { target: Vec<u8> },
    Hardlink { source: PathBuf },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PayloadEntry {
    pub relative: PathBuf,
    pub metadata: PayloadMetadata,
    pub kind: PayloadKind,
}
/// Exact declaration, not a raise of sync_tree's limit. Caller inventory is
/// checked and fully reserved before any stage creation or write.
pub(crate) struct PayloadInventory {
    entries: Vec<PayloadEntry>,
    bytes: u64,
    digest: String,
    metadata_bytes: usize,
    index: BTreeMap<PathBuf, usize>,
    children: Vec<Vec<usize>>,
}
impl PayloadInventory {
    pub(crate) fn new(mut entries: Vec<PayloadEntry>) -> io::Result<Self> {
        if entries.is_empty() || entries.len() > CEILING_ENTRIES {
            return Err(limit());
        }
        entries.sort_by(|a, b| {
            let rank = |e: &PayloadEntry| {
                if matches!(e.kind, PayloadKind::Directory) {
                    0
                } else if matches!(e.kind, PayloadKind::Hardlink { .. }) {
                    2
                } else {
                    1
                }
            };
            rank(a)
                .cmp(&rank(b))
                .then(
                    a.relative
                        .components()
                        .count()
                        .cmp(&b.relative.components().count()),
                )
                .then(a.relative.cmp(&b.relative))
        });
        if entries[0].relative != Path::new("")
            || !matches!(entries[0].kind, PayloadKind::Directory)
        {
            return Err(invalid("payload root must be an explicit directory"));
        }
        let all = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.relative.clone(), i))
            .collect::<BTreeMap<_, _>>();
        let mut seen = BTreeMap::new();
        let mut bytes = 0u64;
        let mut metadata = 0usize;
        for (i, entry) in entries.iter().enumerate() {
            relative(&entry.relative)?;
            if seen.insert(entry.relative.clone(), i).is_some() {
                return Err(invalid("duplicate payload path"));
            }
            if i > 0 {
                let parent = entry
                    .relative
                    .parent()
                    .ok_or_else(|| invalid("payload parent"))?;
                let p = *seen
                    .get(parent)
                    .ok_or_else(|| invalid("payload parent not planned"))?;
                if !matches!(entries[p].kind, PayloadKind::Directory) {
                    return Err(invalid("payload crosses nondirectory or symlink parent"));
                }
            }
            if entry.metadata.mode & !0o7777 != 0
                || entry.metadata.uid != uid()
                || !(0..1_000_000_000).contains(&entry.metadata.atime_nsec)
                || !(0..1_000_000_000).contains(&entry.metadata.mtime_nsec)
            {
                return Err(invalid("unrepresentable payload metadata"));
            }
            metadata = metadata
                .checked_add(entry.relative.as_os_str().as_bytes().len() + 256)
                .ok_or_else(limit)?;
            for (key, value) in &entry.metadata.xattrs {
                if key.is_empty() || key.contains(&0) || value.len() > MAX_ATTR {
                    return Err(invalid("invalid bounded payload xattr"));
                }
                metadata = metadata
                    .checked_add(key.len() + value.len())
                    .ok_or_else(limit)?;
            }
            if metadata > CEILING_METADATA {
                return Err(limit());
            }
            match &entry.kind {
                PayloadKind::File { size } => {
                    bytes = bytes.checked_add(*size).ok_or_else(limit)?;
                }
                PayloadKind::Symlink { target } => {
                    if target.is_empty() || target.len() > 4096 || target.contains(&0) {
                        return Err(invalid("invalid symlink payload"));
                    }
                    metadata = metadata.checked_add(target.len()).ok_or_else(limit)?;
                }
                PayloadKind::Hardlink { source } => {
                    relative(source)?;
                    let target = *all
                        .get(source)
                        .ok_or_else(|| invalid("hardlink source must be internal"))?;
                    if !matches!(entries[target].kind, PayloadKind::File { .. })
                        || entries[target].metadata.mode != entry.metadata.mode
                        || entries[target].metadata.uid != entry.metadata.uid
                        || entries[target].metadata.gid != entry.metadata.gid
                        || entries[target].metadata.xattrs != entry.metadata.xattrs
                        || (
                            entries[target].metadata.atime_sec,
                            entries[target].metadata.atime_nsec,
                            entries[target].metadata.mtime_sec,
                            entries[target].metadata.mtime_nsec,
                        ) != (
                            entry.metadata.atime_sec,
                            entry.metadata.atime_nsec,
                            entry.metadata.mtime_sec,
                            entry.metadata.mtime_nsec,
                        )
                    {
                        return Err(invalid("hardlink metadata/source mismatch"));
                    }
                }
                PayloadKind::Directory => (),
            }
        }
        if bytes > CEILING_BYTES || metadata > CEILING_METADATA {
            return Err(limit());
        }
        // JSON paths are not used as authority; raw-byte path/hash representation
        // avoids accidental lossy conversion in the admission receipt.
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(
                &entries
                    .iter()
                    .map(|e| {
                        (
                            hex::encode(e.relative.as_os_str().as_bytes()),
                            match &e.kind {
                                PayloadKind::Directory => (0, 0, String::new()),
                                PayloadKind::File { size } => (1, *size, String::new()),
                                PayloadKind::Symlink { target } => (2, 0, hex::encode(target)),
                                PayloadKind::Hardlink { source } => {
                                    (3, 0, hex::encode(source.as_os_str().as_bytes()))
                                }
                            },
                            e.metadata.mode,
                            e.metadata.uid,
                            e.metadata.gid,
                            e.metadata.atime_sec,
                            e.metadata.atime_nsec,
                            e.metadata.mtime_sec,
                            e.metadata.mtime_nsec,
                            e.metadata
                                .xattrs
                                .iter()
                                .map(|(k, v)| (hex::encode(k), hex::encode(v)))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .map_err(io::Error::other)?,
        ));
        let index = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.relative.clone(), i))
            .collect::<BTreeMap<_, _>>();
        let mut children = vec![Vec::new(); entries.len()];
        for (i, e) in entries.iter().enumerate().skip(1) {
            let parent = *index
                .get(
                    e.relative
                        .parent()
                        .ok_or_else(|| invalid("payload parent"))?,
                )
                .ok_or_else(|| invalid("payload parent absent"))?;
            children[parent].push(i);
        }
        Ok(Self {
            entries,
            bytes,
            digest,
            metadata_bytes: metadata,
            index,
            children,
        })
    }
}
fn relative(path: &Path) -> io::Result<()> {
    if path.is_absolute()
        || path.components().count() > MAX_DEPTH
        || path.as_os_str().as_bytes().len() > 4096
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(invalid(
            "payload path must be bounded relative normal components",
        ));
    }
    Ok(())
}
#[derive(Clone)]
struct Stamp {
    identity: Identity,
    gid: u32,
    atime: i64,
    atime_ns: i64,
    size: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
fn stamp(file: &File) -> io::Result<Stamp> {
    let m = file.metadata()?;
    Ok(Stamp {
        identity: Identity::metadata(&m),
        gid: m.gid(),
        atime: m.atime(),
        atime_ns: m.atime_nsec(),
        size: m.len(),
        mtime: m.mtime(),
        mtime_ns: m.mtime_nsec(),
        ctime: m.ctime(),
        ctime_ns: m.ctime_nsec(),
    })
}
fn unchanged(a: &Stamp, b: &Stamp) -> bool {
    a.identity == b.identity
        && (a.gid, a.atime, a.atime_ns) == (b.gid, b.atime, b.atime_ns)
        && a.size == b.size
        && (a.mtime, a.mtime_ns, a.ctime, a.ctime_ns) == (b.mtime, b.mtime_ns, b.ctime, b.ctime_ns)
}
/// Source-pin equality. Exact for every kind except a symlink's atime: its
/// content can only be read with readlink(2), which always touches the inode's
/// atime (no O_NOATIME applies to it, and restoring atime with utimensat would
/// itself advance ctime). That atime is the capture's own read side effect, not
/// a source generation change. Identity (dev, ino, uid, mode, links), gid, size,
/// mtime and ctime stay exact: retargeting needs a new inode, and renaming it
/// over the name drops the pinned inode's link count and advances its ctime,
/// which no unprivileged process can restore. Captured metadata keeps the atime
/// stamped BEFORE the read, so the clone carries the pre-read value.
fn source_unchanged(a: &Stamp, b: &Stamp) -> bool {
    if a.identity.mode & libc::S_IFMT == libc::S_IFLNK {
        same_sealed(a, b)
    } else {
        unchanged(a, b)
    }
}
struct OwnedNode {
    pin: File,
    read: Option<File>,
    writer: Option<File>,
    stamp: Option<Stamp>,
    hash: Option<String>,
    written: bool,
    extents: Option<Vec<(u64, u64)>>,
}
#[must_use = "payload production must be sealed and published or remains uncertain"]
pub(crate) struct PayloadBuilder {
    pub(super) guard: MutationGuard,
    pub(super) target: PathBuf,
    pub(super) stage: PathBuf,
    pub(super) token: String,
    pub(super) parent: Directory,
    inventory: PayloadInventory,
    nodes: Vec<OwnedNode>,
    root_initial: Identity,
    sources: Vec<UseGuard>,
    source_pins: Vec<(File, Stamp)>,
    failed: bool,
    groups: BTreeMap<(u64, u64), Vec<usize>>,
    #[cfg(test)]
    fault: Option<&'static str>,
}
pub(crate) struct PayloadSeal {
    pub(super) builder: PayloadBuilder,
    digest: String,
}
fn open_pin(parent: &File, leaf: &OsStr) -> io::Result<File> {
    #[cfg(target_os = "linux")]
    {
        open_at(parent, leaf, libc::O_PATH, 0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (parent, leaf);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "payload sealer requires Linux inode descriptors",
        ))
    }
}
fn chmod(file: &File, mode: u32) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let empty = CString::new("").map_err(|_| invalid("name"))?;
        if unsafe {
            libc::syscall(
                libc::SYS_fchmodat2,
                file.as_raw_fd(),
                empty.as_ptr(),
                mode as libc::mode_t,
                libc::AT_EMPTY_PATH,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (file, mode);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "payload chmod requires fchmodat2",
        ))
    }
}
fn fd_reserve(entries: usize, bytes: u64, parent: &File) -> io::Result<()> {
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    syscall(unsafe { libc::fstatvfs(parent.as_raw_fd(), &mut stat) })?;
    let available = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
    // Explicit per-tree payload capacity + inode admission. Preserve a10% margin;
    // this never changes coordinator reservations or promises physical allocation.
    if bytes > available.saturating_sub(available / 10)
        || (stat.f_favail != 0 && entries as u64 > stat.f_favail as u64)
    {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "payload inventory exceeds filesystem admission",
        ));
    }
    let mut fd_limit: libc::rlimit = unsafe { std::mem::zeroed() };
    syscall(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut fd_limit) })?;
    let descriptors = entries
        .checked_mul(3)
        .and_then(|n| n.checked_add(64))
        .ok_or_else(limit)?;
    if descriptors as libc::rlim_t > fd_limit.rlim_cur {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "payload inventory exceeds available FD profile",
        ));
    }
    let mut reservation = Vec::with_capacity(descriptors);
    for _ in 0..descriptors {
        reservation.push(parent.try_clone()?);
    }
    drop(reservation);
    Ok(())
}
impl PayloadBuilder {
    pub(super) fn start(
        mut guard: MutationGuard,
        target: &Path,
        inventory: PayloadInventory,
    ) -> io::Result<Self> {
        let target = absolute(target)?;
        guard.payload_scope(&target)?;
        let parent = Directory::open(
            target
                .parent()
                .ok_or_else(|| invalid("payload target parent"))?,
        )?;
        parent.private(uid())?;
        fd_reserve(inventory.entries.len(), inventory.bytes, &parent.file)?;
        guard.payload_inventory(
            &inventory.digest,
            inventory.entries.len(),
            inventory.bytes,
            inventory.metadata_bytes,
        )?;
        let (stage, token, root) = guard.payload_stage(&target)?;
        // All producer data FDs are created here; no caller receives a writable
        // handle or pathname to mutate. All paths are from the admitted inventory.
        let root_initial = Identity::of(&root)?;
        let root = open_at(
            &parent.file,
            stage.file_name().ok_or_else(|| invalid("stage name"))?,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOATIME,
            0,
        )?;
        let pin = open_pin(
            &parent.file,
            stage.file_name().ok_or_else(|| invalid("stage name"))?,
        )?;
        let mut builder = Self {
            guard,
            target,
            stage,
            token,
            parent,
            inventory,
            nodes: vec![OwnedNode {
                pin,
                read: Some(root),
                writer: None,
                stamp: None,
                hash: None,
                written: true,
                extents: None,
            }],
            root_initial,
            sources: Vec::new(),
            source_pins: Vec::new(),
            failed: false,
            groups: BTreeMap::new(),
            #[cfg(test)]
            fault: None,
        };
        builder.create_nodes()?;
        for (i, node) in builder.nodes.iter().enumerate() {
            let id = Identity::of(&node.pin)?;
            builder.groups.entry((id.dev, id.ino)).or_default().push(i);
        }
        Ok(builder)
    }
    fn dir(&self, index: usize) -> io::Result<&File> {
        let p = self.inventory.entries[index]
            .relative
            .parent()
            .ok_or_else(|| invalid("entry parent"))?;
        let i = *self
            .inventory
            .index
            .get(p)
            .ok_or_else(|| invalid("parent not inventoried"))?;
        self.nodes[i]
            .read
            .as_ref()
            .ok_or_else(|| invalid("parent descriptor missing"))
    }
    fn create_nodes(&mut self) -> io::Result<()> {
        for index in 1..self.inventory.entries.len() {
            self.guard.checkpoint()?;
            let entry = &self.inventory.entries[index];
            let parent = self.dir(index)?;
            let leaf = entry
                .relative
                .file_name()
                .ok_or_else(|| invalid("payload leaf"))?;
            let (read, writer, written) = match &entry.kind {
                PayloadKind::Directory => {
                    mkdir_at(parent, leaf, 0o700)?;
                    (
                        Some(open_at(
                            parent,
                            leaf,
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOATIME,
                            0,
                        )?),
                        None,
                        true,
                    )
                }
                PayloadKind::File { .. } => {
                    let file = open_at(
                        parent,
                        leaf,
                        libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NONBLOCK,
                        0o600,
                    )?;
                    let read = open_at(
                        parent,
                        leaf,
                        libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOATIME,
                        0,
                    )?;
                    (Some(read), Some(file), false)
                }
                PayloadKind::Symlink { target } => {
                    let target =
                        CString::new(target.as_slice()).map_err(|_| invalid("symlink bytes"))?;
                    let name = name(leaf)?;
                    syscall(unsafe {
                        libc::symlinkat(target.as_ptr(), parent.as_raw_fd(), name.as_ptr())
                    })?;
                    (None, None, true)
                }
                PayloadKind::Hardlink { source } => {
                    let original = *self
                        .inventory
                        .index
                        .get(source)
                        .ok_or_else(|| invalid("hardlink source"))?;
                    let a = name(
                        source
                            .file_name()
                            .ok_or_else(|| invalid("link source name"))?,
                    )?;
                    let b = name(leaf)?;
                    syscall(unsafe {
                        libc::linkat(
                            self.dir(original)?.as_raw_fd(),
                            a.as_ptr(),
                            parent.as_raw_fd(),
                            b.as_ptr(),
                            0,
                        )
                    })?;
                    (
                        Some(
                            self.nodes[original]
                                .read
                                .as_ref()
                                .ok_or_else(|| invalid("source descriptor"))?
                                .try_clone()?,
                        ),
                        None,
                        true,
                    )
                }
            };
            let pin = open_pin(parent, leaf)?;
            if mount_id(&pin)? != mount_id(&self.parent.file)? {
                return Err(invalid("payload nested mount"));
            }
            self.nodes.push(OwnedNode {
                pin,
                read,
                writer,
                stamp: None,
                hash: None,
                written,
                extents: None,
            });
        }
        Ok(())
    }
    #[cfg(test)]
    fn fault(&mut self, at: &'static str) {
        self.fault = Some(at);
    }
    pub(crate) fn write_file(&mut self, path: &Path, reader: &mut dyn Read) -> io::Result<()> {
        if self.failed {
            return Err(invalid("payload producer failed"));
        }
        let result = self.write_inner(path, reader);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn write_inner(&mut self, path: &Path, reader: &mut dyn Read) -> io::Result<()> {
        self.guard.checkpoint()?;
        relative(path)?;
        let i = *self
            .inventory
            .index
            .get(path)
            .ok_or_else(|| invalid("file not in payload inventory"))?;
        let PayloadKind::File { size } = &self.inventory.entries[i].kind else {
            return Err(invalid("payload entry is not a primary file"));
        };
        let size = *size;
        if self.nodes[i].written {
            return Err(invalid("payload file already supplied"));
        }
        let file = self.nodes[i].writer.as_ref().unwrap();
        let mut left = size;
        let mut hash = Sha256::new();
        let mut bytes = vec![0; 128 * 1024];
        let mut offset = 0;
        while left > 0 {
            self.guard.checkpoint()?;
            let n = left.min(bytes.len() as u64) as usize;
            reader.read_exact(&mut bytes[..n])?;
            hash.update(&bytes[..n]);
            file.write_all_at(&bytes[..n], offset)?;
            offset += n as u64;
            left -= n as u64;
        }
        let mut extra = [0];
        if reader.read(&mut extra)? != 0 {
            return Err(invalid("payload input exceeds declared file length"));
        }
        file.set_len(size)?;
        file.sync_all()?;
        self.nodes[i].hash = Some(hex::encode(hash.finalize()));
        self.nodes[i].written = true;
        Ok(())
    }
    fn copy_sparse_file(&mut self, path: &Path, source: &File) -> io::Result<()> {
        let index = *self
            .inventory
            .index
            .get(path)
            .ok_or_else(|| invalid("source file not planned"))?;
        let size = match &self.inventory.entries[index].kind {
            PayloadKind::File { size } => *size,
            _ => return Err(invalid("sparse source is not a primary file")),
        };
        let ranges = sparse_map(source, size)?;
        let writer = self.nodes[index]
            .writer
            .as_ref()
            .ok_or_else(|| invalid("payload writer absent"))?;
        let mut buffer = [0u8; 131072];
        for &(start, end) in &ranges {
            let mut at = start;
            while at < end {
                self.guard.checkpoint()?;
                let n = (end - at).min(buffer.len() as u64) as usize;
                source.read_exact_at(&mut buffer[..n], at)?;
                writer.write_all_at(&buffer[..n], at)?;
                at += n as u64;
            }
        }
        writer.set_len(size)?;
        writer.sync_all()?;
        let reader = self.nodes[index].read.as_ref().unwrap();
        if sparse_map(reader, size)? != ranges {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "destination cannot reproduce exact source data/hole extents",
            ));
        }
        let expected = hash_file(source, size)?;
        if hash_file(reader, size)? != expected {
            return Err(invalid("sparse clone content mismatch"));
        }
        self.nodes[index].hash = Some(expected);
        self.nodes[index].extents = Some(ranges);
        self.nodes[index].written = true;
        Ok(())
    }
    pub(crate) fn seal(mut self) -> io::Result<PayloadSeal> {
        if self.failed || self.nodes.iter().any(|n| !n.written) {
            return Err(invalid("payload producer incomplete"));
        }
        for node in &mut self.nodes {
            if let Some(writer) = node.writer.take() {
                writer.sync_all()?;
                drop(writer);
            }
        }
        let mut handled = BTreeSet::new();
        for index in 0..self.nodes.len() {
            if matches!(self.inventory.entries[index].kind, PayloadKind::Directory) {
                continue;
            }
            self.guard.checkpoint()?;
            let id = Identity::of(&self.nodes[index].pin)?;
            if handled.insert((id.dev, id.ino)) {
                self.apply_metadata(index)?;
            }
        }
        for index in (0..self.nodes.len()).rev() {
            if matches!(self.inventory.entries[index].kind, PayloadKind::Directory) {
                self.guard.checkpoint()?;
                self.apply_metadata(index)?;
            }
        }
        // No externally escaped descriptor can mutate the result. Convert the
        // only core RW file handles to independently opened readonly handles
        // BEFORE final mode closure by keeping read descriptors from creation.
        #[cfg(test)]
        if self.fault == Some("before-seal") {
            return Err(io::Error::other("injected payload seal fault"));
        }
        self.verify()?;
        let digest = super::recovery::digest(&(
            self.inventory.digest.as_str(),
            self.nodes
                .iter()
                .map(|n| (&n.stamp.as_ref().unwrap().identity, &n.hash))
                .collect::<Vec<_>>(),
        ))?;
        self.guard.payload_seal(
            &self.stage,
            &self.root_initial,
            &Identity::of(&self.nodes[0].pin)?,
            &digest,
        )?;
        Ok(PayloadSeal {
            builder: self,
            digest,
        })
    }
    fn apply_metadata(&mut self, index: usize) -> io::Result<()> {
        let entry = &self.inventory.entries[index];
        let meta = &entry.metadata;
        let parent = if index == 0 {
            &self.parent.file
        } else {
            self.dir(index)?
        };
        let leaf = if index == 0 {
            self.stage
                .file_name()
                .ok_or_else(|| invalid("stage leaf"))?
        } else {
            entry.relative.file_name().unwrap()
        };
        let named = open_pin(parent, leaf)?;
        if Identity::of(&named)? != Identity::of(&self.nodes[index].pin)? {
            return Err(invalid("payload entry replaced before final metadata"));
        }
        let cname = name(leaf)?;
        syscall(unsafe {
            libc::fchownat(
                parent.as_raw_fd(),
                cname.as_ptr(),
                meta.uid,
                meta.gid,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        })?;
        if !matches!(entry.kind, PayloadKind::Symlink { .. }) {
            chmod(&self.nodes[index].pin, meta.mode)?;
        }
        #[cfg(test)]
        if self.fault == Some("metadata") {
            return Err(io::Error::other("injected payload metadata fault"));
        }
        super::payload_fs::apply(parent, leaf, &meta.xattrs)?;
        if super::payload_fs::attrs(parent, leaf)? != meta.xattrs {
            return Err(invalid("payload xattr/ACL set mismatch"));
        }
        let times = [
            libc::timespec {
                tv_sec: meta.atime_sec as _,
                tv_nsec: meta.atime_nsec as _,
            },
            libc::timespec {
                tv_sec: meta.mtime_sec as _,
                tv_nsec: meta.mtime_nsec as _,
            },
        ];
        syscall(unsafe {
            libc::utimensat(
                parent.as_raw_fd(),
                cname.as_ptr(),
                times.as_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        })?;
        if let Some(file) = &self.nodes[index].read {
            file.sync_all()?;
        }
        self.parent.file.sync_all()?;
        let value = stamp(&self.nodes[index].pin)?;
        if value.identity.uid != meta.uid
            || value.gid != meta.gid
            || value.identity.mode & 0o7777 != meta.mode
            || value.atime != meta.atime_sec
            || value.atime_ns != meta.atime_nsec
            || value.mtime != meta.mtime_sec
            || value.mtime_ns != meta.mtime_nsec
        {
            return Err(invalid(
                "filesystem did not preserve declared payload metadata",
            ));
        }
        let object = (value.identity.dev, value.identity.ino);
        for &i in self
            .groups
            .get(&object)
            .ok_or_else(|| invalid("payload identity group missing"))?
        {
            self.nodes[i].stamp = Some(value.clone());
        }
        Ok(())
    }
    fn verify(&self) -> io::Result<()> {
        self.parent.revalidate()?;
        if self.nodes.iter().any(|n| n.writer.is_some()) {
            return Err(invalid("payload writer descriptors still open at seal"));
        }
        let named = open_pin(
            &self.parent.file,
            self.stage
                .file_name()
                .ok_or_else(|| invalid("stage name"))?,
        )?;
        if Identity::of(&named)? != Identity::of(&self.nodes[0].pin)? {
            return Err(invalid("payload stage root replaced"));
        }
        for source in &self.sources {
            source.revalidate()?;
        }
        for (file, expected) in &self.source_pins {
            if !source_unchanged(expected, &stamp(file)?) {
                return Err(invalid("source generation changed before publication"));
            }
        }
        let mut groups = BTreeMap::<(u64, u64), u64>::new();
        for node in &self.nodes {
            let id = Identity::of(&node.pin)?;
            if id.mode & libc::S_IFMT == libc::S_IFREG {
                *groups.entry((id.dev, id.ino)).or_default() += 1;
            }
        }
        for (i, node) in self.nodes.iter().enumerate() {
            let meta = &self.inventory.entries[i].metadata;
            let current = stamp(&node.pin)?;
            let expected = node
                .stamp
                .as_ref()
                .ok_or_else(|| invalid("payload metadata not finalized"))?;
            if !unchanged(expected, &current)
                || current.identity.uid != meta.uid
                || current.identity.mode & 0o7777 != meta.mode
                || current.gid != meta.gid
                || current.atime != meta.atime_sec
                || current.atime_ns != meta.atime_nsec
                || current.mtime != meta.mtime_sec
                || current.mtime_ns != meta.mtime_nsec
            {
                return Err(invalid("payload seal identity/metadata changed"));
            }
            if current.identity.mode & libc::S_IFMT == libc::S_IFREG
                && groups.get(&(current.identity.dev, current.identity.ino))
                    != Some(&current.identity.links)
            {
                return Err(invalid("payload has external hardlinks"));
            }
            if let Some(file) = &node.read {
                if matches!(self.inventory.entries[i].kind, PayloadKind::Directory) {
                    let mut expected = self.inventory.children[i]
                        .iter()
                        .map(|&n| {
                            Ok((
                                self.inventory.entries[n]
                                    .relative
                                    .file_name()
                                    .unwrap()
                                    .to_os_string(),
                                Identity::of(&self.nodes[n].pin)?.ino,
                            ))
                        })
                        .collect::<io::Result<Vec<_>>>()?;
                    expected.sort();
                    if super::payload_fs::names(file, CEILING_ENTRIES)? != expected {
                        return Err(invalid("payload directory entry substitution"));
                    }
                }
                if matches!(self.inventory.entries[i].kind, PayloadKind::File { .. }) {
                    let digest = hash_file(file, current.size)?;
                    if let Some(expected) = &node.extents {
                        if &sparse_map(file, current.size)? != expected {
                            return Err(invalid("payload sparse map changed"));
                        }
                    }
                    if node.hash.as_ref() != Some(&digest) {
                        return Err(invalid("payload bytes changed after writer completion"));
                    }
                }
            }
        }
        Ok(())
    }
}
impl PayloadSeal {
    fn publish_inner(&mut self) -> io::Result<()> {
        self.builder.guard.checkpoint()?;
        self.builder.verify()?;
        let root = Identity::of(&self.builder.nodes[0].pin)?;
        self.builder.guard.publish_payload(
            &self.builder.token,
            &self.builder.stage,
            &self.builder.target,
            &self.builder.parent,
            &root,
            &self.digest,
            &self
                .builder
                .inventory
                .entries
                .iter()
                .zip(&self.builder.nodes)
                .map(|(e, n)| Ok((e.relative.clone(), Identity::of(&n.pin)?)))
                .collect::<io::Result<Vec<_>>>()?,
        )
    }
    pub(crate) fn publish_into_guard(mut self) -> io::Result<MutationGuard> {
        self.publish_inner()?;
        Ok(self.builder.guard)
    }
    /// Publish exactly as [`Self::publish_into_guard`] and keep the sealed
    /// members' retained core descriptors as a read capability for the SAME
    /// published tree (for example to build its EROFS lower). The capability is
    /// only ever minted here, from this seal; no path, hash, PID or flag issues it.
    pub(crate) fn publish_into_guard_with_reader(
        mut self,
    ) -> io::Result<(MutationGuard, SealedPayloadRead)> {
        self.publish_inner()?;
        SealedPayloadRead::from_published(self.builder)
    }
    pub(crate) fn publish(self) -> io::Result<()> {
        self.publish_into_guard()?.commit()?;
        Ok(())
    }
    /* publication is implemented above so cache callers may commit the digest
    in the SAME original guard before handoff, without reissuing authority. */
}
fn sparse_map(file: &File, size: u64) -> io::Result<Vec<(u64, u64)>> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    while offset < size {
        let start = unsafe { libc::lseek(file.as_raw_fd(), offset as _, libc::SEEK_DATA) };
        if start < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ENXIO) {
                break;
            }
            return Err(e);
        }
        let end = unsafe { libc::lseek(file.as_raw_fd(), start, libc::SEEK_HOLE) };
        if end < 0 {
            return Err(io::Error::last_os_error());
        }
        let end = (end as u64).min(size);
        if (start as u64) < offset || end <= start as u64 {
            return Err(invalid("invalid sparse extent"));
        }
        if ranges.len() >= 65536 {
            return Err(limit());
        }
        ranges.push((start as u64, end));
        offset = end;
    }
    Ok(ranges)
}
fn hash_file(file: &File, size: u64) -> io::Result<String> {
    let mut bytes = [0; 65536];
    let mut at = 0;
    let mut hash = Sha256::new();
    while at < size {
        let n = (size - at).min(bytes.len() as u64) as usize;
        file.read_exact_at(&mut bytes[..n], at)?;
        hash.update(&bytes[..n]);
        at += n as u64;
    }
    Ok(hex::encode(hash.finalize()))
}
/// Readable Strict source capture; no chmod/source writes, no permission upgrade.
pub(crate) struct ReadablePayload {
    lease: Lease,
    guard: UseGuard,
    inventory: PayloadInventory,
    files: BTreeMap<PathBuf, File>,
    pins: Vec<(File, Stamp)>,
}
impl Lease {
    pub(crate) fn capture_payload(&self, root: &Path) -> io::Result<ReadablePayload> {
        self.capture_payload_inner(root, None)
    }
    /// Test-only race injection: `race` runs with a symlink's relative path
    /// after the symlink is pinned and before its target is read.
    #[cfg(test)]
    pub(crate) fn capture_payload_racing(
        &self,
        root: &Path,
        race: &dyn Fn(&Path),
    ) -> io::Result<ReadablePayload> {
        self.capture_payload_inner(root, Some(race))
    }
    fn capture_payload_inner(
        &self,
        root: &Path,
        race: Option<&dyn Fn(&Path)>,
    ) -> io::Result<ReadablePayload> {
        let guard = self.use_path(root)?;
        let root = absolute(root)?;
        let parent = Directory::open(root.parent().ok_or_else(|| invalid("source parent"))?)?;
        let mut entries = Vec::new();
        let mut files = BTreeMap::new();
        let mut pins = Vec::new();
        let mut links = BTreeMap::<(u64, u64), (PathBuf, u64, u64)>::new();
        let mut budget = (0u64, 0usize);
        capture_node(
            &parent.file,
            root.file_name().ok_or_else(|| invalid("source root"))?,
            PathBuf::new(),
            mount_id(&parent.file)?,
            &mut entries,
            &mut files,
            &mut pins,
            &mut links,
            &mut budget,
            race,
        )?;
        if links.values().any(|(_, found, total)| found != total) {
            return Err(invalid("source hardlinks escape captured tree"));
        }
        let inventory = PayloadInventory::new(entries)?;
        guard.revalidate()?;
        Ok(ReadablePayload {
            lease: self.clone(),
            guard,
            inventory,
            files,
            pins,
        })
    }
}
fn capture_node(
    parent: &File,
    leaf: &OsStr,
    path: PathBuf,
    mount: u64,
    entries: &mut Vec<PayloadEntry>,
    files: &mut BTreeMap<PathBuf, File>,
    pins: &mut Vec<(File, Stamp)>,
    links: &mut BTreeMap<(u64, u64), (PathBuf, u64, u64)>,
    budget: &mut (u64, usize),
    race: Option<&dyn Fn(&Path)>,
) -> io::Result<()> {
    relative(&path)?;
    if entries.len() >= CEILING_ENTRIES {
        return Err(limit());
    }
    let pin = open_pin(parent, leaf)?;
    let meta = pin.metadata()?;
    let original = stamp(&pin)?;
    if mount_id(&pin)? != mount || meta.uid() != uid() {
        return Err(invalid("source owner/mount mismatch"));
    }
    let metadata = PayloadMetadata {
        mode: meta.mode() & 0o7777,
        uid: meta.uid(),
        gid: meta.gid(),
        atime_sec: meta.atime(),
        atime_nsec: meta.atime_nsec(),
        mtime_sec: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
        xattrs: super::payload_fs::attrs(parent, leaf)?,
    };
    budget.1 = budget
        .1
        .checked_add(
            path.as_os_str().as_bytes().len()
                + 256
                + metadata
                    .xattrs
                    .iter()
                    .map(|(k, v)| k.len() + v.len())
                    .sum::<usize>(),
        )
        .ok_or_else(limit)?;
    if meta.is_file() {
        budget.0 = budget.0.checked_add(meta.len()).ok_or_else(limit)?;
    }
    if budget.0 > CEILING_BYTES || budget.1 > CEILING_METADATA {
        return Err(limit());
    }
    let directory = meta.is_dir();
    if directory || meta.is_file() {
        let readable = open_at(
            parent,
            leaf,
            libc::O_RDONLY
                | libc::O_NOATIME
                | libc::O_NONBLOCK
                | if directory { libc::O_DIRECTORY } else { 0 },
            0,
        )?;
        // Native64 FS_IOC_GETFLAGS, verified Linux UAPI used by the existing
        // private fidelity checker. Only extents/index implementation flags are
        // ignored; policy flags are not silently dropped by a readable clone.
        #[cfg(all(
            target_os = "linux",
            target_pointer_width = "64",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ))]
        {
            let mut flags: libc::c_long = 0;
            syscall(unsafe {
                libc::ioctl(
                    readable.as_raw_fd(),
                    0x80086601u64 as libc::c_ulong,
                    &mut flags,
                )
            })?;
            if flags & !(0x80000 | 0x1000) != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "source has unsupported policy inode flags",
                ));
            }
        }
    }
    let kind = if directory {
        PayloadKind::Directory
    } else if meta.is_file() {
        if let Some((source, count, _)) = links.get_mut(&(meta.dev(), meta.ino())) {
            *count += 1;
            PayloadKind::Hardlink {
                source: source.clone(),
            }
        } else {
            links.insert((meta.dev(), meta.ino()), (path.clone(), 1, meta.nlink()));
            let reader = open_at(
                parent,
                leaf,
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOATIME,
                0,
            )?;
            if Identity::of(&reader)? != Identity::of(&pin)? {
                return Err(invalid("source replaced while opening"));
            }
            files.insert(path.clone(), reader);
            PayloadKind::File { size: meta.len() }
        }
    } else if meta.file_type().is_symlink() {
        if let Some(race) = race {
            race(&path);
        }
        // The target is read through the pin itself; the name must still bind
        // that inode afterwards, like the regular-file reader above.
        let target = super::payload_fs::read_link(&pin)?;
        if Identity::of(&open_pin(parent, leaf)?)? != Identity::of(&pin)? {
            return Err(invalid("source replaced while reading link"));
        }
        PayloadKind::Symlink { target }
    } else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "source payload object type unsupported",
        ));
    };
    if !source_unchanged(&original, &stamp(&pin)?) {
        return Err(invalid("source metadata changed while inventorying"));
    }
    entries.push(PayloadEntry {
        relative: path.clone(),
        metadata,
        kind,
    });
    if directory {
        let reader = open_at(
            parent,
            leaf,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOATIME,
            0,
        )?;
        for (name, _) in super::payload_fs::names(&reader, CEILING_ENTRIES - entries.len())? {
            capture_node(
                &reader,
                &name,
                path.join(&name),
                mount,
                entries,
                files,
                pins,
                links,
                budget,
                race,
            )?;
        }
    }
    if !source_unchanged(&original, &stamp(&pin)?) {
        return Err(invalid("source changed during capture"));
    }
    pins.push((pin, original));
    Ok(())
}
impl ReadablePayload {
    pub(crate) fn produce(self, guard: MutationGuard, target: &Path) -> io::Result<PayloadSeal> {
        guard.payload_source(&self.lease)?;
        self.guard.revalidate()?;
        for (fd, expected) in &self.pins {
            if !source_unchanged(expected, &stamp(fd)?) {
                return Err(invalid("captured source changed"));
            }
        }
        let mut builder = PayloadBuilder::start(guard, target, self.inventory)?;
        for (path, file) in self.files {
            builder.copy_sparse_file(&path, &file)?;
        }
        self.guard.revalidate()?;
        for (fd, expected) in &self.pins {
            if !source_unchanged(expected, &stamp(fd)?) {
                return Err(invalid("source changed during copy"));
            }
        }
        builder.sources.push(self.guard);
        builder.source_pins = self.pins;
        builder.seal()
    }
}
/// Read capability for EXACTLY one sealed and published payload tree, minted
/// only by [`PayloadSeal::publish_into_guard_with_reader`]. It retains the core's
/// own member descriptors from production (opened at creation, before final
/// modes, `O_NOATIME`), so owner-unreadable members such as mode 0000 are read
/// without any chmod or grant, and nothing is reopened by pathname. Every
/// served member is first rechecked against its sealed inode identity, link
/// count, metadata stamp (ctime included, atime excluded) and named identity;
/// directories against their exact sealed name set; file bytes are hashed as
/// they are consumed. `finish` re-verifies the whole tree and any content not
/// consumed. The generic `OwnerReadGuard` profile is unchanged.
pub(crate) struct SealedPayloadRead {
    lease: Lease,
    root: PathBuf,
    parent: Directory,
    inventory: PayloadInventory,
    nodes: Vec<SealedNode>,
    contents: BTreeMap<(u64, u64), (usize, String)>,
    failed: Arc<AtomicBool>,
    verified: Arc<Mutex<BTreeSet<(u64, u64)>>>,
}
struct SealedNode {
    pin: File,
    read: Option<File>,
    stamp: Stamp,
}
/// One sealed regular file's bytes, bounded by the sealed size and hashed as
/// read; reaching EOF with bytes that differ from the seal is an error.
pub(crate) struct SealedFile {
    file: File,
    size: u64,
    at: u64,
    hash: Sha256,
    expected: String,
    inode: (u64, u64),
    done: bool,
    failed: Arc<AtomicBool>,
    verified: Arc<Mutex<BTreeSet<(u64, u64)>>>,
}
/// Sealed metadata equality for read-back. Atime is excluded on purpose: any
/// reader (including the caller's own cache inspection) may update it, while
/// every change that can alter content, shape, ownership, mode, links or
/// xattrs also changes ctime, which no unprivileged process can restore.
fn same_sealed(a: &Stamp, b: &Stamp) -> bool {
    a.identity == b.identity
        && a.gid == b.gid
        && a.size == b.size
        && (a.mtime, a.mtime_ns, a.ctime, a.ctime_ns) == (b.mtime, b.mtime_ns, b.ctime, b.ctime_ns)
}
impl SealedPayloadRead {
    fn from_published(builder: PayloadBuilder) -> io::Result<(MutationGuard, Self)> {
        let PayloadBuilder {
            guard,
            target,
            parent,
            inventory,
            nodes,
            sources,
            ..
        } = builder;
        // Source read holds of a clone production end with the production.
        drop(sources);
        let lease = guard.lease_handle();
        let mut sealed = Vec::with_capacity(nodes.len());
        let mut contents = BTreeMap::new();
        for (index, node) in nodes.into_iter().enumerate() {
            let mut recorded = node
                .stamp
                .ok_or_else(|| invalid("payload metadata not finalized"))?;
            if index == 0 {
                // The publication rename this core just performed may only
                // have changed the root's ctime; everything else is the seal.
                let now = stamp(&node.pin)?;
                if now.identity != recorded.identity
                    || now.gid != recorded.gid
                    || now.size != recorded.size
                    || (now.mtime, now.mtime_ns) != (recorded.mtime, recorded.mtime_ns)
                {
                    return Err(invalid("published payload root changed during publication"));
                }
                recorded = now;
            }
            if let Some(hash) = node.hash {
                contents.insert(
                    (recorded.identity.dev, recorded.identity.ino),
                    (index, hash),
                );
            }
            sealed.push(SealedNode {
                pin: node.pin,
                read: node.read,
                stamp: recorded,
            });
        }
        let reader = Self {
            lease,
            root: target,
            parent,
            inventory,
            nodes: sealed,
            contents,
            failed: Arc::new(AtomicBool::new(false)),
            verified: Arc::new(Mutex::new(BTreeSet::new())),
        };
        reader.checkpoint()?;
        Ok((guard, reader))
    }
    fn terminal<T>(&self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() {
            self.failed.store(true, Ordering::Release);
        }
        result
    }
    fn live(&self) -> io::Result<()> {
        if self.failed.load(Ordering::Acquire) {
            return Err(invalid("sealed payload read failure is terminal"));
        }
        Ok(())
    }
    fn locate(&self, path: &Path) -> io::Result<usize> {
        let path = absolute(path)?;
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| invalid("path is outside the sealed payload"))?;
        self.inventory
            .index
            .get(relative)
            .copied()
            .ok_or_else(|| invalid("path is not a sealed payload member"))
    }
    /// The directory descriptor holding member `index` and its exact leaf name.
    fn place(&self, index: usize) -> io::Result<(&File, &OsStr)> {
        if index == 0 {
            return Ok((
                &self.parent.file,
                self.root
                    .file_name()
                    .ok_or_else(|| invalid("sealed payload root name"))?,
            ));
        }
        let relative = &self.inventory.entries[index].relative;
        let parent = *self
            .inventory
            .index
            .get(
                relative
                    .parent()
                    .ok_or_else(|| invalid("sealed member parent"))?,
            )
            .ok_or_else(|| invalid("sealed member parent not inventoried"))?;
        let directory = self.nodes[parent]
            .read
            .as_ref()
            .ok_or_else(|| invalid("sealed directory descriptor missing"))?;
        Ok((
            directory,
            relative
                .file_name()
                .ok_or_else(|| invalid("sealed member name"))?,
        ))
    }
    /// Exact sealed inode, metadata stamp and named identity of one member.
    fn check(&self, index: usize) -> io::Result<()> {
        let node = &self.nodes[index];
        let current = stamp(&node.pin)?;
        if !same_sealed(&node.stamp, &current) {
            return Err(invalid("sealed payload member changed after seal"));
        }
        let (directory, leaf) = self.place(index)?;
        if Identity::of(&open_pin(directory, leaf)?)? != current.identity {
            return Err(invalid("sealed payload member replaced after seal"));
        }
        Ok(())
    }
    /// A directory's live entries are exactly its sealed children (names and inodes).
    fn check_names(&self, index: usize) -> io::Result<()> {
        let directory = self.nodes[index]
            .read
            .as_ref()
            .ok_or_else(|| invalid("sealed directory descriptor missing"))?;
        let mut expected = self.inventory.children[index]
            .iter()
            .map(|&child| {
                Ok((
                    self.inventory.entries[child]
                        .relative
                        .file_name()
                        .ok_or_else(|| invalid("sealed member name"))?
                        .to_os_string(),
                    self.nodes[child].stamp.identity.ino,
                ))
            })
            .collect::<io::Result<Vec<_>>>()?;
        expected.sort();
        if super::payload_fs::names(directory, CEILING_ENTRIES)? != expected {
            return Err(invalid("sealed payload directory entries changed"));
        }
        Ok(())
    }
    pub(crate) fn checkpoint(&self) -> io::Result<()> {
        self.terminal((|| {
            self.live()?;
            self.parent.revalidate()?;
            self.check(0)
        })())
    }
    pub(crate) fn read_dir(&mut self, path: &Path) -> io::Result<Vec<super::OwnerReadEntry>> {
        let result = (|| {
            self.live()?;
            let index = self.locate(path)?;
            if !matches!(self.inventory.entries[index].kind, PayloadKind::Directory) {
                return Err(invalid("sealed payload member is not a directory"));
            }
            self.check(index)?;
            self.check_names(index)?;
            let mut entries = Vec::with_capacity(self.inventory.children[index].len());
            for &child in &self.inventory.children[index] {
                self.check(child)?;
                entries.push(super::OwnerReadEntry {
                    name: self.inventory.entries[child]
                        .relative
                        .file_name()
                        .ok_or_else(|| invalid("sealed member name"))?
                        .to_os_string(),
                    metadata: self.nodes[child].pin.metadata()?,
                });
            }
            Ok(entries)
        })();
        self.terminal(result)
    }
    pub(crate) fn original_metadata(&self, path: &Path) -> io::Result<Metadata> {
        self.terminal((|| {
            self.live()?;
            let index = self.locate(path)?;
            self.check(index)?;
            self.nodes[index].pin.metadata()
        })())
    }
    /// The sealed symlink target from the inventory, after verifying that the
    /// sealed symlink inode is still the one named at this path.
    pub(crate) fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        self.terminal((|| {
            self.live()?;
            let index = self.locate(path)?;
            let PayloadKind::Symlink { target } = &self.inventory.entries[index].kind else {
                return Err(invalid("sealed payload member is not a symlink"));
            };
            self.check(index)?;
            Ok(PathBuf::from(OsString::from_vec(target.clone())))
        })())
    }
    pub(crate) fn open_file(&mut self, path: &Path) -> io::Result<SealedFile> {
        let result = (|| {
            self.live()?;
            let index = self.locate(path)?;
            if !matches!(
                self.inventory.entries[index].kind,
                PayloadKind::File { .. } | PayloadKind::Hardlink { .. }
            ) {
                return Err(invalid("sealed payload member is not a regular file"));
            }
            self.check(index)?;
            let node = &self.nodes[index];
            let inode = (node.stamp.identity.dev, node.stamp.identity.ino);
            let (_, expected) = self
                .contents
                .get(&inode)
                .ok_or_else(|| invalid("sealed file content digest missing"))?;
            Ok(SealedFile {
                file: node
                    .read
                    .as_ref()
                    .ok_or_else(|| invalid("sealed file descriptor missing"))?
                    .try_clone()?,
                size: node.stamp.size,
                at: 0,
                hash: Sha256::new(),
                expected: expected.clone(),
                inode,
                done: false,
                failed: self.failed.clone(),
                verified: self.verified.clone(),
            })
        })();
        self.terminal(result)
    }
    /// Re-verify the whole published tree before its derived image may be
    /// published: lifecycle, root and parent, every member's sealed identity,
    /// stamp and name, every directory's exact entries, and any file content
    /// not already verified while it was consumed.
    pub(crate) fn finish(self) -> io::Result<()> {
        self.live()?;
        {
            let tx = self.lease.0.namespace.transaction()?;
            let op = tx.operation(&self.lease.0.id)?;
            if op.closed || op.uncertain {
                return Err(invalid("sealed payload lifecycle is no longer available"));
            }
        }
        self.parent.revalidate()?;
        for index in 0..self.nodes.len() {
            self.check(index)?;
            if matches!(self.inventory.entries[index].kind, PayloadKind::Directory) {
                self.check_names(index)?;
            }
        }
        let verified = self
            .verified
            .lock()
            .map_err(|_| invalid("sealed payload verification poisoned"))?
            .clone();
        for (inode, (index, expected)) in &self.contents {
            if verified.contains(inode) {
                continue;
            }
            let node = &self.nodes[*index];
            let file = node
                .read
                .as_ref()
                .ok_or_else(|| invalid("sealed file descriptor missing"))?;
            if &hash_file(file, node.stamp.size)? != expected {
                return Err(invalid("sealed payload bytes changed after seal"));
            }
        }
        self.live()
    }
}
impl Read for SealedFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.failed.load(Ordering::Acquire) {
            return Err(invalid("sealed payload read failure is terminal"));
        }
        if self.at >= self.size {
            if !self.done {
                self.done = true;
                if hex::encode(std::mem::take(&mut self.hash).finalize()) != self.expected {
                    self.failed.store(true, Ordering::Release);
                    return Err(invalid("sealed payload bytes differ from the seal"));
                }
                self.verified
                    .lock()
                    .map_err(|_| invalid("sealed payload verification poisoned"))?
                    .insert(self.inode);
            }
            return Ok(0);
        }
        let n = (buf.len() as u64).min(self.size - self.at) as usize;
        let got = self.file.read_at(&mut buf[..n], self.at)?;
        if got == 0 && n > 0 {
            self.failed.store(true, Ordering::Release);
            return Err(invalid("sealed payload file shorter than the seal"));
        }
        self.hash.update(&buf[..got]);
        self.at += got as u64;
        Ok(got)
    }
}
#[cfg(test)]
#[path = "payload_capture_tests.rs"]
mod capture_tests;
#[cfg(test)]
#[path = "payload_read_tests.rs"]
mod read_tests;
#[cfg(test)]
#[path = "payload_tests.rs"]
mod tests;
