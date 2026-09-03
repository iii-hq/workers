//! Harness-owned durable project catalog.
//!
//! The catalog is a small versioned JSON document. Writes are serialized
//! inside the harness process and replace the destination atomically, so two
//! console windows cannot lose one another's updates or observe partial JSON.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::HarnessError;
use crate::types::message::AgentMessage;

const CATALOG_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Project {
    pub path: String,
    pub name: String,
    pub last_used_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProjectCatalog {
    #[serde(default = "catalog_version")]
    version: u32,
    #[serde(default)]
    projects: Vec<Project>,
}

impl Default for ProjectCatalog {
    fn default() -> Self {
        Self {
            version: CATALOG_VERSION,
            projects: Vec::new(),
        }
    }
}

fn catalog_version() -> u32 {
    CATALOG_VERSION
}

#[derive(Clone, Default)]
pub struct ProjectStore {
    gate: Arc<Mutex<()>>,
}

impl ProjectStore {
    pub async fn list(&self, file_path: &str) -> Result<Vec<Project>, HarnessError> {
        let path = storage_path(file_path)?;
        let _guard = self.gate.lock().await;
        let mut catalog = load_catalog(&path).await?;
        sort_projects(&mut catalog.projects);
        Ok(catalog.projects)
    }

    pub async fn upsert(
        &self,
        file_path: &str,
        raw_project_path: &str,
        requested_name: Option<&str>,
    ) -> Result<Project, HarnessError> {
        let path = storage_path(file_path)?;
        let project_path = normalize_path(raw_project_path)?;
        let _guard = self.gate.lock().await;
        let mut catalog = load_catalog(&path).await?;
        let existing_name = catalog
            .projects
            .iter()
            .find(|project| project.path == project_path)
            .map(|project| project.name.as_str());
        let project = Project {
            name: normalized_name(&project_path, requested_name, existing_name),
            path: project_path.clone(),
            last_used_at: AgentMessage::now_ms(),
        };
        if let Some(existing) = catalog
            .projects
            .iter_mut()
            .find(|candidate| candidate.path == project_path)
        {
            *existing = project.clone();
        } else {
            catalog.projects.push(project.clone());
        }
        sort_projects(&mut catalog.projects);
        persist_catalog(&path, &catalog).await?;
        Ok(project)
    }

    pub async fn delete(
        &self,
        file_path: &str,
        raw_project_path: &str,
    ) -> Result<bool, HarnessError> {
        let path = storage_path(file_path)?;
        let project_path = normalize_path(raw_project_path)?;
        let _guard = self.gate.lock().await;
        let mut catalog = load_catalog(&path).await?;
        let previous_len = catalog.projects.len();
        catalog
            .projects
            .retain(|project| project.path != project_path);
        if catalog.projects.len() == previous_len {
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
            "project path must not be empty".to_string(),
        ));
    }
    if !Path::new(trimmed).is_absolute() {
        return Err(HarnessError::InvalidRequest(format!(
            "project path must be absolute: {trimmed}"
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
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HarnessError::ProjectStore(
            "projects_file_path must not be empty".to_string(),
        ));
    }
    let path = PathBuf::from(trimmed);
    if path.file_name().is_none() {
        return Err(HarnessError::ProjectStore(format!(
            "projects_file_path must name a file: {}",
            path.display()
        )));
    }
    Ok(path)
}

async fn load_catalog(path: &Path) -> Result<ProjectCatalog, HarnessError> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(ProjectCatalog::default()),
        Err(error) => {
            return Err(HarnessError::ProjectStore(format!(
                "read {}: {error}",
                path.display()
            )))
        }
    };
    let catalog: ProjectCatalog = serde_json::from_slice(&bytes).map_err(|error| {
        HarnessError::ProjectStore(format!("parse {}: {error}", path.display()))
    })?;
    if catalog.version != CATALOG_VERSION {
        return Err(HarnessError::ProjectStore(format!(
            "unsupported catalog version {} in {}; expected {CATALOG_VERSION}",
            catalog.version,
            path.display()
        )));
    }
    Ok(catalog)
}

