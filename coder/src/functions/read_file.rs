//! `coder::read-file` — return the file's content + metadata. Capped by
//! `max_read_bytes`; non-accessible paths return `C211`. The path is always
//! relative to `base_path`.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::CoderConfig;
use crate::error::{err_to_string, CoderError};
use crate::path::PathResolver;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileInput {
    /// File to read, relative to `base_path`.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReadFileOutput {
    /// The original `path` argument echoed back for caller correlation.
    pub path: String,
    /// File content as a UTF-8 string. Binary files are returned with
    /// invalid bytes replaced by U+FFFD; use a future binary-aware
    /// function if exact bytes matter.
    pub content: String,
    /// Whether `content` lost bytes to UTF-8 sanitisation.
    pub is_utf8: bool,
    pub size: u64,
    /// Unix permission bits (lower 9 bits of `st_mode`), e.g. 0o644.
    pub mode: u32,
    /// Last-modified time as a Unix epoch in seconds.
    pub mtime: i64,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: ReadFileInput,
) -> Result<ReadFileOutput, String> {
    inner(&resolver, &cfg, req).map_err(err_to_string)
}

fn inner(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    req: ReadFileInput,
) -> Result<ReadFileOutput, CoderError> {
    let abs = resolver.require_writable(&req.path)?;
    let md = std::fs::metadata(&abs)?;
    if !md.is_file() {
        return Err(CoderError::BadInput(format!(
            "not a regular file: {}",
            req.path
        )));
    }
    if md.len() > cfg.max_read_bytes {
        return Err(CoderError::TooLarge(format!(
            "{} is {} bytes; max_read_bytes is {}",
            req.path,
            md.len(),
            cfg.max_read_bytes
        )));
    }
    let bytes = std::fs::read(&abs)?;
    let (content, is_utf8) = match String::from_utf8(bytes.clone()) {
        Ok(s) => (s, true),
        Err(_) => (String::from_utf8_lossy(&bytes).into_owned(), false),
    };
    let mode = unix_mode(&md);
    let mtime = unix_mtime(&md);
    Ok(ReadFileOutput {
        path: req.path,
        content,
        is_utf8,
        size: md.len(),
        mode,
        mtime,
    })
}

#[cfg(unix)]
fn unix_mode(md: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    md.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn unix_mode(_md: &std::fs::Metadata) -> u32 {
    0o644
}

fn unix_mtime(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = CoderConfig {
            base_path: tmp.path().to_path_buf(),
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: 1024,
            ..CoderConfig::default()
        };
        let cfg = Arc::new(cfg);
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    #[tokio::test]
    async fn reads_existing_file() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("hi.txt"), b"hello").unwrap();
        let out = handle(
            r,
            c,
            ReadFileInput {
                path: "hi.txt".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content, "hello");
        assert_eq!(out.size, 5);
        assert!(out.is_utf8);
    }

    #[tokio::test]
    async fn refuses_non_accessible() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join(".env"), b"secret").unwrap();
        let err = handle(
            r,
            c,
            ReadFileInput {
                path: ".env".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("C211"), "got: {err}");
    }

    #[tokio::test]
    async fn refuses_file_above_max_read_bytes() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("big.bin"), vec![0u8; 2048]).unwrap();
        let err = handle(
            r,
            c,
            ReadFileInput {
                path: "big.bin".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("C213"), "got: {err}");
    }

    #[tokio::test]
    async fn rejects_directory_with_bad_input() {
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        let err = handle(r, c, ReadFileInput { path: "d".into() })
            .await
            .unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
    }

    #[tokio::test]
    async fn missing_file_returns_c211() {
        let (_tmp, r, c) = setup();
        let err = handle(
            r,
            c,
            ReadFileInput {
                path: "nope.txt".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("C211"), "got: {err}");
    }
}
