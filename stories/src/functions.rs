//! The `stories::*` function catalog: typed request/response structs and the
//! handlers. Every read goes through the store; builds go through
//! [`crate::builder`]; renders through [`crate::render`].

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;

use iii_sdk::RegisterFunction;
use iii_sdk::errors::Error;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::builder::{BuildStatus, Ctx, build_line, ensure_line, new_build_id, spec_label};
use crate::config::{Viewport, WorkspaceConfig};
use crate::lines::{self, LineSpec};
use crate::model::{
    Change, ChangeKind, ChangeSummary, ChangedFile, Component, Index, LineInfo, LineKind,
    ListFilter, ProjectInfo, State, StateChange, compare, matches_change, matches_filter,
    sha256_hex, summarize,
};
use crate::pixels;
use crate::render::{self, RenderRequest, Rendered};
use crate::store::{HistoryEntry, PREV_KEY, WORKTREE_KEY};
use crate::treediff::{self, TreeChange};

pub const FUNCTION_IDS: [&str; 13] = [
    "stories::workspaces",
    "stories::lines",
    "stories::components::list",
    "stories::components::get",
    "stories::tree::fs",
    "stories::builds::create",
    "stories::builds::get",
    "stories::compare",
    "stories::diff::file",
    "stories::screenshot",
    "stories::tree",
    "stories::diff",
    "stories::history",
];

const DEFAULT_WAIT_MS: u64 = 120_000;

fn handler(message: impl Into<String>) -> Error {
    Error::Handler(message.into())
}

/* ── shared pieces ────────────────────────────────────────────────────── */

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Empty {}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ChangeInfo {
    pub kind: ChangeKind,
    /// Inputs that differ between the base and the listed line, closest hop first.
    pub files: Vec<ChangedFile>,
    pub states: Vec<StateChange>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StateSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ComponentSummary {
    pub id: String,
    pub project: String,
    pub title: String,
    pub group: String,
    /// Story file, workspace-relative.
    pub path: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    pub version: String,
    /// Console-relative url of the story document (`/ui-files/...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    pub states: Vec<StateSummary>,
    /// Present when a base line was available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<ChangeInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn preview_url(workspace: &str, line: &str, component: &Component) -> Option<String> {
    component
        .html
        .as_ref()
        .map(|html| format!("/ui-files/stories/{workspace}/{line}/{html}"))
}

fn summary_of(
    workspace: &str,
    line: &str,
    component: &Component,
    change: Option<&Change>,
) -> ComponentSummary {
    ComponentSummary {
        id: component.id.clone(),
        project: component.project.clone(),
        title: component.title.clone(),
        group: component.group.clone(),
        path: component.path.clone(),
        tags: component.tags.clone(),
        component: component.component.clone(),
        version: component.version.clone(),
        preview_url: preview_url(workspace, line, component),
        states: component
            .states
            .iter()
            .map(|s| StateSummary {
                id: s.id.clone(),
                name: s.name.clone(),
                tags: s.tags.clone(),
            })
            .collect(),
        change: change.map(|c| ChangeInfo {
            kind: c.kind,
            files: c.files.clone(),
            states: c.states.clone(),
        }),
        error: component.error.clone(),
    }
}

fn spec_of(value: Option<&Value>) -> Result<LineSpec, Error> {
    LineSpec::parse(value).map_err(handler)
}

/// The index of a line, building it when needed. A build that does not
/// finish inside `wait_ms` surfaces as an error naming the build id.
async fn line_index(
    ctx: &Arc<Ctx>,
    workspace: &WorkspaceConfig,
    spec: &LineSpec,
    wait_ms: u64,
) -> Result<Index, Error> {
    match ensure_line(ctx, workspace, spec, wait_ms)
        .await
        .map_err(handler)?
    {
        Ok(index) => Ok(index),
        Err(build_id) => Err(handler(format!(
            "line '{}' of workspace '{}' is still building (build {build_id}); poll stories::builds::get or retry with a larger wait_ms",
            spec_label(spec),
            workspace.name
        ))),
    }
}

/// The base line's index when it is already built (or builds within
/// `wait_ms`); `None` when it is not available yet.
async fn base_index(
    ctx: &Arc<Ctx>,
    workspace: &WorkspaceConfig,
    base: Option<&Value>,
    wait_ms: u64,
) -> Result<(Option<Index>, Option<String>), Error> {
    let spec = match base {
        Some(value) => spec_of(Some(value))?,
        None => LineSpec::parse(Some(&Value::String(ctx.config().base_of(workspace))))
            .map_err(handler)?,
    };
    match ensure_line(ctx, workspace, &spec, wait_ms).await {
        Ok(Ok(index)) => Ok((Some(index), None)),
        Ok(Err(build_id)) => Ok((None, Some(build_id))),
        Err(error) => {
            tracing::debug!(error, "base line unavailable");
            Ok((None, None))
        }
    }
}

fn changes_by_key(base: Option<&Index>, index: &Index) -> HashMap<(String, String), Change> {
    match base {
        Some(base) => compare(base, index)
            .into_iter()
            .map(|c| ((c.project.clone(), c.id.clone()), c))
            .collect(),
        None => HashMap::new(),
    }
}

fn find_component<'a>(
    index: &'a Index,
    id: &str,
    project: Option<&str>,
) -> Result<&'a Component, Error> {
    let wanted = id.trim();
    let mut hits: Vec<&Component> = index
        .components
        .iter()
        .filter(|c| {
            project.is_none_or(|p| {
                c.project == p || c.path.starts_with(&format!("{}/", p.trim_end_matches('/')))
            })
        })
        .filter(|c| {
            c.id == wanted
                || c.title.eq_ignore_ascii_case(wanted)
                || c.path == wanted
                || c.file == wanted
        })
        .collect();
    if hits.is_empty() {
        return Err(handler(format!(
            "unknown component '{wanted}'; list them with stories::components::list"
        )));
    }
    if hits.len() > 1 {
        let projects: Vec<&str> = hits.iter().map(|c| c.project.as_str()).collect();
        return Err(handler(format!(
            "component '{wanted}' exists in several projects ({}); pass project",
            projects.join(", ")
        )));
    }
    Ok(hits.remove(0))
}

