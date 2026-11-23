//! Small durable JSON documents the harness owns on disk (the saved-folder
//! catalogue and the projects store): path resolution, reads that treat a
//! missing file as empty, and atomic replacement (write a sibling temporary
//! file, then rename it over the destination), so a reader never sees a
//! partial document.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Resolve a configured document path like every other worker path:
/// absolute and `~/` paths are kept, relative ones resolve under
/// `III_COMPOSE_DIR` (or the process cwd outside Compose). `key` names the
/// configuration field in the error.
pub fn storage_path(raw: &str, key: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{key} must not be empty"));
    }
    let path = iii_worker_paths::resolve_path(trimmed);
    if path.file_name().is_none() {
        return Err(format!("{key} must name a file: {}", path.display()));
    }
    Ok(path)
}

/// The document's bytes, or `None` when the file does not exist.
pub async fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("read {}: {error}", path.display())),
    }
}

/// Write `value` as pretty JSON and replace `path` atomically.
pub async fn persist<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("serialize: {error}"))?;
    bytes.push(b'\n');
    persist_bytes(path, &bytes).await
}

/// Replace `path` atomically with `bytes`, creating its folder when needed.
pub async fn persist_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("create {}: {error}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("harness.json");
    let temporary = parent.join(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    if let Err(error) = tokio::fs::write(&temporary, bytes).await {
        return Err(format!("write {}: {error}", temporary.display()));
    }
    if let Err(error) = tokio::fs::rename(&temporary, path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(format!("replace {}: {error}", path.display()));
    }
    Ok(())
}

/// Move `from` to `to`: a rename, or a copy then a removal when the two sit on
/// different filesystems. The destination is written atomically either way.
pub async fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    if tokio::fs::rename(from, to).await.is_ok() {
        return Ok(());
    }
    let bytes = tokio::fs::read(from)
        .await
        .map_err(|error| format!("read {}: {error}", from.display()))?;
    persist_bytes(to, &bytes).await?;
    tokio::fs::remove_file(from)
        .await
        .map_err(|error| format!("remove {}: {error}", from.display()))
}
