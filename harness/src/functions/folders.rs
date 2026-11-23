//! `harness::folders::*`: the saved-folder catalogue, under the name it took
//! from `harness::projects` (D25). Same records and shapes as before; trusted
//! console control-plane, absent from the agent catalogue.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::folders::SavedFolder;

pub const LIST_ID: &str = "harness::folders::list";
pub const LIST_DESC: &str = "Internal control-plane: list the saved folders (the IDE's recent folders, the Chat's \
     folder selector, the console's folder pickers), most recently used first.";
pub const UPSERT_ID: &str = "harness::folders::upsert";
pub const UPSERT_DESC: &str = "Internal control-plane: remember, touch, or rename a saved folder.";
pub const DELETE_ID: &str = "harness::folders::delete";
pub const DELETE_DESC: &str = "Internal control-plane: forget a saved folder.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct FoldersListRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FoldersListResponse {
    pub folders: Vec<SavedFolder>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FolderUpsertRequest {
    pub path: String,
    /// A custom display name. Omit to keep the current name (or use the folder
    /// name for a new folder); pass blank to reset to the folder name.
    #[serde(default)]
    pub name: Option<String>,
}

/// `project` is the key the catalogue answered with before the rename; the
/// shape stays the same (D25). `folder` carries the same record under its
/// new name.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FolderUpsertResponse {
    pub project: SavedFolder,
    pub folder: SavedFolder,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FolderDeleteRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FolderDeleteResponse {
    pub deleted: bool,
}

pub async fn list(
    deps: &Deps,
    _request: FoldersListRequest,
) -> Result<FoldersListResponse, HarnessError> {
    let file_path = deps.cfg().await.folders_file_path.clone();
    Ok(FoldersListResponse {
        folders: deps.folders.list(&file_path).await?,
    })
}

pub async fn upsert(
    deps: &Deps,
    request: FolderUpsertRequest,
) -> Result<FolderUpsertResponse, HarnessError> {
    let file_path = deps.cfg().await.folders_file_path.clone();
    let folder = deps
        .folders
        .upsert(&file_path, &request.path, request.name.as_deref())
        .await?;
    Ok(FolderUpsertResponse {
        project: folder.clone(),
        folder,
    })
}

pub async fn delete(
    deps: &Deps,
    request: FolderDeleteRequest,
) -> Result<FolderDeleteResponse, HarnessError> {
    let file_path = deps.cfg().await.folders_file_path.clone();
    Ok(FolderDeleteResponse {
        deleted: deps.folders.delete(&file_path, &request.path).await?,
    })
}