fn find_state<'a>(component: &'a Component, state: Option<&str>) -> Result<&'a State, Error> {
    let Some(wanted) = state.map(str::trim).filter(|s| !s.is_empty()) else {
        return component
            .states
            .first()
            .ok_or_else(|| handler(format!("component {} has no states", component.id)));
    };
    component
        .states
        .iter()
        .find(|s| s.id == wanted || s.export_name == wanted || s.name.eq_ignore_ascii_case(wanted))
        .or_else(|| {
            component.states.iter().find(|s| {
                s.id.ends_with(&format!("--{}", wanted.to_ascii_lowercase()))
            })
        })
        .ok_or_else(|| {
            handler(format!(
                "unknown state '{wanted}' of {}; available: {}",
                component.id,
                component
                    .states
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

fn object_or_empty(value: Option<Value>, what: &str) -> Result<Value, Error> {
    match value {
        None | Some(Value::Null) => Ok(json!({})),
        Some(Value::Object(map)) => Ok(Value::Object(map)),
        Some(_) => Err(handler(format!("{what} must be an object"))),
    }
}

fn viewport_or_default(ctx: &Ctx, viewport: Option<Viewport>) -> Viewport {
    viewport.unwrap_or_else(|| ctx.config().viewport)
}

#[allow(clippy::too_many_arguments)]
async fn render_state(
    ctx: &Arc<Ctx>,
    workspace: &str,
    line: &str,
    component: &Component,
    state: &State,
    args: Value,
    globals: Value,
    viewport: Viewport,
    deterministic: bool,
    clip: String,
) -> Result<Rendered, Error> {
    render::render(
        ctx,
        RenderRequest {
            workspace,
            line_key: line,
            component,
            state_id: state.id.clone(),
            args,
            globals,
            viewport,
            deterministic,
            clip,
        },
    )
    .await
    .map_err(handler)
}

/* ── workspaces / lines ───────────────────────────────────────────────── */

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkspaceView {
    pub name: String,
    /// Absolute repository root.
    pub path: String,
    /// Default base line for change classification.
    pub base: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
    /// Projects of the last working-tree build.
    pub projects: Vec<ProjectInfo>,
    pub components: usize,
    pub lines: Vec<LineInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkspacesOutput {
    pub workspaces: Vec<WorkspaceView>,
    pub data_path: String,
}

async fn workspace_view(ctx: &Ctx, workspace: &WorkspaceConfig) -> WorkspaceView {
    let root = workspace.path_resolved();
    let index = ctx.store.read_index(&workspace.name, WORKTREE_KEY);
    WorkspaceView {
        name: workspace.name.clone(),
        path: root.to_string_lossy().into_owned(),
        base: ctx.config().base_of(workspace),
        branch: lines::current_branch(&root).await,
        head: lines::head_sha(&root).await,
        dirty: lines::is_dirty(&root).await,
        projects: index
            .as_ref()
            .map(|i| i.projects.clone())
            .unwrap_or_default(),
        components: index.as_ref().map(|i| i.components.len()).unwrap_or(0),
        lines: ctx.store.list_lines(&workspace.name),
    }
}

async fn workspaces(ctx: Arc<Ctx>, _: Empty) -> Result<WorkspacesOutput, Error> {
    let config = ctx.config();
    let mut out = Vec::new();
    for workspace in &config.workspaces {
        out.push(workspace_view(&ctx, workspace).await);
    }
    Ok(WorkspacesOutput {
        workspaces: out,
        data_path: config.data_path_resolved().to_string_lossy().into_owned(),
    })
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct WorkspaceInput {
    /// Workspace slug; the first configured workspace when omitted.
    #[serde(default)]
    pub workspace: Option<String>,
}

async fn lines_fn(ctx: Arc<Ctx>, input: WorkspaceInput) -> Result<WorkspaceView, Error> {
    let workspace = ctx.workspace(input.workspace.as_deref()).map_err(handler)?;
    Ok(workspace_view(&ctx, &workspace).await)
}

/* ── components::list ─────────────────────────────────────────────────── */

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListInput {
    #[serde(default)]
    pub workspace: Option<String>,
    /// Line to list: `worktree` (default), `prev`, a ref, a full sha,
    /// `turn:<session>/<turn>[/before|after]`, or `{ ref }` / `{ sha }` / `{ turn }`.
    #[serde(default)]
    pub line: Option<Value>,
    /// Line changes are classified against. Default: the workspace base
    /// (HEAD). `prev` compares with the previous working-tree build.
    #[serde(default)]
    pub base: Option<Value>,
    #[serde(default)]
    pub filter: Option<ListFilter>,
    /// Milliseconds to wait for a line that still has to build (default 120000).
    #[serde(default)]
    pub wait_ms: Option<u64>,
    /// Milliseconds to wait for the base line (default 0: report it as pending).
    #[serde(default)]
    pub base_wait_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListOutput {
    pub workspace: String,
    pub line: LineInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<LineInfo>,
    /// Build id of the base line when it is still building.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_pending: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ChangeSummary>,
    /// Components in the line before filtering.
    pub total: usize,
    pub components: Vec<ComponentSummary>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

async fn components_list(ctx: Arc<Ctx>, input: ListInput) -> Result<ListOutput, Error> {
    let workspace = ctx.workspace(input.workspace.as_deref()).map_err(handler)?;
    let spec = spec_of(input.line.as_ref())?;
    let index = line_index(
        &ctx,
        &workspace,
        &spec,
        input.wait_ms.unwrap_or(DEFAULT_WAIT_MS),
    )
    .await?;
    let filter = input.filter.unwrap_or_default();
    let wants_change = filter
        .change
        .as_deref()
        .map(str::trim)
        .is_some_and(|c| !c.is_empty());
    let base_wait = input
        .base_wait_ms
        .unwrap_or(if wants_change { DEFAULT_WAIT_MS } else { 0 });
    let (base, base_pending) = base_index(&ctx, &workspace, input.base.as_ref(), base_wait).await?;
    if wants_change && base.is_none() {
        return Err(handler(match base_pending {
            Some(build_id) => format!(
                "the base line is still building (build {build_id}); retry or pass base_wait_ms"
            ),
            None => {
                "the base line could not be resolved; pass base explicitly (a ref, sha, or prev)"
                    .to_string()
            }
        }));
    }
    let changes = changes_by_key(base.as_ref(), &index);
    let all: Vec<Change> = changes.values().cloned().collect();
    let components = index
        .components
        .iter()
        .filter(|c| matches_filter(c, &filter))
        .filter(|c| {
            let kind = changes
                .get(&(c.project.clone(), c.id.clone()))
                .map(|ch| ch.kind);
            matches_change(kind, filter.change.as_deref())
        })
        .map(|c| {
            summary_of(
                &workspace.name,
                &index.line.key,
                c,
                changes.get(&(c.project.clone(), c.id.clone())),
            )
        })
        .collect();
    Ok(ListOutput {
        workspace: workspace.name.clone(),
        line: index.line.clone(),
        base: base.as_ref().map(|b| b.line.clone()),
        base_pending,
        summary: base.as_ref().map(|_| summarize(&all)),
        total: index.components.len(),
        components,
        warnings: index.warnings.clone(),
    })
}

/* ── components::get ──────────────────────────────────────────────────── */

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetInput {
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub line: Option<Value>,
    #[serde(default)]
    pub base: Option<Value>,
    /// Component id (`ui-button`), title (`UI/Button`) or story file path.
    pub id: String,
    /// Disambiguates a title that exists in several projects.
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RelatedComponent {
    pub id: String,
    pub project: String,
    pub title: String,
    /// The shared input.
    pub via: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetOutput {
    pub workspace: String,
    pub line: LineInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<LineInfo>,
    pub component: Component,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<ChangeInfo>,
    /// Other components sharing one of this component's direct inputs.
    pub related: Vec<RelatedComponent>,
    pub history: Vec<HistoryEntry>,
}

async fn components_get(ctx: Arc<Ctx>, input: GetInput) -> Result<GetOutput, Error> {
    let workspace = ctx.workspace(input.workspace.as_deref()).map_err(handler)?;
    let spec = spec_of(input.line.as_ref())?;
    let index = line_index(
        &ctx,
        &workspace,
        &spec,
        input.wait_ms.unwrap_or(DEFAULT_WAIT_MS),
    )
    .await?;
    let component = find_component(&index, &input.id, input.project.as_deref())?.clone();
    let (base, _) = base_index(&ctx, &workspace, input.base.as_ref(), 0).await?;
    let change = base.as_ref().and_then(|b| {
        compare(b, &index)
            .into_iter()
            .find(|c| c.project == component.project && c.id == component.id)
    });
    let direct: Vec<&str> = component
        .inputs
        .iter()
        .filter(|i| i.hop == 1)
        .map(|i| i.path.as_str())
        .collect();
    let related = index
        .components
        .iter()
        .filter(|c| !(c.project == component.project && c.id == component.id))
        .filter_map(|c| {
            c.inputs
                .iter()
                .find(|i| i.hop >= 1 && direct.contains(&i.path.as_str()))
                .map(|i| RelatedComponent {
                    id: c.id.clone(),
                    project: c.project.clone(),
                    title: c.title.clone(),
                    via: i.path.clone(),
                })
        })
        .collect();
    Ok(GetOutput {
        workspace: workspace.name.clone(),
        line: index.line.clone(),
        base: base.as_ref().map(|b| b.line.clone()),
        preview_url: preview_url(&workspace.name, &index.line.key, &component),
        change: change.map(|c| ChangeInfo {
            kind: c.kind,
            files: c.files,
            states: c.states,
        }),
        history: ctx
            .store
            .read_history(&workspace.name, &component.project, &component.id),
        related,
        component,
    })
}

/* ── tree::fs (explorer) ──────────────────────────────────────────────── */

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FsTreeInput {
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub line: Option<Value>,
    #[serde(default)]
    pub base: Option<Value>,
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FsNode {
    pub name: String,
    /// Workspace-relative path.
    pub path: String,
    /// dir | file
    pub kind: String,
    /// `git status --porcelain` code for the file (working tree only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    /// Strongest change below this node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<ChangeKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ComponentSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<FsNode>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FsTreeOutput {
    pub workspace: String,
    pub line: LineInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<LineInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_pending: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ChangeSummary>,
    pub projects: Vec<ProjectInfo>,
    pub root: FsNode,
}

fn severity(kind: Option<ChangeKind>) -> u8 {
    match kind {
        Some(ChangeKind::Direct) => 4,
        Some(ChangeKind::Indirect) => 3,
        Some(ChangeKind::New) => 2,
        Some(ChangeKind::Removed) => 1,
        _ => 0,
    }
}

fn strongest(a: Option<ChangeKind>, b: Option<ChangeKind>) -> Option<ChangeKind> {
    if severity(a) >= severity(b) { a } else { b }
}

#[derive(Default)]
struct Trie {
    dirs: BTreeMap<String, Trie>,
    files: BTreeMap<String, Vec<ComponentSummary>>,
}

fn trie_to_node(name: &str, path: &str, trie: &Trie, git: &HashMap<String, String>) -> FsNode {
    let mut children = Vec::new();
    let mut change = None;
    for (dir, sub) in &trie.dirs {
        let child_path = if path.is_empty() {
            dir.clone()
        } else {
            format!("{path}/{dir}")
        };
        let node = trie_to_node(dir, &child_path, sub, git);
        change = strongest(change, node.change);
        children.push(node);
    }
    for (file, components) in &trie.files {
        let child_path = if path.is_empty() {
            file.clone()
        } else {
            format!("{path}/{file}")
        };
        let file_change = components.iter().fold(None, |acc, c| {
            strongest(
                acc,
                c.change
                    .as_ref()
                    .map(|ch| ch.kind)
                    .filter(|k| *k != ChangeKind::Unchanged),
            )
        });
        change = strongest(change, file_change);
        children.push(FsNode {
            name: file.clone(),
            path: child_path.clone(),
            kind: "file".into(),
            git: git.get(&child_path).cloned(),
            change: file_change,
            components: components.clone(),
            children: vec![],
        });
    }
    FsNode {
        name: name.to_string(),
        path: path.to_string(),
        kind: "dir".into(),
        git: None,
        change,
        components: vec![],
        children,
    }
}

async fn git_status(root: &std::path::Path) -> HashMap<String, String> {
    let Ok(out) = lines::git(root, &["status", "--porcelain", "--untracked-files=all"]).await
    else {
        return HashMap::new();
    };
    out.lines()
        .filter(|l| l.len() > 3)
        .map(|l| {
            let code = l[..2].trim().to_string();
            let path = l[3..]
                .split(" -> ")
                .last()
                .unwrap_or("")
                .trim_matches('"')
                .to_string();
            (path, code)
        })
        .collect()
}

async fn tree_fs(ctx: Arc<Ctx>, input: FsTreeInput) -> Result<FsTreeOutput, Error> {
    let workspace = ctx.workspace(input.workspace.as_deref()).map_err(handler)?;
    let spec = spec_of(input.line.as_ref())?;
    let index = line_index(
        &ctx,
        &workspace,
        &spec,
        input.wait_ms.unwrap_or(DEFAULT_WAIT_MS),
    )
    .await?;
    let (base, base_pending) = base_index(&ctx, &workspace, input.base.as_ref(), 0).await?;
    let changes = changes_by_key(base.as_ref(), &index);
    let all: Vec<Change> = changes.values().cloned().collect();
    let git = if index.line.kind == LineKind::Worktree {
        git_status(&workspace.path_resolved()).await
    } else {
        HashMap::new()
    };
    let mut trie = Trie::default();
    for component in &index.components {
        let summary = summary_of(
            &workspace.name,
            &index.line.key,
            component,
            changes.get(&(component.project.clone(), component.id.clone())),
        );
        let mut parts: Vec<&str> = component
            .path
            .split('/')
            .filter(|p| !p.is_empty())
            .collect();
        let Some(file) = parts.pop() else { continue };
        let mut node = &mut trie;
        for part in parts {
            node = node.dirs.entry(part.to_string()).or_default();
        }
        node.files
            .entry(file.to_string())
            .or_default()
            .push(summary);
    }
    Ok(FsTreeOutput {
        workspace: workspace.name.clone(),
        line: index.line.clone(),
        base: base.as_ref().map(|b| b.line.clone()),
        base_pending,
        summary: base.as_ref().map(|_| summarize(&all)),
        projects: index.projects.clone(),
        root: trie_to_node("", "", &trie, &git),
    })
}

/* ── builds ───────────────────────────────────────────────────────────── */

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct BuildCreateInput {
    #[serde(default)]
    pub workspace: Option<String>,
    /// `worktree` (default), a ref, a full sha, `turn:<session>/<turn>[/side]`,
    /// or `{ ref }` / `{ sha }` / `{ turn: { session, turn, side } }`.
    #[serde(default)]
    pub line: Option<Value>,
    /// Wait for the build to finish (up to wait_ms) instead of returning the queued status.
    #[serde(default)]
    pub wait: Option<bool>,
    #[serde(default)]
    pub wait_ms: Option<u64>,
    /// Rebuild a ref or turn line that is already stored.
    #[serde(default)]
    pub force: Option<bool>,
}

async fn builds_create(ctx: Arc<Ctx>, input: BuildCreateInput) -> Result<BuildStatus, Error> {
    let workspace = ctx.workspace(input.workspace.as_deref()).map_err(handler)?;
    let spec = spec_of(input.line.as_ref())?;
    if matches!(spec, LineSpec::Prev) {
        return Err(handler(
            "the previous working-tree build cannot be rebuilt; build worktree instead",
        ));
    }
    if input.force.unwrap_or(false)
        && let Ok(resolved) =
            lines::resolve(&spec, &workspace.path_resolved(), &ctx.ide_turns_dir()).await
        && matches!(resolved.kind, LineKind::Ref | LineKind::Turn)
    {
        let _ = ctx.store.remove_line(&workspace.name, &resolved.key);
    }
    let build_id = new_build_id();
    let task = {
        let ctx = ctx.clone();
        let workspace = workspace.clone();
        let spec = spec.clone();
        let build_id = build_id.clone();
        tokio::spawn(async move { build_line(&ctx, &workspace, &spec, &build_id).await })
    };
    if input.wait.unwrap_or(false) {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(input.wait_ms.unwrap_or(DEFAULT_WAIT_MS)),
            task,
        )
        .await;
    } else {
        tokio::task::yield_now().await;
    }
    ctx.build_status(&build_id)
        .ok_or_else(|| handler("build was not registered"))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BuildGetInput {
    pub build_id: String,
}

async fn builds_get(ctx: Arc<Ctx>, input: BuildGetInput) -> Result<BuildStatus, Error> {
    ctx.build_status(&input.build_id)
        .ok_or_else(|| handler(format!("unknown build {}", input.build_id)))
}

/* ── compare ──────────────────────────────────────────────────────────── */

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct CompareInput {
    #[serde(default)]
    pub workspace: Option<String>,
    /// Older line. Default: the workspace base (HEAD).
    #[serde(default)]
    pub a: Option<Value>,
    /// Newer line. Default: `worktree`.
    #[serde(default)]
    pub b: Option<Value>,
    #[serde(default)]
    pub filter: Option<ListFilter>,
    /// Also list components that did not change.
    #[serde(default)]
    pub include_unchanged: Option<bool>,
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CompareOutput {
    pub workspace: String,
    pub a: LineInfo,
    pub b: LineInfo,
    pub summary: ChangeSummary,
    pub changes: Vec<Change>,
}

async fn compare_fn(ctx: Arc<Ctx>, input: CompareInput) -> Result<CompareOutput, Error> {
    let workspace = ctx.workspace(input.workspace.as_deref()).map_err(handler)?;
    let wait = input.wait_ms.unwrap_or(DEFAULT_WAIT_MS);
    let a_spec = match input.a.as_ref() {
        Some(value) => spec_of(Some(value))?,
        None => LineSpec::parse(Some(&Value::String(ctx.config().base_of(&workspace))))
            .map_err(handler)?,
    };
    let b_spec = spec_of(input.b.as_ref())?;
    let a = line_index(&ctx, &workspace, &a_spec, wait).await?;
    let b = line_index(&ctx, &workspace, &b_spec, wait).await?;
    let filter = input.filter.unwrap_or_default();
    let include_unchanged =
        input.include_unchanged.unwrap_or(false) || filter.change.as_deref() == Some("unchanged");
    let all = compare(&a, &b);
    let summary = summarize(&all);
    let by_key: HashMap<(String, String), &Component> = b
        .components
        .iter()
        .chain(a.components.iter())
        .map(|c| ((c.project.clone(), c.id.clone()), c))
        .collect();
    let changes = all
        .into_iter()
        .filter(|c| include_unchanged || c.kind != ChangeKind::Unchanged)
        .filter(|c| {
            by_key
                .get(&(c.project.clone(), c.id.clone()))
                .is_none_or(|component| matches_filter(component, &filter))
        })
        .filter(|c| matches_change(Some(c.kind), filter.change.as_deref()))
        .collect();
    Ok(CompareOutput {
        workspace: workspace.name.clone(),
        a: a.line,
        b: b.line,
        summary,
        changes,
    })
}

/* ── diff::file ───────────────────────────────────────────────────────── */

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffFileInput {
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub a: Option<Value>,
    #[serde(default)]
    pub b: Option<Value>,
    /// Workspace-relative input path, as listed in a change's `files`.
    pub path: String,
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiffFileOutput {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_sha256: Option<String>,
    pub identical: bool,
    /// Unified diff.
    pub diff: String,
}

fn input_sha(index: &Index, path: &str) -> Option<String> {
    index
        .components
        .iter()
        .flat_map(|c| c.inputs.iter())
        .find(|i| i.path == path)
        .map(|i| i.sha256.clone())
}

async fn diff_file(ctx: Arc<Ctx>, input: DiffFileInput) -> Result<DiffFileOutput, Error> {
    let workspace = ctx.workspace(input.workspace.as_deref()).map_err(handler)?;
    let wait = input.wait_ms.unwrap_or(DEFAULT_WAIT_MS);
    let a_spec = match input.a.as_ref() {
        Some(value) => spec_of(Some(value))?,
        None => LineSpec::parse(Some(&Value::String(ctx.config().base_of(&workspace))))
            .map_err(handler)?,
    };
    let a = line_index(&ctx, &workspace, &a_spec, wait).await?;
    let b = line_index(&ctx, &workspace, &spec_of(input.b.as_ref())?, wait).await?;
    let path = input.path.trim().to_string();
    let (a_sha, b_sha) = (input_sha(&a, &path), input_sha(&b, &path));
    if a_sha.is_none() && b_sha.is_none() {
        return Err(handler(format!(
            "{path} is not an input of any component in either line"
        )));
    }
    if a_sha == b_sha {
        return Ok(DiffFileOutput {
            path,
            a_sha256: a_sha,
            b_sha256: b_sha,
            identical: true,
            diff: String::new(),
        });
    }
    let empty = ctx
        .store
        .put_blob(&workspace.name, b"")
        .map_err(|e| handler(e.to_string()))?;
    let a_path = ctx
        .store
        .blob_path(&workspace.name, a_sha.as_deref().unwrap_or(&empty));
    let b_path = ctx
        .store
        .blob_path(&workspace.name, b_sha.as_deref().unwrap_or(&empty));
    let diff = lines::diff_blobs(&a_path, &b_path, &path, &path)
        .await
        .map_err(handler)?;
    Ok(DiffFileOutput {
        path,
        a_sha256: a_sha,
        b_sha256: b_sha,
        identical: false,
        diff,
    })
}

/* ── screenshot / tree ────────────────────────────────────────────────── */

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenderInput {
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub line: Option<Value>,
    /// Component id, title or story file path.
    pub id: String,
    #[serde(default)]
    pub project: Option<String>,
    /// State id (`ui-button--primary`), export name or display name. Default: the first state.
    #[serde(default)]
    pub state: Option<String>,
    /// Arg overrides merged over the state's args (serializable values only).
    #[serde(default)]
    pub args: Option<Value>,
    /// Global overrides, e.g. `{ "theme": "dark" }`.
    #[serde(default)]
    pub globals: Option<Value>,
    #[serde(default)]
    pub viewport: Option<Viewport>,
    /// `content` (default: the story's box) or `viewport`.
    #[serde(default)]
    pub clip: Option<String>,
    /// Freeze time, randomness, locale and motion (default true).
    #[serde(default)]
    pub deterministic: Option<bool>,
    /// all (default) | react | dom | a11y | html — for stories::tree.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScreenshotOutput {
    /// Absolute path of the PNG.
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
    /// Render cache key.
    pub hash: String,
    pub cached: bool,
    pub component: String,
    pub state: String,
    pub version: String,
    #[serde(rename = "box")]
    pub content_box: Value,
    pub url: String,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
async fn resolve_render(
    ctx: &Arc<Ctx>,
    input: &RenderInput,
) -> Result<
    (
        WorkspaceConfig,
        Index,
        Component,
        State,
        Value,
        Value,
        Viewport,
        bool,
        String,
    ),
    Error,
> {
    let workspace = ctx.workspace(input.workspace.as_deref()).map_err(handler)?;
    let index = line_index(
        ctx,
        &workspace,
        &spec_of(input.line.as_ref())?,
        input.wait_ms.unwrap_or(DEFAULT_WAIT_MS),
    )
    .await?;
    let component = find_component(&index, &input.id, input.project.as_deref())?.clone();
    let state = find_state(&component, input.state.as_deref())?.clone();
    let args = object_or_empty(input.args.clone(), "args")?;
    let globals = object_or_empty(input.globals.clone(), "globals")?;
    let viewport = viewport_or_default(ctx, input.viewport);
    let clip = match input.clip.as_deref().map(str::trim) {
        None | Some("") | Some("content") => "content".to_string(),
        Some("viewport") => "viewport".to_string(),
        Some(other) => {
            return Err(handler(format!(
                "clip must be content or viewport, not {other}"
            )));
        }
    };
    Ok((
        workspace,
        index,
        component,
        state,
        args,
        globals,
        viewport,
        input.deterministic.unwrap_or(true),
        clip,
    ))
}

async fn screenshot(ctx: Arc<Ctx>, input: RenderInput) -> Result<ScreenshotOutput, Error> {
    let (workspace, index, component, state, args, globals, viewport, deterministic, clip) =
        resolve_render(&ctx, &input).await?;
    let rendered = render_state(
        &ctx,
        &workspace.name,
        &index.line.key,
        &component,
        &state,
        args,
        globals,
        viewport,
        deterministic,
        clip,
    )
    .await?;
    let bytes = std::fs::read(&rendered.shot).map_err(|e| handler(e.to_string()))?;
    Ok(ScreenshotOutput {
        path: rendered.shot.to_string_lossy().into_owned(),
        width: rendered.meta.width,
        height: rendered.meta.height,
        sha256: sha256_hex(&bytes),
        hash: rendered.meta.hash,
        cached: rendered.cached,
        component: component.id,
        state: state.id,
        version: component.version,
        content_box: rendered.meta.content_box,
        url: rendered.meta.url,
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TreeOutput {
    pub hash: String,
    pub cached: bool,
    pub component: String,
    pub state: String,
    pub version: String,
    #[serde(rename = "box")]
    pub content_box: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub react: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dom: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a11y: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    /// `fn()` calls recorded while rendering.
    #[serde(default)]
    pub actions: Value,
    /// Screenshot of the same render.
    pub screenshot: String,
}

async fn tree(ctx: Arc<Ctx>, input: RenderInput) -> Result<TreeOutput, Error> {
    let kind = input.kind.clone().unwrap_or_else(|| "all".into());
    let (workspace, index, component, state, args, globals, viewport, deterministic, clip) =
        resolve_render(&ctx, &input).await?;
    let rendered = render_state(
        &ctx,
        &workspace.name,
        &index.line.key,
        &component,
        &state,
        args,
        globals,
        viewport,
        deterministic,
        clip,
    )
    .await?;
    let want = |k: &str| kind == "all" || kind == k;
    let tree = &rendered.tree;
    Ok(TreeOutput {
        hash: rendered.meta.hash.clone(),
        cached: rendered.cached,
        component: component.id,
        state: state.id,
        version: component.version,
        content_box: rendered.meta.content_box.clone(),
        react: want("react").then(|| tree.get("react").cloned()).flatten(),
        dom: want("dom").then(|| tree.get("dom").cloned()).flatten(),
        a11y: want("a11y").then(|| tree.get("a11y").cloned()).flatten(),
        html: want("html")
            .then(|| tree.get("html").and_then(Value::as_str).map(str::to_string))
            .flatten(),
        actions: tree.get("actions").cloned().unwrap_or(Value::Array(vec![])),
        screenshot: rendered.shot.to_string_lossy().into_owned(),
    })
}

/* ── diff ─────────────────────────────────────────────────────────────── */

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffInput {
    #[serde(default)]
    pub workspace: Option<String>,
    pub id: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    /// Older line. Default: the workspace base (HEAD).
    #[serde(default)]
    pub a: Option<Value>,
    /// Newer line. Default: `worktree`.
    #[serde(default)]
    pub b: Option<Value>,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(default)]
    pub globals: Option<Value>,
    #[serde(default)]
    pub viewport: Option<Viewport>,
    /// both (default) | tree | pixels
    #[serde(default)]
    pub mode: Option<String>,
    /// Pixel colour threshold, 0..1 (default 0.1).
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RegionReport {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub changed: u32,
    pub ratio: f64,
    /// Element paths under the region in the newer render.
    pub elements: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PixelReport {
    pub a_path: String,
    pub b_path: String,
    pub diff_path: String,
    pub width: u32,
    pub height: u32,
    pub changed: u32,
    pub total: u32,
    pub ratio: f64,
    pub size_mismatch: bool,
    pub regions: Vec<RegionReport>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiffSummary {
    pub tree_changes: usize,
    pub pixel_ratio: f64,
    pub regions: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiffOutput {
    pub workspace: String,
    pub component: String,
    pub state: String,
    pub a: LineInfo,
    pub b: LineInfo,
    pub a_version: String,
    pub b_version: String,
    /// The component's inputs are byte-identical in both lines.
    pub same_version: bool,
    /// identical | invisible | visual | unexplained | tree-only | pixels-only
    pub verdict: String,
    pub summary: DiffSummary,
    #[serde(default)]
    pub tree: Vec<TreeChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels: Option<PixelReport>,
}

async fn diff(ctx: Arc<Ctx>, input: DiffInput) -> Result<DiffOutput, Error> {
    let workspace = ctx.workspace(input.workspace.as_deref()).map_err(handler)?;
    let wait = input.wait_ms.unwrap_or(DEFAULT_WAIT_MS);
    let a_spec = match input.a.as_ref() {
        Some(value) => spec_of(Some(value))?,
        None => LineSpec::parse(Some(&Value::String(ctx.config().base_of(&workspace))))
            .map_err(handler)?,
    };
    let a = line_index(&ctx, &workspace, &a_spec, wait).await?;
    let b = line_index(&ctx, &workspace, &spec_of(input.b.as_ref())?, wait).await?;
    let cb = find_component(&b, &input.id, input.project.as_deref())?.clone();
    let ca = find_component(
        &a,
        &input.id,
        input.project.as_deref().or(Some(&cb.project)),
    )?
    .clone();
    let sb = find_state(&cb, input.state.as_deref())?.clone();
    let sa = find_state(&ca, Some(&sb.id))?.clone();
    let args = object_or_empty(input.args, "args")?;
    let globals = object_or_empty(input.globals, "globals")?;
    let viewport = viewport_or_default(&ctx, input.viewport);
    let mode = input.mode.unwrap_or_else(|| "both".into());
    if !matches!(mode.as_str(), "both" | "tree" | "pixels") {
        return Err(handler("mode must be both, tree or pixels"));
    }
    let ra = render_state(
        &ctx,
        &workspace.name,
        &a.line.key,
        &ca,
        &sa,
        args.clone(),
        globals.clone(),
        viewport,
        true,
        "content".into(),
    )
    .await?;
    let rb = render_state(
        &ctx,
        &workspace.name,
        &b.line.key,
        &cb,
        &sb,
        args,
        globals,
        viewport,
        true,
        "content".into(),
    )
    .await?;

    let tree_changes = if mode == "pixels" {
        Vec::new()
    } else {
        let empty = Vec::new();
        let da = ra
            .tree
            .get("dom")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        let db = rb
            .tree
            .get("dom")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        treediff::diff(da, db)
    };
    let pixels = if mode == "tree" {
        None
    } else {
        let ia = image::open(&ra.shot)
            .map_err(|e| handler(format!("reading {}: {e}", ra.shot.display())))?
            .to_rgba8();
        let ib = image::open(&rb.shot)
            .map_err(|e| handler(format!("reading {}: {e}", rb.shot.display())))?
            .to_rgba8();
        let result = pixels::diff(&ia, &ib, input.threshold.unwrap_or(0.1).clamp(0.0, 1.0));
        let diff_dir = ctx.store.renders_dir(&workspace.name).join("diffs");
        std::fs::create_dir_all(&diff_dir).map_err(|e| handler(e.to_string()))?;
        let diff_path = diff_dir.join(format!(
            "{}-{}.png",
            &ra.meta.hash[..16],
            &rb.meta.hash[..16]
        ));
        result
            .image
            .save(&diff_path)
            .map_err(|e| handler(format!("saving diff image: {e}")))?;
        let empty = Vec::new();
        let dom_b = rb
            .tree
            .get("dom")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        let dpr = viewport.dpr.max(1) as f64;
        let regions = result
            .regions
            .iter()
            .take(40)
            .map(|r| {
                // Boxes in the tree are CSS px relative to the content box; the
                // screenshot is device px of that same box.
                let css = pixels::Region {
                    x: (r.x as f64 / dpr) as u32,
                    y: (r.y as f64 / dpr) as u32,
                    w: (r.w as f64 / dpr).ceil() as u32,
                    h: (r.h as f64 / dpr).ceil() as u32,
                    changed: r.changed,
                    ratio: r.ratio,
                };
                RegionReport {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                    changed: r.changed,
                    ratio: r.ratio,
                    elements: treediff::elements_for_region(dom_b, &css),
                }
            })
            .collect();
        Some(PixelReport {
            a_path: ra.shot.to_string_lossy().into_owned(),
            b_path: rb.shot.to_string_lossy().into_owned(),
            diff_path: diff_path.to_string_lossy().into_owned(),
            width: result.width,
            height: result.height,
            changed: result.changed,
            total: result.total,
            ratio: result.ratio,
            size_mismatch: result.size_mismatch,
            regions,
        })
    };
    let pixel_changed = pixels.as_ref().map(|p| p.changed > 0);
    let verdict = match (mode.as_str(), tree_changes.is_empty(), pixel_changed) {
        ("tree", true, _) => "identical",
        ("tree", false, _) => "tree-only",
        ("pixels", _, Some(false)) => "identical",
        ("pixels", _, _) => "pixels-only",
        (_, true, Some(false)) => "identical",
        (_, false, Some(false)) => "invisible",
        (_, false, Some(true)) => "visual",
        (_, true, Some(true)) => "unexplained",
        _ => "identical",
    };
    Ok(DiffOutput {
        workspace: workspace.name.clone(),
        component: cb.id.clone(),
        state: sb.id.clone(),
        a: a.line,
        b: b.line,
        a_version: ca.version.clone(),
        b_version: cb.version.clone(),
        same_version: ca.version == cb.version,
        verdict: verdict.into(),
        summary: DiffSummary {
            tree_changes: tree_changes.len(),
            pixel_ratio: pixels.as_ref().map(|p| p.ratio).unwrap_or(0.0),
            regions: pixels.as_ref().map(|p| p.regions.len()).unwrap_or(0),
        },
        tree: tree_changes,
        pixels,
    })
}

/* ── history ──────────────────────────────────────────────────────────── */

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryInput {
    #[serde(default)]
    pub workspace: Option<String>,
    pub id: String,
    #[serde(default)]
    pub project: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HistoryOutput {
    pub component: String,
    pub project: String,
    pub entries: Vec<HistoryEntry>,
}

async fn history(ctx: Arc<Ctx>, input: HistoryInput) -> Result<HistoryOutput, Error> {
    let workspace = ctx.workspace(input.workspace.as_deref()).map_err(handler)?;
    let (project, id) = match ctx
        .store
        .read_index(&workspace.name, WORKTREE_KEY)
        .or_else(|| ctx.store.read_index(&workspace.name, PREV_KEY))
    {
        Some(index) => {
            let component = find_component(&index, &input.id, input.project.as_deref())?;
            (component.project.clone(), component.id.clone())
        }
        None => (input.project.clone().unwrap_or_default(), input.id.clone()),
    };
    Ok(HistoryOutput {
        entries: ctx.store.read_history(&workspace.name, &project, &id),
        component: id,
        project,
    })
}

/* ── registration ─────────────────────────────────────────────────────── */

fn register<I, O, F, Fut>(ctx: &Arc<Ctx>, id: &'static str, description: &'static str, handler: F)
where
    I: DeserializeOwned + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Ctx>, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Error>> + Send + 'static,
{
    let captured = ctx.clone();
    ctx.iii.register_function(
        id,
        RegisterFunction::new_async(move |input: I| handler(captured.clone(), input))
            .description(description),
    );
}

pub fn register_functions(ctx: &Arc<Ctx>) {
    register(
        ctx,
        "stories::workspaces",
        "Configured workspaces with their git state, projects, component count and built lines.",
        workspaces,
    );
    register(
        ctx,
        "stories::lines",
        "One workspace's git state and every built line (worktree, previous build, refs, turns).",
        lines_fn,
    );
    register(
        ctx,
        "stories::components::list",
        "List components (story files) of a line with their states, filtered by project, group, tag, path, query, or change kind (direct, indirect, new, removed, any) against a base line.",
        components_list,
    );
    register(
        ctx,
        "stories::components::get",
        "One component with every state (args, argTypes, controls), its inputs, the change against the base line, related components and its history.",
        components_get,
    );
    register(
        ctx,
        "stories::tree::fs",
        "The explorer tree: folders and story files as on disk with git status, components and change kinds.",
        tree_fs,
    );
    register(
        ctx,
        "stories::builds::create",
        "Build a line: the working tree, a ref or commit, or a chat turn's before/after tree. Returns the build status; wait=true blocks until done.",
        builds_create,
    );
    register(
        ctx,
        "stories::builds::get",
        "Status of a build started by stories::builds::create.",
        builds_get,
    );
    register(
        ctx,
        "stories::compare",
        "Compare two lines (default: the base line against the working tree): every component that differs, classified direct/indirect/new/removed with the files that changed.",
        compare_fn,
    );
    register(
        ctx,
        "stories::diff::file",
        "Unified diff of one input file between two lines.",
        diff_file,
    );
    register(
        ctx,
        "stories::screenshot",
        "Deterministic screenshot of one component state through the browser worker, with arg and global overrides; cached by content.",
        screenshot,
    );
    register(
        ctx,
        "stories::tree",
        "Rendered DOM tree (attributes, computed styles, layout boxes, owning React component), React tree, accessibility outline and html of one component state.",
        tree,
    );
    register(
        ctx,
        "stories::diff",
        "Deterministic comparison of one component state between two lines: tree changes, pixel regions mapped to elements, and a verdict (identical, invisible, visual, unexplained).",
        diff,
    );
    register(
        ctx,
        "stories::history",
        "Versions of one component seen by the working-tree builds, with the change kind and files of each.",
        history,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_change_kinds() {
        assert_eq!(
            strongest(Some(ChangeKind::Indirect), Some(ChangeKind::Direct)),
            Some(ChangeKind::Direct)
        );
        assert_eq!(
            strongest(None, Some(ChangeKind::New)),
            Some(ChangeKind::New)
        );
        assert_eq!(
            strongest(Some(ChangeKind::Removed), None),
            Some(ChangeKind::Removed)
        );
    }

    #[test]
    fn every_function_id_is_namespaced() {
        assert!(FUNCTION_IDS.iter().all(|id| id.starts_with("stories::")));
    }
}