async fn persist_catalog(path: &Path, catalog: &ProjectCatalog) -> Result<(), HarnessError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        HarnessError::ProjectStore(format!("create {}: {error}", parent.display()))
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("harness-projects.json");
    let temporary = parent.join(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let mut bytes = serde_json::to_vec_pretty(catalog)
        .map_err(|error| HarnessError::ProjectStore(format!("serialize catalog: {error}")))?;
    bytes.push(b'\n');
    if let Err(error) = tokio::fs::write(&temporary, bytes).await {
        return Err(HarnessError::ProjectStore(format!(
            "write {}: {error}",
            temporary.display()
        )));
    }
    if let Err(error) = tokio::fs::rename(&temporary, path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(HarnessError::ProjectStore(format!(
            "replace {}: {error}",
            path.display()
        )));
    }
    Ok(())
}

fn sort_projects(projects: &mut [Project]) {
    projects.sort_by(|a, b| {
        b.last_used_at
            .cmp(&a.last_used_at)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    });
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
        let project_path = root.path().join("my-project");
        let store = ProjectStore::default();

        let created = store
            .upsert(
                catalog_path.to_str().unwrap(),
                project_path.to_str().unwrap(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(created.name, "my-project");

        store
            .upsert(
                catalog_path.to_str().unwrap(),
                project_path.to_str().unwrap(),
                Some("Runtime"),
            )
            .await
            .unwrap();
        let touched = store
            .upsert(
                catalog_path.to_str().unwrap(),
                project_path.to_str().unwrap(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(touched.name, "Runtime");

        let restarted = ProjectStore::default();
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
        assert_eq!(document["projects"][0]["name"], "Runtime");
    }

    #[tokio::test]
    async fn concurrent_updates_share_one_complete_catalog() {
        let root = tempfile::tempdir().unwrap();
        let catalog_path = root.path().join("projects.json");
        let store = ProjectStore::default();
        let mut tasks = Vec::new();
        for index in 0..12 {
            let store = store.clone();
            let catalog_path = catalog_path.clone();
            let project_path = root.path().join(format!("project-{index}"));
            tasks.push(tokio::spawn(async move {
                store
                    .upsert(
                        catalog_path.to_str().unwrap(),
                        project_path.to_str().unwrap(),
                        None,
                    )
                    .await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }

        let projects = store.list(catalog_path.to_str().unwrap()).await.unwrap();
        assert_eq!(projects.len(), 12);
        assert!(projects.iter().all(|project| !project.name.contains('/')));
    }

    #[tokio::test]
    async fn delete_is_durable_and_missing_delete_is_a_noop() {
        let root = tempfile::tempdir().unwrap();
        let catalog_path = root.path().join("projects.json");
        let project_path = root.path().join("project");
        let store = ProjectStore::default();
        store
            .upsert(
                catalog_path.to_str().unwrap(),
                project_path.to_str().unwrap(),
                None,
            )
            .await
            .unwrap();

        assert!(store
            .delete(
                catalog_path.to_str().unwrap(),
                project_path.to_str().unwrap()
            )
            .await
            .unwrap());
        assert!(!store
            .delete(
                catalog_path.to_str().unwrap(),
                project_path.to_str().unwrap()
            )
            .await
            .unwrap());
        assert!(store
            .list(catalog_path.to_str().unwrap())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn malformed_catalog_is_reported_without_being_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let catalog_path = root.path().join("projects.json");
        std::fs::write(&catalog_path, b"not-json").unwrap();
        let error = ProjectStore::default()
            .list(catalog_path.to_str().unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "harness/project_store");
        assert_eq!(std::fs::read(&catalog_path).unwrap(), b"not-json");
    }
}
