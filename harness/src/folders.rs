//! Harness-owned durable catalogue of saved folders (`harness::folders`).
//!
//! Until chat-projects phase 1 this catalogue was `harness::projects` and
//! lived at `projects_file_path`. It keeps its records (`{path, name,
//! last_used_at}`) under its new name (D25), and moves to
//! `folders_file_path` once, at start ([`migrate`]). Writes are serialized
//! inside the harness process and replace the destination atomically, so two
//! console windows cannot lose one another's updates or observe partial JSON.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::error::HarnessError;
use crate::json_file;
use crate::types::message::AgentMessage;

const CATALOG_VERSION: u32 = 1;

/// One saved folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SavedFolder {
    pub path: String,
    pub name: String,
    pub last_used_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct FolderCatalog {
    #[serde(default = "catalog_version")]
    version: u32,
    /// `projects` is the key the catalogue had before the rename.
    #[serde(default, alias = "projects")]
    folders: Vec<SavedFolder>,
}

impl Default for FolderCatalog {
    fn default() -> Self {
        Self {
            version: CATALOG_VERSION,
            folders: Vec::new(),
        }
    }
}

fn catalog_version() -> u32 {
    CATALOG_VERSION
}

fn store_error(message: String) -> HarnessError {
    HarnessError::FolderStore(message)
}

#[derive(Clone, Default)]
pub struct FolderStore {
    gate: Arc<Mutex<()>>,
}

impl FolderStore {
    pub async fn list(&self, file_path: &str) -> Result<Vec<SavedFolder>, HarnessError> {
        let path = storage_path(file_path)?;
        let _guard = self.gate.lock().await;
        let mut catalog = load_catalog(&path).await?;
        sort_folders(&mut catalog.folders);
        Ok(catalog.folders)
    }

    pub async fn upsert(
        &self,
        file_path: &str,
        raw_folder_path: &str,
        requested_name: Option<&str>,
    ) -> Result<SavedFolder, HarnessError> {
        let path = storage_path(file_path)?;
        let folder_path = normalize_path(raw_folder_path)?;
        let _guard = self.gate.lock().await;
        let mut catalog = load_catalog(&path).await?;
        let existing_name = catalog
            .folders
            .iter()
            .find(|folder| folder.path == folder_path)
            .map(|folder| folder.name.as_str());
        let folder = SavedFolder {
            name: normalized_name(&folder_path, requested_name, existing_name),
            path: folder_path.clone(),
            last_used_at: AgentMessage::now_ms(),
        };
        if let Some(existing) = catalog
            .folders
            .iter_mut()
            .find(|candidate| candidate.path == folder_path)
        {
            *existing = folder.clone();
        } else {
            catalog.folders.push(folder.clone());
        }
        sort_folders(&mut catalog.folders);
        persist_catalog(&path, &catalog).await?;
        Ok(folder)
    }

    pub async fn delete(
        &self,
        file_path: &str,
        raw_folder_path: &str,
    ) -> Result<bool, HarnessError> {
        let path = storage_path(file_path)?;
        let folder_path = normalize_path(raw_folder_path)?;
        let _guard = self.gate.lock().await;
        let mut catalog = load_catalog(&path).await?;
        let previous_len = catalog.folders.len();
        catalog.folders.retain(|folder| folder.path != folder_path);
        if catalog.folders.len() == previous_len {
            return Ok(false);
        }
        persist_catalog(&path, &catalog).await?;
        Ok(true)
    }
}

pub fn normalize_path(raw: &str) -> Result<String, HarnessError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HarnessError::InvalidRequest(
            "folder path must not be empty".to_string(),
        ));
    }
    if !Path::new(trimmed).is_absolute() {
        return Err(HarnessError::InvalidRequest(format!(
            "folder path must be absolute: {trimmed}"
        )));
    }
    let normalized = if trimmed == "/" {
        "/".to_string()
    } else {
        trimmed.trim_end_matches('/').to_string()
    };
    Ok(normalized)
}

