//! Host-filesystem backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::{Channel, IIIError};

use crate::fs::error::FsError;

/// RAII guard that unlinks a temp path on drop unless `commit()` is called.
/// Sync `remove_file` in `Drop` is deliberate — best-effort cleanup on
/// panic/early-return without awaiting.
struct TempGuard {
    path: Option<PathBuf>,
}
impl TempGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }
    fn commit(mut self) {
        self.path = None;
    }
    fn path(&self) -> &std::path::Path {
        self.path.as_deref().expect("path always Some until commit")
    }
}
impl Drop for TempGuard {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[async_trait]
pub trait ChannelMaker: Send + Sync + std::fmt::Debug {
    async fn create_channel(&self, buffer: usize) -> Result<Channel, IIIError>;
    fn engine_address(&self) -> String;
}

pub struct IiiChannelMaker {
    iii: iii_sdk::III,
}

impl std::fmt::Debug for IiiChannelMaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IiiChannelMaker")
            .field("address", &self.iii.address())
            .finish()
    }
}

impl IiiChannelMaker {
    pub fn new(iii: iii_sdk::III) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl ChannelMaker for IiiChannelMaker {
    async fn create_channel(&self, buffer: usize) -> Result<Channel, IIIError> {
        self.iii.create_channel(Some(buffer)).await
    }
    fn engine_address(&self) -> String {
        self.iii.address().to_string()
    }
}

#[derive(Debug, Clone)]
pub struct HostFsConfig {
    pub host_root: Option<PathBuf>,
    pub max_read_bytes: usize,
    pub max_write_bytes: usize,
    pub denylist_paths: Vec<PathBuf>,
}

impl Default for HostFsConfig {
    fn default() -> Self {
        Self {
            host_root: None,
            max_read_bytes: 0,
            max_write_bytes: 0,
            denylist_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostFsBackend {
    cfg: Arc<HostFsConfig>,
    chan: Arc<dyn ChannelMaker>,
    /// Canonical form of `cfg.host_root`, computed once at construction.
    /// Pre-fix `validate_path` recanonicalized this on every fs op (and
    /// every denylist entry) — a real perf hit on hot paths and an
    /// operator-config-error vector that surfaced silently. Caching here
    /// also fails loudly at startup if `host_root` is unreachable.
    host_root_canon: Option<PathBuf>,
    /// Canonical form of each `cfg.denylist_paths` entry. Same rationale
    /// as `host_root_canon`; an entry that can't canonicalize is a config
    /// error and the worker refuses to start.
    denylist_canon: Vec<PathBuf>,
}

impl HostFsBackend {
    pub fn new(cfg: Arc<HostFsConfig>, chan: Arc<dyn ChannelMaker>) -> Self {
        match Self::try_new(cfg.clone(), chan.clone()) {
            Ok(b) => b,
            Err(e) => panic!(
                "HostFsBackend init failed: code={} message={}. Either fix \
                 the config or call try_new directly to handle the error.",
                e.code, e.message
            ),
        }
    }

    /// Resolve `host_root` and every `denylist_paths` entry to canonical form
    /// once at startup. Errors here are operator config bugs (path doesn't
    /// exist, can't be canonicalized, etc.) and the worker should refuse to
    /// start instead of degrading to lexical fallback per-call.
    pub fn try_new(cfg: Arc<HostFsConfig>, chan: Arc<dyn ChannelMaker>) -> Result<Self, FsError> {
        let host_root_canon = match &cfg.host_root {
            Some(root) => Some(std::fs::canonicalize(root).map_err(|e| {
                FsError::new(
                    "S216",
                    format!("host_root unreachable ({}): {e}", root.display()),
                )
            })?),
            None => None,
        };
        let mut denylist_canon = Vec::with_capacity(cfg.denylist_paths.len());
        for deny in &cfg.denylist_paths {
            let canon = canonicalize_with_fallback(Path::new(deny)).map_err(|e| {
                FsError::new(
                    "S216",
                    format!("denylist entry unreachable ({}): {e}", deny.display()),
                )
            })?;
            denylist_canon.push(canon);
        }
        Ok(Self {
            cfg,
            chan,
            host_root_canon,
            denylist_canon,
        })
    }

    /// TOCTOU: check-then-use gate, open to a race against an attacker who
    /// can mutate the filesystem between validation and subsequent syscalls.
    /// Use the sandbox backend for untrusted input.
    pub(crate) fn validate_path(&self, path: &str) -> Result<PathBuf, FsError> {
        let p = Path::new(path);
        if !p.is_absolute() {
            return Err(FsError::new(
                "S210",
                format!("path must be absolute: {path}"),
            ));
        }
        let canon = canonicalize_with_fallback(p).map_err(|e| {
            // Dangling-symlink errors are structurally jail violations
            // (the path would otherwise resolve through a link that
            // pre-fix slipped past the lexical fallback). Map them to
            // S215 so wire telemetry treats them as such.
            let msg = format!("{e}");
            if msg.contains("dangling symlink in path") {
                FsError::new("S215", format!("{path}: {msg}"))
            } else {
                FsError::new("S210", format!("{path}: {msg}"))
            }
        })?;
        if let Some(root_canon) = &self.host_root_canon {
            if !canon.starts_with(root_canon) {
                return Err(FsError::new(
                    "S215",
                    format!("path escapes host_root: {path}"),
                ));
            }
        }
        for deny_canon in &self.denylist_canon {
            if canon.starts_with(deny_canon) {
                return Err(FsError::new("S215", format!("path is denylisted: {path}")));
            }
        }
        Ok(canon)
    }
}

/// Resolve `p` to a canonical path that is symlink-free for every existing
/// ancestor, even when `p` itself doesn't yet exist. The naive fallback —
/// "canonicalize, on ENOENT use the lexical path" — is a jail-escape vector
/// when the path traverses a symlink whose target is outside the jail: the
/// lexical form still `starts_with(host_root)`, but the kernel will follow
/// the link on the subsequent syscall. Walking up to the longest existing
/// ancestor and canonicalizing *that* forces every symlink in the existing
/// portion to be resolved; the non-existent tail can't itself contain
/// symlinks (it doesn't exist) but can still contain `..`/`.`, which we
/// then collapse lexically against the canonical prefix so the
/// `starts_with(host_root)` check is sound.
fn canonicalize_with_fallback(p: &Path) -> std::io::Result<PathBuf> {
    if let Ok(c) = std::fs::canonicalize(p) {
        return Ok(c);
    }
    // Walk ancestors top-down (longest prefix first) until canonicalize
    // succeeds. ancestors() yields p first; skip it because we already
    // tried it. After we find the longest canonicalizable ancestor, walk
    // forward through each tail component and reject if any of them is a
    // *dangling* symlink: canonicalize fails on dangling symlinks (target
    // doesn't exist), so they'd otherwise survive into the lexical tail
    // and let `starts_with(host_root)` succeed against a path the kernel
    // would resolve outside the jail. Existing-but-resolvable symlinks
    // are caught by the top-of-function canonicalize.
    for anc in p.ancestors().skip(1) {
        if let Ok(canon_anc) = std::fs::canonicalize(anc) {
            let suffix = match p.strip_prefix(anc) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut walk = canon_anc;
            for component in suffix.components() {
                walk.push(component);
                if let Ok(md) = std::fs::symlink_metadata(&walk) {
                    if md.file_type().is_symlink() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!("dangling symlink in path: {}", walk.display()),
                        ));
                    }
                }
            }
            return Ok(normalize_lexical(&walk));
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "no existing ancestor to canonicalize",
    ))
}

fn normalize_lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Mirrors `iii-init/fs_handler/ops.rs::temp_sibling`.
fn temp_sibling(target: &Path) -> PathBuf {
    let mut t = target.as_os_str().to_os_string();
    t.push(".iii-tmp-");
    t.push(uuid::Uuid::new_v4().to_string());
    t.into()
}

/// Mirrors `iii-init/fs_handler/ops.rs::glob_matches_path`.
fn glob_matches_path(pattern: &str, relpath: &str) -> bool {
    if pattern.contains('/') {
        glob_match(pattern, relpath)
    } else {
        let base = relpath.rsplit('/').next().unwrap_or(relpath);
        glob_match(pattern, base)
    }
}

fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern == "**" {
        return true;
    }
    if let Some(rest) = pattern.strip_prefix("**/") {
        let base = path.rsplit('/').next().unwrap_or(path);
        if glob_match_simple(rest, base) {
            return true;
        }
        return glob_match_simple(rest, path);
    }
    glob_match_simple(pattern, path)
}

