// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
use super::{Error, Faults, MAX_ENTRIES, MAX_XATTR_BYTES, Result, Step, fs};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, RawFd},
        unix::fs::MetadataExt,
    },
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
enum Kind {
    Directory,
    File,
    Symlink(Vec<u8>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Meta {
    uid: u32,
    gid: u32,
    mode: u32,
    atime: i64,
    atime_ns: i64,
    mtime: i64,
    mtime_ns: i64,
    size: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Entry {
    path: Vec<u8>,
    kind: Kind,
    meta: Meta,
    xattrs: Vec<(Vec<u8>, Vec<u8>)>,
    content: Option<String>,
    hardlink: Option<Vec<u8>>,
}
struct Node {
    entry: Entry,
    _parent: Arc<File>,
    file: Option<Arc<File>>,
    name: CString,
    identity: (u64, u64, u64, i64, i64),
}
pub(super) struct Inventory {
    nodes: Vec<Node>,
    digest: String,
}
fn meta(m: &Metadata) -> Meta {
    Meta {
        uid: m.uid(),
        gid: m.gid(),
        mode: m.mode() & 0o7777,
        atime: m.atime(),
        atime_ns: m.atime_nsec(),
        mtime: m.mtime(),
        mtime_ns: m.mtime_nsec(),
        size: if m.is_file() { m.len() } else { 0 },
    }
}
fn identity(m: &Metadata) -> (u64, u64, u64, i64, i64) {
    (m.dev(), m.ino(), m.nlink(), m.ctime(), m.ctime_nsec())
}
fn stat(parent: RawFd, name: &CString) -> Result<libc::stat> {
    let mut st = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatat(parent, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(st)
}
fn link_path(parent: RawFd, name: &CString) -> Result<CString> {
    let mut bytes = format!("/proc/self/fd/{parent}/").into_bytes();
    bytes.extend_from_slice(name.to_bytes());
    CString::new(bytes).map_err(|_| Error::Custody)
}
fn xattrs(fd: Option<RawFd>, parent: RawFd, name: &CString) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
    let path = link_path(parent, name)?;
    let list = |p: *mut libc::c_char, n: usize| unsafe {
        match fd {
            Some(fd) => libc::flistxattr(fd, p, n),
            None => libc::llistxattr(path.as_ptr(), p, n),
        }
    };
    let size = list(std::ptr::null_mut(), 0);
    if size < 0 {
        return Err(Error::Unsupported("xattr enumeration unavailable"));
    }
    if size as usize > MAX_XATTR_BYTES {
        return Err(Error::Unsupported("xattr list limit"));
    }
    let mut bytes = vec![0u8; size as usize];
    let got = list(bytes.as_mut_ptr().cast(), bytes.len());
    if got != size {
        return Err(Error::Custody);
    }
    let mut result = BTreeMap::new();
    let mut total = 0;
    for name in bytes.split(|b| *b == 0).filter(|v| !v.is_empty()) {
        let key = CString::new(name).map_err(|_| Error::Custody)?;
        let get = |p: *mut libc::c_void, n: usize| unsafe {
            match fd {
                Some(fd) => libc::fgetxattr(fd, key.as_ptr(), p, n),
                None => libc::lgetxattr(path.as_ptr(), key.as_ptr(), p, n),
            }
        };
        let size = get(std::ptr::null_mut(), 0);
        if size < 0 {
            return Err(Error::Custody);
        }
        total += size as usize;
        if total > MAX_XATTR_BYTES {
            return Err(Error::Unsupported("xattr value limit"));
        }
        let mut value = vec![0; size as usize];
        if get(value.as_mut_ptr().cast(), value.len()) != size {
            return Err(Error::Custody);
        }
        result.insert(name.to_vec(), value);
    }
    Ok(result)
}
fn hash(file: &File) -> Result<String> {
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}
fn flags(file: &File) -> Result<()> {
    let mut value: libc::c_long = 0;
    // FS_IOC_GETFLAGS: extents/index describe implementation, not application
    // policy. Immutable/append/encrypted/project-inherit/etc are NOT flattened.
    if unsafe { libc::ioctl(file.as_raw_fd(), 0x80086601u64 as libc::c_ulong, &mut value) } != 0 {
        return Err(Error::Unsupported("inode flags unavailable"));
    }
    if value & !(0x80000 | 0x1000) != 0 {
        return Err(Error::Unsupported("inode policy flags"));
    }
    Ok(())
}
impl Inventory {
    pub(super) fn scan(root: &fs::Dir) -> Result<Self> {
        let root_file = Arc::new(fs::file(unsafe {
            libc::openat(
                root.fd(),
                c".".as_ptr(),
                libc::O_RDONLY
                    | libc::O_DIRECTORY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NOATIME,
            )
        })?);
        let device = root_file.metadata()?.dev();
        let mut nodes = vec![];
        let mut links = BTreeMap::<(u64, u64), (Vec<u8>, u64, u64)>::new();
        visit(
            Arc::clone(&root_file),
            CString::new(".").unwrap(),
            vec![],
            Some(root_file),
            device,
            0,
            &mut nodes,
            &mut links,
        )?;
        // Detect concurrent changes during inventory itself, not only a later
        // copy. The coordinator still must prove quiescence; this is detection,
        // never a substitute for fencing an uncooperative writer.
        for node in &nodes {
            let st = stat(node._parent.as_raw_fd(), &node.name)?;
            if (
                st.st_dev,
                st.st_ino,
                st.st_nlink,
                st.st_ctime,
                st.st_ctime_nsec,
            ) != node.identity
            {
                return Err(Error::Custody);
            }
        }
        for (_, (_, expected, seen)) in links {
            if expected != seen {
                return Err(Error::Unsupported("hardlink crosses capture boundary"));
            }
        }
        let encoded = serde_json::to_vec(&nodes.iter().map(|n| &n.entry).collect::<Vec<_>>())
            .map_err(|_| Error::Corrupt)?;
        Ok(Self {
            nodes,
            digest: fs::digest(&encoded),
        })
    }
    pub(super) fn digest(&self) -> &str {
        &self.digest
    }
    pub(super) fn unchanged(&self, root: &fs::Dir) -> Result<()> {
        let other = Self::scan(root)?;
        if self.digest != other.digest
            || self.nodes.len() != other.nodes.len()
            || self
                .nodes
                .iter()
                .zip(&other.nodes)
                .any(|(a, b)| a.identity != b.identity)
        {
            return Err(Error::Custody);
        }
        Ok(())
    }
    pub(super) fn file_digest(&self, path: &[u8]) -> Option<&str> {
        self.nodes
            .iter()
            .find(|n| n.entry.path == path)
            .and_then(|n| n.entry.content.as_deref())
    }
    pub(super) fn is_directory(&self, path: &[u8]) -> bool {
        self.nodes
            .iter()
            .any(|n| n.entry.path == path && matches!(n.entry.kind, Kind::Directory))
    }
    pub(super) fn top_names(&self) -> Vec<Vec<u8>> {
        self.nodes
            .iter()
            .filter(|n| !n.entry.path.is_empty() && !n.entry.path.contains(&b'/'))
            .map(|n| n.entry.path.clone())
            .collect()
    }
    pub(super) fn check_destination(&self, dest: &fs::Dir) -> Result<()> {
        if self
            .nodes
            .iter()
            .any(|n| matches!(n.entry.kind, Kind::Symlink(_)))
        {
            let mut vfs = unsafe { std::mem::zeroed::<libc::statvfs>() };
            if unsafe { libc::fstatvfs(dest.fd(), &mut vfs) } != 0
                || vfs.f_flag & libc::ST_NOATIME == 0
            {
                return Err(Error::Unsupported(
                    "symlink fidelity requires noatime destination",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn copy(&self, source: &fs::Dir, dest: &fs::Dir, fault: &mut Faults) -> Result<()> {
        self.unchanged(source)?;
        self.check_destination(dest)?;
        let mut parents = BTreeMap::<Vec<u8>, Arc<File>>::new();
        parents.insert(
            vec![],
            Arc::new(fs::file(unsafe {
                libc::openat(
                    dest.fd(),
                    c".".as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            })?),
        );
        for node in self.nodes.iter().skip(1) {
            let split = node.entry.path.iter().rposition(|b| *b == b'/');
            let parent_path = split.map_or(&b""[..], |n| &node.entry.path[..n]);
            let parent = parents.get(parent_path).ok_or(Error::Corrupt)?;
            let fd = parent.as_raw_fd();
            let name = &node.name;
            fault.at(Step::Copy)?;
            match &node.entry.kind {
                Kind::Directory => {
                    if unsafe { libc::mkdirat(fd, name.as_ptr(), 0o700) } != 0 {
                        return Err(std::io::Error::last_os_error().into());
                    }
                    let file = fs::file(unsafe {
                        libc::openat(
                            fd,
                            name.as_ptr(),
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        )
                    })?;
                    parents.insert(node.entry.path.clone(), Arc::new(file));
                }
                Kind::File => {
                    if let Some(first) = &node.entry.hardlink {
                        let first = CString::new(first.clone()).map_err(|_| Error::Corrupt)?;
                        if unsafe { libc::linkat(dest.fd(), first.as_ptr(), fd, name.as_ptr(), 0) }
                            != 0
                        {
                            return Err(std::io::Error::last_os_error().into());
                        }
                    } else {
                        let mut out = fs::file(unsafe {
                            libc::openat(
                                fd,
                                name.as_ptr(),
                                libc::O_WRONLY
                                    | libc::O_CREAT
                                    | libc::O_EXCL
                                    | libc::O_NOFOLLOW
                                    | libc::O_CLOEXEC,
                                0o600,
                            )
                        })?;
                        sparse_copy(node.file.as_ref().ok_or(Error::Corrupt)?, &mut out, fault)?;
                        metadata(node, Some(&out), fd, fault)?;
                        fault.at(Step::FileSync)?;
                        out.sync_all()?;
                    }
                }
                Kind::Symlink(target) => {
                    let target = CString::new(target.clone()).map_err(|_| Error::Corrupt)?;
                    if unsafe { libc::symlinkat(target.as_ptr(), fd, name.as_ptr()) } != 0 {
                        return Err(std::io::Error::last_os_error().into());
                    }
                    metadata(node, None, fd, fault)?;
                }
            }
        }
        // linkat changed the first inode's link count after its first fsync.
        // Flush every linked inode once more before directory publication.
        for node in self.nodes.iter().filter(|n| n.entry.hardlink.is_some()) {
            let path = CString::new(node.entry.path.clone()).map_err(|_| Error::Corrupt)?;
            let file = fs::file(unsafe {
                libc::openat(
                    dest.fd(),
                    path.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NOATIME,
                )
            })?;
            fault.at(Step::FileSync)?;
            file.sync_all()?;
        }
        // Directory times/ACLs are restored after children. No fsync unsupported
        // warning fallback: failure retains incomplete staging and original.
        for node in self
            .nodes
            .iter()
            .rev()
            .filter(|n| matches!(n.entry.kind, Kind::Directory))
        {
            let file = parents.get(&node.entry.path).ok_or(Error::Corrupt)?;
            metadata(node, Some(file), dest.fd(), fault)?;
            fault.at(Step::DirectorySync)?;
            file.sync_all()?;
        }
        self.unchanged(source)?;
        let copied = Self::scan(dest)?;
        if copied.digest != self.digest {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
}
fn visit(
    parent: Arc<File>,
    name: CString,
    path: Vec<u8>,
    preopened: Option<Arc<File>>,
    device: u64,
    depth: usize,
    nodes: &mut Vec<Node>,
    links: &mut BTreeMap<(u64, u64), (Vec<u8>, u64, u64)>,
) -> Result<()> {
    if depth > 128 || nodes.len() >= MAX_ENTRIES || path.len() > 4096 {
        return Err(Error::Unsupported("capture tree limit"));
    }
    let st = stat(parent.as_raw_fd(), &name)?;
    if st.st_dev != device {
        return Err(Error::Unsupported("mount crosses tree boundary"));
    }
    let typ = st.st_mode & libc::S_IFMT;
    if ![libc::S_IFDIR, libc::S_IFREG, libc::S_IFLNK].contains(&typ) {
        return Err(Error::Unsupported("special file/device/socket/FIFO"));
    }
    let file = match preopened {
        Some(f) => Some(f),
        None if typ != libc::S_IFLNK => Some(Arc::new(fs::file(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NOATIME
                    | if typ == libc::S_IFDIR {
                        libc::O_DIRECTORY
                    } else {
                        0
                    },
            )
        })?)),
        _ => None,
    };
    if let Some(opened) = &file {
        if fs::mount_id(opened.as_raw_fd())? != fs::mount_id(parent.as_raw_fd())? {
            return Err(Error::Unsupported("nested mount or bind alias"));
        }
    }
    let (m, ident) = if let Some(f) = &file {
        let m = f.metadata()?;
        if m.dev() != st.st_dev || m.ino() != st.st_ino {
            return Err(Error::Custody);
        }
        flags(f)?;
        (meta(&m), identity(&m))
    } else {
        (
            Meta {
                uid: st.st_uid,
                gid: st.st_gid,
                mode: st.st_mode & 0o7777,
                atime: st.st_atime,
                atime_ns: st.st_atime_nsec,
                mtime: st.st_mtime,
                mtime_ns: st.st_mtime_nsec,
                size: 0,
            },
            (
                st.st_dev,
                st.st_ino,
                st.st_nlink,
                st.st_ctime,
                st.st_ctime_nsec,
            ),
        )
    };
    if fs::uid() != 0 && (m.uid != fs::uid() || m.gid != unsafe { libc::getegid() }) {
        return Err(Error::Unsupported("numeric ownership requires privileges"));
    }
    if typ == libc::S_IFLNK {
        if st.st_nlink != 1 {
            return Err(Error::Unsupported("hardlinked symlink"));
        }
        // readlinkat may update symlink atime even through O_PATH. Never mutate
        // a source snapshot's metadata to restore it: require noatime custody.
        let mut vfs = unsafe { std::mem::zeroed::<libc::statvfs>() };
        if unsafe { libc::fstatvfs(parent.as_raw_fd(), &mut vfs) } != 0
            || vfs.f_flag & libc::ST_NOATIME == 0
        {
            return Err(Error::Unsupported(
                "symlink capture requires noatime source filesystem",
            ));
        }
    }
    let kind = if typ == libc::S_IFDIR {
        Kind::Directory
    } else if typ == libc::S_IFREG {
        Kind::File
    } else {
        let mut buf = vec![0u8; 4097];
        let n = unsafe {
            libc::readlinkat(
                parent.as_raw_fd(),
                name.as_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if n as usize >= buf.len() {
            return Err(Error::Unsupported("symlink target limit"));
        }
        buf.truncate(n as usize);
        Kind::Symlink(buf)
    };
    let hardlink = if typ == libc::S_IFREG {
        let group = links
            .entry((st.st_dev, st.st_ino))
            .or_insert((path.clone(), st.st_nlink, 0));
        group.2 += 1;
        if group.0 == path {
            None
        } else {
            Some(group.0.clone())
        }
    } else {
        None
    };
    let attrs = xattrs(
        file.as_ref().map(|f| f.as_raw_fd()),
        parent.as_raw_fd(),
        &name,
    )?;
    let content = if typ == libc::S_IFREG {
        let source = file.as_ref().unwrap();
        if m.size > 0 {
            let offset = unsafe { libc::lseek(source.as_raw_fd(), 0, libc::SEEK_DATA) };
            if offset < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ENXIO) {
                return Err(Error::Unsupported("SEEK_DATA unavailable"));
            }
            if offset >= 0
                && unsafe { libc::lseek(source.as_raw_fd(), offset, libc::SEEK_HOLE) } < 0
            {
                return Err(Error::Unsupported("SEEK_HOLE unavailable"));
            }
        }
        Some(hash(source)?)
    } else {
        None
    };
    let entry = Entry {
        path: path.clone(),
        kind,
        meta: m,
        xattrs: attrs.into_iter().collect(),
        content,
        hardlink,
    };
    nodes.push(Node {
        entry,
        _parent: Arc::clone(&parent),
        file: file.clone(),
        name: name.clone(),
        identity: ident,
    });
    if typ == libc::S_IFDIR {
        let dir = file.unwrap();
        for name in fs::names(dir.as_raw_fd())? {
            let mut child = path.clone();
            if !child.is_empty() {
                child.push(b'/');
            }
            child.extend_from_slice(&name);
            visit(
                Arc::clone(&dir),
                fs::name(&name)?,
                child,
                None,
                device,
                depth + 1,
                nodes,
                links,
            )?;
        }
    }
    Ok(())
}
fn sparse_copy(source: &File, out: &mut File, fault: &mut Faults) -> Result<()> {
    let size = source.metadata()?.len();
    out.set_len(size)?;
    let mut at = 0;
    while at < size {
        let data = unsafe { libc::lseek(source.as_raw_fd(), at as i64, libc::SEEK_DATA) };
        if data < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ENXIO) {
                break;
            }
            return Err(Error::Unsupported("SEEK_DATA unavailable"));
        }
        if data as u64 >= size {
            return Err(Error::Custody);
        }
        let hole = unsafe { libc::lseek(source.as_raw_fd(), data, libc::SEEK_HOLE) };
        if hole <= data {
            return Err(Error::Unsupported("SEEK_HOLE unavailable"));
        }
        let mut input = source.try_clone()?;
        input.seek(SeekFrom::Start(data as u64))?;
        out.seek(SeekFrom::Start(data as u64))?;
        let mut left = (hole as u64).min(size) - data as u64;
        let mut buf = [0u8; 65536];
        while left > 0 {
            fault.at(Step::Copy)?;
            let limit = (left as usize).min(buf.len());
            let n = input.read(&mut buf[..limit])?;
            if n == 0 {
                return Err(Error::Custody);
            }
            out.write_all(&buf[..n])?;
            left -= n as u64;
        }
        at = hole as u64;
    }
    Ok(())
}
fn metadata(node: &Node, file: Option<&File>, parent: RawFd, fault: &mut Faults) -> Result<()> {
    fault.at(Step::Metadata)?;
    let m = &node.entry.meta;
    let rc = if let Some(file) = file {
        unsafe { libc::fchown(file.as_raw_fd(), m.uid, m.gid) }
    } else {
        unsafe {
            libc::fchownat(
                parent,
                node.name.as_ptr(),
                m.uid,
                m.gid,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        }
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if let Some(file) = file {
        if unsafe { libc::fchmod(file.as_raw_fd(), m.mode) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    // Destinations start empty. Preserve every xattr (ACLs/capabilities included)
    // or fail, never silently drop security metadata.
    let current = xattrs(file.map(AsRawFd::as_raw_fd), parent, &node.name)?;
    let path = link_path(parent, &node.name)?;
    for key in current
        .keys()
        .filter(|k| !node.entry.xattrs.iter().any(|(key, _)| key == *k))
    {
        let key = CString::new(key.clone()).map_err(|_| Error::Corrupt)?;
        let rc = unsafe {
            match file {
                Some(f) => libc::fremovexattr(f.as_raw_fd(), key.as_ptr()),
                None => libc::lremovexattr(path.as_ptr(), key.as_ptr()),
            }
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    for (key, value) in &node.entry.xattrs {
        let key = CString::new(key.clone()).map_err(|_| Error::Corrupt)?;
        let rc = unsafe {
            match file {
                Some(f) => libc::fsetxattr(
                    f.as_raw_fd(),
                    key.as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                ),
                None => libc::lsetxattr(
                    path.as_ptr(),
                    key.as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                ),
            }
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    let times = [
        libc::timespec {
            tv_sec: m.atime,
            tv_nsec: m.atime_ns,
        },
        libc::timespec {
            tv_sec: m.mtime,
            tv_nsec: m.mtime_ns,
        },
    ];
    let rc = unsafe {
        match file {
            Some(f) => libc::futimens(f.as_raw_fd(), times.as_ptr()),
            None => libc::utimensat(
                parent,
                node.name.as_ptr(),
                times.as_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            ),
        }
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
