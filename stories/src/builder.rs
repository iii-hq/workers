//! Building one line end to end: resolve the spec, put the tree on disk,
//! run the compiler per project, hash every input, assemble the index,
//! import the vite output into the content-addressed store, and — for the
//! working tree — classify the changes against the previous build and emit
//! `stories:changed`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};

use crate::compiler::Compiler;
use crate::config::{ConfigCell, StoriesConfig, WorkspaceConfig};
use crate::events::{BuildEvent, ChangedEvent, Subscribers, emit_build, emit_changed};
use crate::lines::{self, LineSpec, Resolved};
use crate::model::{
    ChangeKind, Component, Index, Input, LineInfo, LineKind, ProjectInfo, State, compare,
    sha256_hex, summarize, version_of,
};
use crate::store::{HistoryEntry, Manifest, PREV_KEY, Store, WORKTREE_KEY};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BuildStatus {
    pub build_id: String,
    pub workspace: String,
    /// The line as requested (`worktree`, a ref, a sha, `turn:…`).
    pub line: String,
    /// queued | running | done | failed
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// The built line, once done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<LineInfo>,
}

pub struct Ctx {
    pub iii: Arc<IIIClient>,
    pub config: ConfigCell,
    pub store: Store,
    pub compiler: Compiler,
    pub subscribers: Subscribers,
    // ponytail: one global build lock; per-workspace locks if parallel
    // workspaces ever matter.
    pub build_lock: Mutex<()>,
    pub builds: RwLock<HashMap<String, BuildStatus>>,
    /// Workspace names the watcher wants rebuilt; drained by [`build_loop`].
    pub rebuild_tx: mpsc::UnboundedSender<String>,
}

