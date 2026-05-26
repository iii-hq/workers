//! In-process registration of the `coder` function surface against the
//! shared SDK handle. Re-uses the production entry point
//! `coder::functions::register_all` so scenarios exercise identical
//! code paths. Registration is idempotent (`OnceCell`); we wipe the
//! per-test `base_path` between scenarios so leftover files from one
//! scenario don't pollute the next.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use iii_sdk::III;
use tokio::sync::OnceCell;

use coder::config::CoderConfig;
use coder::functions;
use coder::path::PathResolver;

/// Caps used by the shared in-process worker. Small enough that a few
/// kilobytes of fixture content triggers `C213` for oversize scenarios,
/// large enough that ordinary scenario fixtures (line-oriented text
/// files) fit comfortably.
const TEST_MAX_READ_BYTES: u64 = 4_096;
const TEST_MAX_WRITE_BYTES: u64 = 4_096;
/// Small enough that pagination scenarios can fill a page without
/// writing hundreds of fixtures.
const TEST_LIST_DEFAULT_PAGE_SIZE: u32 = 5;
const TEST_LIST_MAX_PAGE_SIZE: u32 = 100;

pub struct Shared {
    pub cfg: Arc<CoderConfig>,
    /// Canonicalised `base_path` the worker reads + writes against.
    /// Fixture writes from step defs go here.
    pub base_path: PathBuf,
}

static SHARED: OnceCell<Arc<Shared>> = OnceCell::const_new();

/// Idempotent: the first caller registers; subsequent callers reuse.
pub async fn register_all(iii: &Arc<III>) -> Result<Arc<Shared>> {
    if let Some(s) = SHARED.get() {
        return Ok(s.clone());
    }

    // Leak the tempdir so it lives for the lifetime of the test binary.
    let tmp = tempfile::tempdir()?;
    let base_path = tmp.keep();
    std::fs::create_dir_all(&base_path)?;
    // Canonicalise so the recorded path matches what `PathResolver`
    // sees after its own canonicalisation. On macOS `/var/folders/...`
    // and `/private/var/folders/...` are aliases; using the canonical
    // form makes fixture writes from steps line up with handler reads.
    let base_path = std::fs::canonicalize(&base_path)?;

    let cfg = Arc::new(CoderConfig {
        base_path: base_path.clone(),
        non_accessible_globs: vec!["**/.env".to_string(), "**/*.pem".to_string()],
        max_read_bytes: TEST_MAX_READ_BYTES,
        max_write_bytes: TEST_MAX_WRITE_BYTES,
        list_default_page_size: TEST_LIST_DEFAULT_PAGE_SIZE,
        list_max_page_size: TEST_LIST_MAX_PAGE_SIZE,
        ..CoderConfig::default()
    });
    let resolver = Arc::new(PathResolver::new(&cfg)?);

    functions::register_all(iii, resolver, cfg.clone());

    // Give the SDK a beat to publish the function registrations before
    // scenarios start triggering them.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let shared = Arc::new(Shared { cfg, base_path });
    let _ = SHARED.set(shared.clone());
    Ok(shared)
}

pub fn shared() -> Option<Arc<Shared>> {
    SHARED.get().cloned()
}

/// Wipe the per-test `base_path` between scenarios so leftover fixtures
/// don't leak. We clear the directory's contents rather than removing
/// the directory itself — `PathResolver` cached its canonical path at
/// startup and we want subsequent fixture writes to land at the same
/// inode.
pub fn reset_fs(base_path: &Path) {
    let Ok(entries) = std::fs::read_dir(base_path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let md = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let _ = if md.file_type().is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
    }
}
