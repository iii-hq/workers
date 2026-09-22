//! File-backed store for the console workspace layout — the tab strip, its
//! panes and the active-tab pointer.
//!
//! The layout is ephemeral per-instance state: it changes on every tab
//! switch, split and divider drag, so it does not belong in the `console`
//! configuration entry (whose YAML is meant to be committed to VCS). It lives
//! in `<data_dir>/workspace.json` instead, where `data_dir` is the
//! configuration's `data_dir` (default `data/ade`, resolved against the
//! Compose project directory like every other worker's data directory).
//!
//! The directory can be re-pointed live (`configuration:updated`); a new
//! location that has no file yet simply starts from the default layout.
//! Writes are atomic (temp file + rename) so a browser polling the file
//! through `console::workspace::get` never observes a half-written document.

use std::path::PathBuf;

use serde_json::Value;
use tokio::sync::{Mutex, MutexGuard, RwLock};

/// File name inside `data_dir` holding the layout document.
pub const WORKSPACE_FILE: &str = "workspace.json";

/// Portable default for `data_dir`, relative to the Compose project
/// directory. Resolve with [`iii_worker_paths::resolve_path`] before use.
pub fn default_data_dir() -> String {
    iii_worker_paths::default_path("data/ade")
}

#[derive(Debug)]
pub struct WorkspaceStore {
    dir: RwLock<PathBuf>,
    /// Serializes every read-modify-write of the document (SPA `set`,
    /// `open`, `close`) so two writers cannot interleave a stale copy.
    write_lock: Mutex<()>,
}

impl WorkspaceStore {
    /// `dir` is the RESOLVED data directory (see `iii_worker_paths`).
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir: RwLock::new(dir),
            write_lock: Mutex::new(()),
        }
    }

    pub async fn dir(&self) -> PathBuf {
        self.dir.read().await.clone()
    }

    /// Re-point the store. Returns `true` when the directory actually changed.
    pub async fn set_dir(&self, dir: PathBuf) -> bool {
        let mut current = self.dir.write().await;
        if *current == dir {
            return false;
        }
        *current = dir;
        true
    }

    pub async fn path(&self) -> PathBuf {
        self.dir.read().await.join(WORKSPACE_FILE)
    }

    /// Hold this across a `load` → mutate → `save` cycle.
    pub async fn lock(&self) -> MutexGuard<'_, ()> {
        self.write_lock.lock().await
    }

    pub async fn exists(&self) -> bool {
        tokio::fs::metadata(self.path().await)
            .await
            .map(|meta| meta.is_file())
            .unwrap_or(false)
    }

    /// `Ok(None)` when the file does not exist yet. A file that is not a JSON
    /// object (hand-edited, truncated) is reported and treated as absent: the
    /// layout is disposable UI state, and the next write replaces it, so a
    /// bad file must never wedge every tab operation until someone deletes it.
    pub async fn load(&self) -> Result<Option<Value>, String> {
        let path = self.path().await;
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "cannot read workspace layout {}: {error}",
                    path.display()
                ))
            }
        };
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(map)) => Ok(Some(Value::Object(map))),
            Ok(_) => {
                tracing::warn!(
                    path = %path.display(),
                    "workspace layout is not a JSON object; starting from the default layout"
                );
                Ok(None)
            }
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "workspace layout is not valid JSON; starting from the default layout"
                );
                Ok(None)
            }
        }
    }

    /// Replace the document wholesale. Creates `data_dir` on first use and
    /// writes through a temp file + rename so readers see old or new, never
    /// a torn file.
    pub async fn save(&self, value: &Value) -> Result<(), String> {
        let dir = self.dir().await;
        let path = dir.join(WORKSPACE_FILE);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|error| format!("cannot create data_dir {}: {error}", dir.display()))?;
        let body = serde_json::to_vec_pretty(value)
            .map_err(|error| format!("cannot serialize workspace layout: {error}"))?;
        let tmp = dir.join(format!("{WORKSPACE_FILE}.{}.tmp", std::process::id()));
        tokio::fs::write(&tmp, &body)
            .await
            .map_err(|error| format!("cannot write {}: {error}", tmp.display()))?;
        if let Err(error) = tokio::fs::rename(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(format!(
                "cannot move {} into place at {}: {error}",
                tmp.display(),
                path.display()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ade-workspace-store-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn default_data_dir_is_portable() {
        assert_eq!(default_data_dir(), "data/ade");
    }

    #[tokio::test]
    async fn missing_file_is_absent_not_an_error() {
        let store = WorkspaceStore::new(scratch_dir("missing"));
        assert!(!store.exists().await);
        assert_eq!(store.load().await.unwrap(), None);
    }

    #[tokio::test]
    async fn save_creates_the_directory_and_round_trips() {
        let dir = scratch_dir("roundtrip").join("nested");
        let store = WorkspaceStore::new(dir.clone());
        let doc = json!({ "tabs": [{ "id": "a", "screens": ["chat"] }], "activeTabId": "a" });
        store.save(&doc).await.unwrap();
        assert!(store.exists().await);
        assert_eq!(store.path().await, dir.join(WORKSPACE_FILE));
        assert_eq!(store.load().await.unwrap(), Some(doc));
        // No temp file is left behind.
        let mut entries = tokio::fs::read_dir(&dir).await.unwrap();
        let mut names = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        assert_eq!(names, vec![WORKSPACE_FILE.to_string()]);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[tokio::test]
    async fn corrupt_or_non_object_files_read_as_absent() {
        let dir = scratch_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let store = WorkspaceStore::new(dir.clone());
        std::fs::write(dir.join(WORKSPACE_FILE), b"{ not json").unwrap();
        assert_eq!(store.load().await.unwrap(), None);
        std::fs::write(dir.join(WORKSPACE_FILE), b"[1, 2]").unwrap();
        assert_eq!(store.load().await.unwrap(), None);
        // A later save replaces the bad file.
        store.save(&json!({ "tabs": [] })).await.unwrap();
        assert_eq!(store.load().await.unwrap(), Some(json!({ "tabs": [] })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn set_dir_reports_changes_and_repoints_reads() {
        let first = scratch_dir("first");
        let second = scratch_dir("second");
        let store = WorkspaceStore::new(first.clone());
        store
            .save(&json!({ "tabs": [], "activeTabId": "one" }))
            .await
            .unwrap();
        assert!(!store.set_dir(first.clone()).await);
        assert!(store.set_dir(second.clone()).await);
        assert_eq!(store.dir().await, second);
        // The new location starts empty; the old file is untouched.
        assert_eq!(store.load().await.unwrap(), None);
        assert!(first.join(WORKSPACE_FILE).is_file());
        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }
}
