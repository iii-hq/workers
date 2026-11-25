//! Chat projects (`harness::projects`, chat-projects phase 1): a named,
//! coloured group of folders on the engine's computer, and the conversations
//! started in them.
//!
//! The store is one JSON document at `projects_file_path`,
//! `{version: 2, revision, projects: [...]}`, replaced atomically on every
//! change, with `revision` going up each time. Writes are serialized inside
//! the harness process. A write never drops a field this harness does not
//! know (a project's or the document's), so a phase-1 client cannot erase
//! later phases' settings.
//!
//! Membership is a pure rule, [`project_of`]: a path belongs to the project
//! whose folder is the longest prefix of it (equal, or followed by `/`). A
//! folder may sit in only one project; a folder nested inside another
//! project's folder is allowed.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use crate::error::HarnessError;
use crate::json_file;
use crate::types::message::AgentMessage;

const STORE_VERSION: u32 = 2;
/// Project ids are `prj_` + a ULID.
pub const ID_PREFIX: &str = "prj_";
/// The project colours (the clients' six hues).
pub const COLORS: [&str; 6] = ["blue", "purple", "teal", "green", "amber", "rose"];
pub const MAX_NAME_CHARS: usize = 40;
pub const MAX_FOLDERS: usize = 20;
/// `metadata.project_id` value for a conversation kept out of every project
/// on purpose (D8): the harness never stamps it again.
pub const NONE: &str = "none";
/// The session metadata key naming a conversation's project (D25).
pub const METADATA_KEY: &str = "project_id";

/// One project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    /// `prj_` + a ULID, made by the server.
    pub id: String,
    /// 1-40 characters, trimmed, unique on the engine without regard to case.
    pub name: String,
    /// One of `blue`, `purple`, `teal`, `green`, `amber`, `rose`.
    pub color: String,
    /// 1-20 absolute, normalised folders on the engine's computer.
    pub folders: Vec<String>,
    /// The project's Claude Code account (phase 4); `{mode: "global"}` until
    /// then. Kept whole, whatever fields it carries.
    #[serde(default = "default_account")]
    #[schemars(with = "Map<String, Value>")]
    pub account: Value,
    /// The project's autocompact setting (phase 3); `{mode: "inherit"}`
    /// until then. Kept whole, whatever fields it carries.
    #[serde(default = "default_autocompact")]
    #[schemars(with = "Map<String, Value>")]
    pub autocompact: Value,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Fields a later harness wrote; kept on every write.
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Map<String, Value>,
}

pub fn default_account() -> Value {
    json!({ "mode": "global" })
}

pub fn default_autocompact() -> Value {
    json!({ "mode": "inherit" })
}

/// The store document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectsDocument {
    pub version: u32,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for ProjectsDocument {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            revision: 0,
            projects: Vec::new(),
            extra: Map::new(),
        }
    }
}

// ── pure rules ──

/// Whether `path` lies in `folder`: equal, or `folder` followed by `/`.
pub fn folder_contains(folder: &str, path: &str) -> bool {
    if folder == "/" {
        return path.starts_with('/');
    }
    path == folder
        || (path.len() > folder.len()
            && path.starts_with(folder)
            && path.as_bytes()[folder.len()] == b'/')
}

/// The project a path belongs to, and the folder that decided it: the one
/// whose folder is the longest prefix of `path` (equal, or a prefix
/// followed by `/`). `None` when no folder contains it.
pub fn project_and_folder_of<'a>(
    path: &str,
    projects: &'a [Project],
) -> Option<(&'a Project, &'a str)> {
    let path = trim_trailing_slashes(path);
    projects
        .iter()
        .flat_map(|project| {
            project
                .folders
                .iter()
                .map(move |folder| (project, folder.as_str()))
        })
        .filter(|(_, folder)| folder_contains(folder, path))
        .max_by_key(|(_, folder)| folder.len())
}

/// `projectOf(path, projects)`: the project a path belongs to (see
/// [`project_and_folder_of`]).
pub fn project_of<'a>(path: &str, projects: &'a [Project]) -> Option<&'a Project> {
    project_and_folder_of(path, projects).map(|(project, _)| project)
}

