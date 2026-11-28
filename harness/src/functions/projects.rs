//! `harness::projects::*`: chat projects (chat-projects phase 1, D25), plus
//! the shims that keep builds released before it working (D26).
//!
//! - `list {version: 2}` answers the projects; `list` without it answers the
//!   saved-folder catalogue in its old shape, `{projects: [{path, name,
//!   last_used_at}]}`, and logs the deprecation once per connection.
//! - `upsert {path, name?}` forwards to `harness::folders::upsert`.
//! - `delete {path}` without an `id` forwards to `harness::folders::delete`.
//!
//! A conversation belongs to a project through `metadata.project_id` (D8,
//! D25); every write of it goes through `session::patch-meta` (D9).
//! Refusals are one JSON object in the error, `{code, message, ...}`.

use std::collections::{BTreeSet, HashSet};
use std::sync::Mutex;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::clients::session::{MetaPatch, SessionClient, SessionSummary};
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::folders::SavedFolder;
use crate::projects::{self, METADATA_KEY, NONE, Project, ProjectPatch};

use super::folders::{
    FolderDeleteRequest, FolderUpsertRequest, FolderUpsertResponse, FoldersListRequest,
};

pub const LIST_ID: &str = "harness::projects::list";
pub const LIST_DESC: &str = "List the chat projects: named, coloured groups of folders, and the conversations \
     started in them (a conversation's `metadata.project_id`). Always send `{ \"version\": 2 }`; \
     it answers { version, revision, projects: [{ id, name, color, folders, account, autocompact, \
     created_at_ms, updated_at_ms }] }. Without `version: 2` it answers the old saved-folder \
     list, kept for apps released before projects.";
pub const CREATE_ID: &str = "harness::projects::create";
pub const CREATE_DESC: &str = "Internal control-plane: create a chat project { name, color, folders, account?, \
     autocompact?, bring_existing? }; account is { mode: \"global\" } (the default) or { mode: \"pinned\", \
     account_id } (invalid_account otherwise); autocompact is { mode: \"inherit\" } (the default), \
     { mode: \"off\" } or { mode: \"tokens\", tokens } with 20,000-2,000,000 tokens (bad_setting, \
     bad_tokens otherwise); bring_existing stamps the root conversations already in its folders. \
     Answers { project, moved }.";
pub const UPDATE_ID: &str = "harness::projects::update";
pub const UPDATE_DESC: &str = "Internal control-plane: change a chat project (a patch: only the fields sent change); \
     autocompact is checked as on create (bad_setting, bad_tokens); bring_existing applies to the \
     folders added in this call. Answers { project, moved }.";
pub const DELETE_ID: &str = "harness::projects::delete";
pub const DELETE_DESC: &str = "Internal control-plane: delete a chat project { id }; its conversations go to No project \
     ({ released }). { path } without an id forwards to harness::folders::delete (apps released \
     before projects).";
pub const ASSIGN_ID: &str = "harness::projects::assign";
pub const ASSIGN_DESC: &str = "Internal control-plane: move conversations { session_ids, project: <id> | \"none\" } and \
     their sub-agents and side chats into a project, or keep them out of every project. Answers { updated }.";
pub const UPSERT_ID: &str = "harness::projects::upsert";
pub const UPSERT_DESC: &str =
    "Deprecated: forwards to harness::folders::upsert (apps released before chat projects).";
pub const ON_CHANGED_FN: &str = "harness::projects::on-changed";
pub const ON_CHANGED_DESC: &str = "A chat project was created, updated or deleted: { revision, project_id, change }. Refetch \
     harness::projects::list { version: 2 }.";

