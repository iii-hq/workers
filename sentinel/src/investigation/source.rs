//! Session-bound source reads. No profile field can name a root or grant.
use super::{record::Caller, Investigations};
use crate::{Db, SentinelError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::Read;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReadRequest {
    pub group_id: String,
    /// Relative to the repository configured for this member's worker.
    pub path: String,
    #[serde(default)]
    pub line_from: Option<usize>,
    #[serde(default)]
    pub line_to: Option<usize>,
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SourceReadResponse {
    pub group_id: String,
    pub path: String,
    pub content: String,
    pub line_from: usize,
    pub truncated: bool,
    pub redactions: u64,
}

impl<D: Db> Investigations<D> {
    pub async fn source_read(
        &self,
        caller: Caller,
        request: SourceReadRequest,
    ) -> Result<SourceReadResponse, SentinelError> {
        let session = caller
            .session_id
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                SentinelError::NoInvestigation(
                    "source reads require invocation session identity".into(),
                )
            })?;
        let investigation = self
            .store()
            .investigation_by_session(&session)
            .await?
            .ok_or_else(|| {
                SentinelError::NoInvestigation("unknown investigation session".into())
            })?;
        if !crate::ids::is_investigation_session(&session)
            || !self
                .member_ids(&investigation.id)
                .await?
                .contains(&request.group_id)
        {
            return Err(SentinelError::NotThisGroup(
                "source read is not an authorized member".into(),
            ));
        }
        let group = self
            .store()
            .db()
            .query(
                "SELECT service_name FROM sentinel_groups WHERE id = ?",
                vec![json!(request.group_id)],
            )
            .await?;
        let worker = group
            .first()
            .and_then(|row| row.get("service_name"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| SentinelError::NotFound("member issue".into()))?;
        let config = self.config.read().await.clone();
        let repository = config.repository_for_worker(worker).ok_or_else(|| {
            SentinelError::NoEvidence(
                "source_unavailable: no authorized Sentinel project mapping".into(),
            )
        })?;
        let from = request.line_from.unwrap_or(1);
        let to = request.line_to.unwrap_or(from.saturating_add(199));
        if from == 0 || to < from || to - from >= 200 || from > 100_000 {
            return Err(SentinelError::invalid("source window must be 1..200 lines"));
        }
        let mut file = jailed_open(&repository.path, &request.path)?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(262145)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                SentinelError::NoEvidence("source_unavailable: cannot read file".into())
            })?;
        // Refuse oversized files entirely: a prefix cannot establish whether
        // a PEM block closes outside the read ceiling. Never redact a partial
        // file and pretend that its body-only window is safe.
        if bytes.len() > 262144 {
            return Err(SentinelError::NoEvidence(
                "source_unavailable: file exceeds safe whole-file redaction limit (262144 bytes)"
                    .into(),
            ));
        }
        let mut truncated = false;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            SentinelError::NoEvidence(
                "source_unavailable: non-UTF8 or oversized character boundary".into(),
            )
        })?;
        let (safe, redactions) = crate::Redactor::new(&config.redaction.patterns)
            .map_err(SentinelError::dependency)?
            .redact_source(text)
            .map_err(SentinelError::NoEvidence)?;
        let mut content = safe
            .lines()
            .skip(from - 1)
            .take(to - from + 1)
            .collect::<Vec<_>>()
            .join("\n");
        if content.len() > 12000 {
            let mut end = 12000;
            while !content.is_char_boundary(end) {
                end -= 1;
            }
            content.truncate(end);
            truncated = true;
        }
        Ok(SourceReadResponse {
            group_id: request.group_id,
            path: request.path,
            content,
            line_from: from,
            truncated,
            redactions,
        })
    }
}