pub fn default_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

pub fn normalized_name(path: &str, requested: Option<&str>, current: Option<&str>) -> String {
    match requested {
        Some(name) if !name.trim().is_empty() => name.trim().to_string(),
        Some(_) => default_name(path),
        None => current
            .filter(|name| !name.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| default_name(path)),
    }
}

fn storage_path(raw: &str) -> Result<PathBuf, HarnessError> {
    json_file::storage_path(raw, "folders_file_path").map_err(store_error)
}

async fn load_catalog(path: &Path) -> Result<FolderCatalog, HarnessError> {
    let Some(bytes) = json_file::read(path).await.map_err(store_error)? else {
        return Ok(FolderCatalog::default());
    };
    let catalog: FolderCatalog = serde_json::from_slice(&bytes)
        .map_err(|error| store_error(format!("parse {}: {error}", path.display())))?;
    if catalog.version != CATALOG_VERSION {
        return Err(store_error(format!(
            "unsupported catalog version {} in {}; expected {CATALOG_VERSION}",
            catalog.version,
            path.display()
        )));
    }
    Ok(catalog)
}

async fn persist_catalog(path: &Path, catalog: &FolderCatalog) -> Result<(), HarnessError> {
    json_file::persist(path, catalog).await.map_err(store_error)
}

fn sort_folders(folders: &mut [SavedFolder]) {
    folders.sort_by(|a, b| {
        b.last_used_at
            .cmp(&a.last_used_at)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    });
}

// ── the move at start ──

/// What [`migrate`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migration {
    /// The old catalogue moved from the projects path to the folders path.
    Moved { from: PathBuf, to: PathBuf },
    /// The folders file already exists; nothing moved.
    AlreadyMoved,
    /// The folders file exists and the projects path still held the old
    /// catalogue too: that copy was set aside so the projects store can start.
    SetAside { from: PathBuf, to: PathBuf },
    /// Nothing to move: no file at the projects path, or it already holds the
    /// projects store (`version: 2`), or a document this harness does not
    /// recognise (left untouched).
    NothingToMove,
}

/// Whether a document at `projects_file_path` is the folder catalogue of
/// before the rename: an object without `version: 2` whose `projects` (or
/// `folders`) entries carry a `path`.
pub fn is_legacy_catalog(document: &Value) -> bool {
    let Some(object) = document.as_object() else {
        return false;
    };
    if object.get("version").and_then(Value::as_u64) == Some(2) {
        return false;
    }
    let entries = object
        .get("projects")
        .or_else(|| object.get("folders"))
        .and_then(Value::as_array);
    match entries {
        Some(entries) => entries
            .iter()
            .all(|entry| entry.get("path").and_then(Value::as_str).is_some()),
        None => false,
    }
}

async fn holds_legacy_catalog(path: &Path) -> Result<bool, HarnessError> {
    let Some(bytes) = json_file::read(path).await.map_err(store_error)? else {
        return Ok(false);
    };
    Ok(serde_json::from_slice::<Value>(&bytes)
        .map(|document| is_legacy_catalog(&document))
        .unwrap_or(false))
}