fn trim_trailing_slashes(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() && path.starts_with('/') {
        "/"
    } else {
        trimmed
    }
}

/// A folder as the store keeps it: `~` expanded to `home`, absolute, `.` and
/// `..` resolved lexically, no repeated or trailing slash.
pub fn normalize_folder(raw: &str, home: Option<&str>) -> Result<String, HarnessError> {
    let trimmed = raw.trim();
    let expanded: String = if trimmed == "~" || trimmed.starts_with("~/") {
        let Some(home) = home.filter(|home| !home.is_empty()) else {
            return Err(folder_not_absolute(trimmed));
        };
        format!("{}{}", home.trim_end_matches('/'), &trimmed[1..])
    } else {
        trimmed.to_string()
    };
    if !expanded.starts_with('/') {
        return Err(folder_not_absolute(trimmed));
    }
    let mut parts: Vec<String> = Vec::new();
    for component in Path::new(&expanded).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir => {
                parts.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

fn folder_not_absolute(raw: &str) -> HarnessError {
    HarnessError::rejected(
        "folder_not_absolute",
        format!("{raw} is not an absolute path"),
        json!({ "folder": raw }),
    )
}

/// The name as stored: trimmed, 1-40 characters.
pub fn normalize_name(raw: &str) -> Result<String, HarnessError> {
    let name = raw.trim();
    let chars = name.chars().count();
    if chars == 0 {
        return Err(HarnessError::rejected(
            "bad_name",
            "Name is required",
            json!({}),
        ));
    }
    if chars > MAX_NAME_CHARS {
        return Err(HarnessError::rejected(
            "bad_name",
            format!("A project name has at most {MAX_NAME_CHARS} characters"),
            json!({}),
        ));
    }
    Ok(name.to_string())
}

pub fn validate_color(color: &str) -> Result<String, HarnessError> {
    let color = color.trim();
    if COLORS.contains(&color) {
        Ok(color.to_string())
    } else {
        Err(HarnessError::rejected(
            "bad_color",
            format!(
                "{color:?} is not a project colour; use one of {}",
                COLORS.join(", ")
            ),
            json!({ "colors": COLORS }),
        ))
    }
}

/// The folders as stored: normalised, deduplicated in order, 1-20.
pub fn normalize_folders(raw: &[String], home: Option<&str>) -> Result<Vec<String>, HarnessError> {
    let mut folders: Vec<String> = Vec::new();
    for folder in raw {
        let folder = normalize_folder(folder, home)?;
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    if folders.is_empty() {
        return Err(HarnessError::rejected(
            "no_folders",
            "Add at least one folder",
            json!({}),
        ));
    }
    if folders.len() > MAX_FOLDERS {
        return Err(HarnessError::rejected(
            "too_many_folders",
            format!("A project has at most {MAX_FOLDERS} folders"),
            json!({ "max": MAX_FOLDERS }),
        ));
    }
    Ok(folders)
}

/// Refuse `name` when another project (not `own_id`) has it, whatever the case.
pub fn check_name_free(
    name: &str,
    projects: &[Project],
    own_id: Option<&str>,
) -> Result<(), HarnessError> {
    let wanted = name.to_lowercase();
    if let Some(other) = projects
        .iter()
        .filter(|project| Some(project.id.as_str()) != own_id)
        .find(|project| project.name.to_lowercase() == wanted)
    {
        return Err(HarnessError::rejected(
            "name_taken",
            format!("A project named “{}” already exists", other.name),
            json!({ "project_id": other.id, "project_name": other.name }),
        ));
    }
    Ok(())
}

/// Refuse a folder another project (not `own_id`) already lists.
pub fn check_folders_free(
    folders: &[String],
    projects: &[Project],
    own_id: Option<&str>,
    home: Option<&str>,
) -> Result<(), HarnessError> {
    for folder in folders {
        if let Some(other) = projects
            .iter()
            .filter(|project| Some(project.id.as_str()) != own_id)
            .find(|project| project.folders.contains(folder))
        {
            return Err(HarnessError::rejected(
                "folder_taken",
                format!("{} is already in “{}”", tilde(folder, home), other.name),
                json!({
                    "folder": folder,
                    "project_id": other.id,
                    "project_name": other.name,
                }),
            ));
        }
    }
    Ok(())
}

/// `folder` with the home folder written `~`, as the clients show it.
fn tilde(folder: &str, home: Option<&str>) -> String {
    match home.map(|home| home.trim_end_matches('/')) {
        Some(home) if !home.is_empty() && folder_contains(home, folder) => {
            format!("~{}", &folder[home.len()..])
        }
        _ => folder.to_string(),
    }
}

pub fn not_found(id: &str) -> HarnessError {
    HarnessError::rejected(
        "not_found",
        format!("no project {id}"),
        json!({ "project_id": id }),
    )
}

/// The user's home folder, for `~`.
pub fn home_dir() -> Option<String> {
    std::env::var("HOME").ok().filter(|home| !home.is_empty())
}

// ── ids ──

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A ULID: 48 bits of milliseconds, then 80 random bits, in Crockford
/// base32 (26 characters). Sorted by creation time.
pub fn ulid(now_ms: i64) -> String {
    let random = uuid::Uuid::new_v4().as_u128() & ((1u128 << 80) - 1);
    let value: u128 = ((now_ms.max(0) as u128 & ((1u128 << 48) - 1)) << 80) | random;
    (0..26)
        .rev()
        .map(|index| CROCKFORD[((value >> (index * 5)) & 0x1f) as usize] as char)
        .collect()
}

pub fn new_id() -> String {
    format!("{ID_PREFIX}{}", ulid(AgentMessage::now_ms()))
}

// ── store ──

fn store_error(message: String) -> HarnessError {
    HarnessError::ProjectStore(message)
}

fn storage_path(raw: &str) -> Result<PathBuf, HarnessError> {
    json_file::storage_path(raw, "projects_file_path").map_err(store_error)
}

/// Read the document. A missing file, or a folder catalogue the start-up
/// move has not taken yet, is an empty store.
pub async fn load(path: &Path) -> Result<ProjectsDocument, HarnessError> {
    let Some(bytes) = json_file::read(path).await.map_err(store_error)? else {
        return Ok(ProjectsDocument::default());
    };
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| store_error(format!("parse {}: {error}", path.display())))?;
    if crate::folders::is_legacy_catalog(&value) {
        return Ok(ProjectsDocument::default());
    }
    let document: ProjectsDocument = serde_json::from_value(value)
        .map_err(|error| store_error(format!("parse {}: {error}", path.display())))?;
    if document.version != STORE_VERSION {
        return Err(store_error(format!(
            "unsupported projects store version {} in {}; expected {STORE_VERSION}",
            document.version,
            path.display()
        )));
    }
    Ok(document)
}

/// A project to create.
#[derive(Debug, Clone)]
pub struct NewProject {
    pub name: String,
    pub color: String,
    pub folders: Vec<String>,
}

/// The fields an update changes; `None` keeps the stored value.
#[derive(Debug, Clone, Default)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub color: Option<String>,
    pub folders: Option<Vec<String>>,
    pub account: Option<Value>,
    pub autocompact: Option<Value>,
}

/// What a write did.
#[derive(Debug, Clone)]
pub struct Written {
    pub revision: u64,
    pub project: Project,
    /// Every project after the write.
    pub projects: Vec<Project>,
    /// The folders this write added to the project (all of them on create).
    pub added_folders: Vec<String>,
}

#[derive(Clone, Default)]
pub struct ProjectsStore {
    gate: Arc<Mutex<()>>,
}

impl ProjectsStore {
    pub async fn list(&self, file_path: &str) -> Result<ProjectsDocument, HarnessError> {
        let path = storage_path(file_path)?;
        let _guard = self.gate.lock().await;
        load(&path).await
    }

    pub async fn create(
        &self,
        file_path: &str,
        input: NewProject,
        home: Option<&str>,
    ) -> Result<Written, HarnessError> {
        let name = normalize_name(&input.name)?;
        let color = validate_color(&input.color)?;
        let folders = normalize_folders(&input.folders, home)?;
        let path = storage_path(file_path)?;
        let _guard = self.gate.lock().await;
        let mut document = load(&path).await?;
        check_name_free(&name, &document.projects, None)?;
        check_folders_free(&folders, &document.projects, None, home)?;
        let now = AgentMessage::now_ms();
        let project = Project {
            id: new_id(),
            name,
            color,
            folders: folders.clone(),
            account: default_account(),
            autocompact: default_autocompact(),
            created_at_ms: now,
            updated_at_ms: now,
            extra: Map::new(),
        };
        document.projects.push(project.clone());
        document.revision += 1;
        json_file::persist(&path, &document)
            .await
            .map_err(store_error)?;
        Ok(Written {
            revision: document.revision,
            project,
            projects: document.projects,
            added_folders: folders,
        })
    }

    pub async fn update(
        &self,
        file_path: &str,
        id: &str,
        patch: ProjectPatch,
        home: Option<&str>,
    ) -> Result<Written, HarnessError> {
        let name = patch.name.as_deref().map(normalize_name).transpose()?;
        let color = patch.color.as_deref().map(validate_color).transpose()?;
        let folders = patch
            .folders
            .as_deref()
            .map(|folders| normalize_folders(folders, home))
            .transpose()?;
        for (field, value) in [
            ("account", &patch.account),
            ("autocompact", &patch.autocompact),
        ] {
            if value.as_ref().is_some_and(|value| !value.is_object()) {
                return Err(HarnessError::rejected(
                    "bad_request",
                    format!("{field} must be an object"),
                    json!({ "field": field }),
                ));
            }
        }
        let path = storage_path(file_path)?;
        let _guard = self.gate.lock().await;
        let mut document = load(&path).await?;
        let index = document
            .projects
            .iter()
            .position(|project| project.id == id)
            .ok_or_else(|| not_found(id))?;
        if let Some(name) = &name {
            check_name_free(name, &document.projects, Some(id))?;
        }
        if let Some(folders) = &folders {
            check_folders_free(folders, &document.projects, Some(id), home)?;
        }
        let project = &mut document.projects[index];
        let mut added_folders = Vec::new();
        let before = project.clone();
        if let Some(name) = name {
            project.name = name;
        }
        if let Some(color) = color {
            project.color = color;
        }
        if let Some(folders) = folders {
            added_folders = folders
                .iter()
                .filter(|folder| !project.folders.contains(folder))
                .cloned()
                .collect();
            project.folders = folders;
        }
        if let Some(account) = patch.account {
            project.account = account;
        }
        if let Some(autocompact) = patch.autocompact {
            project.autocompact = autocompact;
        }
        if *project == before {
            let project = project.clone();
            return Ok(Written {
                revision: document.revision,
                project,
                projects: document.projects,
                added_folders,
            });
        }
        project.updated_at_ms = AgentMessage::now_ms().max(before.updated_at_ms);
        let project = project.clone();
        document.revision += 1;
        json_file::persist(&path, &document)
            .await
            .map_err(store_error)?;
        Ok(Written {
            revision: document.revision,
            project,
            projects: document.projects,
            added_folders,
        })
    }

    pub async fn delete(&self, file_path: &str, id: &str) -> Result<Written, HarnessError> {
        let path = storage_path(file_path)?;
        let _guard = self.gate.lock().await;
        let mut document = load(&path).await?;
        let index = document
            .projects
            .iter()
            .position(|project| project.id == id)
            .ok_or_else(|| not_found(id))?;
        let project = document.projects.remove(index);
        document.revision += 1;
        json_file::persist(&path, &document)
            .await
            .map_err(store_error)?;
        Ok(Written {
            revision: document.revision,
            project,
            projects: document.projects,
            added_folders: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, name: &str, folders: &[&str]) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            color: "blue".into(),
            folders: folders.iter().map(|f| f.to_string()).collect(),
            account: default_account(),
            autocompact: default_autocompact(),
            created_at_ms: 1,
            updated_at_ms: 1,
            extra: Map::new(),
        }
    }

    /// The `projectOf` table every surface ports: equal path, prefix with
    /// `/`, prefix without `/`, nested folders, no match.
    #[test]
    fn project_of_table() {
        let projects = vec![
            project("prj_web", "acme-web", &["/w/acme/web", "/w/acme/api"]),
            project("prj_docs", "docs", &["/w/acme/web/docs"]),
            project("prj_root", "everything", &["/srv"]),
        ];
        let id = |path: &str| project_of(path, &projects).map(|p| p.id.as_str());
        // Equal path.
        assert_eq!(id("/w/acme/web"), Some("prj_web"));
        assert_eq!(id("/w/acme/web/"), Some("prj_web"));
        // A prefix followed by `/`.
        assert_eq!(id("/w/acme/api/src/main.rs"), Some("prj_web"));
        // A prefix NOT followed by `/` is another folder.
        assert_eq!(id("/w/acme/webapp"), None);
        assert_eq!(id("/w/acme/api2"), None);
        // Nested folders: the longest prefix wins.
        assert_eq!(id("/w/acme/web/docs"), Some("prj_docs"));
        assert_eq!(id("/w/acme/web/docs/guide"), Some("prj_docs"));
        assert_eq!(id("/w/acme/web/docsite"), Some("prj_web"));
        // No match.
        assert_eq!(id("/w/other"), None);
        assert_eq!(id("/w"), None);
        assert_eq!(id(""), None);
        assert_eq!(id("/srv/x"), Some("prj_root"));
        assert_eq!(
            project_and_folder_of("/w/acme/api/x", &projects).map(|(_, f)| f),
            Some("/w/acme/api")
        );
    }

    #[test]
    fn folders_are_absolute_normalised_and_home_expanded() {
        let home = Some("/home/u");
        assert_eq!(
            normalize_folder("~/work/acme/", home).unwrap(),
            "/home/u/work/acme"
        );
        assert_eq!(normalize_folder("~", home).unwrap(), "/home/u");
        assert_eq!(
            normalize_folder(" /a//b/./c/../d/ ", home).unwrap(),
            "/a/b/d"
        );
        assert_eq!(normalize_folder("/", home).unwrap(), "/");
        for bad in ["work/acme", "./x", "", "~other/x"] {
            let error = normalize_folder(bad, home).unwrap_err();
            assert_eq!(
                error.rejected_code().as_deref(),
                Some("folder_not_absolute"),
                "{bad}"
            );
        }
        assert_eq!(
            normalize_folder("~/x", None)
                .unwrap_err()
                .rejected_code()
                .as_deref(),
            Some("folder_not_absolute")
        );
    }

    #[test]
    fn names_are_trimmed_one_to_forty_and_unique_without_case() {
        assert_eq!(normalize_name("  acme-web ").unwrap(), "acme-web");
        assert_eq!(normalize_name(&"é".repeat(40)).unwrap().chars().count(), 40);
        for bad in ["", "   ", &"x".repeat(41)] {
            assert_eq!(
                normalize_name(bad).unwrap_err().rejected_code().as_deref(),
                Some("bad_name")
            );
        }
        let projects = vec![project("prj_1", "Acme-Web", &["/a"])];
        let taken = check_name_free("acme-web", &projects, None).unwrap_err();
        assert_eq!(taken.rejected_code().as_deref(), Some("name_taken"));
        // Renaming a project to its own name (any case) is fine.
        check_name_free("ACME-WEB", &projects, Some("prj_1")).unwrap();
    }

    #[test]
    fn colours_folder_counts_and_taken_folders_are_checked() {
        for color in COLORS {
            validate_color(color).unwrap();
        }
        assert_eq!(
            validate_color("violet")
                .unwrap_err()
                .rejected_code()
                .as_deref(),
            Some("bad_color")
        );
        assert_eq!(
            normalize_folders(&[], None)
                .unwrap_err()
                .rejected_code()
                .as_deref(),
            Some("no_folders")
        );
        let many: Vec<String> = (0..21).map(|i| format!("/f/{i}")).collect();
        assert_eq!(
            normalize_folders(&many, None)
                .unwrap_err()
                .rejected_code()
                .as_deref(),
            Some("too_many_folders")
        );
        assert_eq!(
            normalize_folders(&["/a/".into(), "/a".into()], None).unwrap(),
            vec!["/a".to_string()]
        );
        let projects = vec![project(
            "prj_billing",
            "billing-api",
            &["/home/u/work/acme/api"],
        )];
        let taken = check_folders_free(
            &["/home/u/work/acme/api".into()],
            &projects,
            None,
            Some("/home/u"),
        )
        .unwrap_err();
        let wire: Value = serde_json::from_str(&taken.to_string()).unwrap();
        assert_eq!(wire["code"], "folder_taken");
        assert_eq!(wire["project_id"], "prj_billing");
        assert_eq!(wire["project_name"], "billing-api");
        assert_eq!(
            wire["message"],
            "~/work/acme/api is already in “billing-api”"
        );
        // A folder nested in another project's folder is allowed.
        check_folders_free(&["/home/u/work/acme/api/v2".into()], &projects, None, None).unwrap();
        // A project keeps its own folders on update.
        check_folders_free(
            &["/home/u/work/acme/api".into()],
            &projects,
            Some("prj_billing"),
            None,
        )
        .unwrap();
    }

    #[test]
    fn ulids_are_26_crockford_characters_in_time_order() {
        let early = ulid(1_000);
        let late = ulid(2_000);
        assert_eq!(early.len(), 26);
        assert!(early.chars().all(|c| CROCKFORD.contains(&(c as u8))));
        assert!(early < late);
        assert!(new_id().starts_with("prj_"));
        assert_ne!(new_id(), new_id());
    }

    #[tokio::test]
    async fn the_store_bumps_its_revision_and_keeps_unknown_fields() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("data/harness-projects.json");
        let file_str = file.to_str().unwrap();
        let store = ProjectsStore::default();
        let created = store
            .create(
                file_str,
                NewProject {
                    name: " acme-web ".into(),
                    color: "teal".into(),
                    folders: vec!["/w/acme/web".into(), "/w/acme/api/".into()],
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(created.revision, 1);
        assert_eq!(created.project.name, "acme-web");
        assert_eq!(created.project.account, json!({ "mode": "global" }));
        assert_eq!(created.project.autocompact, json!({ "mode": "inherit" }));
        assert_eq!(created.added_folders.len(), 2);

        // A later phase's fields, in the project and in the document.
        let mut raw: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        raw["projects"][0]["autocompact"] = json!({ "mode": "tokens", "tokens": 160000 });
        raw["projects"][0]["future_field"] = json!({ "kept": true });
        raw["future_doc_field"] = json!(7);
        std::fs::write(&file, serde_json::to_vec(&raw).unwrap()).unwrap();

        let updated = store
            .update(
                file_str,
                &created.project.id,
                ProjectPatch {
                    name: Some("acme".into()),
                    folders: Some(vec!["/w/acme/web".into(), "/w/acme/infra".into()]),
                    ..ProjectPatch::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.added_folders, vec!["/w/acme/infra".to_string()]);
        assert_eq!(updated.project.color, "teal", "an update is a patch");
        let raw: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(raw["version"], 2);
        assert_eq!(raw["revision"], 2);
        assert_eq!(raw["future_doc_field"], 7);
        assert_eq!(raw["projects"][0]["future_field"], json!({ "kept": true }));
        assert_eq!(
            raw["projects"][0]["autocompact"],
            json!({ "mode": "tokens", "tokens": 160000 })
        );

        // A no-op update keeps the revision.
        let same = store
            .update(file_str, &created.project.id, ProjectPatch::default(), None)
            .await
            .unwrap();
        assert_eq!(same.revision, 2);

        let missing = store
            .update(file_str, "prj_missing", ProjectPatch::default(), None)
            .await
            .unwrap_err();
        assert_eq!(missing.rejected_code().as_deref(), Some("not_found"));

        let deleted = store.delete(file_str, &created.project.id).await.unwrap();
        assert_eq!(deleted.revision, 3);
        assert!(deleted.projects.is_empty());
        assert_eq!(store.list(file_str).await.unwrap().revision, 3);
    }

    #[tokio::test]
    async fn a_folder_catalogue_not_moved_yet_reads_as_an_empty_store() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("harness-projects.json");
        std::fs::write(
            &file,
            r#"{"version":1,"projects":[{"path":"/w","name":"w","last_used_at":1}]}"#,
        )
        .unwrap();
        let document = ProjectsStore::default()
            .list(file.to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(document.revision, 0);
        assert!(document.projects.is_empty());
    }
}