fn glob_match_simple(pattern: &str, path: &str) -> bool {
    let p = pattern.as_bytes();
    let t = path.as_bytes();
    let mut pi = 0usize;
    let mut ti = 0usize;
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < t.len() {
        let pc = p.get(pi).copied();
        let tc = t[ti];
        match pc {
            Some(b'*') => {
                star_pi = Some(pi);
                star_ti = ti;
                pi += 1;
            }
            Some(b'?') if tc != b'/' => {
                pi += 1;
                ti += 1;
            }
            Some(c) if c == tc => {
                pi += 1;
                ti += 1;
            }
            _ => {
                if let Some(sp) = star_pi {
                    if t[star_ti] == b'/' {
                        return false;
                    }
                    pi = sp + 1;
                    star_ti += 1;
                    ti = star_ti;
                } else {
                    return false;
                }
            }
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

fn looks_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 8192];
    let n = f.read(&mut buf).unwrap_or(0);
    buf[..n].contains(&0)
}

fn expand_regex_replacement(caps: &regex::Captures, template: &str) -> String {
    let mut out = String::new();
    caps.expand(template, &mut out);
    out
}

/// When `case_insensitive` is true, delegates to regex with `(?i)` and an
/// escaped pattern. Hand-rolling case-fold over UTF-8 is unsound (e.g.
/// `'İ'` U+0130 folds to a length-changing sequence).
fn literal_replace_line(
    line: &str,
    needle: &str,
    case_insensitive: bool,
    replacement: &str,
    first_only: bool,
) -> (String, u64) {
    if line.is_empty() || needle.is_empty() {
        return (line.to_string(), 0);
    }
    if case_insensitive {
        let escaped_needle = regex::escape(needle);
        let escaped_replacement = replacement.replace('$', "$$");
        let pattern = format!("(?i){escaped_needle}");
        let re = match regex::Regex::new(&pattern) {
            Ok(r) => r,
            Err(_) => return (line.to_string(), 0),
        };
        let mut count = 0u64;
        let out = if first_only {
            re.replacen(line, 1, |_caps: &regex::Captures| {
                count += 1;
                escaped_replacement.clone()
            })
            .into_owned()
        } else {
            re.replace_all(line, |_caps: &regex::Captures| {
                count += 1;
                escaped_replacement.clone()
            })
            .into_owned()
        };
        return (out, count);
    }
    let mut out = String::with_capacity(line.len());
    let mut n = 0u64;
    let mut i = 0usize;
    let bytes = line.as_bytes();
    let needle_bytes = needle.as_bytes();
    while i < line.len() {
        if i + needle_bytes.len() <= bytes.len()
            && &bytes[i..i + needle_bytes.len()] == needle_bytes
        {
            out.push_str(replacement);
            i += needle_bytes.len();
            n += 1;
            if first_only {
                out.push_str(&line[i..]);
                return (out, n);
            }
        } else {
            let next = line[i..]
                .char_indices()
                .nth(1)
                .map(|(b, _)| i + b)
                .unwrap_or(line.len());
            out.push_str(&line[i..next]);
            i = next;
        }
    }
    (out, n)
}

fn collect_files_to_sed(
    root: &Path,
    recursive: bool,
    include_glob: &[String],
    exclude_glob: &[String],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let lmd = match root.symlink_metadata() {
        Ok(m) => m,
        Err(_) => return out,
    };
    let passes = |rel: &str| -> bool {
        if !include_glob.is_empty() && !include_glob.iter().any(|g| glob_matches_path(g, rel)) {
            return false;
        }
        if exclude_glob.iter().any(|g| glob_matches_path(g, rel)) {
            return false;
        }
        true
    };
    let target_is_dir = if lmd.file_type().is_symlink() {
        match std::fs::metadata(root) {
            Ok(m) => m.is_dir(),
            Err(_) => return out,
        }
    } else {
        lmd.is_dir()
    };
    if !target_is_dir {
        let rel = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if passes(&rel) {
            out.push(root.to_path_buf());
        }
        return out;
    }
    let resolved_root: PathBuf = if lmd.file_type().is_symlink() {
        match std::fs::canonicalize(root) {
            Ok(p) => p,
            Err(_) => return out,
        }
    } else {
        root.to_path_buf()
    };
    let walker = walkdir::WalkDir::new(&resolved_root).follow_links(false);
    let walker = if recursive {
        walker
    } else {
        walker.max_depth(1)
    };
    for entry in walker
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let rel = entry
            .path()
            .strip_prefix(&resolved_root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .into_owned();
        if passes(&rel) {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

use crate::fs::wire::FsEntry;
use crate::fs::{FsBackend, FsCallResult, LsArgs, LsResponse, StatArgs, StatResponse};
use std::os::unix::fs::PermissionsExt;

/// Mode formatted as 4-octal-digit zero-padded, matching engine daemon's
/// `entry_from_metadata`.
fn fs_entry_from_metadata(name: String, md: &std::fs::Metadata) -> FsEntry {
    FsEntry {
        name,
        is_dir: md.is_dir(),
        size: md.len(),
        mode: format!("{:04o}", md.permissions().mode() & 0o7777),
        mtime: md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        is_symlink: md.file_type().is_symlink(),
    }
}

async fn pump_file_to_channel(
    path: &std::path::Path,
    writer: &iii_sdk::channels::ChannelWriter,
) -> Result<(), String> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("open failed: {e}"))?;
    let mut buf = vec![0u8; 64 * 1024];
    // Open the WS connection eagerly: for a zero-byte file the loop exits
    // without write(), leaving the sender never dropped and blocking the
    // reader side forever. This empty write forces the connect.
    writer
        .write(&[])
        .await
        .map_err(|e| format!("channel connect failed: {e}"))?;
    loop {
        let n = file
            .read(&mut buf)
            .await
            .map_err(|e| format!("file read failed: {e}"))?;
        if n == 0 {
            return Ok(());
        }
        writer
            .write(&buf[..n])
            .await
            .map_err(|e| format!("channel write failed: {e}"))?;
    }
}

#[async_trait]
impl FsBackend for HostFsBackend {
    async fn ls(&self, req: LsArgs) -> FsCallResult<LsResponse> {
        let p = self.validate_path(&req.path)?;
        let md = std::fs::symlink_metadata(&p).map_err(|e| FsError::from_io(&req.path, e))?;
        if !md.is_dir() {
            return Err(FsError::new(
                "S212",
                format!("not a directory: {}", req.path),
            ));
        }
        let rd = std::fs::read_dir(&p).map_err(|e| FsError::from_io(&req.path, e))?;
        let mut entries = Vec::new();
        for ent in rd {
            let ent = match ent {
                Ok(e) => e,
                Err(_) => continue,
            };
            let md = match ent.path().symlink_metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let name = ent.file_name().to_string_lossy().into_owned();
            entries.push(fs_entry_from_metadata(name, &md));
        }
        Ok(LsResponse { entries })
    }

    async fn stat(&self, req: StatArgs) -> FsCallResult<StatResponse> {
        let p = self.validate_path(&req.path)?;
        let md = std::fs::symlink_metadata(&p).map_err(|e| FsError::from_io(&req.path, e))?;
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| req.path.clone());
        Ok(StatResponse(fs_entry_from_metadata(name, &md)))
    }

    async fn mkdir(&self, req: crate::fs::MkdirArgs) -> FsCallResult<crate::fs::MkdirResponse> {
        let p = self.validate_path(&req.path)?;
        let bits = crate::fs::error::parse_mode(&req.mode)?;
        if p.exists() {
            if req.parents {
                return Ok(crate::fs::MkdirResponse { created: false });
            }
            return Err(FsError::new(
                "S213",
                format!("path already exists: {}", req.path),
            ));
        }
        let res = if req.parents {
            std::fs::create_dir_all(&p)
        } else {
            std::fs::create_dir(&p)
        };
        res.map_err(|e| FsError::from_io(&req.path, e))?;
        let perms = std::fs::Permissions::from_mode(bits);
        std::fs::set_permissions(&p, perms).map_err(|e| FsError::from_io(&req.path, e))?;
        Ok(crate::fs::MkdirResponse { created: true })
    }

    async fn rm(&self, req: crate::fs::RmArgs) -> FsCallResult<crate::fs::RmResponse> {
        // Lexical form required: rm of a symlink must remove the link, not
        // the target. validate_path canonicalizes for jail confinement; we
        // operate on the lexical path to preserve unlink semantics.
        self.validate_path(&req.path)?;
        let p = normalize_lexical(Path::new(&req.path));

        let md = std::fs::symlink_metadata(&p).map_err(|e| FsError::from_io(&req.path, e))?;

        if md.is_dir() && !md.file_type().is_symlink() {
            if req.recursive {
                std::fs::remove_dir_all(&p).map_err(|e| FsError::from_io(&req.path, e))?;
            } else {
                let mut rd = std::fs::read_dir(&p).map_err(|e| FsError::from_io(&req.path, e))?;
                if rd.next().is_some() {
                    return Err(FsError::new(
                        "S214",
                        format!("directory not empty: {}", req.path),
                    ));
                }
                std::fs::remove_dir(&p).map_err(|e| FsError::from_io(&req.path, e))?;
            }
        } else {
            std::fs::remove_file(&p).map_err(|e| FsError::from_io(&req.path, e))?;
        }
        Ok(crate::fs::RmResponse { removed: true })
    }
    async fn chmod(&self, req: crate::fs::ChmodArgs) -> FsCallResult<crate::fs::ChmodResponse> {
        self.validate_path(&req.path)?;
        let p = normalize_lexical(Path::new(&req.path));
        let bits = crate::fs::error::parse_mode(&req.mode)?;
        if !p.exists() {
            return Err(FsError::new(
                "S211",
                format!("path not found: {}", req.path),
            ));
        }
        let uid = req.uid;
        let gid = req.gid;
        let apply = |target: &Path| -> Result<(), FsError> {
            let perms = std::fs::Permissions::from_mode(bits);
            std::fs::set_permissions(target, perms)
                .map_err(|e| FsError::from_io(&target.to_string_lossy(), e))?;
            if uid.is_some() || gid.is_some() {
                std::os::unix::fs::chown(target, uid, gid)
                    .map_err(|e| FsError::from_io(&target.to_string_lossy(), e))?;
            }
            Ok(())
        };
        let mut updated: u64 = 0;
        if req.recursive {
            // Reject if the walk root itself is a symlink: descending into
            // a symlink target would change perms outside the recursive
            // root, and skipping the root entry silently (which is what
            // the per-entry skip below would do) is a quiet no-op that
            // looks like success to the caller. S212 = wrong file type.
            let root_md =
                std::fs::symlink_metadata(&p).map_err(|e| FsError::from_io(&req.path, e))?;
            if root_md.file_type().is_symlink() {
                return Err(FsError::new(
                    "S212",
                    format!(
                        "recursive chmod refuses to follow symlink at root: {}",
                        req.path
                    ),
                ));
            }
            for entry in walkdir::WalkDir::new(&p)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                // Skip symlink entries inside the walk: chmod(2)/chown(2)
                // follow symlinks and would rewrite the target's
                // mode/owner — possibly outside the recursive root or the
                // jail. lchmod isn't portable.
                if entry.file_type().is_symlink() {
                    continue;
                }
                apply(entry.path())?;
                updated += 1;
            }
        } else {
            apply(&p)?;
            updated = 1;
        }
        Ok(crate::fs::ChmodResponse { updated })
    }

    async fn mv(&self, req: crate::fs::MvArgs) -> FsCallResult<crate::fs::MvResponse> {
        self.validate_path(&req.src)?;
        self.validate_path(&req.dst)?;
        let src_p = normalize_lexical(Path::new(&req.src));
        let dst_p = normalize_lexical(Path::new(&req.dst));
        if !src_p.exists() {
            return Err(FsError::new("S211", format!("src not found: {}", req.src)));
        }
        if dst_p.exists() && !req.overwrite {
            return Err(FsError::new(
                "S213",
                format!("dst already exists: {}", req.dst),
            ));
        }
        match std::fs::rename(&src_p, &dst_p) {
            Ok(()) => Ok(crate::fs::MvResponse { moved: true }),
            // EXDEV: cross-fs move — fall back to copy+rename+unlink.
            // File-only; directories are unsupported (matches engine daemon).
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                let tmp = temp_sibling(&dst_p);
                std::fs::copy(&src_p, &tmp).map_err(|e| FsError::from_io(&req.dst, e))?;
                if let Err(e) = std::fs::rename(&tmp, &dst_p) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(FsError::from_io(&req.dst, e));
                }
                std::fs::remove_file(&src_p).map_err(|e| FsError::from_io(&req.src, e))?;
                Ok(crate::fs::MvResponse { moved: true })
            }
            Err(e) => Err(FsError::from_io(&req.dst, e)),
        }
    }
    async fn grep(&self, req: crate::fs::GrepArgs) -> FsCallResult<crate::fs::GrepResponse> {
        let root = self.validate_path(&req.path)?;
        let md = std::fs::symlink_metadata(&root).map_err(|e| FsError::from_io(&req.path, e))?;
        let re = regex::RegexBuilder::new(&req.pattern)
            .case_insensitive(req.ignore_case)
            .build()
            .map_err(|e| FsError::new("S217", format!("bad regex: {e}")))?;
        let max_matches_usize = req.max_matches as usize;
        let max_line_usize = req.max_line_bytes as usize;
        let include_glob = req.include_glob;
        let exclude_glob = req.exclude_glob;

        let should_scan = |rel: &str| -> bool {
            if !include_glob.is_empty() && !include_glob.iter().any(|g| glob_matches_path(g, rel)) {
                return false;
            }
            if exclude_glob.iter().any(|g| glob_matches_path(g, rel)) {
                return false;
            }
            true
        };

        let mut out: Vec<crate::fs::wire::FsMatch> = Vec::new();
        let mut truncated = false;

        let mut scan = |file_path: &Path| -> Result<bool, FsError> {
            if looks_binary(file_path) {
                return Ok(false);
            }
            let f = std::fs::File::open(file_path)
                .map_err(|e| FsError::from_io(&file_path.to_string_lossy(), e))?;
            let reader = std::io::BufReader::new(f);
            use std::io::BufRead;
            for (idx, line_res) in reader.lines().enumerate() {
                let Ok(mut line) = line_res else {
                    continue;
                };
                if re.is_match(&line) {
                    if max_line_usize > 0 && line.len() > max_line_usize {
                        // Floor to nearest char boundary so a multi-byte
                        // codepoint straddling the cut doesn't panic.
                        let cut = (0..=max_line_usize)
                            .rev()
                            .find(|&i| line.is_char_boundary(i))
                            .unwrap_or(0);
                        line.truncate(cut);
                        line.push('…');
                    }
                    out.push(crate::fs::wire::FsMatch {
                        path: file_path.to_string_lossy().into_owned(),
                        line: (idx + 1) as u64,
                        content: line,
                    });
                    if max_matches_usize > 0 && out.len() >= max_matches_usize {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        };

        if md.is_dir() {
            if !req.recursive {
                return Err(FsError::new(
                    "S210",
                    "recursive=false on a directory is unsupported; \
                     pass a file path or set recursive=true",
                ));
            }
            for entry in walkdir::WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let rel = entry
                    .path()
                    .strip_prefix(&root)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .into_owned();
                if !should_scan(&rel) {
                    continue;
                }
                if scan(entry.path())? {
                    truncated = true;
                    break;
                }
            }
        } else {
            let rel = root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if should_scan(&rel) && scan(&root)? {
                truncated = true;
            }
        }

        Ok(crate::fs::GrepResponse {
            matches: out,
            truncated,
        })
    }
    async fn sed(&self, req: crate::fs::SedArgs) -> FsCallResult<crate::fs::SedResponse> {
        let files: Vec<String> = match (req.files.is_empty(), req.path.as_ref()) {
            (false, None) => req.files.clone(),
            (true, Some(root)) => {
                self.validate_path(root)?;
                let root_path = Path::new(root);
                let _ = root_path
                    .symlink_metadata()
                    .map_err(|e| FsError::from_io(root, e))?;
                let target_is_dir = match std::fs::metadata(root_path) {
                    Ok(m) => m.is_dir(),
                    Err(e) => return Err(FsError::from_io(root, e)),
                };
                if target_is_dir && !req.recursive {
                    return Err(FsError::new(
                        "S210",
                        "recursive=false on a directory is unsupported; \
                         pass a file path or set recursive=true",
                    ));
                }
                let collected = collect_files_to_sed(
                    root_path,
                    req.recursive,
                    &req.include_glob,
                    &req.exclude_glob,
                );
                collected
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            }
            (false, Some(_)) => {
                return Err(FsError::new(
                    "S210",
                    "sed: provide exactly one of `files` or `path`, not both",
                ));
            }
            (true, None) => {
                return Err(FsError::new(
                    "S210",
                    "sed: must provide exactly one of `files` or `path`",
                ));
            }
        };

        for f in &files {
            self.validate_path(f)?;
        }

        let matcher: Option<regex::Regex> = if req.regex {
            Some(
                regex::RegexBuilder::new(&req.pattern)
                    .case_insensitive(req.ignore_case)
                    .build()
                    .map_err(|e| FsError::new("S217", format!("bad regex: {e}")))?,
            )
        } else if req.pattern.is_empty() {
            return Err(FsError::new("S210", "pattern is empty"));
        } else {
            None
        };
        let case_fold = req.ignore_case && !req.regex;

        let mut results: Vec<crate::fs::wire::FsSedFileResult> = Vec::with_capacity(files.len());
        let mut total: u64 = 0;
        use std::os::unix::fs::PermissionsExt;

        for file in files {
            let p = Path::new(&file);
            let original = match std::fs::read_to_string(p) {
                Ok(s) => s,
                Err(e) => {
                    results.push(crate::fs::wire::FsSedFileResult {
                        path: file.clone(),
                        replacements: 0,
                        success: false,
                        error: Some(format!("{e}")),
                    });
                    continue;
                }
            };
            let mut replacements: u64 = 0;
            let mut output = String::with_capacity(original.len());
            for line in original.split_inclusive('\n') {
                let (new_line, n) = match &matcher {
                    Some(re) => {
                        let mut count_here = 0u64;
                        let produced = if req.first_only {
                            re.replacen(line, 1, |caps: &regex::Captures| {
                                count_here += 1;
                                expand_regex_replacement(caps, &req.replacement)
                            })
                            .into_owned()
                        } else {
                            re.replace_all(line, |caps: &regex::Captures| {
                                count_here += 1;
                                expand_regex_replacement(caps, &req.replacement)
                            })
                            .into_owned()
                        };
                        (produced, count_here)
                    }
                    None => literal_replace_line(
                        line,
                        &req.pattern,
                        case_fold,
                        &req.replacement,
                        req.first_only,
                    ),
                };
                replacements += n;
                output.push_str(&new_line);
            }
            let tmp = temp_sibling(p);
            let write_result: Result<(), std::io::Error> = (|| {
                let original_md = std::fs::metadata(p)?;
                std::fs::write(&tmp, output.as_bytes())?;
                std::fs::set_permissions(
                    &tmp,
                    std::fs::Permissions::from_mode(original_md.permissions().mode()),
                )?;
                std::fs::rename(&tmp, p)?;
                Ok(())
            })();
            match write_result {
                Ok(()) => {
                    total += replacements;
                    results.push(crate::fs::wire::FsSedFileResult {
                        path: file,
                        replacements,
                        success: true,
                        error: None,
                    });
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    results.push(crate::fs::wire::FsSedFileResult {
                        path: file,
                        replacements: 0,
                        success: false,
                        error: Some(format!("{e}")),
                    });
                }
            }
        }
        Ok(crate::fs::SedResponse {
            results,
            total_replacements: total,
        })
    }
    async fn write(&self, req: crate::fs::WriteArgs) -> FsCallResult<crate::fs::WriteResponse> {
        let p = self.validate_path(&req.path)?;
        let bits = crate::fs::error::parse_mode(&req.mode)?;

        // Defense-in-depth: re-check parent against the precomputed
        // canonical root before creating intermediate directories.
        // validate_path already enforces this on `p`, but parents:true
        // is S-C1's second site so we keep the belt.
        if req.parents {
            if let Some(parent) = p.parent() {
                if let Some(root_canon) = &self.host_root_canon {
                    if !parent.starts_with(root_canon) {
                        return Err(FsError::new(
                            "S215",
                            format!("parent escapes host_root: {}", req.path),
                        ));
                    }
                }
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| FsError::from_io(&req.path, e))?;
            }
        }

        let suffix = format!(".tmp.{}", uuid::Uuid::new_v4().simple());
        let mut temp_str = p.as_os_str().to_os_string();
        temp_str.push(&suffix);
        let temp = std::path::PathBuf::from(temp_str);

        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .await
            .map_err(|e| FsError::from_io(&req.path, e))?;
        let guard = TempGuard::new(temp.clone());

        let reader =
            iii_sdk::channels::ChannelReader::new(&self.chan.engine_address(), &req.content);
        let cap = self.cfg.max_write_bytes;
        let mut total: u64 = 0;
        // Per-chunk idle timeout: if the caller opens a write but never sends
        // data and never closes the channel, the worker would hold the temp
        // file open indefinitely. N parked writers = resource exhaustion.
        let idle = std::time::Duration::from_secs(30);
        loop {
            let next = match tokio::time::timeout(idle, reader.next_binary()).await {
                Ok(r) => r,
                Err(_) => {
                    return Err(FsError::new(
                        "S216",
                        format!("channel idle for {}s — aborting write", idle.as_secs()),
                    ));
                }
            };
            match next {
                Ok(Some(chunk)) => {
                    let new_total = total + chunk.len() as u64;
                    if cap > 0 && new_total > cap as u64 {
                        return Err(FsError::new(
                            "S218",
                            format!(
                                "write payload exceeds max_write_bytes {} (after {} bytes)",
                                cap, total
                            ),
                        ));
                    }
                    if let Err(e) = file.write_all(&chunk).await {
                        return Err(FsError::from_io(&req.path, e));
                    }
                    total = new_total;
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(FsError::new("S216", format!("channel read failed: {e}")));
                }
            }
        }

        if let Err(e) = file.flush().await {
            return Err(FsError::from_io(&req.path, e));
        }
        if let Err(e) = file.sync_all().await {
            return Err(FsError::from_io(&req.path, e));
        }
        drop(file);

        let perms = std::fs::Permissions::from_mode(bits);
        if let Err(e) = tokio::fs::set_permissions(guard.path(), perms).await {
            return Err(FsError::from_io(&req.path, e));
        }
        if let Err(e) = tokio::fs::rename(guard.path(), &p).await {
            return Err(FsError::from_io(&req.path, e));
        }
        guard.commit();

        Ok(crate::fs::WriteResponse {
            bytes_written: total,
            path: req.path,
        })
    }
    async fn read(&self, req: crate::fs::ReadArgs) -> FsCallResult<crate::fs::ReadResponse> {
        use std::os::unix::fs::PermissionsExt;

        let p = self.validate_path(&req.path)?;
        let md = tokio::fs::symlink_metadata(&p)
            .await
            .map_err(|e| FsError::from_io(&req.path, e))?;
        if md.is_dir() {
            return Err(FsError::new(
                "S212",
                format!("path is a directory, not a file: {}", req.path),
            ));
        }
        let cap = self.cfg.max_read_bytes;
        if cap > 0 && (md.len() as usize) > cap {
            return Err(FsError::new(
                "S218",
                format!("file size {} exceeds max_read_bytes {}", md.len(), cap),
            ));
        }
        let size = md.len();
        let mode = format!("{:04o}", md.permissions().mode() & 0o7777);
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let channel = self
            .chan
            .create_channel(64)
            .await
            .map_err(|e| FsError::new("S216", format!("create_channel: {e}")))?;
        let reader_ref = channel.reader_ref.clone();
        let writer = channel.writer;
        let path_for_pump = p;

        tokio::spawn(async move {
            let result = pump_file_to_channel(&path_for_pump, &writer).await;
            if let Err(msg) = result {
                let _ = writer
                    .send_message(&serde_json::json!({"error": msg}).to_string())
                    .await;
            }
            let _ = writer.close().await;
        });

        Ok(crate::fs::ReadResponse {
            content: reader_ref,
            size,
            mode,
            mtime,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("iii-shell-fs-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[derive(Debug)]
    struct StubChan;
    #[async_trait::async_trait]
    impl super::ChannelMaker for StubChan {
        async fn create_channel(&self, _: usize) -> Result<iii_sdk::Channel, iii_sdk::IIIError> {
            Err(iii_sdk::IIIError::Handler("stub".into()))
        }
        fn engine_address(&self) -> String {
            "ws://stub:0".into()
        }
    }
    fn stub_backend(cfg: HostFsConfig) -> HostFsBackend {
        HostFsBackend::new(Arc::new(cfg), Arc::new(StubChan))
    }
    fn stub_ref() -> iii_sdk::channels::StreamChannelRef {
        iii_sdk::channels::StreamChannelRef {
            channel_id: "c".into(),
            access_key: "k".into(),
            direction: iii_sdk::channels::ChannelDirection::Read,
        }
    }

    #[test]
    fn rejects_relative_path() {
        let h = stub_backend(HostFsConfig::default());
        let err = h.validate_path("relative/path").unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[test]
    fn allows_absolute_when_no_root() {
        let h = stub_backend(HostFsConfig::default());
        let _ = h.validate_path("/tmp").unwrap();
    }

    #[test]
    fn rejects_path_outside_host_root() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_root: Some(root.clone()),
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let err = h.validate_path("/etc").unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[test]
    fn allows_descendant_of_host_root() {
        let root = tmp();
        fs::create_dir(root.join("sub")).unwrap();
        let cfg = HostFsConfig {
            host_root: Some(root.clone()),
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let _ = h.validate_path(root.join("sub").to_str().unwrap()).unwrap();
    }

    #[test]
    fn denylist_blocks_even_inside_root() {
        let root = tmp();
        fs::create_dir(root.join("etc")).unwrap();
        fs::write(root.join("etc/shadow"), b"x").unwrap();
        let cfg = HostFsConfig {
            host_root: Some(root.clone()),
            denylist_paths: vec![root.join("etc/shadow")],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let err = h
            .validate_path(root.join("etc/shadow").to_str().unwrap())
            .unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[test]
    fn nonexistent_path_with_dotdot_cannot_escape_root() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_root: Some(root.clone()),
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let escape = root.join("nonexistent/../../../etc/passwd");
        let err = h.validate_path(escape.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[tokio::test]
    async fn ls_lists_dir_entries() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), b"hi").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .ls(LsArgs {
                path: root.to_str().unwrap().into(),
            })
            .await
            .unwrap();
        assert_eq!(resp.entries.len(), 1);
        assert_eq!(resp.entries[0].name, "a.txt");
        assert_eq!(resp.entries[0].size, 2);
        assert!(!resp.entries[0].is_dir);
    }

    #[tokio::test]
    async fn ls_missing_returns_s211() {
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .ls(LsArgs {
                path: "/nope/never/exists/iii".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn stat_returns_file_entry() {
        let root = tmp();
        let f = root.join("x.txt");
        std::fs::write(&f, b"data").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .stat(StatArgs {
                path: f.to_str().unwrap().into(),
            })
            .await
            .unwrap();
        assert_eq!(resp.0.name, "x.txt");
        assert_eq!(resp.0.size, 4);
    }

    #[tokio::test]
    async fn ls_on_file_returns_s212() {
        let root = tmp();
        let f = root.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .ls(LsArgs {
                path: f.to_str().unwrap().into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S212");
    }

    #[tokio::test]
    async fn ls_includes_symlink_with_is_symlink_true() {
        use std::os::unix::fs::symlink;
        let root = tmp();
        let target = root.join("real.txt");
        std::fs::write(&target, b"x").unwrap();
        symlink(&target, root.join("alias")).unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .ls(LsArgs {
                path: root.to_str().unwrap().into(),
            })
            .await
            .unwrap();
        let alias = resp.entries.iter().find(|e| e.name == "alias").unwrap();
        assert!(
            alias.is_symlink,
            "ls should report is_symlink=true for symlink entries"
        );
    }

    #[tokio::test]
    async fn ls_with_broken_symlink_does_not_abort() {
        use std::os::unix::fs::symlink;
        let root = tmp();
        std::fs::write(root.join("real.txt"), b"x").unwrap();
        symlink(root.join("nonexistent"), root.join("dangling")).unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .ls(LsArgs {
                path: root.to_str().unwrap().into(),
            })
            .await
            .unwrap();
        assert_eq!(resp.entries.len(), 2);
    }

    #[tokio::test]
    async fn ls_returns_mode_padded_to_four_digits() {
        let root = tmp();
        let f = root.join("p.txt");
        std::fs::write(&f, b"x").unwrap();
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&f, perms).unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .ls(LsArgs {
                path: root.to_str().unwrap().into(),
            })
            .await
            .unwrap();
        let entry = resp.entries.iter().find(|e| e.name == "p.txt").unwrap();
        assert_eq!(entry.mode, "0600");
    }

    #[tokio::test]
    async fn mkdir_creates_directory() {
        let root = tmp();
        let p = root.join("new");
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .mkdir(crate::fs::MkdirArgs {
                path: p.to_str().unwrap().into(),
                mode: "0755".into(),
                parents: false,
            })
            .await
            .unwrap();
        assert!(resp.created);
        assert!(p.is_dir());
    }

    #[tokio::test]
    async fn mkdir_returns_s213_on_exists_without_parents() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .mkdir(crate::fs::MkdirArgs {
                path: root.to_str().unwrap().into(),
                mode: "0755".into(),
                parents: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S213");
    }

    #[tokio::test]
    async fn mkdir_with_parents_tolerates_existing_path() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .mkdir(crate::fs::MkdirArgs {
                path: root.to_str().unwrap().into(),
                mode: "0755".into(),
                parents: true,
            })
            .await
            .unwrap();
        assert!(!resp.created);
    }

    #[tokio::test]
    async fn mkdir_bad_mode_returns_s210() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .mkdir(crate::fs::MkdirArgs {
                path: root.join("x").to_str().unwrap().into(),
                mode: "garbage".into(),
                parents: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn rm_file_succeeds() {
        let root = tmp();
        let f = root.join("d.txt");
        std::fs::write(&f, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .rm(crate::fs::RmArgs {
                path: f.to_str().unwrap().into(),
                recursive: false,
            })
            .await
            .unwrap();
        assert!(resp.removed);
        assert!(!f.exists());
    }

    #[tokio::test]
    async fn rm_missing_path_returns_s211() {
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .rm(crate::fs::RmArgs {
                path: "/nope/never/iii-rm-test".into(),
                recursive: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn rm_nonempty_dir_returns_s214() {
        let root = tmp();
        let sub = root.join("d");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("f"), b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .rm(crate::fs::RmArgs {
                path: sub.to_str().unwrap().into(),
                recursive: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S214");
    }

    #[tokio::test]
    async fn rm_symlink_to_dir_uses_remove_file() {
        use std::os::unix::fs::symlink;
        let root = tmp();
        let target = root.join("real-dir");
        std::fs::create_dir(&target).unwrap();
        let link = root.join("link-to-dir");
        symlink(&target, &link).unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .rm(crate::fs::RmArgs {
                path: link.to_str().unwrap().into(),
                recursive: false,
            })
            .await
            .unwrap();
        assert!(resp.removed);
        assert!(!link.exists());
        assert!(target.is_dir());
    }

    #[tokio::test]
    async fn mkdir_with_parents_creates_nested() {
        let root = tmp();
        let deep = root.join("a/b/c");
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .mkdir(crate::fs::MkdirArgs {
                path: deep.to_str().unwrap().into(),
                mode: "0755".into(),
                parents: true,
            })
            .await
            .unwrap();
        assert!(resp.created);
        assert!(deep.is_dir());
        assert!(root.join("a").is_dir());
        assert!(root.join("a/b").is_dir());
    }

    #[tokio::test]
    async fn rm_recursive_removes_tree() {
        let root = tmp();
        let tree = root.join("tree");
        std::fs::create_dir(&tree).unwrap();
        std::fs::create_dir(tree.join("sub")).unwrap();
        std::fs::write(tree.join("sub/leaf.txt"), b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .rm(crate::fs::RmArgs {
                path: tree.to_str().unwrap().into(),
                recursive: true,
            })
            .await
            .unwrap();
        assert!(resp.removed);
        assert!(!tree.exists());
    }

    #[tokio::test]
    async fn chmod_changes_mode() {
        let root = tmp();
        let f = root.join("c.txt");
        std::fs::write(&f, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .chmod(crate::fs::ChmodArgs {
                path: f.to_str().unwrap().into(),
                mode: "0600".into(),
                uid: None,
                gid: None,
                recursive: false,
            })
            .await
            .unwrap();
        assert_eq!(resp.updated, 1);
        let perms = std::fs::metadata(&f).unwrap().permissions().mode() & 0o7777;
        assert_eq!(perms, 0o600);
    }

    #[tokio::test]
    async fn chmod_missing_returns_s211() {
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .chmod(crate::fs::ChmodArgs {
                path: "/nope/never/iii-chmod".into(),
                mode: "0600".into(),
                uid: None,
                gid: None,
                recursive: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn chmod_bad_mode_returns_s210() {
        let root = tmp();
        let f = root.join("c.txt");
        std::fs::write(&f, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .chmod(crate::fs::ChmodArgs {
                path: f.to_str().unwrap().into(),
                mode: "garbage".into(),
                uid: None,
                gid: None,
                recursive: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn chmod_recursive_walks_tree_and_counts() {
        let root = tmp();
        let tree = root.join("tree");
        std::fs::create_dir(&tree).unwrap();
        std::fs::create_dir(tree.join("sub")).unwrap();
        std::fs::write(tree.join("sub/leaf.txt"), b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .chmod(crate::fs::ChmodArgs {
                path: tree.to_str().unwrap().into(),
                mode: "0700".into(),
                uid: None,
                gid: None,
                recursive: true,
            })
            .await
            .unwrap();
        assert_eq!(resp.updated, 3);
        let leaf_perms = std::fs::metadata(tree.join("sub/leaf.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(leaf_perms, 0o700);
    }

    #[tokio::test]
    async fn mv_renames_file() {
        let root = tmp();
        let a = root.join("a");
        let b = root.join("b");
        std::fs::write(&a, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .mv(crate::fs::MvArgs {
                src: a.to_str().unwrap().into(),
                dst: b.to_str().unwrap().into(),
                overwrite: false,
            })
            .await
            .unwrap();
        assert!(resp.moved);
        assert!(!a.exists());
        assert!(b.exists());
    }

    #[tokio::test]
    async fn mv_missing_src_returns_s211() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .mv(crate::fs::MvArgs {
                src: root.join("nope").to_str().unwrap().into(),
                dst: root.join("dst").to_str().unwrap().into(),
                overwrite: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
        assert!(err.message.contains("src not found"));
    }

    #[tokio::test]
    async fn mv_existing_dst_no_overwrite_returns_s213() {
        let root = tmp();
        let a = root.join("a");
        let b = root.join("b");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"y").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .mv(crate::fs::MvArgs {
                src: a.to_str().unwrap().into(),
                dst: b.to_str().unwrap().into(),
                overwrite: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S213");
        assert!(err.message.contains("dst already exists"));
    }

    #[tokio::test]
    async fn mv_with_overwrite_replaces_dst() {
        let root = tmp();
        let a = root.join("a");
        let b = root.join("b");
        std::fs::write(&a, b"new").unwrap();
        std::fs::write(&b, b"old").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .mv(crate::fs::MvArgs {
                src: a.to_str().unwrap().into(),
                dst: b.to_str().unwrap().into(),
                overwrite: true,
            })
            .await
            .unwrap();
        assert!(resp.moved);
        assert_eq!(std::fs::read(&b).unwrap(), b"new");
        assert!(!a.exists());
    }

    #[tokio::test]
    async fn chmod_with_uid_unprivileged_returns_s215() {
        let root = tmp();
        let f = root.join("c.txt");
        std::fs::write(&f, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .chmod(crate::fs::ChmodArgs {
                path: f.to_str().unwrap().into(),
                mode: "0644".into(),
                uid: Some(0),
                gid: None,
                recursive: false,
            })
            .await;
        match resp {
            Ok(_) => {}

            Err(e) => assert_eq!(e.code, "S215"),
        }
    }

    #[tokio::test]
    async fn write_rejects_non_absolute_path_with_s210() {
        let b = stub_backend(HostFsConfig::default());
        let err = b
            .write(crate::fs::WriteArgs {
                path: "rel/path".into(),
                mode: "0644".into(),
                parents: false,
                content: stub_ref(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn write_rejects_invalid_mode_with_s210() {
        let b = stub_backend(HostFsConfig::default());
        let err = b
            .write(crate::fs::WriteArgs {
                path: "/tmp/iii-shell-write-bad-mode".into(),
                mode: "not-octal".into(),
                parents: false,
                content: stub_ref(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn write_rejects_path_outside_host_root_with_s215() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_root: Some(root.clone()),
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .write(crate::fs::WriteArgs {
                path: "/etc/iii-shell-escape".into(),
                mode: "0644".into(),
                parents: false,
                content: stub_ref(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[tokio::test]
    async fn write_with_missing_parent_and_parents_false_returns_s211() {
        let root = tmp();
        let target = root
            .join("nonexistent")
            .join("file.txt")
            .to_string_lossy()
            .to_string();
        let b = stub_backend(HostFsConfig::default());
        let err = b
            .write(crate::fs::WriteArgs {
                path: target,
                mode: "0644".into(),
                parents: false,
                content: stub_ref(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn read_rejects_missing_file_with_s211() {
        let root = tmp();
        let canon_root = std::fs::canonicalize(&root).unwrap();
        let missing = canon_root.join("nope.txt").to_string_lossy().to_string();
        let cfg = HostFsConfig {
            host_root: Some(root.clone()),
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .read(crate::fs::ReadArgs { path: missing })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn read_rejects_directory_with_s212() {
        let root = tmp();
        let dir = root.to_string_lossy().to_string();
        let cfg = HostFsConfig {
            host_root: Some(root.clone()),
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b.read(crate::fs::ReadArgs { path: dir }).await.unwrap_err();
        assert_eq!(err.code, "S212");
    }

    #[tokio::test]
    async fn read_rejects_oversize_file_with_s218_preflight() {
        let root = tmp();
        let f = root.join("big.bin");
        std::fs::write(&f, vec![0u8; 1024]).unwrap();
        let cfg = HostFsConfig {
            host_root: Some(root.clone()),
            max_read_bytes: 10,
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .read(crate::fs::ReadArgs {
                path: f.to_string_lossy().to_string(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S218");
    }

    #[tokio::test]
    async fn read_rejects_path_outside_host_root_with_s215() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_root: Some(root.clone()),
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .read(crate::fs::ReadArgs {
                path: "/etc/iii-shell-escape".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[tokio::test]
    async fn read_create_channel_failure_maps_to_s216() {
        let root = tmp();
        let f = root.join("ok.txt");
        std::fs::write(&f, b"hello").unwrap();
        let cfg = HostFsConfig {
            host_root: Some(root.clone()),
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .read(crate::fs::ReadArgs {
                path: f.to_string_lossy().to_string(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S216");
    }

    #[tokio::test]
    async fn grep_finds_matches_in_dir() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "alpha\nbeta\nalpha-beta\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .grep(crate::fs::GrepArgs {
                path: root.to_str().unwrap().into(),
                pattern: "alpha".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 100,
                max_line_bytes: 8192,
            })
            .await
            .unwrap();
        assert_eq!(resp.matches.len(), 2);
        assert!(!resp.truncated);
        assert_eq!(resp.matches[0].line, 1);
    }

    #[tokio::test]
    async fn grep_recursive_false_on_dir_returns_s210() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .grep(crate::fs::GrepArgs {
                path: root.to_str().unwrap().into(),
                pattern: "x".into(),
                recursive: false,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 100,
                max_line_bytes: 8192,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn grep_bad_regex_returns_s217() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .grep(crate::fs::GrepArgs {
                path: root.to_str().unwrap().into(),
                pattern: "[unclosed".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 1,
                max_line_bytes: 8192,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S217");
    }

    #[tokio::test]
    async fn grep_truncates_at_max_matches() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "x\nx\nx\nx\nx\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .grep(crate::fs::GrepArgs {
                path: root.to_str().unwrap().into(),
                pattern: "x".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 2,
                max_line_bytes: 8192,
            })
            .await
            .unwrap();
        assert_eq!(resp.matches.len(), 2);
        assert!(resp.truncated);
    }

    #[tokio::test]
    async fn grep_skips_binary_files() {
        let root = tmp();
        std::fs::write(root.join("bin.dat"), b"x\0x\0x\nx\nx\n").unwrap();
        std::fs::write(root.join("a.txt"), "x\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .grep(crate::fs::GrepArgs {
                path: root.to_str().unwrap().into(),
                pattern: "x".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 100,
                max_line_bytes: 8192,
            })
            .await
            .unwrap();
        assert!(resp.matches.iter().all(|m| m.path.ends_with("a.txt")));
    }

    #[tokio::test]
    async fn grep_include_glob_filters_by_extension() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "match\n").unwrap();
        std::fs::write(root.join("b.md"), "match\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .grep(crate::fs::GrepArgs {
                path: root.to_str().unwrap().into(),
                pattern: "match".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec!["*.txt".into()],
                exclude_glob: vec![],
                max_matches: 100,
                max_line_bytes: 8192,
            })
            .await
            .unwrap();
        assert_eq!(resp.matches.len(), 1);
        assert!(resp.matches[0].path.ends_with("a.txt"));
    }

    #[tokio::test]
    async fn sed_literal_replaces_files_form() {
        let root = tmp();
        let f = root.join("s.txt");
        std::fs::write(&f, "hello world hello").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .sed(crate::fs::SedArgs {
                files: vec![f.to_str().unwrap().into()],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "hello".into(),
                replacement: "HI".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap();
        assert_eq!(resp.total_replacements, 2);
        assert_eq!(resp.results.len(), 1);
        assert!(resp.results[0].success);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "HI world HI");
    }

    #[tokio::test]
    async fn sed_first_only_stops_after_one() {
        let root = tmp();
        let f = root.join("s.txt");
        std::fs::write(&f, "x x x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .sed(crate::fs::SedArgs {
                files: vec![f.to_str().unwrap().into()],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "x".into(),
                replacement: "Y".into(),
                regex: false,
                first_only: true,
                ignore_case: false,
            })
            .await
            .unwrap();
        assert_eq!(resp.total_replacements, 1);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "Y x x");
    }

    #[tokio::test]
    async fn sed_neither_files_nor_path_returns_s210() {
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .sed(crate::fs::SedArgs {
                files: vec![],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "x".into(),
                replacement: "Y".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn sed_both_files_and_path_returns_s210() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .sed(crate::fs::SedArgs {
                files: vec!["/x".into()],
                path: Some(root.to_str().unwrap().into()),
                recursive: true,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "x".into(),
                replacement: "Y".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn sed_path_form_recursive_false_on_dir_returns_s210() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .sed(crate::fs::SedArgs {
                files: vec![],
                path: Some(root.to_str().unwrap().into()),
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "x".into(),
                replacement: "Y".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn sed_regex_with_capture_refs_works() {
        let root = tmp();
        let f = root.join("r.txt");
        std::fs::write(&f, "Alice loves Bob\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .sed(crate::fs::SedArgs {
                files: vec![f.to_str().unwrap().into()],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "(\\w+) loves (\\w+)".into(),
                replacement: "$2 then $1".into(),
                regex: true,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap();
        assert_eq!(resp.total_replacements, 1);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "Bob then Alice\n");
    }
}