/// The folder catalogue's one move at start (D25): when the folders file does
/// not exist and the file at `projects_path` holds the old catalogue, it moves
/// to `folders_path`. `projects_overridden` says the operator set
/// `projects_file_path` themselves; that path is honoured for this one move
/// and the setting is logged as deprecated for the catalogue.
pub async fn migrate(
    folders_path: &Path,
    projects_path: &Path,
    projects_overridden: bool,
) -> Result<Migration, HarnessError> {
    let legacy = holds_legacy_catalog(projects_path).await?;
    let folders_exist = json_file::read(folders_path)
        .await
        .map_err(store_error)?
        .is_some();
    if folders_exist {
        if !legacy {
            return Ok(Migration::AlreadyMoved);
        }
        let aside =
            projects_path.with_extension(format!("folders-v1.{}.bak", AgentMessage::now_ms()));
        json_file::move_file(projects_path, &aside)
            .await
            .map_err(store_error)?;
        tracing::warn!(
            from = %projects_path.display(),
            to = %aside.display(),
            "the folder catalogue was already moved to folders_file_path; its old copy at \
             projects_file_path was set aside so the projects store can start there"
        );
        return Ok(Migration::SetAside {
            from: projects_path.to_path_buf(),
            to: aside,
        });
    }
    if !legacy {
        return Ok(Migration::NothingToMove);
    }
    json_file::move_file(projects_path, folders_path)
        .await
        .map_err(store_error)?;
    if projects_overridden {
        tracing::warn!(
            from = %projects_path.display(),
            to = %folders_path.display(),
            "projects_file_path pointed at the saved-folder catalogue; it moved to \
             folders_file_path. Using projects_file_path for the folder catalogue is deprecated: \
             it now names the harness::projects store"
        );
    } else {
        tracing::info!(
            from = %projects_path.display(),
            to = %folders_path.display(),
            "moved the saved-folder catalogue to folders_file_path (harness::folders)"
        );
    }
    Ok(Migration::Moved {
        from: projects_path.to_path_buf(),
        to: folders_path.to_path_buf(),
    })
}