/// Each open is relative to an already-open directory descriptor. Refusing
/// symlinks at every step closes both traversal and rename/symlink races.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn jailed_open(root: &str, relative: &str) -> Result<std::fs::File, SentinelError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::{Component, Path};
    #[cfg(target_os = "macos")]
    const NOFOLLOW: i32 = 0x100;
    #[cfg(target_os = "macos")]
    const DIRECTORY: i32 = 0x100000;
    #[cfg(target_os = "macos")]
    const CLOEXEC: i32 = 0x1000000;
    #[cfg(target_os = "linux")]
    const NOFOLLOW: i32 = 0x20000;
    #[cfg(target_os = "linux")]
    const DIRECTORY: i32 = 0x10000;
    #[cfg(target_os = "linux")]
    const CLOEXEC: i32 = 0x80000;
    #[cfg(target_os = "macos")]
    const NONBLOCK: i32 = 0x4;
    #[cfg(target_os = "linux")]
    const NONBLOCK: i32 = 0x800;
    extern "C" {
        fn openat(fd: i32, path: *const std::os::raw::c_char, flags: i32, ...) -> i32;
    }
    let refused = || {
        SentinelError::NoEvidence(
            "source_unavailable: path must be a regular non-symlink file inside its mapped project"
                .into(),
        )
    };
    if relative.is_empty() || relative.len() > 1024 || relative.contains('\0') {
        return Err(refused());
    }
    let components: Vec<_> = Path::new(relative).components().collect();
    if components.is_empty()
        || components.iter().any(|c| {
            !matches!(c, Component::Normal(_)) || c.as_os_str().to_string_lossy().starts_with('.')
        })
    {
        return Err(refused());
    }
    let extension = Path::new(relative)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if ![
        "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "kt", "swift", "c", "h", "cpp", "cs",
        "rb", "md", "toml",
    ]
    .contains(&extension)
    {
        return Err(refused());
    }
    let canonical = std::fs::canonicalize(root).map_err(|_| refused())?;
    let mut directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(NOFOLLOW | DIRECTORY | CLOEXEC)
        .open(canonical)
        .map_err(|_| refused())?;
    for (index, component) in components.iter().enumerate() {
        use std::os::unix::ffi::OsStrExt;
        let name = CString::new(component.as_os_str().as_bytes()).map_err(|_| refused())?;
        let last = index + 1 == components.len();
        let fd = unsafe {
            openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                NOFOLLOW | CLOEXEC | NONBLOCK | if last { 0 } else { DIRECTORY },
            )
        };
        if fd < 0 {
            return Err(refused());
        }
        let next = unsafe { std::fs::File::from_raw_fd(fd) };
        if last {
            if !next.metadata().map_err(|_| refused())?.is_file() {
                return Err(refused());
            }
            return Ok(next);
        }
        directory = next;
    }
    Err(refused())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn jailed_open(_root: &str, _relative: &str) -> Result<std::fs::File, SentinelError> {
    Err(SentinelError::NoEvidence(
        "source_unavailable: jailed reads unsupported on this platform".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapped_source_rejects_traversal_absolute_and_symlinks() {
        let root = env!("CARGO_MANIFEST_DIR");
        assert!(jailed_open(root, "src/lib.rs").is_ok());
        for path in [
            "../sentinel/src/lib.rs",
            "/etc/passwd",
            "src/../../Cargo.toml",
            ".env",
            "secrets/token.txt",
            "",
        ] {
            assert!(jailed_open(root, path).is_err(), "{path}");
        }
        // Real allowed-extension and intermediate symlinks are exercised by
        // the synthetic fixtures below, not by a whitelist-rejected YAML.
    }

    #[cfg(unix)]
    fn synthetic_root(label: &str) -> std::path::PathBuf {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("docs/evidence/group-investigation-a73c/r2-source-fixtures")
            .join(format!("{label}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[cfg(unix)]
    #[test]
    fn allowed_extension_symlinks_and_intermediate_symlinks_are_refused() {
        use std::os::unix::fs::symlink;
        let base = synthetic_root("nofollow");
        let root = base.join("mapped");
        let outside = base.join("unmapped");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            outside.join("synthetic.rs"),
            "SYNTHETIC_OUTSIDE_SENTINEL_MAPPING",
        )
        .unwrap();
        symlink(outside.join("synthetic.rs"), root.join("linked.rs")).unwrap();
        symlink(&outside, root.join("linked-directory")).unwrap();
        assert!(std::fs::read_to_string(root.join("linked.rs"))
            .unwrap()
            .contains("SYNTHETIC_OUTSIDE"));
        assert!(jailed_open(root.to_str().unwrap(), "linked.rs").is_err());
        assert!(jailed_open(root.to_str().unwrap(), "linked-directory/synthetic.rs").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn opened_inode_stays_safe_after_path_replaced_by_symlink() {
        use std::os::unix::fs::symlink;
        let base = synthetic_root("inode");
        let root = base.join("mapped");
        std::fs::create_dir_all(&root).unwrap();
        let original = root.join("source.rs");
        std::fs::write(&original, "SAFE_ORIGINAL_INODE").unwrap();
        let outside = base.join("outside.rs");
        std::fs::write(&outside, "SYNTHETIC_OUTSIDE_MAPPING").unwrap();
        let mut opened = jailed_open(root.to_str().unwrap(), "source.rs").unwrap();
        std::fs::rename(&original, root.join("preserved-original.rs")).unwrap();
        symlink(outside, &original).unwrap();
        let mut text = String::new();
        opened.read_to_string(&mut text).unwrap();
        assert_eq!(text, "SAFE_ORIGINAL_INODE");
        assert!(jailed_open(root.to_str().unwrap(), "source.rs").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn bounded_directory_swap_race_never_reads_symlink_target() {
        use std::os::unix::fs::symlink;
        let base = synthetic_root("race");
        let root = base.join("mapped");
        let outside = base.join("unmapped");
        std::fs::create_dir_all(root.join("source")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("source/code.rs"), "SAFE_MAPPED_SOURCE").unwrap();
        std::fs::write(outside.join("code.rs"), "SYNTHETIC_UNMAPPED_SOURCE").unwrap();
        let root2 = root.clone();
        let base2 = base.clone();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let barrier2 = barrier.clone();
        let writer = std::thread::spawn(move || {
            barrier2.wait();
            for n in 0..100 {
                let parked = base2.join(format!("preserved-directory-{n}"));
                std::fs::rename(root2.join("source"), &parked).unwrap();
                symlink(&outside, root2.join("source")).unwrap();
                std::thread::yield_now();
                std::fs::rename(
                    root2.join("source"),
                    base2.join(format!("preserved-symlink-{n}")),
                )
                .unwrap();
                std::fs::rename(parked, root2.join("source")).unwrap();
            }
        });
        barrier.wait();
        for _ in 0..1000 {
            if let Ok(mut file) = jailed_open(root.to_str().unwrap(), "source/code.rs") {
                let mut text = String::new();
                file.read_to_string(&mut text).unwrap();
                assert_eq!(text, "SAFE_MAPPED_SOURCE");
            }
            std::thread::yield_now();
        }
        writer.join().unwrap();
        assert!(jailed_open(root.to_str().unwrap(), "source/code.rs").is_ok());
    }
}