// ── wire shapes ──

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ProjectsListRequest {
    /// `2` lists the chat projects. Without it, the saved-folder catalogue in
    /// its pre-projects shape (deprecated).
    #[serde(default)]
    pub version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectsListV2 {
    /// Always 2.
    pub version: u32,
    /// Goes up on every change of the store.
    pub revision: u64,
    pub projects: Vec<Project>,
}

/// The saved folders, as `harness::projects::list` answered before projects.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LegacyFolderList {
    pub projects: Vec<SavedFolder>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ProjectsListResponse {
    Projects(ProjectsListV2),
    Folders(LegacyFolderList),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ProjectCreateRequest {
    pub name: String,
    /// blue, purple, teal, green, amber or rose.
    pub color: String,
    /// 1-20 absolute folders; `~` is expanded on the engine's computer.
    pub folders: Vec<String>,
    /// Stamp the root conversations already in these folders whose
    /// `project_id` is missing or `"none"`.
    #[serde(default)]
    pub bring_existing: Option<bool>,
    /// The project's Claude Code account: `{mode: "global"}` (the default)
    /// or `{mode: "pinned", account_id}`. Anything else is `invalid_account`.
    #[serde(default)]
    #[schemars(with = "Option<Map<String, Value>>")]
    pub account: Option<Value>,
    /// The project's autocompact setting (chat-projects phase 3):
    /// `{mode: "inherit"}` (the default), `{mode: "off"}` or
    /// `{mode: "tokens", tokens}` with 20,000-2,000,000 tokens. Anything else
    /// is `bad_setting` or `bad_tokens`.
    #[serde(default)]
    #[schemars(with = "Option<Map<String, Value>>")]
    pub autocompact: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ProjectUpdateRequest {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    /// The whole new folder list.
    #[serde(default)]
    pub folders: Option<Vec<String>>,
    /// The project's Claude Code account: `{mode: "global"}` or
    /// `{mode: "pinned", account_id}`. Anything else is `invalid_account`.
    #[serde(default)]
    #[schemars(with = "Option<Map<String, Value>>")]
    pub account: Option<Value>,
    /// The project's autocompact setting (chat-projects phase 3), checked
    /// as on create: `{mode: "inherit" | "off"}` or `{mode: "tokens", tokens}`.
    #[serde(default)]
    #[schemars(with = "Option<Map<String, Value>>")]
    pub autocompact: Option<Value>,
    /// Bring the conversations in the folders this call adds.
    #[serde(default)]
    pub bring_existing: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectWriteResponse {
    pub project: Project,
    /// Conversations `bring_existing` stamped.
    pub moved: u64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ProjectDeleteRequest {
    /// The project to delete.
    #[serde(default)]
    pub id: Option<String>,
    /// Deprecated: without `id`, forget this saved folder (harness::folders::delete).
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectDeleted {
    /// Conversations that left the project (now under No project).
    pub released: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FolderDeleted {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ProjectDeleteResponse {
    Project(ProjectDeleted),
    Folder(FolderDeleted),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ProjectAssignRequest {
    pub session_ids: Vec<String>,
    /// A project id, or `"none"` to keep the conversations out of every
    /// project on purpose.
    pub project: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectAssignResponse {
    /// Sessions written: the named ones and their descendants.
    pub updated: u64,
}

/// `harness::projects::on-changed`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectsChangedEvent {
    pub revision: u64,
    pub project_id: String,
    /// `created`, `updated` or `deleted`.
    pub change: String,
}

// ── handlers ──

/// Callers already told that `list` without `version: 2` is deprecated, by
/// connection (the caller's worker id). Bounded: a long-lived engine with
/// many short connections only logs again.
static WARNED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn warn_deprecated_once(what: &str) {
    let caller = iii_sdk::current_invocation()
        .and_then(|ctx| ctx.caller)
        .map(|caller| {
            caller
                .worker_id
                .or(caller.worker_name)
                .unwrap_or_else(|| "unknown".into())
        })
        .unwrap_or_else(|| "unknown".into());
    let mut warned = WARNED.lock().unwrap_or_else(|p| p.into_inner());
    if warned.len() >= 1024 {
        warned.clear();
    }
    if warned.insert(caller.clone()) {
        tracing::warn!(
            caller = %caller,
            "{what} is deprecated: call harness::folders::* for saved folders, and \
             harness::projects::list {{ version: 2 }} for projects"
        );
    }
}

pub async fn list(
    deps: &Deps,
    request: ProjectsListRequest,
) -> Result<ProjectsListResponse, HarnessError> {
    if request.version != Some(2) {
        warn_deprecated_once("harness::projects::list without version: 2");
        let folders = super::folders::list(deps, FoldersListRequest {}).await?;
        return Ok(ProjectsListResponse::Folders(LegacyFolderList {
            projects: folders.folders,
        }));
    }
    let file_path = deps.cfg().await.projects_file_path.clone();
    let document = deps.projects.list(&file_path).await?;
    Ok(ProjectsListResponse::Projects(ProjectsListV2 {
        version: 2,
        revision: document.revision,
        projects: document.projects,
    }))
}

pub async fn upsert_shim(
    deps: &Deps,
    request: FolderUpsertRequest,
) -> Result<FolderUpsertResponse, HarnessError> {
    warn_deprecated_once("harness::projects::upsert");
    super::folders::upsert(deps, request).await
}

pub async fn create(
    deps: &Deps,
    request: ProjectCreateRequest,
) -> Result<ProjectWriteResponse, HarnessError> {
    let file_path = deps.cfg().await.projects_file_path.clone();
    let home = projects::home_dir();
    let written = deps
        .projects
        .create(
            &file_path,
            projects::NewProject {
                name: request.name,
                color: request.color,
                folders: request.folders,
                account: request.account,
                autocompact: request.autocompact,
            },
            home.as_deref(),
        )
        .await?;
    let moved = if request.bring_existing.unwrap_or(false) {
        bring_existing(
            deps,
            &written.project,
            &written.projects,
            &written.added_folders,
        )
        .await?
    } else {
        0
    };
    emit_changed(deps, written.revision, &written.project.id, "created").await;
    Ok(ProjectWriteResponse {
        project: written.project,
        moved,
    })
}

pub async fn update(
    deps: &Deps,
    request: ProjectUpdateRequest,
) -> Result<ProjectWriteResponse, HarnessError> {
    let file_path = deps.cfg().await.projects_file_path.clone();
    let home = projects::home_dir();
    let before = deps.projects.list(&file_path).await?.revision;
    let written = deps
        .projects
        .update(
            &file_path,
            &request.id,
            ProjectPatch {
                name: request.name,
                color: request.color,
                folders: request.folders,
                account: request.account,
                autocompact: request.autocompact,
            },
            home.as_deref(),
        )
        .await?;
    let moved = if request.bring_existing.unwrap_or(false) && !written.added_folders.is_empty() {
        bring_existing(
            deps,
            &written.project,
            &written.projects,
            &written.added_folders,
        )
        .await?
    } else {
        0
    };
    if written.revision != before {
        emit_changed(deps, written.revision, &written.project.id, "updated").await;
    }
    Ok(ProjectWriteResponse {
        project: written.project,
        moved,
    })
}

pub async fn delete(
    deps: &Deps,
    request: ProjectDeleteRequest,
) -> Result<ProjectDeleteResponse, HarnessError> {
    match (request.id, request.path) {
        (Some(id), _) => {
            let file_path = deps.cfg().await.projects_file_path.clone();
            let written = deps.projects.delete(&file_path, &id).await?;
            let released = release(deps, &id).await?;
            emit_changed(deps, written.revision, &id, "deleted").await;
            Ok(ProjectDeleteResponse::Project(ProjectDeleted { released }))
        }
        (None, Some(path)) => {
            warn_deprecated_once("harness::projects::delete { path }");
            let out = super::folders::delete(deps, FolderDeleteRequest { path }).await?;
            Ok(ProjectDeleteResponse::Folder(FolderDeleted {
                deleted: out.deleted,
            }))
        }
        (None, None) => Err(HarnessError::rejected(
            "bad_request",
            "harness::projects::delete needs the project's `id`",
            json!({}),
        )),
    }
}

pub async fn assign(
    deps: &Deps,
    request: ProjectAssignRequest,
) -> Result<ProjectAssignResponse, HarnessError> {
    let project = request.project.trim().to_string();
    if project != NONE {
        let file_path = deps.cfg().await.projects_file_path.clone();
        let document = deps.projects.list(&file_path).await?;
        if !document.projects.iter().any(|p| p.id == project) {
            return Err(projects::not_found(&project));
        }
    }
    if request.session_ids.is_empty() {
        return Ok(ProjectAssignResponse { updated: 0 });
    }
    let session = deps.session().await;
    let all = session.list_summaries(None).await?;
    let targets = with_descendants(&request.session_ids, &all);
    let mut updated = 0;
    for id in targets {
        let current = all
            .iter()
            .find(|s| s.session_id == id)
            .and_then(|s| s.meta_str(METADATA_KEY));
        if current == Some(project.as_str()) {
            continue;
        }
        session.patch_meta(&id, &membership_patch(&project)).await?;
        updated += 1;
    }
    Ok(ProjectAssignResponse { updated })
}

async fn emit_changed(deps: &Deps, revision: u64, project_id: &str, change: &str) {
    deps.events
        .emit_projects_changed(json!({
            "revision": revision,
            "project_id": project_id,
            "change": change,
        }))
        .await;
}

// ── membership ──

/// The write that puts a conversation in `project` (an id or `"none"`).
/// Membership is not activity: it keeps the session's `updated_at`, so
/// bringing, moving or releasing conversations never re-orders the lists
/// (D50).
pub fn membership_patch(project: &str) -> MetaPatch {
    MetaPatch::set(METADATA_KEY, json!(project)).quiet()
}

/// The write that takes a conversation out of a deleted project, keeping its
/// `updated_at` (D50).
pub fn release_patch() -> MetaPatch {
    MetaPatch::unset(METADATA_KEY).quiet()
}

/// The named sessions and every descendant: sub-agents through
/// `parent_session_id`, side chats through `side_chat_of`, at any depth.
/// Unknown ids are left out. Order: the named ones first, then breadth-first.
pub fn with_descendants(roots: &[String], sessions: &[SessionSummary]) -> Vec<String> {
    let known: HashSet<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: std::collections::VecDeque<String> = roots
        .iter()
        .filter(|id| known.contains(id.as_str()))
        .cloned()
        .collect();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        for child in sessions.iter().filter(|s| {
            s.meta_str("parent_session_id") == Some(id.as_str())
                || s.meta_str("side_chat_of") == Some(id.as_str())
        }) {
            queue.push_back(child.session_id.clone());
        }
        out.push(id);
    }
    out
}

/// A session's working folder as its metadata records it (`fs_scope.root`).
pub fn session_root(summary: &SessionSummary) -> Option<&str> {
    summary
        .metadata
        .as_ref()?
        .get(crate::types::turn::FS_SCOPE_KEY)?
        .get("root")?
        .as_str()
        .filter(|root| !root.is_empty())
}

/// Whether a session is a root conversation: not a sub-agent, not a side chat.
pub fn is_root(summary: &SessionSummary) -> bool {
    summary.meta_str("parent_session_id").is_none() && summary.meta_str("side_chat_of").is_none()
}

/// The root sessions `bring_existing` stamps: in `project`'s folders by the
/// longest-prefix rule over every project, through one of `added` folders,
/// with `project_id` missing or `"none"`.
pub fn bring_candidates(
    sessions: &[SessionSummary],
    project: &Project,
    all_projects: &[Project],
    added: &[String],
) -> Vec<String> {
    sessions
        .iter()
        .filter(|s| is_root(s))
        .filter(|s| matches!(s.meta_str(METADATA_KEY), None | Some(NONE)))
        .filter(|s| {
            session_root(s)
                .and_then(|root| projects::project_and_folder_of(root, all_projects))
                .is_some_and(|(owner, folder)| {
                    owner.id == project.id && added.iter().any(|a| a == folder)
                })
        })
        .map(|s| s.session_id.clone())
        .collect()
}

async fn bring_existing(
    deps: &Deps,
    project: &Project,
    all_projects: &[Project],
    added: &[String],
) -> Result<u64, HarnessError> {
    let session = deps.session().await;
    let sessions = session.list_summaries(None).await?;
    let mut moved = 0;
    for id in bring_candidates(&sessions, project, all_projects, added) {
        session
            .patch_meta(&id, &membership_patch(&project.id))
            .await?;
        moved += 1;
    }
    Ok(moved)
}

/// Unset `project_id` on every session of a deleted project.
async fn release(deps: &Deps, project_id: &str) -> Result<u64, HarnessError> {
    let session = deps.session().await;
    let members = session
        .list_summaries(Some(json!({ METADATA_KEY: project_id })))
        .await?;
    let mut released = 0;
    for member in members
        .iter()
        .filter(|s| s.meta_str(METADATA_KEY) == Some(project_id))
    {
        session
            .patch_meta(&member.session_id, &release_patch())
            .await?;
        released += 1;
    }
    Ok(released)
}

/// The project `harness::send` stamps on a conversation (D8): only when its
/// metadata has no `project_id` at all (`"none"` means kept out on purpose)
/// and its working folder belongs to a project.
pub fn stamp_decision<'a>(
    current: Option<&Map<String, Value>>,
    root: Option<&str>,
    projects: &'a [Project],
) -> Option<&'a Project> {
    if current.is_some_and(|metadata| metadata.contains_key(METADATA_KEY)) {
        return None;
    }
    projects::project_of(root?, projects)
}

/// Stamp `project_id` on a conversation before its turn (D8), best effort: a
/// store or session-manager failure is logged and the send goes on.
pub async fn stamp_on_send(
    deps: &Deps,
    session: &SessionClient,
    session_id: &str,
    current: Option<&Map<String, Value>>,
    root: Option<&str>,
) {
    if root.is_none() || current.is_some_and(|m| m.contains_key(METADATA_KEY)) {
        return;
    }
    let file_path = deps.cfg().await.projects_file_path.clone();
    let document = match deps.projects.list(&file_path).await {
        Ok(document) => document,
        Err(error) => {
            tracing::warn!(session_id, %error, "projects store unreadable; send not stamped");
            return;
        }
    };
    let Some(project) = stamp_decision(current, root, &document.projects) else {
        return;
    };
    if let Err(error) = session
        .patch_meta(session_id, &membership_patch(&project.id))
        .await
    {
        tracing::warn!(session_id, %error, "could not stamp the conversation's project");
    }
}

/// A session's `project_id` (or `"none"`), best effort, for a sub-agent to
/// copy from its parent.
pub async fn session_project(session: &SessionClient, session_id: &str) -> Option<String> {
    match session.summary(session_id).await {
        Ok(summary) => summary?.meta_str(METADATA_KEY).map(str::to_string),
        Err(error) => {
            tracing::warn!(session_id, %error, "could not read the parent's project");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, metadata: Value) -> SessionSummary {
        SessionSummary {
            session_id: id.into(),
            metadata: metadata.as_object().cloned(),
            ..SessionSummary::default()
        }
    }

    fn project(id: &str, folders: &[&str]) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            color: "blue".into(),
            folders: folders.iter().map(|f| f.to_string()).collect(),
            account: projects::default_account(),
            autocompact: projects::default_autocompact(),
            created_at_ms: 1,
            updated_at_ms: 1,
            extra: Map::new(),
        }
    }

    fn ids(list: Vec<String>) -> Vec<&'static str> {
        list.into_iter()
            .map(|s| &*Box::leak(s.into_boxed_str()))
            .collect()
    }

    #[test]
    fn bring_existing_takes_unassigned_roots_in_the_added_folders() {
        let web = project("prj_web", &["/w/web", "/w/api"]);
        let docs = project("prj_docs", &["/w/web/docs"]);
        let all = vec![web.clone(), docs.clone()];
        let sessions = vec![
            session("s_web", json!({ "fs_scope": { "root": "/w/web" } })),
            session("s_api_sub", json!({ "fs_scope": { "root": "/w/api/src" } })),
            session(
                "s_none",
                json!({ "fs_scope": { "root": "/w/web" }, "project_id": "none" }),
            ),
            // Already in another project: left alone.
            session(
                "s_other",
                json!({ "fs_scope": { "root": "/w/web" }, "project_id": "prj_x" }),
            ),
            // Nested folder of another project: belongs there.
            session("s_docs", json!({ "fs_scope": { "root": "/w/web/docs" } })),
            // Not a root.
            session(
                "s_child",
                json!({ "fs_scope": { "root": "/w/web" }, "parent_session_id": "s_web" }),
            ),
            session(
                "s_side",
                json!({ "fs_scope": { "root": "/w/web" }, "side_chat_of": "s_web" }),
            ),
            // Outside every folder, and without a folder.
            session("s_out", json!({ "fs_scope": { "root": "/w/webapp" } })),
            session("s_bare", json!({})),
        ];
        assert_eq!(
            ids(bring_candidates(&sessions, &web, &all, &web.folders)),
            ["s_web", "s_api_sub", "s_none"]
        );
        // With update, only the folders added in that call.
        assert_eq!(
            ids(bring_candidates(&sessions, &web, &all, &["/w/api".into()])),
            ["s_api_sub"]
        );
        assert_eq!(
            ids(bring_candidates(&sessions, &docs, &all, &docs.folders)),
            ["s_docs"]
        );
    }

    #[test]
    fn assign_moves_subagents_and_side_chats_at_every_depth() {
        let sessions = vec![
            session("s_root", json!({})),
            session("s_sub", json!({ "parent_session_id": "s_root" })),
            session("s_subsub", json!({ "parent_session_id": "s_sub" })),
            session("s_side", json!({ "side_chat_of": "s_root" })),
            session("s_side_sub", json!({ "parent_session_id": "s_side" })),
            session("s_other", json!({ "parent_session_id": "s_elsewhere" })),
        ];
        assert_eq!(
            ids(with_descendants(
                &["s_root".into(), "s_missing".into()],
                &sessions
            )),
            ["s_root", "s_sub", "s_side", "s_subsub", "s_side_sub"]
        );
        // A descendant named too is written once.
        assert_eq!(
            with_descendants(&["s_sub".into(), "s_root".into()], &sessions).len(),
            5
        );
    }

    #[test]
    fn send_stamps_only_conversations_without_a_project_key() {
        let all = vec![project("prj_web", &["/w/web"])];
        let empty = Map::new();
        let none = json!({ "project_id": "none" })
            .as_object()
            .cloned()
            .unwrap();
        let other = json!({ "project_id": "prj_x" })
            .as_object()
            .cloned()
            .unwrap();
        let stamp = |current: Option<&Map<String, Value>>, root: Option<&str>| {
            stamp_decision(current, root, &all).map(|p| p.id.clone())
        };
        assert_eq!(stamp(None, Some("/w/web/src")).as_deref(), Some("prj_web"));
        assert_eq!(
            stamp(Some(&empty), Some("/w/web")).as_deref(),
            Some("prj_web")
        );
        // Kept out on purpose, or already in a project: never stamped.
        assert_eq!(stamp(Some(&none), Some("/w/web")), None);
        assert_eq!(stamp(Some(&other), Some("/w/web")), None);
        // Outside every project, or no folder.
        assert_eq!(stamp(None, Some("/w/other")), None);
        assert_eq!(stamp(None, None), None);
    }

    fn offline_deps(dir: &std::path::Path) -> Deps {
        let iii = std::sync::Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:0"));
        let cfg = crate::config::WorkerConfig {
            folders_file_path: dir.join("harness-folders.json").to_string_lossy().into(),
            projects_file_path: dir.join("harness-projects.json").to_string_lossy().into(),
            ..crate::config::WorkerConfig::default()
        };
        Deps::new(
            iii.clone(),
            std::sync::Arc::new(tokio::sync::RwLock::new(std::sync::Arc::new(cfg))),
            crate::discovery::new_cell(),
            crate::skills::new_cell(),
            crate::events::TurnEvents::register(&iii),
            crate::hooks::HookRegistry::register(&iii),
        )
    }

    /// The three D26 shims answer released apps with the folder catalogue:
    /// `list` without `version`, `upsert {path}`, `delete {path}`; and
    /// `list {version: 2}` answers the projects beside them.
    #[tokio::test]
    async fn the_shims_forward_to_the_folder_catalogue() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path());
        let upserted = upsert_shim(
            &deps,
            FolderUpsertRequest {
                path: "/w/acme/web/".into(),
                name: Some("Web".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(upserted.project.path, "/w/acme/web");
        let folders = super::super::folders::list(&deps, FoldersListRequest {})
            .await
            .unwrap();
        assert_eq!(
            folders.folders.len(),
            1,
            "upsert forwarded to harness::folders"
        );

        let ProjectsListResponse::Folders(old) = list(&deps, ProjectsListRequest { version: None })
            .await
            .unwrap()
        else {
            panic!("list without version answers the folders");
        };
        assert_eq!(old.projects[0].name, "Web");
        let wire = serde_json::to_value(
            list(&deps, ProjectsListRequest { version: Some(1) })
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(wire["projects"][0]["path"], "/w/acme/web");

        let created = create(
            &deps,
            ProjectCreateRequest {
                name: "acme-web".into(),
                color: "blue".into(),
                folders: vec!["/w/acme/web".into()],
                bring_existing: None,
                account: None,
                autocompact: None,
            },
        )
        .await
        .unwrap();
        let ProjectsListResponse::Projects(v2) =
            list(&deps, ProjectsListRequest { version: Some(2) })
                .await
                .unwrap()
        else {
            panic!("version 2 answers the projects");
        };
        assert_eq!(v2.revision, 1);
        assert_eq!(v2.projects[0].id, created.project.id);

        let ProjectDeleteResponse::Folder(deleted) = delete(
            &deps,
            ProjectDeleteRequest {
                id: None,
                path: Some("/w/acme/web".into()),
            },
        )
        .await
        .unwrap() else {
            panic!("delete {{path}} forwards to harness::folders::delete");
        };
        assert!(deleted.deleted);
        assert!(
            super::super::folders::list(&deps, FoldersListRequest {})
                .await
                .unwrap()
                .folders
                .is_empty()
        );
        // The project is untouched by the folder delete.
        let ProjectsListResponse::Projects(v2) =
            list(&deps, ProjectsListRequest { version: Some(2) })
                .await
                .unwrap()
        else {
            panic!()
        };
        assert_eq!(v2.projects.len(), 1);

        let taken = create(
            &deps,
            ProjectCreateRequest {
                name: "ACME-WEB".into(),
                color: "rose".into(),
                folders: vec!["/w/other".into()],
                bring_existing: None,
                account: None,
                autocompact: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(taken.rejected_code().as_deref(), Some("name_taken"));
    }

    /// A fake session-manager on a fake engine: `session::list` (with the
    /// metadata subset filter), `session::get` and `session::patch-meta`
    /// over `sessions`, bumping `updated_at` unless a patch says
    /// `touch: false`, as session-manager does. Returns the deps, the
    /// sessions and every patch-meta payload it received.
    async fn fake_session_manager(
        dir: &std::path::Path,
        sessions: Vec<Value>,
    ) -> (
        Deps,
        std::sync::Arc<Mutex<std::collections::BTreeMap<String, Value>>>,
        std::sync::Arc<Mutex<Vec<Value>>>,
    ) {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let store = std::sync::Arc::new(Mutex::new(
            sessions
                .into_iter()
                .map(|s| (s["session_id"].as_str().unwrap().to_string(), s))
                .collect::<std::collections::BTreeMap<_, _>>(),
        ));
        let patches = std::sync::Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (served, recorded) = (store.clone(), patches.clone());
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let hello = json!({"type": "workerregistered", "worker_id": "w",
                "features": ["observe.v2"]});
            socket
                .send(Message::Text(hello.to_string().into()))
                .await
                .unwrap();
            let mut clock = 10_000i64;
            while let Some(Ok(frame)) = socket.next().await {
                let Ok(text) = frame.to_text() else { continue };
                let Ok(message) = serde_json::from_str::<Value>(text) else {
                    continue;
                };
                if message["type"] != "invokefunction" || message["invocation_id"].is_null() {
                    continue;
                }
                let function = message["function_id"].as_str().unwrap_or_default();
                let data = &message["data"];
                let result = {
                    let mut store = served.lock().unwrap();
                    match function {
                        "session::list" => {
                            let filter = data["metadata"].as_object().cloned().unwrap_or_default();
                            let sessions: Vec<Value> = store
                                .values()
                                .filter(|s| {
                                    filter.iter().all(|(k, v)| s["metadata"].get(k) == Some(v))
                                })
                                .cloned()
                                .collect();
                            json!({ "sessions": sessions })
                        }
                        "session::get" => store
                            .get(data["session_id"].as_str().unwrap_or_default())
                            .map(|meta| json!({ "meta": meta }))
                            .unwrap_or(Value::Null),
                        "session::patch-meta" => {
                            recorded.lock().unwrap().push(data.clone());
                            let id = data["session_id"].as_str().unwrap_or_default();
                            let meta = store.get_mut(id).unwrap();
                            let metadata = meta["metadata"].as_object_mut().unwrap();
                            for key in data["unset"].as_array().into_iter().flatten() {
                                metadata.remove(key.as_str().unwrap());
                            }
                            for (key, value) in data["set"].as_object().into_iter().flatten() {
                                metadata.insert(key.clone(), value.clone());
                            }
                            clock += 1000;
                            if data["touch"] != json!(false) {
                                meta["updated_at"] = json!(clock);
                            }
                            json!({ "meta": meta })
                        }
                        "engine::workers::register" => json!({ "success": true }),
                        _ => Value::Null,
                    }
                };
                let reply = json!({"type": "invocationresult",
                    "invocation_id": message["invocation_id"],
                    "function_id": function, "result": result});
                if socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let iii = std::sync::Arc::new(iii_sdk::register_worker(
            &url,
            iii_sdk::InitOptions::default(),
        ));
        let cfg = crate::config::WorkerConfig {
            folders_file_path: dir.join("harness-folders.json").to_string_lossy().into(),
            projects_file_path: dir.join("harness-projects.json").to_string_lossy().into(),
            session_timeout_ms: 5_000,
            ..crate::config::WorkerConfig::default()
        };
        let deps = Deps::new(
            iii.clone(),
            std::sync::Arc::new(tokio::sync::RwLock::new(std::sync::Arc::new(cfg))),
            crate::discovery::new_cell(),
            crate::skills::new_cell(),
            crate::events::TurnEvents::register(&iii),
            crate::hooks::HookRegistry::register(&iii),
        );
        (deps, store, patches)
    }

    fn stored(id: &str, metadata: Value) -> Value {
        json!({ "session_id": id, "title": id, "description": "", "status": "done",
                "message_count": 2, "created_at": 100, "updated_at": 500, "metadata": metadata })
    }

    /// D50: bringing (create and update), moving (assign, with descendants),
    /// releasing (delete) and stamping on send write `project_id` with
    /// `touch: false`, so no conversation's `updated_at` moves and the lists
    /// keep their order.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn membership_writes_never_move_updated_at() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = vec![
            stored("s_web", json!({ "fs_scope": { "root": "/w/web" } })),
            stored("s_api", json!({ "fs_scope": { "root": "/w/api/src" } })),
            stored(
                "s_sub",
                json!({ "fs_scope": { "root": "/w/web" }, "parent_session_id": "s_web" }),
            ),
            stored("s_infra", json!({ "fs_scope": { "root": "/w/infra" } })),
            stored("s_new", json!({ "fs_scope": { "root": "/w/web/x" } })),
        ];
        let (deps, store, patches) = fake_session_manager(dir.path(), sessions).await;
        let updated_at = |id: &str| store.lock().unwrap()[id]["updated_at"].clone();
        let project_of = |id: &str| store.lock().unwrap()[id]["metadata"]["project_id"].clone();

        let created = create(
            &deps,
            ProjectCreateRequest {
                name: "acme".into(),
                color: "blue".into(),
                folders: vec!["/w/web".into(), "/w/api".into()],
                bring_existing: Some(true),
                account: None,
                autocompact: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            created.moved, 3,
            "s_web, s_api and s_new are roots in its folders"
        );
        let updated = update(
            &deps,
            ProjectUpdateRequest {
                id: created.project.id.clone(),
                name: None,
                color: None,
                folders: Some(vec!["/w/web".into(), "/w/api".into(), "/w/infra".into()]),
                account: None,
                autocompact: None,
                bring_existing: Some(true),
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.moved, 1, "only the folder this update added");
        assert_eq!(project_of("s_infra"), json!(created.project.id));

        let other = create(
            &deps,
            ProjectCreateRequest {
                name: "other".into(),
                color: "rose".into(),
                folders: vec!["/w/elsewhere".into()],
                bring_existing: None,
                account: None,
                autocompact: None,
            },
        )
        .await
        .unwrap();
        let moved = assign(
            &deps,
            ProjectAssignRequest {
                session_ids: vec!["s_web".into()],
                project: other.project.id.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(moved.updated, 2, "the conversation and its subagent");
        assert_eq!(project_of("s_sub"), json!(other.project.id));

        let ProjectDeleteResponse::Project(released) = delete(
            &deps,
            ProjectDeleteRequest {
                id: Some(created.project.id.clone()),
                path: None,
            },
        )
        .await
        .unwrap() else {
            panic!("delete by id releases its conversations");
        };
        assert_eq!(released.released, 3);
        assert_eq!(project_of("s_api"), Value::Null);

        let session = deps.session().await;
        stamp_on_send(&deps, &session, "s_api", None, Some("/w/elsewhere/repo")).await;
        assert_eq!(project_of("s_api"), json!(other.project.id));

        for id in ["s_web", "s_api", "s_sub", "s_infra", "s_new"] {
            assert_eq!(updated_at(id), json!(500), "{id} kept its updated_at");
        }
        let patches = patches.lock().unwrap();
        assert_eq!(patches.len(), 3 + 1 + 2 + 3 + 1, "{patches:?}");
        assert!(
            patches.iter().all(|p| p["touch"] == json!(false)),
            "every membership write is touch: false: {patches:?}"
        );
    }

    #[test]
    fn membership_patches_keep_updated_at_and_descriptions_do_not() {
        let patch = membership_patch("prj_1");
        assert!(!patch.touch);
        assert_eq!(patch.set["project_id"], "prj_1");
        let release = release_patch();
        assert!(!release.touch);
        assert_eq!(release.unset, vec!["project_id".to_string()]);
        assert!(MetaPatch::default().touch, "other writes keep the default");
    }

    #[test]
    fn the_list_shim_answers_the_old_folder_shape_and_v2_its_own() {
        let legacy = serde_json::to_value(ProjectsListResponse::Folders(LegacyFolderList {
            projects: vec![SavedFolder {
                path: "/w/web".into(),
                name: "web".into(),
                last_used_at: 5,
            }],
        }))
        .unwrap();
        assert_eq!(
            legacy,
            json!({ "projects": [{ "path": "/w/web", "name": "web", "last_used_at": 5 }] })
        );
        let v2 = serde_json::to_value(ProjectsListResponse::Projects(ProjectsListV2 {
            version: 2,
            revision: 3,
            projects: vec![],
        }))
        .unwrap();
        assert_eq!(v2, json!({ "version": 2, "revision": 3, "projects": [] }));
        let request: ProjectsListRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(request.version, None);
        let request: ProjectDeleteRequest =
            serde_json::from_value(json!({ "path": "/w/web" })).unwrap();
        assert!(request.id.is_none() && request.path.is_some());
    }
}