/// [`migrate`] for the live configuration, at start. A failure is logged and
/// the harness starts anyway: both stores then report their own errors.
pub async fn migrate_at_start(cfg: &crate::config::WorkerConfig) -> Option<Migration> {
    let folders = match storage_path(&cfg.folders_file_path) {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(%error, "folder catalogue move skipped");
            return None;
        }
    };
    let projects = match json_file::storage_path(&cfg.projects_file_path, "projects_file_path") {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(%error, "folder catalogue move skipped");
            return None;
        }
    };
    let overridden = cfg.projects_file_path != crate::config::default_projects_file_path();
    match migrate(&folders, &projects, overridden).await {
        Ok(outcome) => Some(outcome),
        Err(error) => {
            tracing::warn!(%error, "folder catalogue move failed; harness::folders starts empty");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_name_is_the_folder_not_the_absolute_path() {
        assert_eq!(default_name("/Users/me/workspaces/harness"), "harness");
        assert_eq!(default_name("/"), "/");
    }

    #[test]
    fn omitted_name_preserves_a_custom_name_and_blank_resets_it() {
        assert_eq!(
            normalized_name("/work/harness", None, Some("Agent runtime")),
            "Agent runtime"
        );
        assert_eq!(
            normalized_name("/work/harness", Some("  "), Some("Agent runtime")),
            "harness"
        );
        assert_eq!(
            normalized_name("/work/harness", Some("  Harness core  "), None),
            "Harness core"
        );
    }

    #[test]
    fn storage_path_expands_home_and_keeps_absolute_paths() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            storage_path("~/.iii/data/harness/folders.json").unwrap(),
            PathBuf::from(home).join(".iii/data/harness/folders.json")
        );
        assert_eq!(
            storage_path("/var/lib/iii/folders.json").unwrap(),
            PathBuf::from("/var/lib/iii/folders.json")
        );
    }

    #[test]
    fn paths_must_be_absolute_and_trailing_slashes_are_normalized() {
        assert!(normalize_path("relative/project").is_err());
        assert_eq!(
            normalize_path(" /work/harness/// ").unwrap(),
            "/work/harness"
        );
        assert_eq!(normalize_path("/").unwrap(), "/");
    }

    #[tokio::test]
    async fn catalog_round_trips_custom_names_through_its_own_file() {
        let root = tempfile::tempdir().unwrap();
        let catalog_path = root.path().join("custom/catalog.json");
        let folder_path = root.path().join("my-project");
        let store = FolderStore::default();

        let created = store
            .upsert(
                catalog_path.to_str().unwrap(),
                folder_path.to_str().unwrap(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(created.name, "my-project");

        store
            .upsert(
                catalog_path.to_str().unwrap(),
                folder_path.to_str().unwrap(),
                Some("Runtime"),
            )
            .await
            .unwrap();
        let touched = store
            .upsert(
                catalog_path.to_str().unwrap(),
                folder_path.to_str().unwrap(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(touched.name, "Runtime");

        let restarted = FolderStore::default();
        assert_eq!(
            restarted
                .list(catalog_path.to_str().unwrap())
                .await
                .unwrap(),
            vec![touched]
        );
        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(catalog_path).unwrap()).unwrap();
        assert_eq!(document["version"], CATALOG_VERSION);
        assert_eq!(document["folders"][0]["name"], "Runtime");
    }

    #[tokio::test]
    async fn concurrent_updates_share_one_complete_catalog() {
        let root = tempfile::tempdir().unwrap();
        let catalog_path = root.path().join("folders.json");
        let store = FolderStore::default();
        let mut tasks = Vec::new();
        for index in 0..12 {
            let store = store.clone();
            let catalog_path = catalog_path.clone();
            let folder_path = root.path().join(format!("project-{index}"));
            tasks.push(tokio::spawn(async move {
                store
                    .upsert(
                        catalog_path.to_str().unwrap(),
                        folder_path.to_str().unwrap(),
                        None,
                    )
                    .await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }

        let folders = store.list(catalog_path.to_str().unwrap()).await.unwrap();
        assert_eq!(folders.len(), 12);
        assert!(folders.iter().all(|folder| !folder.name.contains('/')));
    }

    #[tokio::test]
    async fn delete_is_durable_and_missing_delete_is_a_noop() {
        let root = tempfile::tempdir().unwrap();
        let catalog_path = root.path().join("folders.json");
        let folder_path = root.path().join("project");
        let store = FolderStore::default();
        store
            .upsert(
                catalog_path.to_str().unwrap(),
                folder_path.to_str().unwrap(),
                None,
            )
            .await
            .unwrap();

        assert!(
            store
                .delete(
                    catalog_path.to_str().unwrap(),
                    folder_path.to_str().unwrap()
                )
                .await
                .unwrap()
        );
        assert!(
            !store
                .delete(
                    catalog_path.to_str().unwrap(),
                    folder_path.to_str().unwrap()
                )
                .await
                .unwrap()
        );
        assert!(
            store
                .list(catalog_path.to_str().unwrap())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn malformed_catalog_is_reported_without_being_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let catalog_path = root.path().join("folders.json");
        std::fs::write(&catalog_path, b"not-json").unwrap();
        let error = FolderStore::default()
            .list(catalog_path.to_str().unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "harness/folder_store");
        assert_eq!(std::fs::read(&catalog_path).unwrap(), b"not-json");
    }

    const OLD_CATALOG: &str = r#"{
  "version": 1,
  "projects": [
    { "path": "/work/acme/web", "name": "web", "last_used_at": 2 },
    { "path": "/work/acme/api", "name": "API", "last_used_at": 1 }
  ]
}"#;

    #[test]
    fn only_the_old_catalogue_shape_is_legacy() {
        assert!(is_legacy_catalog(
            &serde_json::json!({ "version": 1, "projects": [] })
        ));
        assert!(is_legacy_catalog(
            &serde_json::from_str::<Value>(OLD_CATALOG).unwrap()
        ));
        assert!(is_legacy_catalog(
            &serde_json::json!({ "projects": [{ "path": "/a" }] })
        ));
        assert!(!is_legacy_catalog(
            &serde_json::json!({ "version": 2, "revision": 3, "projects": [] })
        ));
        assert!(!is_legacy_catalog(
            &serde_json::json!({ "projects": [{ "id": "prj_1", "folders": [] }] })
        ));
        assert!(!is_legacy_catalog(&serde_json::json!([])));
    }

    /// The default path: the old catalogue at `data/harness-projects.json`
    /// moves to the folders file and reads back through harness::folders.
    #[tokio::test]
    async fn the_old_catalogue_moves_to_the_folders_file_at_start() {
        let root = tempfile::tempdir().unwrap();
        let projects = root.path().join("data/harness-projects.json");
        let folders = root.path().join("data/harness-folders.json");
        std::fs::create_dir_all(projects.parent().unwrap()).unwrap();
        std::fs::write(&projects, OLD_CATALOG).unwrap();

        let outcome = migrate(&folders, &projects, false).await.unwrap();
        assert_eq!(
            outcome,
            Migration::Moved {
                from: projects.clone(),
                to: folders.clone()
            }
        );
        assert!(
            !projects.exists(),
            "the projects path is free for the projects store"
        );
        let listed = FolderStore::default()
            .list(folders.to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(
            listed.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["/work/acme/web", "/work/acme/api"]
        );
        assert_eq!(listed[1].name, "API");
    }

    /// An operator's `projects_file_path` is honoured for the one move.
    #[tokio::test]
    async fn an_operators_projects_path_is_honoured_for_the_move() {
        let root = tempfile::tempdir().unwrap();
        let projects = root.path().join("operator/catalog.json");
        let folders = root.path().join("data/harness-folders.json");
        std::fs::create_dir_all(projects.parent().unwrap()).unwrap();
        std::fs::write(&projects, OLD_CATALOG).unwrap();

        let outcome = migrate(&folders, &projects, true).await.unwrap();
        assert!(matches!(outcome, Migration::Moved { .. }));
        assert!(!projects.exists());
        assert_eq!(
            FolderStore::default()
                .list(folders.to_str().unwrap())
                .await
                .unwrap()
                .len(),
            2
        );
    }

    /// Already moved: a second start changes nothing, and a projects store
    /// (`version: 2`) at the projects path is never taken for the catalogue.
    #[tokio::test]
    async fn an_already_moved_catalogue_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let projects = root.path().join("harness-projects.json");
        let folders = root.path().join("harness-folders.json");
        std::fs::write(&projects, OLD_CATALOG).unwrap();
        migrate(&folders, &projects, false).await.unwrap();
        let v2 = r#"{"version":2,"revision":1,"projects":[]}"#;
        std::fs::write(&projects, v2).unwrap();

        assert_eq!(
            migrate(&folders, &projects, false).await.unwrap(),
            Migration::AlreadyMoved
        );
        assert_eq!(std::fs::read_to_string(&projects).unwrap(), v2);
        assert_eq!(
            FolderStore::default()
                .list(folders.to_str().unwrap())
                .await
                .unwrap()
                .len(),
            2
        );

        // Nothing at either path: nothing moves.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            migrate(
                &empty.path().join("f.json"),
                &empty.path().join("p.json"),
                false
            )
            .await
            .unwrap(),
            Migration::NothingToMove
        );
    }

    /// Both files exist and the projects path still holds the old shape (an
    /// older harness wrote it again): it is set aside, never merged or lost.
    #[tokio::test]
    async fn a_stale_old_copy_is_set_aside() {
        let root = tempfile::tempdir().unwrap();
        let projects = root.path().join("harness-projects.json");
        let folders = root.path().join("harness-folders.json");
        std::fs::write(&folders, r#"{"version":1,"folders":[]}"#).unwrap();
        std::fs::write(&projects, OLD_CATALOG).unwrap();
        let Migration::SetAside { to, .. } = migrate(&folders, &projects, false).await.unwrap()
        else {
            panic!("expected the old copy to be set aside");
        };
        assert!(!projects.exists());
        assert_eq!(std::fs::read_to_string(to).unwrap(), OLD_CATALOG);
    }
}
