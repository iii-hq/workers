//! Download sources: workers registry (HTTP) and GitHub repo (git clone).
//!
//! Each source produces a [`DownloadResult`] describing what landed on
//! disk. The high-level `skills::download` function in
//! [`crate::functions::download`] picks one of these based on the
//! incoming arguments and fires the `skills::on-change` /
//! `prompts::on-change` triggers afterwards.

pub mod git;
pub mod registry;

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// User-agent string sent on every outbound HTTP request from this
/// worker. Mirrors the `iii-directory/<version>` convention so the
/// registry can correlate traffic to a specific worker release.
pub const HTTP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// Shared `reqwest::Client` builder used by every HTTP caller in this
/// crate (`sources::registry::download`, `functions::registry::*`).
/// Centralises TLS settings, timeout, and the `User-Agent` header so
/// every outbound call looks the same on the wire.
pub fn build_http_client(timeout_ms: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|e| format!("build http client: {e}"))
}

/// Outcome of a single `skills::download` invocation. The high-level
/// function turns this into a JSON response and uses the counts to
/// decide which `on-change` triggers to fan out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadResult {
    pub namespace: String,
    pub skills_written: Vec<String>,
    pub prompts_written: Vec<String>,
}

impl DownloadResult {
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            skills_written: Vec::new(),
            prompts_written: Vec::new(),
        }
    }

    pub fn total_files(&self) -> usize {
        self.skills_written.len() + self.prompts_written.len()
    }
}

/// Reject any path that:
///
/// - is absolute,
/// - contains `..` components, or
/// - contains a `prefix:` (Windows drive) component.
///
/// Returns the cleaned `PathBuf` (still relative) on success. Used by
/// every download source before it joins the path under
/// `<skills_folder>/<namespace>/`.
pub fn validate_relative_path(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() {
        return Err("path must be non-empty".into());
    }
    let candidate = Path::new(raw);
    if candidate.is_absolute() {
        return Err(format!("path may not be absolute: {raw:?}"));
    }
    let mut out = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("path may not contain '..': {raw:?}"));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(format!("path may not contain a prefix or root: {raw:?}"));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(format!("path resolved to empty: {raw:?}"));
    }
    Ok(out)
}

/// Atomically write `contents` to `dest` by writing to `dest.tmp` first
/// and then renaming. Creates the parent directory if missing.
pub fn write_file_atomic(dest: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create_dir_all {}: {e}", parent.display()))?;
    }
    let tmp = dest.with_extension(match dest.extension() {
        Some(ext) => format!("{}.tmp", ext.to_string_lossy()),
        None => "tmp".to_string(),
    });
    std::fs::write(&tmp, contents).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, dest)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), dest.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_relative_path_rejects_absolute() {
        assert!(validate_relative_path("/etc/passwd").is_err());
    }

    #[test]
    fn validate_relative_path_rejects_parent_traversal() {
        assert!(validate_relative_path("../etc/passwd").is_err());
        assert!(validate_relative_path("a/../b").is_err());
    }

    #[test]
    fn validate_relative_path_rejects_empty() {
        assert!(validate_relative_path("").is_err());
        assert!(validate_relative_path(".").is_err());
        assert!(validate_relative_path("./").is_err());
    }

    #[test]
    fn validate_relative_path_accepts_simple_paths() {
        assert_eq!(
            validate_relative_path("foo.md").unwrap(),
            PathBuf::from("foo.md")
        );
        assert_eq!(
            validate_relative_path("a/b/c.md").unwrap(),
            PathBuf::from("a/b/c.md")
        );
    }

    #[test]
    fn validate_relative_path_strips_curdir_components() {
        // "./a/./b.md" → "a/b.md"
        assert_eq!(
            validate_relative_path("./a/./b.md").unwrap(),
            PathBuf::from("a/b.md")
        );
    }

    #[test]
    fn write_file_atomic_creates_parent_and_writes_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("nested/dir/foo.md");
        write_file_atomic(&dest, b"hello").unwrap();
        let read = std::fs::read_to_string(&dest).unwrap();
        assert_eq!(read, "hello");
    }

    #[test]
    fn write_file_atomic_overwrites_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("foo.md");
        std::fs::write(&dest, "old").unwrap();
        write_file_atomic(&dest, b"new").unwrap();
        let read = std::fs::read_to_string(&dest).unwrap();
        assert_eq!(read, "new");
    }

    #[test]
    fn download_result_counts_total() {
        let mut r = DownloadResult::new("foo");
        r.skills_written.push("a.md".into());
        r.skills_written.push("b.md".into());
        r.prompts_written.push("p1".into());
        assert_eq!(r.total_files(), 3);
    }
}
