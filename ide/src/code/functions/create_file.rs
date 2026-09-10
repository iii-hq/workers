//! `coder::create-file` — write one or more new files. Each entry is
//! treated independently so a single bad input never aborts the rest.
//! Non-accessible paths and oversized payloads are rejected.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::code::change_journal::ChangeJournal;
use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError, WireError};
use crate::code::path::PathResolver;

// examples are wire-contract; goldens pin them.
#[derive(Debug, JsonSchema)]
#[schemars(example = "example_create_file_input")]
pub struct CreateFileInput {
    pub files: Vec<CreateFileSpec>,
    /// Internal harness filesystem scope; omitted from published schema.
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

/// Hand-rolled (not `#[serde(try_from)]`, which would swap the published
/// schema for the raw type): accepts the canonical batch AND a flat single
/// `{ path, content }` — see [`super::files_batch_or_single`].
impl<'de> Deserialize<'de> for CreateFileInput {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let (files, fs_scope) = super::files_batch_or_single(value, "coder::create-file")
            .map_err(serde::de::Error::custom)?;
        Ok(Self { files, fs_scope })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateFileSpec {
    /// File to create.
    pub path: String,
    pub content: String,
    /// Octal permission bits as a string, e.g. "0644". Defaults to "0644".
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Create missing parent directories. Defaults to true so a single
    /// `coder::create-file` call can scaffold a fresh subtree.
    #[serde(default = "default_true")]
    pub parents: bool,
    /// When false (the default), refuse to write if `path` already exists.
    #[serde(default)]
    pub overwrite: bool,
    /// Optimistic-concurrency guard for an overwrite: the `revision` from
    /// coder::read-file; if the content changed the entry fails C221 unwritten.
    #[serde(default)]
    pub expected_revision: Option<String>,
}

fn default_mode() -> String {
    "0644".to_string()
}
fn default_true() -> bool {
    true
}

// examples are wire-contract; goldens pin them.
fn example_create_file_input() -> serde_json::Value {
    serde_json::json!({
        "files": [
            {
                "path": "src/lib.rs",
                "content": "pub mod utils;\n",
                "overwrite": false
            },
            {
                "path": "/tmp/scratch/notes.md",
                "content": "# scratch notes\n",
                "overwrite": true
            }
        ]
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreateFileOutput {
    pub results: Vec<CreateFileResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreateFileResult {
    /// Canonical absolute path (resolved through the jail); the caller's
    /// input verbatim when resolution failed.
    pub path: String,
    pub success: bool,
    pub bytes_written: u64,
    /// Opaque id for the console UI to retrieve the exact before/after diff.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub change_id: Option<String>,
    /// Opaque revision for the exact bytes written. Supply this as
    /// `expected_revision` on a later overwrite to avoid lost updates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Structured error for this entry. `code` is stable for programmatic
    /// branching (e.g. `"C213"` means already-exists; pass `overwrite=true`
    /// to replace). `message` carries the corrective action an LLM agent
    /// needs to make a successful second call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

#[allow(dead_code)] // Public compatibility path used by integration callers without UI journaling.
pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: CreateFileInput,
) -> Result<CreateFileOutput, String> {
    handle_impl(resolver, cfg, None, req).await
}

pub async fn handle_with_journal(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    journal: ChangeJournal,
    req: CreateFileInput,
) -> Result<CreateFileOutput, String> {
    handle_impl(resolver, cfg, Some(journal), req).await
}

async fn handle_impl(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    journal: Option<ChangeJournal>,
    req: CreateFileInput,
) -> Result<CreateFileOutput, String> {
    if req.files.is_empty() {
        return Err(err_to_string(CoderError::BadInput(
            "`files` must not be empty".into(),
        )));
    }
    let fs_scope = req.fs_scope.as_ref();
    let mut entries = Vec::with_capacity(req.files.len());
    for spec in req.files {
        match resolver.require_writable_scope(fs_scope, &spec.path) {
            Ok(abs) => entries.push((spec, Ok(abs))),
            Err(e) if is_jail_scope_error(&e) => return Err(err_to_string(e)),
            Err(e) => entries.push((spec, Err(e))),
        }
    }
    // File hashing, syncing, and publication are blocking filesystem work.
    // Keep them off the async worker runtime so a slow disk or large target
    // cannot stall unrelated function handling.
    let results = tokio::task::spawn_blocking(move || {
        entries
            .into_iter()
            .map(|(spec, resolved)| create_one(&cfg, journal.as_ref(), spec, resolved))
            .collect()
    })
    .await
    .map_err(|e| format!("coder::create-file blocking task failed: {e}"))?;
    Ok(CreateFileOutput { results })
}

fn create_one(
    cfg: &CoderConfig,
    journal: Option<&ChangeJournal>,
    spec: CreateFileSpec,
    resolved: Result<std::path::PathBuf, CoderError>,
) -> CreateFileResult {
    // Resolve up front: from here on every filesystem operation uses ONLY
    // the resolver-returned path (never re-derived from the raw request),
    // and the result echoes that canonical absolute path. When resolution
    // fails there is no canonical path, so the input is echoed verbatim.
    let abs = match resolved {
        Ok(abs) => abs,
        Err(e) => {
            return CreateFileResult {
                path: spec.path,
                success: false,
                bytes_written: 0,
                change_id: None,
                revision: None,
                error: Some((&e).into()),
            }
        }
    };
    let wire_path = abs.display().to_string();
    match try_create_one(cfg, journal, &abs, spec) {
        Ok(written) => CreateFileResult {
            path: wire_path,
            success: true,
            bytes_written: written.bytes,
            change_id: written.change_id,
            revision: Some(written.revision),
            error: None,
        },
        Err(e) => CreateFileResult {
            path: wire_path,
            success: false,
            bytes_written: 0,
            change_id: None,
            revision: None,
            error: Some((&e).into()),
        },
    }
}

fn is_jail_scope_error(e: &CoderError) -> bool {
    matches!(
        e,
        CoderError::OutsideBase(_) | CoderError::OutsideSession(_)
    )
}

struct WriteSuccess {
    bytes: u64,
    revision: String,
    change_id: Option<String>,
}

/// Serialize publication within this worker. The revision is checked while
/// this lock is held, so two `coder::create-file` callers presenting the same
/// revision cannot both succeed. A non-cooperating external process can still
/// write at any time, so `atomic_write` repeats the check immediately before
/// the rename to make that race window as small as the filesystem permits.
static WRITE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn try_create_one(
    cfg: &CoderConfig,
    journal: Option<&ChangeJournal>,
    abs: &Path,
    spec: CreateFileSpec,
) -> Result<WriteSuccess, CoderError> {
    let bytes = spec.content.as_bytes();
    if (bytes.len() as u64) > cfg.max_write_bytes {
        return Err(CoderError::TooLarge(format!(
            "{} is {} bytes, which exceeds max_write_bytes ({}). \
             Split the content into smaller files or raise \
             max_write_bytes in coder config.",
            spec.path,
            bytes.len(),
            cfg.max_write_bytes
        )));
    }
    let mode = parse_mode(&spec.mode)?;
    if spec.expected_revision.is_some() && !spec.overwrite {
        return Err(CoderError::BadInput(format!(
            "{}: expected_revision requires overwrite=true; either enable overwrite or omit the precondition",
            spec.path
        )));
    }
    if let Some(expected) = spec.expected_revision.as_deref() {
        validate_revision(expected)?;
    }

    if spec.parents {
        if let Some(parent) = abs.parent() {
            // io_for_path names spec.path (caller-supplied, redaction-safe)
            // rather than the derived parent directory.
            std::fs::create_dir_all(parent).map_err(|e| CoderError::io_for_path(e, &spec.path))?;
        }
    }
    let before = atomic_write(
        abs,
        bytes,
        AtomicWriteOptions {
            wire_path: &spec.path,
            mode,
            overwrite: spec.overwrite,
            expected_revision: spec.expected_revision.as_deref(),
            max_read_bytes: cfg.max_read_bytes,
            capture_before: journal.is_some(),
        },
    )?;
    let change_id = before.and_then(|before| {
        journal
            .and_then(|journal| journal.record(abs.display().to_string(), before, bytes.to_vec()))
    });
    Ok(WriteSuccess {
        bytes: bytes.len() as u64,
        revision: content_revision(bytes),
        change_id,
    })
}

fn parse_mode(mode_str: &str) -> Result<u32, CoderError> {
    let digits = mode_str.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    u32::from_str_radix(digits, 8)
        .map(|mode| mode & 0o777)
        .map_err(|e| CoderError::BadInput(format!("bad mode {mode_str:?}: {e}")))
}

#[cfg(unix)]
fn apply_mode(path: &Path, mode: u32, wire_path: &str) -> Result<(), CoderError> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(mode & 0o777);
    std::fs::set_permissions(path, perms).map_err(|e| CoderError::io_for_path(e, wire_path))
}

#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode: u32, _wire_path: &str) -> Result<(), CoderError> {
    Ok(())
}

/// Strong content identity shared with `coder::read-file`. The algorithm
/// prefix keeps this opaque token forward-compatible if the digest changes.
pub(crate) fn content_revision(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn validate_revision(revision: &str) -> Result<(), CoderError> {
    let Some(hex) = revision.strip_prefix("sha256:") else {
        return Err(CoderError::BadInput(
            "expected_revision must be the opaque sha256:<64 lowercase hex> token returned by coder::read-file"
                .into(),
        ));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(CoderError::BadInput(
            "expected_revision must be the opaque sha256:<64 lowercase hex> token returned by coder::read-file"
                .into(),
        ));
    }
    Ok(())
}

fn file_revision(path: &Path, wire_path: &str, max_read_bytes: u64) -> Result<String, CoderError> {
    let mut file = std::fs::File::open(path).map_err(|e| CoderError::io_for_path(e, wire_path))?;
    let size = file
        .metadata()
        .map_err(|e| CoderError::io_for_path(e, wire_path))?
        .len();
    if size > max_read_bytes {
        return Err(conflict(wire_path));
    }
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|e| CoderError::io_for_path(e, wire_path))?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > max_read_bytes {
            // The file grew after metadata was read. A legal full read could
            // not have produced the caller's revision, so fail closed.
            return Err(conflict(wire_path));
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn verify_expected_revision(
    path: &Path,
    wire_path: &str,
    expected: &str,
    max_read_bytes: u64,
) -> Result<(), CoderError> {
    let actual = match file_revision(path, wire_path, max_read_bytes) {
        Ok(revision) => revision,
        Err(CoderError::NotFoundOrDenied(_)) => {
            return Err(conflict(wire_path));
        }
        Err(other) => return Err(other),
    };
    if actual != expected {
        return Err(conflict(wire_path));
    }
    Ok(())
}

fn conflict(wire_path: &str) -> CoderError {
    CoderError::Conflict(format!(
        "{wire_path} changed since it was read; no bytes were written. Reload it with coder::read-file and retry using the returned revision, or omit expected_revision only after explicitly choosing to overwrite the newer content."
    ))
}

struct TempGuard(PathBuf);

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct AtomicWriteOptions<'a> {
    wire_path: &'a str,
    mode: u32,
    overwrite: bool,
    expected_revision: Option<&'a str>,
    max_read_bytes: u64,
    capture_before: bool,
}

/// Publish a complete file through a permissioned sibling temp and rename.
/// The original remains intact if writing, syncing, chmod, or the optimistic
/// recheck fails. Sibling placement guarantees rename stays on one filesystem.
fn atomic_write(
    target: &Path,
    bytes: &[u8],
    options: AtomicWriteOptions<'_>,
) -> Result<Option<Vec<u8>>, CoderError> {
    let AtomicWriteOptions {
        wire_path,
        mode,
        overwrite,
        expected_revision,
        max_read_bytes,
        capture_before,
    } = options;
    let parent = target
        .parent()
        .ok_or_else(|| CoderError::Io(format!("{wire_path}: target has no parent directory")))?;
    let file_name = target
        .file_name()
        .ok_or_else(|| CoderError::BadInput(format!("{wire_path}: target must name a file")))?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(".coder-tmp-{}-{sequence}", std::process::id()));
    let temp_path = parent.join(temp_name);
    let guard = TempGuard(temp_path.clone());

    let mut temp = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .map_err(|e| CoderError::io_for_path(e, wire_path))?;
    temp.write_all(bytes)
        .map_err(|e| CoderError::io_for_path(e, wire_path))?;
    temp.flush()
        .map_err(|e| CoderError::io_for_path(e, wire_path))?;
    temp.sync_all()
        .map_err(|e| CoderError::io_for_path(e, wire_path))?;
    drop(temp);
    apply_mode(&temp_path, mode, wire_path)?;

    // Only the final optimistic check and rename need serialization. Temp
    // creation, writes, chmod, and fsync above can proceed concurrently.
    let _write_guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(expected) = expected_revision {
        verify_expected_revision(target, wire_path, expected, max_read_bytes)?;
    } else if !overwrite && target.exists() {
        return Err(CoderError::AlreadyExists(format!(
            "{wire_path} already exists; pass overwrite=true to replace"
        )));
    }

    // Capture the exact pre-publication body while cooperative writers are
    // serialized. Journaling is best-effort and bounded by the normal read
    // limit, so an oversized or transiently unreadable target never blocks a
    // successful write merely for the console artifact.
    let before = if capture_before {
        match std::fs::metadata(target) {
            Ok(metadata) if metadata.is_file() && metadata.len() <= max_read_bytes => {
                std::fs::read(target).ok()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(Vec::new()),
            _ => None,
        }
    } else {
        None
    };

    std::fs::rename(&temp_path, target).map_err(|e| CoderError::io_for_path(e, wire_path))?;
    drop(guard);
    if let Ok(parent_dir) = std::fs::File::open(parent) {
        let _ = parent_dir.sync_all();
    }
    Ok(before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// The canonical batch, a flat single `{ path, content }`, and garbage:
    /// the first two deserialize to the same request; the third names the
    /// contract instead of a raw serde "missing field `files`"
    /// (verify-wake-fix-3 postmortem).
    #[test]
    fn input_accepts_flat_single_file_and_names_the_contract_on_garbage() {
        let batch: CreateFileInput = serde_json::from_value(serde_json::json!({
            "files": [{ "path": "a.txt", "content": "hi" }]
        }))
        .unwrap();
        assert_eq!(batch.files.len(), 1);

        let flat: CreateFileInput = serde_json::from_value(serde_json::json!({
            "path": "a.txt", "content": "hi", "overwrite": true
        }))
        .unwrap();
        assert_eq!(flat.files.len(), 1);
        assert_eq!(flat.files[0].path, "a.txt");
        assert!(flat.files[0].overwrite);

        let err = serde_json::from_value::<CreateFileInput>(serde_json::json!({
            "file": "a.txt"
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("coder::create-file takes"), "got: {err}");
        assert!(err.contains("\"files\""), "got: {err}");
    }

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: 1024 * 1024,
            max_write_bytes: 1024 * 1024,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    #[tokio::test]
    async fn creates_simple_file() {
        let (tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a.txt".into(),
                    content: "hello".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: false,
                    expected_revision: None,
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert_eq!(out.results[0].bytes_written, 5);
        assert_eq!(
            out.results[0].revision.as_deref(),
            Some(content_revision(b"hello").as_str())
        );
        // Successful entries echo the canonical absolute path.
        assert_eq!(
            out.results[0].path,
            std::fs::canonicalize(tmp.path())
                .unwrap()
                .join("a.txt")
                .display()
                .to_string()
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn creates_with_parents() {
        let (tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a/b/c.txt".into(),
                    content: "hi".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: false,
                    expected_revision: None,
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success, "{:?}", out.results[0].error);
        assert!(tmp.path().join("a/b/c.txt").exists());
    }

    #[tokio::test]
    async fn rejects_existing_without_overwrite() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "old").unwrap();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a.txt".into(),
                    content: "new".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: false,
                    expected_revision: None,
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        let err = out.results[0].error.as_ref().unwrap();
        assert_eq!(err.code, "C213");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "old"
        );
    }

    #[tokio::test]
    async fn overwrite_replaces_existing() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "old").unwrap();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a.txt".into(),
                    content: "new".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: true,
                    expected_revision: None,
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "new"
        );
    }

    #[tokio::test]
    async fn matching_revision_overwrites_and_returns_the_new_revision() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "old").unwrap();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a.txt".into(),
                    content: "new".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: true,
                    expected_revision: Some(content_revision(b"old")),
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success, "{:?}", out.results[0].error);
        assert_eq!(
            out.results[0].revision.as_deref(),
            Some(content_revision(b"new").as_str())
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "new"
        );
    }

    #[tokio::test]
    async fn stale_revision_conflicts_without_overwriting() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "agent changed it").unwrap();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a.txt".into(),
                    content: "my draft".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: true,
                    expected_revision: Some(content_revision(b"old")),
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C221");
        assert_eq!(out.results[0].revision, None);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "agent changed it"
        );
    }

    #[tokio::test]
    async fn oversized_current_file_conflicts_without_overwriting() {
        let (tmp, r, _c) = setup();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec![],
            max_read_bytes: 4,
            max_write_bytes: 1024,
            ..CoderConfig::default()
        });
        std::fs::write(tmp.path().join("a.txt"), "external content is too large").unwrap();

        let out = handle(
            r,
            cfg,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a.txt".into(),
                    content: "mine".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: true,
                    expected_revision: Some(content_revision(b"old")),
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();

        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C221");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "external content is too large"
        );
    }

    #[test]
    fn concurrent_writers_cannot_both_use_the_same_revision() {
        let (tmp, _r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "old").unwrap();
        let expected = content_revision(b"old");
        let target = std::fs::canonicalize(tmp.path()).unwrap().join("a.txt");
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let writers = ["first", "second"].map(|content| {
            let barrier = barrier.clone();
            let cfg = c.clone();
            let target = target.clone();
            let expected = expected.clone();
            std::thread::spawn(move || {
                barrier.wait();
                try_create_one(
                    &cfg,
                    None,
                    &target,
                    CreateFileSpec {
                        path: "a.txt".into(),
                        content: content.into(),
                        mode: "0644".into(),
                        parents: true,
                        overwrite: true,
                        expected_revision: Some(expected),
                    },
                )
            })
        });
        barrier.wait();
        let results = writers.map(|writer| writer.join().unwrap());

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .filter(|error| error.code() == "C221")
                .count(),
            1
        );
        let final_content = std::fs::read_to_string(target).unwrap();
        assert!(final_content == "first" || final_content == "second");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn atomic_overwrite_publishes_the_requested_mode() {
        use std::os::unix::fs::PermissionsExt;

        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "old").unwrap();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a.txt".into(),
                    content: "new".into(),
                    mode: "0600".into(),
                    parents: true,
                    overwrite: true,
                    expected_revision: None,
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        let mode = std::fs::metadata(tmp.path().join("a.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn failed_atomic_publish_keeps_original_and_cleans_the_temp() {
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join("target")).unwrap();
        std::fs::write(tmp.path().join("target/child.txt"), "keep").unwrap();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "target".into(),
                    content: "replacement".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: true,
                    expected_revision: None,
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("target/child.txt")).unwrap(),
            "keep"
        );
        let names = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            names.iter().all(|name| !name.contains(".coder-tmp-")),
            "orphan temp file after failed publish: {names:?}"
        );
    }

    #[tokio::test]
    async fn refuses_non_accessible() {
        let (_tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: ".env".into(),
                    content: "secret".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: true,
                    expected_revision: None,
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C211");
    }

    #[tokio::test]
    async fn jail_escape_aborts_batch_before_any_write() {
        // Jail-scope failures must be top-level call errors so the harness
        // post-trigger approval hook can see the C215/C220 and hold the call.
        // Preflight all paths before I/O: a later escape must not leave earlier
        // entries partially written before the call is re-invoked after a grant.
        let (tmp, r, c) = setup();
        let err = handle(
            r,
            c,
            CreateFileInput {
                files: vec![
                    CreateFileSpec {
                        path: "ok.txt".into(),
                        content: "y".into(),
                        mode: "0644".into(),
                        parents: true,
                        overwrite: false,
                        expected_revision: None,
                    },
                    CreateFileSpec {
                        path: "../escape.txt".into(),
                        content: "x".into(),
                        mode: "0644".into(),
                        parents: true,
                        overwrite: false,
                        expected_revision: None,
                    },
                ],
                fs_scope: None,
            },
        )
        .await
        .unwrap_err();
        let wire: serde_json::Value = serde_json::from_str(&err).unwrap();
        assert_eq!(wire["code"], "C215");
        assert!(!tmp.path().join("ok.txt").exists());
        assert!(
            !tmp.path().join("../escape.txt").exists(),
            "the escaping path must never be created"
        );
    }

    #[tokio::test]
    async fn refuses_oversize() {
        let (_tmp, r, _c) = setup();
        let small_cfg = Arc::new(CoderConfig {
            base_paths: vec![_tmp.path().to_path_buf()],
            non_accessible_globs: vec![],
            max_write_bytes: 4,
            ..CoderConfig::default()
        });
        let out = handle(
            r,
            small_cfg,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "big.txt".into(),
                    content: "abcdefg".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: false,
                    expected_revision: None,
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C218");
    }

    #[tokio::test]
    async fn multi_file_partial_success() {
        let (tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![
                    CreateFileSpec {
                        path: ".env".into(),
                        content: "x".into(),
                        mode: "0644".into(),
                        parents: true,
                        overwrite: false,
                        expected_revision: None,
                    },
                    CreateFileSpec {
                        path: "ok.txt".into(),
                        content: "y".into(),
                        mode: "0644".into(),
                        parents: true,
                        overwrite: false,
                        expected_revision: None,
                    },
                ],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.results.len(), 2);
        assert!(!out.results[0].success);
        assert!(out.results[1].success);
        assert!(tmp.path().join("ok.txt").exists());
    }

    /// The per-entry `error` field must serialize as a raw JSON object —
    /// NOT a JSON string containing escaped JSON. An LLM agent reading
    /// `"code":"C2` directly as an object key requires no mental
    /// unescaping; the old wire shape `\"code\":\"C2` was a double-encode.
    #[tokio::test]
    async fn error_field_serializes_as_structured_object_not_escaped_string() {
        let (_tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: ".env".into(),
                    content: "x".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: false,
                    expected_revision: None,
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let serialized = serde_json::to_string(&out.results[0]).unwrap();
        // Structured object key must appear raw.
        assert!(
            serialized.contains(r#""code":"C2"#),
            "expected raw object key; got: {serialized}"
        );
        // Double-encoded form must NOT appear.
        assert!(
            !serialized.contains(r#"\"code\""#),
            "double-encoded JSON detected; got: {serialized}"
        );
    }
}