impl Ctx {
    pub fn config(&self) -> StoriesConfig {
        self.config
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn workspace(&self, name: Option<&str>) -> Result<WorkspaceConfig, String> {
        let config = self.config();
        config.workspace(name).cloned().ok_or_else(|| match name {
            Some(name) => format!(
                "unknown workspace '{name}'; configured: {}",
                config
                    .workspaces
                    .iter()
                    .map(|w| w.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None => "no workspace configured".to_string(),
        })
    }

    pub fn ide_turns_dir(&self) -> PathBuf {
        iii_worker_paths::resolve_path(
            std::env::var("III_STORIES_IDE_TURNS")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "data/shell/turns".to_string()),
        )
    }

    pub fn build_status(&self, build_id: &str) -> Option<BuildStatus> {
        self.builds
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(build_id)
            .cloned()
    }

    fn set_build(&self, status: BuildStatus) {
        emit_build(
            &self.iii,
            &self.subscribers,
            &BuildEvent {
                workspace: status.workspace.clone(),
                build_id: status.build_id.clone(),
                status: status.status.clone(),
                line: status.line.clone(),
                message: status.message.clone(),
                at: now(),
            },
        );
        let mut builds = self.builds.write().unwrap_or_else(|p| p.into_inner());
        builds.insert(status.build_id.clone(), status);
        // ponytail: unbounded registry trimmed to the last 200 entries.
        if builds.len() > 200 {
            let mut keys: Vec<(String, String)> = builds
                .values()
                .map(|b| (b.started_at.clone(), b.build_id.clone()))
                .collect();
            keys.sort();
            for (_, key) in keys.into_iter().take(builds.len() - 200) {
                builds.remove(&key);
            }
        }
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

static BUILD_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn new_build_id() -> String {
    format!(
        "b-{}-{:04x}-{:x}",
        chrono::Utc::now().format("%Y%m%d%H%M%S%3f"),
        std::process::id() & 0xffff,
        BUILD_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

pub fn spec_label(spec: &LineSpec) -> String {
    match spec {
        LineSpec::Worktree => WORKTREE_KEY.to_string(),
        LineSpec::Prev => PREV_KEY.to_string(),
        LineSpec::Ref(name) => name.clone(),
        LineSpec::Sha(sha) => sha.clone(),
        LineSpec::Turn {
            session,
            turn,
            side,
        } => format!("turn:{session}/{turn}/{side}"),
    }
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    for c in value.to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn rel_to(root: &Path, path: &str) -> String {
    let p = Path::new(path);
    match p.strip_prefix(root) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => path.replace('\\', "/"),
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Turn one compiler result into a component record, hashing and storing
/// every input on the way.
pub fn component_from(
    store: &Store,
    workspace: &str,
    source_root: &Path,
    project: &crate::compiler::DiscoveredProject,
    built: &crate::compiler::BuiltFile,
) -> Component {
    let mut inputs = Vec::new();
    let mut warnings = Vec::new();
    for module in &built.modules {
        let path = Path::new(&module.path);
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                warnings.push(format!("{}: {error}", module.path));
                continue;
            }
        };
        let sha256 = sha256_hex(&bytes);
        let _ = store.put_blob(workspace, &bytes);
        inputs.push(Input {
            path: rel_to(source_root, &module.path),
            hop: module.hop,
            sha256,
        });
    }
    inputs.sort_by(|a, b| a.hop.cmp(&b.hop).then_with(|| a.path.cmp(&b.path)));
    let version = version_of(&inputs);
    let file_path = if project.path == "." {
        built.file.clone()
    } else {
        format!("{}/{}", project.path, built.file)
    };
    let fallback_title = built
        .file
        .trim_end_matches(".tsx")
        .trim_end_matches(".ts")
        .trim_end_matches(".jsx")
        .trim_end_matches(".js")
        .trim_end_matches(".stories")
        .to_string();
    let manifest = built.manifest.as_ref();
    let get = |key: &str| manifest.and_then(|m| m.get(key));
    let text = |key: &str| get(key).and_then(Value::as_str).map(str::to_string);
    let title = text("title").unwrap_or_else(|| fallback_title.clone());
    let states: Vec<State> = get("states")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let mut error = built.error.clone();
    if !warnings.is_empty() {
        error = Some(format!(
            "{}{}",
            error.map(|e| format!("{e}\n")).unwrap_or_default(),
            warnings.join("\n")
        ));
    }
    Component {
        id: text("id").unwrap_or_else(|| slug(&fallback_title)),
        project: project.name.clone(),
        file: built.file.clone(),
        path: file_path,
        group: text("group").unwrap_or_default(),
        title,
        tags: strings(get("tags")),
        component: text("component"),
        version,
        html: built.html.as_ref().map(|html| {
            if project.path == "." {
                html.clone()
            } else {
                format!("{}/{html}", project.path)
            }
        }),
        states,
        inputs,
        globals: get("globals").cloned().unwrap_or(Value::Null),
        global_types: get("global_types").cloned().unwrap_or(Value::Null),
        error,
    }
}

/// Build the line and record it. Ref and turn lines already built are
/// returned from the store without rebuilding.
pub async fn build_line(
    ctx: &Arc<Ctx>,
    workspace: &WorkspaceConfig,
    spec: &LineSpec,
    build_id: &str,
) -> Result<Index, String> {
    let label = spec_label(spec);
    let status = |status: &str,
                  message: Option<String>,
                  result: Option<LineInfo>,
                  started_at: &str| BuildStatus {
        build_id: build_id.to_string(),
        workspace: workspace.name.clone(),
        line: label.clone(),
        status: status.to_string(),
        message,
        started_at: started_at.to_string(),
        finished_at: if status == "done" || status == "failed" {
            Some(now())
        } else {
            None
        },
        result,
    };
    let started_at = now();
    ctx.set_build(status("queued", None, None, &started_at));
    let _guard = ctx.build_lock.lock().await;
    ctx.set_build(status("running", None, None, &started_at));
    match build_line_inner(ctx, workspace, spec, build_id).await {
        Ok(index) => {
            ctx.set_build(status("done", None, Some(index.line.clone()), &started_at));
            Ok(index)
        }
        Err(error) => {
            tracing::warn!(workspace = workspace.name, line = label, error = %error, "stories build failed");
            ctx.set_build(status("failed", Some(error.clone()), None, &started_at));
            Err(error)
        }
    }
}

async fn build_line_inner(
    ctx: &Arc<Ctx>,
    workspace: &WorkspaceConfig,
    spec: &LineSpec,
    build_id: &str,
) -> Result<Index, String> {
    let ws = workspace.name.as_str();
    let root = workspace.path_resolved();
    if !root.is_dir() {
        return Err(format!(
            "workspace path {} is not a directory",
            root.display()
        ));
    }
    let resolved = lines::resolve(spec, &root, &ctx.ide_turns_dir()).await?;
    if resolved.kind == LineKind::Prev {
        return ctx
            .store
            .read_index(ws, PREV_KEY)
            .ok_or_else(|| "no previous working-tree build yet".to_string());
    }
    if matches!(resolved.kind, LineKind::Ref | LineKind::Turn)
        && let Some(index) = ctx.store.read_index(ws, &resolved.key)
    {
        return Ok(index);
    }
    let source_root = match resolved.kind {
        LineKind::Worktree => root.clone(),
        _ => {
            let dest = ctx.store.worktrees_dir(ws).join(&resolved.key);
            lines::materialize(&resolved, &root, &dest, &ctx.ide_turns_dir()).await?;
            dest
        }
    };
    let result = compile_tree(ctx, workspace, &resolved, &source_root, build_id).await;
    if resolved.kind != LineKind::Worktree {
        let _ = lines::cleanup(&resolved, &root, &source_root).await;
    }
    let _ = std::fs::remove_dir_all(ctx.store.tmp_dir(ws, build_id));
    result
}

async fn compile_tree(
    ctx: &Arc<Ctx>,
    workspace: &WorkspaceConfig,
    resolved: &Resolved,
    source_root: &Path,
    build_id: &str,
) -> Result<Index, String> {
    let ws = workspace.name.as_str();
    let config = ctx.config();
    let discovered = ctx
        .compiler
        .discover(source_root, &workspace.stories, &workspace.ignore)
        .await?;
    let projects: Vec<(
        crate::compiler::DiscoveredProject,
        Option<&crate::config::ProjectConfig>,
    )> = if workspace.projects.is_empty() {
        discovered.into_iter().map(|p| (p, None)).collect()
    } else {
        let mut chosen = Vec::new();
        for configured in &workspace.projects {
            let wanted = configured.path.trim_matches('/');
            match discovered
                .iter()
                .find(|p| p.path.trim_matches('/') == wanted)
            {
                Some(found) => {
                    let mut project = found.clone();
                    if let Some(name) = &configured.name {
                        project.name = name.clone();
                    }
                    chosen.push((project, Some(configured)));
                }
                None => chosen.push((
                    crate::compiler::DiscoveredProject {
                        name: configured
                            .name
                            .clone()
                            .unwrap_or_else(|| wanted.to_string()),
                        path: wanted.to_string(),
                        files: vec![],
                    },
                    Some(configured),
                )),
            }
        }
        chosen
    };

    let mut manifest = Manifest::new();
    let mut components = Vec::new();
    let mut infos = Vec::new();
    let mut warnings = Vec::new();
    for (project, configured) in &projects {
        let project_dir = if project.path == "." {
            source_root.to_path_buf()
        } else {
            source_root.join(&project.path)
        };
        let out = ctx.store.tmp_dir(ws, build_id).join(slug(&project.path));
        tracing::info!(
            workspace = ws,
            project = project.name,
            files = project.files.len(),
            "building stories"
        );
        let built = ctx
            .compiler
            .build(
                &project_dir,
                &out,
                &project.files,
                configured.and_then(|c| c.preview.as_deref()),
                configured.and_then(|c| c.config.as_deref()),
                &workspace.stories,
                &workspace.ignore,
            )
            .await;
        match built {
            Ok(output) => {
                warnings.extend(
                    output
                        .warnings
                        .iter()
                        .map(|w| format!("{}: {w}", project.name)),
                );
                for file in &output.files {
                    components.push(component_from(&ctx.store, ws, source_root, project, file));
                }
                if let Err(error) = ctx
                    .store
                    .import_dist(ws, &out, &project.path, &mut manifest)
                {
                    warnings.push(format!("{}: importing build output: {error}", project.name));
                }
                infos.push(ProjectInfo {
                    name: project.name.clone(),
                    path: project.path.clone(),
                    vite: output.vite.clone(),
                    files: output.files.len(),
                    error: output.error.clone(),
                });
            }
            Err(error) => {
                infos.push(ProjectInfo {
                    name: project.name.clone(),
                    path: project.path.clone(),
                    vite: None,
                    files: project.files.len(),
                    error: Some(error),
                });
            }
        }
    }
    components.sort_by(|a, b| {
        a.project
            .cmp(&b.project)
            .then_with(|| a.title.cmp(&b.title))
    });
    // A project that failed to build is a warning on the line, and a line
    // where nothing built at all is a failed build, never an empty index a
    // comparison would read as "everything is new".
    let failures: Vec<String> = infos
        .iter()
        .filter_map(|p| p.error.as_ref().map(|e| format!("{}: {e}", p.name)))
        .collect();
    if components.is_empty() && !failures.is_empty() {
        return Err(failures.join("\n"));
    }
    warnings.extend(failures);

    let dirty = if resolved.kind == LineKind::Worktree {
        lines::is_dirty(&workspace.path_resolved()).await
    } else {
        None
    };
    let index = Index {
        workspace: ws.to_string(),
        line: LineInfo {
            key: resolved.key.clone(),
            kind: resolved.kind,
            label: resolved.label.clone(),
            sha: resolved.sha.clone(),
            dirty,
            built_at: now(),
        },
        projects: infos,
        components,
        warnings,
    };

    if resolved.kind == LineKind::Worktree {
        let previous = ctx.store.read_index(ws, WORKTREE_KEY);
        ctx.store
            .rotate_worktree(ws)
            .map_err(|e| format!("rotating the previous build: {e}"))?;
        ctx.store
            .write_line(ws, &index, &manifest)
            .map_err(|e| format!("writing the index: {e}"))?;
        if let Some(previous) = previous {
            let changes: Vec<_> = compare(&previous, &index)
                .into_iter()
                .filter(|c| c.kind != ChangeKind::Unchanged)
                .collect();
            for change in &changes {
                let _ = ctx.store.append_history(
                    ws,
                    &change.project,
                    &change.id,
                    &HistoryEntry {
                        version: change
                            .b_version
                            .clone()
                            .or_else(|| change.a_version.clone())
                            .unwrap_or_default(),
                        line: WORKTREE_KEY.into(),
                        at: index.line.built_at.clone(),
                        change: Some(change.kind),
                        files: change.files.iter().map(|f| f.path.clone()).collect(),
                    },
                );
            }
            if !changes.is_empty() {
                emit_changed(
                    &ctx.iii,
                    &ctx.subscribers,
                    &ChangedEvent {
                        workspace: ws.to_string(),
                        line: index.line.clone(),
                        base: previous.line.clone(),
                        summary: summarize(&changes),
                        changes,
                        at: now(),
                    },
                );
            }
        }
    } else {
        ctx.store
            .write_line(ws, &index, &manifest)
            .map_err(|e| format!("writing the index: {e}"))?;
        ctx.store.prune(ws, config.keep_lines as usize);
    }
    Ok(index)
}

/// Make sure a line is built. Returns the index when it is, or the build id
/// of the build started (or already running) when it is not ready within
/// `wait_ms`.
pub async fn ensure_line(
    ctx: &Arc<Ctx>,
    workspace: &WorkspaceConfig,
    spec: &LineSpec,
    wait_ms: u64,
) -> Result<Result<Index, String>, String> {
    let ws = workspace.name.as_str();
    if let LineSpec::Prev = spec {
        return Ok(ctx
            .store
            .read_index(ws, PREV_KEY)
            .ok_or_else(|| "no previous working-tree build yet".to_string()));
    }
    let key = match spec {
        LineSpec::Worktree => Some(WORKTREE_KEY.to_string()),
        LineSpec::Prev => Some(PREV_KEY.to_string()),
        _ => lines::resolve(spec, &workspace.path_resolved(), &ctx.ide_turns_dir())
            .await
            .ok()
            .map(|r| r.key),
    };
    if let Some(key) = &key
        && let Some(index) = ctx.store.read_index(ws, key)
    {
        return Ok(Ok(index));
    }
    let build_id = new_build_id();
    let task = {
        let ctx = ctx.clone();
        let workspace = workspace.clone();
        let spec = spec.clone();
        let build_id = build_id.clone();
        tokio::spawn(async move { build_line(&ctx, &workspace, &spec, &build_id).await })
    };
    if wait_ms == 0 {
        return Ok(Err(build_id));
    }
    match tokio::time::timeout(std::time::Duration::from_millis(wait_ms), task).await {
        Ok(Ok(result)) => result.map(Ok),
        Ok(Err(join)) => Err(format!("build task failed: {join}")),
        Err(_) => Ok(Err(build_id)),
    }
}

/// Drain rebuild requests from the watcher, one working-tree build per
/// workspace at a time, coalescing bursts.
pub async fn build_loop(ctx: Arc<Ctx>, mut rx: mpsc::UnboundedReceiver<String>) {
    while let Some(first) = rx.recv().await {
        let mut wanted = vec![first];
        while let Ok(next) = rx.try_recv() {
            if !wanted.contains(&next) {
                wanted.push(next);
            }
        }
        for name in wanted {
            let Ok(workspace) = ctx.workspace(Some(&name)) else {
                continue;
            };
            let _ = build_line(&ctx, &workspace, &LineSpec::Worktree, &new_build_id()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::new_build_id;

    #[test]
    fn build_ids_are_unique_within_a_millisecond() {
        let ids: std::collections::HashSet<String> = (0..64).map(|_| new_build_id()).collect();
        assert_eq!(ids.len(), 64);
    }
}
