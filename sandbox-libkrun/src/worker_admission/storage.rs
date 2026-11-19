// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Descriptor-relative coordination storage. Never locks a data directory.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    ffi::CString,
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{FileExt, MetadataExt},
        },
    },
    path::{Component, Path, PathBuf},
};

pub(super) const MAX_BYTES: usize = 4 * 1024 * 1024;
pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
pub(super) fn busy() -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, "admission interval is busy")
}
pub(super) fn limit() -> io::Error {
    io::Error::other("admission metadata limit reached; explicit recovery required")
}
pub(super) fn uid() -> u32 {
    unsafe { libc::geteuid() }
}
pub(super) fn name(value: &std::ffi::OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| invalid("NUL in path"))
}
pub(super) fn syscall(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
pub(super) fn open_at(
    parent: &File,
    component: &std::ffi::OsStr,
    flags: i32,
    mode: u32,
) -> io::Result<File> {
    let component = name(component)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            component.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode as libc::mode_t,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub(super) fn mkdir_at(parent: &File, component: &std::ffi::OsStr, mode: u32) -> io::Result<()> {
    let component = name(component)?;
    syscall(unsafe { libc::mkdirat(parent.as_raw_fd(), component.as_ptr(), mode as libc::mode_t) })
}
pub(super) fn unlink_at(parent: &File, component: &std::ffi::OsStr) -> io::Result<()> {
    let component = name(component)?;
    syscall(unsafe { libc::unlinkat(parent.as_raw_fd(), component.as_ptr(), 0) })
}
pub(super) fn rename_at(
    from: &File,
    source: &std::ffi::OsStr,
    to: &File,
    target: &std::ffi::OsStr,
) -> io::Result<()> {
    let (source, target) = (name(source)?, name(target)?);
    syscall(unsafe {
        libc::renameat(
            from.as_raw_fd(),
            source.as_ptr(),
            to.as_raw_fd(),
            target.as_ptr(),
        )
    })
}
pub(super) fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.as_os_str().is_empty() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(invalid("empty or traversing path"));
    }
    let input = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut output = PathBuf::from("/");
    for component in input.components() {
        match component {
            Component::RootDir | Component::CurDir => (),
            Component::Normal(n) => output.push(n),
            _ => return Err(invalid("invalid path component")),
        }
    }
    if output.as_os_str().as_bytes().len() > 4096 || output.components().count() > 128 {
        return Err(limit());
    }
    Ok(output)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub(super) struct HostPath(String);
impl HostPath {
    pub(super) fn new(path: &Path) -> io::Result<Self> {
        Ok(Self(hex::encode(absolute(path)?.as_os_str().as_bytes())))
    }
    pub(super) fn get(&self) -> io::Result<PathBuf> {
        if self.0.len() > 8192 {
            return Err(limit());
        }
        let bytes = hex::decode(&self.0).map_err(|_| invalid("invalid path encoding"))?;
        let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
        if !path.is_absolute() || absolute(&path)? != path || Self::new(&path)? != *self {
            return Err(invalid("noncanonical stored path"));
        }
        Ok(path)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub dev: u64,
    pub ino: u64,
    pub uid: u32,
    pub mode: u32,
    pub links: u64,
}
impl Identity {
    pub(super) fn of(file: &File) -> io::Result<Self> {
        Ok(Self::metadata(&file.metadata()?))
    }
    pub(super) fn metadata(m: &std::fs::Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            uid: m.uid(),
            mode: m.mode(),
            links: m.nlink(),
        }
    }
    pub(super) fn same_inode(&self, other: &Self) -> bool {
        self.dev == other.dev
            && self.ino == other.ino
            && self.uid == other.uid
            && self.mode == other.mode
    }
    pub(super) fn regular(&self, expected_uid: u32) -> io::Result<()> {
        if self.mode & libc::S_IFMT != libc::S_IFREG
            || self.mode & 0o7777 != 0o600
            || self.uid != expected_uid
            || self.links != 1
        {
            return Err(invalid("unsafe coordination file"));
        }
        Ok(())
    }
}
pub(super) struct Directory {
    pub path: PathBuf,
    pub file: File,
    chain: Vec<(PathBuf, Identity)>,
}
impl Directory {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let path = absolute(path)?;
        let root = CString::new("/").map_err(|_| invalid("root"))?;
        let fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let mut here = PathBuf::from("/");
        let mut chain = vec![(here.clone(), Identity::of(&file)?)];
        for component in path.components() {
            if let Component::Normal(n) = component {
                file = open_at(&file, n, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
                here.push(n);
                chain.push((here.clone(), Identity::of(&file)?));
            }
        }
        Ok(Self { path, file, chain })
    }
    pub(super) fn revalidate(&self) -> io::Result<()> {
        let now = Self::open(&self.path)?;
        if self.chain.len() != now.chain.len()
            || self
                .chain
                .iter()
                .zip(&now.chain)
                .any(|((p, a), (q, b))| p != q || !a.same_inode(b))
        {
            return Err(invalid("directory namespace replaced"));
        }
        Ok(())
    }
    pub(super) fn administrative(&self) -> io::Result<()> {
        if self
            .chain
            .iter()
            .any(|(_, id)| id.uid != 0 || id.mode & 0o022 != 0)
        {
            return Err(invalid(
                "namespace ancestor is not administrative-owned and protected",
            ));
        }
        Ok(())
    }
    pub(super) fn private(&self, expected_uid: u32) -> io::Result<()> {
        let id = Identity::of(&self.file)?;
        if id.uid != expected_uid || id.mode & 0o7777 != 0o700 {
            return Err(invalid(
                "namespace must be private0700 and owned by expected uid",
            ));
        }
        Ok(())
    }
    pub(super) fn file(&self, n: &str, create: bool) -> io::Result<File> {
        open_at(
            &self.file,
            n.as_ref(),
            libc::O_RDWR
                | libc::O_NONBLOCK
                | if create {
                    libc::O_CREAT | libc::O_EXCL
                } else {
                    0
                },
            0o600,
        )
    }
    pub(super) fn names(&self) -> io::Result<Vec<String>> {
        self.revalidate()?;
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.path)? {
            if names.len() >= 2048 {
                return Err(limit());
            }
            names.push(
                entry?
                    .file_name()
                    .into_string()
                    .map_err(|_| invalid("unknown namespace entry"))?,
            );
        }
        self.revalidate()?;
        Ok(names)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    sha256: String,
    payload: String,
}
pub(super) fn read_json<T: DeserializeOwned>(file: &File) -> io::Result<T> {
    use sha2::{Digest, Sha256};
    let length = file.metadata()?.len();
    if length > MAX_BYTES as u64 {
        return Err(limit());
    }
    let mut bytes = vec![0; length as usize];
    file.read_exact_at(&mut bytes, 0)?;
    let envelope: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| invalid("corrupt record envelope"))?;
    if envelope.version != 1
        || hex::encode(Sha256::digest(envelope.payload.as_bytes())) != envelope.sha256
    {
        return Err(invalid("record integrity mismatch"));
    }
    serde_json::from_str(&envelope.payload)
        .map_err(|_| invalid("invalid versioned admission record"))
}
pub(super) fn write_json<T: Serialize>(file: &File, value: &T) -> io::Result<()> {
    use sha2::{Digest, Sha256};
    let payload = serde_json::to_string(value).map_err(io::Error::other)?;
    if payload.len() > MAX_BYTES / 2 {
        return Err(limit());
    }
    let envelope = Envelope {
        version: 1,
        sha256: hex::encode(Sha256::digest(payload.as_bytes())),
        payload,
    };
    let bytes = serde_json::to_vec(&envelope).map_err(io::Error::other)?;
    if bytes.len() > MAX_BYTES {
        return Err(limit());
    }
    file.write_all_at(&bytes, 0)?;
    file.set_len(bytes.len() as u64)?;
    file.sync_all()
}
pub(super) fn flock(file: &File) -> io::Result<()> {
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
// OFD byte lock supplements flock so imports can verify ownership without
// acquiring a new lock or ever issuing LOCK_UN on an inherited description.
#[cfg(target_os = "linux")]
pub(super) fn ofd(file: &File, query: bool) -> io::Result<bool> {
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    lock.l_start = 0;
    lock.l_len = 1;
    syscall(unsafe {
        libc::fcntl(
            file.as_raw_fd(),
            if query {
                libc::F_OFD_GETLK
            } else {
                libc::F_OFD_SETLK
            },
            &mut lock,
        )
    })?;
    Ok(lock.l_type == libc::F_UNLCK as _)
}
#[cfg(not(target_os = "linux"))]
pub(super) fn ofd(_file: &File, _query: bool) -> io::Result<bool> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "admission v2 requires Linux OFD lock verification",
    ))
}
pub(super) fn sentinel(file: &File) -> io::Result<()> {
    flock(file)?;
    ofd(file, false)?;
    Ok(())
}
pub(super) fn duplicate(file: &File) -> io::Result<OwnedFd> {
    let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}
pub(super) fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
pub(super) fn id_valid(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn mount_id(file: &File) -> io::Result<u64> {
    #[cfg(target_os = "linux")]
    {
        let empty = CString::new("").map_err(|_| invalid("empty statx name"))?;
        let mut stat: libc::statx = unsafe { std::mem::zeroed() };
        syscall(unsafe {
            libc::statx(
                file.as_raw_fd(),
                empty.as_ptr(),
                libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
                libc::STATX_MNT_ID,
                &mut stat,
            )
        })?;
        if stat.stx_mask & libc::STATX_MNT_ID == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "mount identity unavailable",
            ));
        }
        Ok(stat.stx_mnt_id)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = file;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "mount identity requires statx",
        ))
    }
}
