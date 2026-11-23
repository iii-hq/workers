//! Trusted console control-plane for the durable harness project catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::projects::Project;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ProjectsListRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectsListResponse {
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ProjectUpsertRequest {
    pub path: String,
    /// A custom display name. Omit to keep the current name (or use the folder
    /// name for a new project); pass blank to reset to the folder name.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectUpsertResponse {
    pub project: Project,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ProjectDeleteRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectDeleteResponse {
    pub deleted: bool,
}

pub async fn list(
    deps: &Deps,
    _request: ProjectsListRequest,
) -> Result<ProjectsListResponse, HarnessError> {
    let file_path = deps.cfg().await.projects_file_path.clone();
    Ok(ProjectsListResponse {
        projects: deps.projects.list(&file_path).await?,
    })
}

pub async fn upsert(
    deps: &Deps,
    request: ProjectUpsertRequest,
) -> Result<ProjectUpsertResponse, HarnessError> {
    let file_path = deps.cfg().await.projects_file_path.clone();
    Ok(ProjectUpsertResponse {
        project: deps
            .projects
            .upsert(&file_path, &request.path, request.name.as_deref())
            .await?,
    })
}

pub async fn delete(
    deps: &Deps,
    request: ProjectDeleteRequest,
) -> Result<ProjectDeleteResponse, HarnessError> {
    let file_path = deps.cfg().await.projects_file_path.clone();
    Ok(ProjectDeleteResponse {
        deleted: deps.projects.delete(&file_path, &request.path).await?,
    })
}
