//! `iii-cloud::project::resolve` and `::use` (spec §3).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::cloud::Req;
use crate::ctx::{Ctx, now};
use crate::error::{CloudError, Res};
use crate::tool::{self, git};
use crate::types::*;

/// `owner/name` from the `https://github.com/`, `git@github.com:` and
/// `ssh://git@github.com/` forms.
pub fn parse_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = [
        "https://github.com/",
        "http://github.com/",
        "git@github.com:",
        "ssh://git@github.com/",
        "ssh://github.com/",
    ]
    .iter()
    .find_map(|prefix| url.strip_prefix(prefix))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    let ok = |s: &str| !s.is_empty() && !s.contains('/') && !s.contains(char::is_whitespace);
    (ok(owner) && ok(name)).then(|| format!("{owner}/{name}"))
}

pub async fn toplevel(path: &str) -> Res<PathBuf> {
    let dir = Path::new(path);
    if !dir.is_dir() {
        return Err(CloudError::new(
            "NOT_A_GIT_REPO",
            "The folder does not exist.",
        ));
    }
    let out = tool::run(
        "git",
        &["rev-parse", "--show-toplevel"],
        Some(dir),
        None,
        "TOOL_MISSING",
    )
    .await?;
    if !out.ok() {
        return Err(CloudError::new(
            "NOT_A_GIT_REPO",
            "The folder is not inside a git repository.",
        ));
    }
    Ok(PathBuf::from(out.stdout.trim()))
}

pub async fn checkout_facts(dir: &Path) -> Res<Checkout> {
    let branch = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .ok()
        .map(|b| b.trim().to_string())
        .filter(|b| b != "HEAD");
    let head = git(dir, &["rev-parse", "HEAD"]).await?.trim().to_string();
    let changed_files = git(dir, &["status", "--porcelain"])
        .await?
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| l[3..].to_string())
        .collect();
    let ahead = git(dir, &["rev-list", "--count", "@{u}..HEAD"])
        .await
        .ok()
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0);
    Ok(Checkout {
        branch,
        head,
        changed_files,
        ahead,
        not_served: None,
        branches: vec![],
    })
}

fn str_of(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

pub fn env_info(v: &Value) -> Option<EnvironmentInfo> {
    Some(EnvironmentInfo {
        id: str_of(v, "id")?,
        name: str_of(v, "name")?,
        // An environment whose `production` can't be read counts as production: the
        // worker pushes to a branch only for one it knows is not.
        production: v.get("production").and_then(Value::as_bool).unwrap_or(true),
        branch: str_of(v, "branch")?,
        manifest: str_of(v, "manifest"),
        url: str_of(v, "url"),
    })
}

fn brief(e: &EnvironmentInfo) -> EnvironmentBrief {
    EnvironmentBrief {
        id: e.id.clone(),
        name: e.name.clone(),
        production: e.production,
        branch: e.branch.clone(),
    }
}

/// The cloud soft-deletes projects and environments: `delete_after` is set, or
/// the status says deleting or deleted. Such a one is not offered.
pub fn is_removed(v: &Value) -> bool {
    v.get("delete_after").is_some_and(|d| !d.is_null())
        || matches!(
            v.get("status").and_then(Value::as_str),
            Some("deleting" | "deleted")
        )
}

/// The default environment: production, else the repository's default branch.
pub fn default_environment<'a>(
    envs: &'a [EnvironmentInfo],
    default_branch: &str,
) -> Option<&'a EnvironmentInfo> {
    envs.iter()
        .find(|e| e.production)
        .or_else(|| envs.iter().find(|e| e.branch == default_branch))
        .or(envs.first())
}

/// `iii-cloud::project::resolve`: read the folder, then keep it as the
/// selection, or as `candidate` while another project with a workspace is selected.
pub async fn resolve(ctx: &Arc<Ctx>, request: ResolveRequest) -> Res<Project> {
    let project = read_project(ctx, request).await?;
    store_project(ctx, &project);
    Ok(project)
}

/// A folder that is a git worktree this worker made for the agent (a linked worktree on an `iii-cloud/` branch,
/// or one it recorded) is never the person's folder: `AGENT_COPY`, with the main checkout's path in `fix`.
async fn refuse_agent_copy(ctx: &Ctx, path: &str) -> Res<()> {
    let top = toplevel(path).await?;
    // Both, as canonical paths (no `--path-format`: git before 2.31 lacks it; the
    // answers may be relative to the folder).
    let dirs = git(&top, &["rev-parse", "--git-dir", "--git-common-dir"]).await?;
    let mut lines = dirs.lines();
    let (Some(dir), Some(common)) = (lines.next(), lines.next()) else {
        return Ok(());
    };
    let canon = |p: &str| std::fs::canonicalize(top.join(p)).unwrap_or_else(|_| top.join(p));
    let (dir, common) = (canon(dir), canon(common));
    if dir == common {
        return Ok(());
    }
    let top_s = top.to_string_lossy().into_owned();
    let branch = git(&top, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .unwrap_or_default();
    let made = ctx.store.read(|s| worktrees_made(s).contains(&top_s));
    if !made && !branch.trim().starts_with("iii-cloud/") {
        return Ok(());
    }
    let common = common.as_path();
    let main = match common.file_name() {
        Some(n) if n == ".git" => common.parent().unwrap_or(common),
        _ => common,
    };
    Err(CloudError::new(
        "AGENT_COPY",
        format!(
            "This is the copy your agent works in. Pick your own folder: {}",
            main.display()
        ),
    )
    .with_fix(Some(main.to_string_lossy().into_owned())))
}

/// The folder, its repository and what iii cloud has for it; stores nothing.
async fn read_project(ctx: &Arc<Ctx>, request: ResolveRequest) -> Res<Project> {
    // Every read of a folder (`resolve`, `use`) refuses the agent's own copy.
    refuse_agent_copy(ctx, &request.path).await?;
    let top = toplevel(&request.path).await?;
    let remote = tool::run(
        "git",
        &["config", "--get", "remote.origin.url"],
        Some(&top),
        None,
        "TOOL_MISSING",
    )
    .await?;
    let repository = remote
        .ok()
        .then(|| parse_remote(&remote.stdout))
        .flatten()
        .ok_or_else(|| {
            CloudError::new(
                "NO_GITHUB_REMOTE",
                "remote.origin.url is not a GitHub repository.",
            )
        })?;
    let checkout = checkout_facts(&top).await?;
    let mut project = Project {
        path: top.to_string_lossy().into_owned(),
        repository: repository.clone(),
        checkout,
        team: None,
        teams: vec![],
        cloud_project: None,
        environment: None,
        environments: vec![],
        importable: None,
        github_access: None,
        cloud_projects: vec![],
        other_cloud_projects: vec![],
        default_branch: None,
        workspace: None,
        fell_back_from: None,
    };
    let cfg = ctx.config().await;
    let conn = ctx.connection(&cfg);
    ctx.credential().await?;
    if conn.method == Some(Method::ApiKey) {
        if let Some(key) = conn.api_key {
            let body = ctx
                .call(&Req::get(format!(
                    "/teams/{}/environments/{}",
                    key.team_id, key.environment_id
                )))
                .await?;
            if let Some(info) = env_info(&body) {
                project.environments = vec![brief(&info)];
                project.environment = Some(info);
            }
        }
    } else {
        resolve_cloud(
            ctx,
            &mut project,
            request.team_id.as_deref(),
            request.cloud_project_id.as_deref(),
            None,
        )
        .await?;
    }
    if project.default_branch.is_none() {
        project.default_branch = default_branch_of(&top, None).await;
    }
    Ok(project)
}

/// The environments of a project that follow a branch, as iii cloud has them
/// now (the store's list may be older than a console edit): the ones the push
/// webhook deploys, so not one that is deleting or whose destroy failed.
pub async fn live_environments(
    ctx: &Ctx,
    team_id: &str,
    project_id: &str,
) -> Res<Vec<EnvironmentInfo>> {
    let envs = ctx
        .call(&Req::get(format!(
            "/teams/{team_id}/projects/{project_id}/environments"
        )))
        .await?;
    Ok(envs
        .get("environments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|e| !is_removed(e))
        .filter(|e| e.get("status").and_then(Value::as_str) != Some("destroy_failed"))
        .filter_map(env_info)
        .collect())
}

/// The repository's default branch as origin has it now (`ls-remote --symref`).
pub async fn live_default_branch(ws: &Path) -> Res<String> {
    let out = git(ws, &["ls-remote", "--symref", "origin", "HEAD"]).await?;
    out.lines()
        .find_map(|l| l.strip_prefix("ref: refs/heads/"))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_owned)
        .ok_or_else(|| CloudError::new("GIT_FAILED", "origin did not name its default branch."))
}

/// The project's environments, as the cloud has them now.
pub async fn read_environments(
    ctx: &Ctx,
    team_id: &str,
    project_id: &str,
) -> Res<Vec<EnvironmentInfo>> {
    let envs = ctx
        .call(&Req::get(format!(
            "/teams/{team_id}/projects/{project_id}/environments"
        )))
        .await?;
    Ok(envs
        .get("environments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|e| !is_removed(e))
        .filter_map(env_info)
        .collect())
}

/// A cloud project of the repository, with its team and environments.
#[derive(Clone)]
struct Candidate {
    team: TeamRef,
    info: CloudProjectInfo,
    default_branch: String,
    envs: Vec<EnvironmentInfo>,
}

/// Every cloud project that uses `repository` (lowercase `owner/name`), across
/// the teams, each with its environments as the cloud has them now.
async fn read_cloud_projects(
    ctx: &Ctx,
    teams: &[TeamRef],
    repository: &str,
) -> Res<(Vec<Candidate>, Vec<OtherCloudProject>)> {
    let mut out = vec![];
    let mut others = vec![];
    for team in teams {
        let list = ctx
            .call(&Req::get(format!("/teams/{}/projects", team.id)))
            .await?;
        for p in list
            .get("projects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let named = p.pointer("/repository/full_name").and_then(Value::as_str);
            if is_removed(p) {
                continue;
            }
            if !named.is_some_and(|n| n.to_lowercase() == repository) {
                others.push(OtherCloudProject {
                    id: str_of(p, "id").unwrap_or_default(),
                    name: str_of(p, "name").unwrap_or_default(),
                    team: TeamName {
                        id: team.id.clone(),
                        name: team.name.clone(),
                    },
                    repository: named.unwrap_or_default().to_string(),
                });
                continue;
            }
            let id = str_of(p, "id").unwrap_or_default();
            let envs = read_environments(ctx, &team.id, &id).await?;
            out.push(Candidate {
                team: team.clone(),
                default_branch: p
                    .pointer("/repository/default_branch")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                info: CloudProjectInfo {
                    id,
                    name: str_of(p, "name").unwrap_or_default(),
                    slug: str_of(p, "endpoint_slug").unwrap_or_default(),
                    created_at: str_of(p, "created_at"),
                    last_deployment_at: None,
                    team: TeamName {
                        id: team.id.clone(),
                        name: team.name.clone(),
                    },
                    environments: envs
                        .iter()
                        .map(|e| EnvironmentFull {
                            id: e.id.clone(),
                            name: e.name.clone(),
                            branch: e.branch.clone(),
                            manifest: e.manifest.clone(),
                            production: e.production,
                            head: None,
                        })
                        .collect(),
                },
                envs,
            });
        }
    }
    Ok((out, others))
}

/// Re-read the cloud projects of the stored project's repository (a refresh).
pub async fn refresh_cloud_projects(ctx: &Ctx) {
    let Some(project) = ctx.store.read(|s| s.project.clone()) else {
        return;
    };
    // The default branch is known without the cloud: fill it on refresh too.
    if project.default_branch.is_none()
        && let Some(d) = default_branch_of(Path::new(&project.path), None).await
    {
        ctx.store.update(|s| {
            if let Some(p) = s.project.as_mut() {
                p.default_branch.get_or_insert(d);
            }
        });
    }
    if ctx.conn().await.method == Some(Method::ApiKey) {
        return;
    }
    let Ok(me) = ctx.call(&Req::get("/me")).await else {
        return;
    };
    let teams: Vec<TeamRef> = me
        .get("teams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(TeamRef {
                id: str_of(t, "id")?,
                name: str_of(t, "name")?,
                slug: str_of(t, "slug")?,
            })
        })
        .collect();
    let Ok((found, others)) =
        read_cloud_projects(ctx, &teams, &project.repository.to_lowercase()).await
    else {
        return;
    };
    let infos: Vec<CloudProjectInfo> = found.into_iter().map(|c| c.info).collect();
    ctx.store.update(|s| {
        if let Some(p) = s.project.as_mut() {
            // The selected project is gone from the list: the cloud removed it.
            let gone = p
                .cloud_project
                .as_ref()
                .is_some_and(|c| !infos.iter().any(|i| i.id == c.id));
            p.cloud_projects = infos;
            p.other_cloud_projects = others;
            s.project_deleted = gone;
        }
    });
}

/// The newest deployment time across a project's environments.
pub async fn last_deployment_at(ctx: &Ctx, team_id: &str, project_id: &str) -> Option<String> {
    let envs = read_environments(ctx, team_id, project_id).await.ok()?;
    let mut newest: Option<String> = None;
    for env in envs {
        let list = ctx
            .call(&Req::get(format!(
                "/teams/{team_id}/environments/{}/deployments",
                env.id
            )))
            .await
            .ok()?;
        for d in list
            .get("deployments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let at = str_of(d, "created_at");
            if at > newest {
                newest = at;
            }
        }
    }
    newest
}

async fn resolve_cloud(
    ctx: &Arc<Ctx>,
    project: &mut Project,
    only_team: Option<&str>,
    cloud_project_id: Option<&str>,
    environment_id: Option<&str>,
) -> Res<()> {
    let me = ctx.call(&Req::get("/me")).await?;
    let teams: Vec<TeamRef> = me
        .get("teams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(TeamRef {
                id: str_of(t, "id")?,
                name: str_of(t, "name")?,
                slug: str_of(t, "slug")?,
            })
        })
        .filter(|t| only_team.is_none_or(|id| id == t.id))
        .collect();
    let want = project.repository.to_lowercase();
    let (mut candidates, others) = read_cloud_projects(ctx, &teams, &want).await?;
    project.other_cloud_projects = others;
    // A project of another repository can't be used with this folder.
    if let Some(other) =
        cloud_project_id.and_then(|id| project.other_cloud_projects.iter().find(|o| o.id == id))
    {
        return Err(CloudError::new(
            "REPOSITORY_MISMATCH",
            format!(
                "This folder is {}. {} is built from {}: pick a folder of that repository.",
                project.repository, other.name, other.repository
            ),
        ));
    }
    // Asked for by id and listed by no team any more. Gone when the person is
    // still in the team that owned it (its Recent entries go too); out of that
    // team, the entry stays; with no team known, it stays as well.
    if let Some(id) = cloud_project_id
        && only_team.is_none()
        && !candidates.iter().any(|c| c.info.id == id)
    {
        let recent = ctx.store.read(|s| {
            s.recent_projects
                .iter()
                .find(|r| r.cloud_project_id == id)
                .cloned()
        });
        let name = recent
            .as_ref()
            .map(|r| r.cloud_project_name.clone())
            .unwrap_or_else(|| "That cloud project".into());
        match recent.and_then(|r| r.team_id) {
            Some(team) if !teams.iter().any(|t| t.id == team) => {
                return Err(CloudError::new(
                    "NO_ACCESS",
                    format!("You're no longer in the team that owns {name}."),
                ));
            }
            Some(_) => ctx
                .store
                .update(|s| s.recent_projects.retain(|r| r.cloud_project_id != id)),
            None => {}
        }
        return Err(CloudError::new(
            "PROJECT_DELETED",
            format!("{name} no longer exists in iii cloud."),
        ));
    }
    if !candidates.is_empty() {
        // Several projects use the repository: never silently take the first.
        if candidates.len() > 1 {
            for c in candidates.iter_mut() {
                c.info.last_deployment_at = last_deployment_at(ctx, &c.team.id, &c.info.id).await;
            }
        }
        let by_env = |env_id: &str, c: &Candidate| c.envs.iter().any(|e| e.id == env_id);
        let chosen = match (cloud_project_id, environment_id) {
            (Some(id), _) => candidates.iter().position(|c| c.info.id == id),
            // Only an environment: the project it belongs to.
            (None, Some(env)) => candidates.iter().position(|c| by_env(env, c)),
            // The most recent deployment, else the newest project.
            (None, None) => (0..candidates.len()).max_by(|&a, &b| {
                let key = |i: usize| {
                    (
                        candidates[i].info.last_deployment_at.clone(),
                        candidates[i].info.created_at.clone(),
                    )
                };
                key(a).cmp(&key(b))
            }),
        }
        .ok_or_else(|| {
            CloudError::invalid("no such cloud project (or environment) for this repository")
        })?;
        let c = candidates[chosen].clone();
        project.cloud_projects = candidates.iter().map(|c| c.info.clone()).collect();
        project.cloud_project = Some(CloudProject {
            id: c.info.id.clone(),
            name: c.info.name.clone(),
            slug: c.info.slug.clone(),
            created_at: c.info.created_at.clone(),
            last_deployment_at: c.info.last_deployment_at.clone(),
        });
        project.team = Some(c.team.clone());
        project.github_access = Some(true);
        if !c.default_branch.is_empty() {
            project.default_branch = Some(c.default_branch.clone());
        }
        project.environment = default_environment(&c.envs, &c.default_branch).cloned();
        project.environments = c.envs.iter().map(brief).collect();
        return Ok(());
    }
    // Not imported: which team's GitHub App can see the repository?
    let mut access = None;
    for team in &teams {
        let (status, body) = ctx
            // One repository's compose files, not every repository's: the whole list takes the
            // cloud a round trip to GitHub per repository (25 s for a team of 71).
            .raw(&Req::get(format!(
                "/teams/{}/github/repositories?full_name={want}",
                team.id
            )))
            .await?;
        if status != 200 {
            continue;
        }
        access.get_or_insert(false);
        let repo = body
            .get("repositories")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|r| {
                r.get("full_name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.to_lowercase() == want)
            });
        if let Some(r) = repo {
            access = Some(true);
            project.team = Some(team.clone());
            project.importable = Some(Importable {
                team_id: team.id.clone(),
                repository_id: r.get("id").and_then(Value::as_i64).ok_or_else(|| {
                    CloudError::new(
                        "CLOUD_ERROR",
                        "iii cloud sent the repository without an id.",
                    )
                })?,
                manifests: r
                    .get("manifests")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|m| {
                        let path = str_of(m, "path")?;
                        let (workers, has_cloud_block) =
                            crate::local::compose_facts(Path::new(&project.path), &path);
                        Some(ImportableManifest {
                            // Not in this checkout: the cloud's list, of the default branch.
                            workers: workers.or_else(|| {
                                m.get("workers")
                                    .and_then(Value::as_array)
                                    .map(|w| w.len() as u32)
                            }),
                            has_cloud_block,
                            choosable: m.get("choosable").and_then(Value::as_bool).unwrap_or(false),
                            suggested_environment: str_of(m, "suggested_environment"),
                            issue: str_of(m, "issue"),
                            path,
                        })
                    })
                    .collect(),
            });
            break;
        }
    }
    project.github_access = access;
    if project.team.is_none() {
        match teams.as_slice() {
            [only] => project.team = Some(only.clone()),
            _ => project.teams = teams,
        }
    }
    Ok(())
}

/// A selected project with a workspace is never replaced by a resolve, whatever
/// the folder: the resolved one becomes the candidate. Otherwise it is the selection.
fn store_project(ctx: &Ctx, project: &Project) {
    let stored = ctx.store.update(|s| {
        if s.project.as_ref().is_some_and(|p| p.workspace.is_some()) {
            s.candidate = Some(project.clone());
            return false;
        }
        s.project = Some(project.clone());
        s.candidate = None;
        true
    });
    if stored {
        ctx.emit(
            EventKind::Project,
            "current",
            true,
            format!("project {}", project.repository),
            project,
        );
    }
}

#[derive(Debug)]
struct WorktreeEntry {
    path: String,
    branch: Option<String>,
}

fn parse_worktrees(porcelain: &str) -> Vec<WorktreeEntry> {
    let mut entries = vec![];
    for block in porcelain.split("\n\n") {
        let mut path = None;
        let mut branch = None;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(p.to_string());
            } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                branch = Some(b.to_string());
            }
        }
        if let Some(path) = path {
            entries.push(WorktreeEntry { path, branch });
        }
    }
    entries
}

pub async fn use_project(ctx: &Arc<Ctx>, request: UseRequest) -> Res<Project> {
    if let Some(e) = &request.environment_id {
        uuid::Uuid::parse_str(e)
            .map_err(|_| CloudError::invalid("environment_id must be a UUID"))?;
    }
    // Nothing is stored before the end: a `use` that fails leaves the selection as it was.
    let mut project = read_project(
        ctx,
        ResolveRequest {
            path: request.path.clone(),
            team_id: None,
            cloud_project_id: request.cloud_project_id.clone(),
        },
    )
    .await?;
    // Either one: an environment alone names its project, a project alone its default
    // environment, and an environment must belong to the project.
    let mut environment_id = request.environment_id.clone();
    let mut fell_back_from = None;
    if project.cloud_project.is_some()
        && let Some(env) = &request.environment_id
        && !project.environments.iter().any(|e| e.id == *env)
    {
        let owner = project
            .cloud_projects
            .iter()
            .find(|c| c.environments.iter().any(|e| e.id == *env))
            .map(|c| c.id.clone());
        match (owner, &request.cloud_project_id) {
            (Some(id), None) => {
                project = read_project(
                    ctx,
                    ResolveRequest {
                        path: request.path.clone(),
                        team_id: None,
                        cloud_project_id: Some(id),
                    },
                )
                .await?;
            }
            // Listed nowhere any more (a Recent entry's environment was removed): the project's default.
            (None, Some(_)) => {
                fell_back_from = environment_id.take();
            }
            _ => {
                return Err(CloudError::invalid(
                    "the environment does not belong to that cloud project",
                ));
            }
        }
    }
    let environment_id = match (&environment_id, project.environment.as_ref()) {
        (Some(e), _) => e.clone(),
        (None, Some(default)) => default.id.clone(),
        (None, None) => {
            return Err(CloudError::invalid(
                "environment_id (or a cloud_project_id with environments) is required",
            ));
        }
    };
    let cfg = ctx.config().await;
    let conn = ctx.connection(&cfg);
    let team_id = project
        .team
        .as_ref()
        .map(|t| t.id.clone())
        .or_else(|| conn.api_key.as_ref().map(|k| k.team_id.clone()))
        .ok_or_else(|| {
            CloudError::invalid("the team is not known; pass team_id to project::resolve first")
        })?;
    let body = ctx
        .call(&Req::get(format!(
            "/teams/{team_id}/environments/{environment_id}"
        )))
        .await?;
    if is_removed(&body) {
        return Err(CloudError::new(
            "ENVIRONMENT_DELETED",
            "This environment was removed in iii cloud; choose another.",
        ));
    }
    let env = env_info(&body).ok_or_else(|| {
        CloudError::new(
            "CLOUD_ERROR",
            "iii cloud sent an environment without a branch.",
        )
    })?;
    if !project.environments.iter().any(|e| e.id == env.id) {
        project.environments.push(brief(&env));
    }
    let from_checkout = request.from == Some(UseFrom::Checkout);
    if request.source.is_some() && !from_checkout {
        return Err(CloudError::invalid("source is for from: checkout"));
    }
    if from_checkout && env.production {
        return Err(CloudError::invalid(
            "production deploys through a pull request: use from: environment",
        ));
    }
    let top = PathBuf::from(&project.path);
    // Before any file comes over: the run's files and `.env` stay out of git.
    exclude_run_files(&top).await?;
    let mut start = Start::Reused;
    let workspace = match request.mode {
        WorkspaceMode::Checkout => Workspace {
            mode: WorkspaceMode::Checkout,
            path: project.path.clone(),
            branch: project
                .checkout
                .branch
                .clone()
                .unwrap_or_else(|| "HEAD".into()),
            from_head: None,
            source: None,
            source_sha: None,
        },
        WorkspaceMode::Worktree => {
            let (workspace, how) = prepare_worktree(
                &top,
                &env,
                &project
                    .environments
                    .iter()
                    .map(|e| e.branch.clone())
                    .collect::<Vec<_>>(),
                project.default_branch.as_deref(),
                &request,
                &ctx.store.read(worktrees_made),
            )
            .await?;
            start = how;
            workspace
        }
    };
    let mut workspace = workspace;
    // A reused worktree keeps what it took from the checkout; a fresh or
    // resumed one has only what it took now.
    if start == Start::Reused {
        restore_taken(ctx, &mut workspace);
    }
    let base = match start == Start::Fresh {
        true => {
            let label = workspace
                .source
                .clone()
                .or_else(|| project.checkout.branch.clone())
                .unwrap_or_else(|| "HEAD".into());
            fresh_base(&top, &workspace, label, &env.branch).await
        }
        false => None,
    };
    project.environment = Some(env.clone());
    project.workspace = Some(workspace);
    ctx.store.update(|s| {
        // An API-key session has no cloud project to switch back to: not kept.
        if let Some(cp) = &project.cloud_project {
            s.remember_project(RecentProject {
                path: project.path.clone(),
                repository: project.repository.clone(),
                cloud_project_id: cp.id.clone(),
                cloud_project_name: cp.name.clone(),
                team_id: project.team.as_ref().map(|t| t.id.clone()),
                team_name: project.team.as_ref().map(|t| t.name.clone()),
                environment_id: Some(env.id.clone()),
                environment_name: Some(env.name.clone()),
                mode: request.mode,
                used_at: now(),
            });
        }
        s.project = Some(project.clone());
        s.candidate = None;
        s.project_deleted = false;
        s.environment_deleted = false;
        s.followed_since = Some(now());
        // The checks stay across folders: the environment's latest is its check again.
        s.check = s
            .checks
            .iter()
            .find(|c| c.environment_id == env.id)
            .cloned();
        // What belonged to the environment left behind is not shown for the new one.
        s.pr = None;
        s.pr_error = None;
        s.variables.clear();
        if let Some(w) = project.workspace.as_ref() {
            remember_worktree(s, w, start, base.clone());
        }
    });
    ctx.emit(
        EventKind::Project,
        "current",
        true,
        format!("project {} on {}", project.repository, env.name),
        &project,
    );
    crate::watch::ensure_poller(ctx);
    project.fell_back_from = fell_back_from;
    Ok(project)
}

/// The paths of the worktrees this worker made: the ones it recorded something about.
fn worktrees_made(s: &crate::store::State) -> Vec<String> {
    s.worktree_started
        .keys()
        .filter_map(|k| k.split('\n').next())
        .map(str::to_owned)
        .chain(s.worktree_base.keys().cloned())
        .chain(s.worktree_from_head.keys().cloned())
        .collect()
}

/// A reused worktree keeps what it took from the checkout, and from which branch.
fn restore_taken(ctx: &Ctx, w: &mut Workspace) {
    if w.from_head.is_none() {
        (w.from_head, w.source, w.source_sha) = ctx.store.read(|s| {
            (
                s.worktree_from_head.get(&w.path).cloned(),
                s.worktree_source.get(&w.path).cloned(),
                s.worktree_source_sha.get(&w.path).cloned(),
            )
        });
    }
}

/// Where a fresh worktree started, for "start again from the base": the
/// commit it took from the person (named `label`), else the environment's head.
async fn fresh_base(
    top: &Path,
    w: &Workspace,
    label: String,
    env_branch: &str,
) -> Option<(String, String)> {
    match &w.from_head {
        Some(sha) => Some((label, sha.clone())),
        None => git(
            top,
            &["rev-parse", &format!("refs/remotes/origin/{env_branch}")],
        )
        .await
        .ok()
        .map(|s| (env_branch.to_string(), s.trim().to_string())),
    }
}

/// What the worker keeps about a worktree it prepared.
fn remember_worktree(
    s: &mut crate::store::State,
    w: &Workspace,
    start: Start,
    base: Option<(String, String)>,
) {
    match &w.from_head {
        Some(head) => s.worktree_from_head.insert(w.path.clone(), head.clone()),
        None => s.worktree_from_head.remove(&w.path),
    };
    match &w.source {
        Some(source) => s.worktree_source.insert(w.path.clone(), source.clone()),
        None => s.worktree_source.remove(&w.path),
    };
    match &w.source_sha {
        Some(sha) => s.worktree_source_sha.insert(w.path.clone(), sha.clone()),
        None => s.worktree_source_sha.remove(&w.path),
    };
    // A resumed branch's commits are the agent's: what an earlier worktree in
    // this folder started from is not its start.
    if start == Start::Resumed {
        let key = format!("{}\n", w.path);
        s.worktree_started.retain(|k, _| !k.starts_with(&key));
        s.worktree_base.remove(&w.path);
    }
    if start == Start::Fresh {
        // What an earlier run left in a worktree at this path is not this one's.
        s.run_wrote.remove(&w.path);
        s.worktree_started
            .insert(format!("{}\n{}", w.path, w.branch), now());
        if let Some(base) = base {
            s.worktree_base.insert(w.path.clone(), base);
        }
    }
}

/// The repository's main checkout (the first `git worktree list` entry), from any of its worktrees.
pub async fn main_checkout(dir: &Path) -> Res<PathBuf> {
    parse_worktrees(&git(dir, &["worktree", "list", "--porcelain"]).await?)
        .into_iter()
        .next()
        .map(|w| PathBuf::from(w.path))
        .ok_or_else(|| CloudError::new("GIT_FAILED", "git lists no worktree here."))
}

/// A hand-off's own worktree, by the same rules as `use {mode: worktree, from:
/// checkout}` (with `source`, the selection's): the
/// worker's worktree for `env` when there is one, else a new one. When the
/// person's commits conflict with the environment's branch, the worktree stays
/// at the base and `conflict` tells the agent to merge them. Never changes the
/// selection.
pub async fn handoff_worktree(
    ctx: &Ctx,
    top: &Path,
    env: &EnvironmentInfo,
    source: Option<String>,
) -> Res<HandoffWorktree> {
    let (env_branches, default_branch) = ctx.store.read(|s| match &s.project {
        Some(p) if same_path(Path::new(&p.path), top) => (
            p.environments.iter().map(|e| e.branch.clone()).collect(),
            p.default_branch.clone(),
        ),
        _ => (vec![env.branch.clone()], None),
    });
    let ours = ctx.store.read(worktrees_made);
    let before = parse_worktrees(&git(top, &["worktree", "list", "--porcelain"]).await?);
    exclude_run_files(top).await?;
    let label = match &source {
        Some(s) => s.clone(),
        None => checkout_branch(top).await,
    };
    let request = UseRequest {
        path: top.to_string_lossy().into_owned(),
        environment_id: Some(env.id.clone()),
        mode: WorkspaceMode::Worktree,
        cloud_project_id: None,
        from: Some(UseFrom::Checkout),
        source,
    };
    let (env_branches, default_branch, ours) = (&env_branches, default_branch.as_deref(), &ours);
    let prepare = |r: UseRequest| async move {
        prepare_worktree(top, env, env_branches, default_branch, &r, ours).await
    };
    let (mut w, start, conflict) = match prepare(request.clone()).await {
        Ok((w, start)) => (w, start, None),
        Err(e) if e.code == "MERGE_CONFLICT" => {
            let plain = UseRequest {
                from: None,
                source: None,
                ..request
            };
            let (w, start) = prepare(plain).await?;
            let note = format!(
                "{label} did not come in ({}). In {}, run `git merge {label}`, resolve the conflicts, commit, then check.",
                e.message.trim_end_matches('.'),
                w.path
            );
            (w, start, Some(note))
        }
        Err(e) => return Err(e),
    };
    let created = !before
        .iter()
        .any(|b| same_path(Path::new(&b.path), Path::new(&w.path)));
    // A worktree made now from the base is a fresh start; one resumed on an
    // earlier fix branch is not.
    let start = match start {
        Start::Reused if created => Start::Fresh,
        other => other,
    };
    if start == Start::Reused {
        restore_taken(ctx, &mut w);
    }
    let base = match start == Start::Fresh {
        true => fresh_base(top, &w, label.clone(), &env.branch).await,
        false => None,
    };
    ctx.store.update(|s| remember_worktree(s, &w, start, base));
    let base = ctx.store.read(|s| s.worktree_base.get(&w.path).cloned());
    Ok(HandoffWorktree {
        merged_from: (conflict.is_none() && w.from_head.is_some()).then_some(label),
        path: w.path,
        branch: w.branch,
        base: base.map(|(branch, sha)| BriefBase { branch, sha }),
        created,
        conflict,
    })
}

/// The repository's default branch: the cloud's word, else what origin's HEAD points at.
pub async fn default_branch_of(top: &Path, known: Option<&str>) -> Option<String> {
    if let Some(k) = known.filter(|k| !k.is_empty()) {
        return Some(k.to_string());
    }
    git(
        top,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .await
    .ok()
    .map(|b| b.trim().trim_start_matches("origin/").to_string())
    .filter(|b| !b.is_empty())
}

/// How an environment gets a deploy: production by a pull request into its
/// branch; every other environment by an assisted deploy, which pushes the
/// checked commit to a branch only that environment follows (`own`: the branch
/// it moves to on its first deploy, when its current one is not its own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Way {
    PullRequest,
    /// `taken`: the branch it would push (`own`, else its own current one) is
    /// followed by another environment, so nothing may be pushed to it.
    Assisted {
        own: Option<String>,
        taken: bool,
    },
}

/// The branch an environment's assisted deploys push to.
pub fn own_branch(name: &str) -> String {
    format!("iii-cloud/{name}")
}

/// `others`: the branches the project's other environments follow. A branch
/// that is the default, shared, or whose default is unknown is never pushed:
/// the environment gets its own, and when another environment follows that one
/// too, nothing is pushed at all (`taken`). Mirrored by `deployWay` in `ui/src/model.ts`.
pub fn deploy_way(default: Option<&str>, env: &EnvironmentInfo, others: &[String]) -> Way {
    if env.production {
        return Way::PullRequest;
    }
    let own = own_branch(&env.name);
    let moves = env.branch != own
        && (default.is_none_or(|d| d == env.branch) || others.contains(&env.branch));
    let target = if moves { &own } else { &env.branch };
    Way::Assisted {
        taken: others.contains(target),
        own: moves.then_some(own),
    }
}

/// An environment that tracks a branch other than the default. Only the
/// worktree setup uses it, to start from the default branch while the
/// environment's own branch is not on origin yet.
pub fn tracks_other_branch(default: Option<&str>, env_branch: &str) -> bool {
    default.is_some_and(|d| d != env_branch)
}

/// The workspace, and whether this is a fresh start (a new worktree, or a reused
/// one that began again from the base).
async fn prepare_worktree(
    top: &Path,
    env: &EnvironmentInfo,
    env_branches: &[String],
    default_branch: Option<&str>,
    request: &UseRequest,
    ours: &[String],
) -> Res<(Workspace, Start)> {
    let from_checkout = request.from == Some(UseFrom::Checkout);
    let source = request.source.as_deref();
    // Forget remote branches that are gone, so an earlier try's branch is not mistaken for this one's.
    let _ = git(top, &["fetch", "--prune", "origin"]).await;
    let default = default_branch_of(top, default_branch).await;
    let direct = default
        .as_deref()
        .filter(|d| tracks_other_branch(Some(d), &env.branch));
    // The fix branch starts from the environment's branch; an environment's own
    // branch that is not on origin yet starts from the default branch.
    let origin = match (git(top, &["fetch", "origin", &env.branch]).await, direct) {
        (Ok(_), _) => format!("origin/{}", env.branch),
        (Err(_), Some(d)) => {
            git(top, &["fetch", "origin", d]).await?;
            format!("origin/{d}")
        }
        (Err(e), None) => return Err(e),
    };
    // `from: checkout`: the person's HEAD, only read; the worktree takes it below.
    let from = match (from_checkout, source) {
        (false, _) => None,
        (true, Some(source)) => Some(source_commit(top, source).await?),
        (true, None) => Some(git(top, &["rev-parse", "HEAD"]).await?.trim().to_string()),
    };
    let dir_name = top
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let wanted = top
        .parent()
        .unwrap_or(top)
        .join(format!("{dir_name}-iii-cloud"));
    // A worktree folder deleted by hand still holds its branch until pruned.
    let _ = git(top, &["worktree", "prune"]).await;
    let list = parse_worktrees(&git(top, &["worktree", "list", "--porcelain"]).await?);
    // A worktree this worker made (it recorded it, or it has its name); never one the person made.
    let named = |p: &Path| {
        let parent = top.parent().unwrap_or(top);
        [
            format!("{dir_name}-iii-cloud"),
            format!("{dir_name}-iii-cloud-{}", env.name),
        ]
        .iter()
        .any(|n| same_path(p, &parent.join(n)))
    };
    let made_here = |p: &Path| named(p) || ours.iter().any(|o| same_path(Path::new(o), p));
    let base_name = format!("iii-cloud/ready-{}", env.name);
    let has = |r: String| async move {
        git(top, &["rev-parse", "--verify", "--quiet", &r])
            .await
            .is_ok()
    };
    // The first of `ready-<env>`, `ready-<env>-2`, ... that is free here: a
    // worktree of ours on it is reused, a branch left without a worktree (here
    // or on origin) holds the agent's earlier work and is resumed. A name out
    // in the person's checkout, or in a folder this worker did not make, is
    // theirs: the next name.
    let mut branch = base_name.clone();
    let mut n = 1;
    let found = loop {
        // A name some environment tracks is that environment's, not this try's.
        let taken = env_branches.contains(&branch)
            || list.iter().any(|w| {
                w.branch.as_deref() == Some(branch.as_str()) && {
                    let p = Path::new(&w.path);
                    same_path(top, p) || !made_here(p)
                }
            });
        if taken {
            n += 1;
            branch = format!("{base_name}-{n}");
            continue;
        }
        if let Some(w) = list
            .iter()
            .find(|w| w.branch.as_deref() == Some(branch.as_str()))
        {
            break Found::Worktree(PathBuf::from(&w.path));
        }
        let local = has(format!("refs/heads/{branch}")).await;
        let remote = has(format!("refs/remotes/origin/{branch}")).await;
        break match (local, remote) {
            (false, false) => Found::New,
            _ => Found::Branch { local, remote },
        };
    };
    // An older worker put a direct environment's worktree on the environment's
    // own branch: it moves to the fix branch with its commits and changes (the
    // local environment branch keeps them too).
    let older = match (&found, direct) {
        (Found::New, Some(_)) => list
            .iter()
            .filter(|w| w.branch.as_deref() == Some(env.branch.as_str()))
            .map(|w| PathBuf::from(&w.path))
            .find(|p| p.exists() && !same_path(top, p) && made_here(p)),
        _ => None,
    };
    // `<dir>-iii-cloud` may be the worktree of another environment (a
    // switch): this environment gets its own, named after it.
    let new_path = || {
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let wanted = if list
            .iter()
            .any(|w| w.branch.is_some() && canon(Path::new(&w.path)) == canon(&wanted))
        {
            wanted.with_file_name(format!("{dir_name}-iii-cloud-{}", env.name))
        } else {
            wanted.clone()
        };
        if wanted.exists() {
            return Err(CloudError::new(
                "WORKTREE_PATH_TAKEN",
                format!(
                    "{} exists and is not a worktree on {branch}.",
                    wanted.display()
                ),
            ));
        }
        Ok(wanted)
    };
    let add = |args: Vec<String>| async move {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        git(top, &args)
            .await
            .map(|_| ())
            .map_err(|e| CloudError::new("WORKTREE_FAILED", e.message))
    };
    let (path, start) = match (found, older) {
        (Found::Worktree(path), _) => {
            let reset = refresh_merged_worktree(top, &path, &branch, &origin).await;
            (path, if reset { Start::Fresh } else { Start::Reused })
        }
        (Found::New, Some(path)) => {
            git(&path, &["checkout", "-b", &branch])
                .await
                .map_err(|e| CloudError::new("WORKTREE_FAILED", e.message))?;
            // Not a fresh start: its commits are the agent's work, not an earlier try's.
            (path, Start::Reused)
        }
        (Found::Branch { local, remote }, _) => {
            let path = new_path()?;
            let target = path.to_string_lossy().into_owned();
            let theirs = format!("origin/{branch}");
            if local {
                add(vec![
                    "worktree".into(),
                    "add".into(),
                    target,
                    branch.clone(),
                ])
                .await?;
                // Pushed from elsewhere since: the local branch catches up when it can.
                if remote {
                    let _ = git(&path, &["merge", "--ff-only", "-q", &theirs]).await;
                }
            } else {
                add(vec![
                    "worktree".into(),
                    "add".into(),
                    "-b".into(),
                    branch.clone(),
                    target,
                    theirs,
                ])
                .await?;
            }
            catch_up(&path, &branch, &origin, &env.branch).await?;
            (path, Start::Resumed)
        }
        (Found::New, None) => {
            let path = new_path()?;
            let target = path.to_string_lossy().into_owned();
            add(vec![
                "worktree".into(),
                "add".into(),
                "-b".into(),
                branch.clone(),
                target,
                origin.clone(),
            ])
            .await?;
            (path, Start::Fresh)
        }
    };
    // Never in the person's own checkout, even when it sits on this branch.
    if let Some(head) = &from
        && !same_path(top, &path)
    {
        let label = match source {
            Some(s) => s.to_string(),
            None => checkout_branch(top).await,
        };
        // All or nothing: what the worktree was on before anything touches it.
        let before = snapshot(&path).await.ok();
        if let Err(e) = take_checkout(&path, head, &label, &origin, &env.branch).await {
            return Err(put_back(&path, before.as_ref(), true, e).await);
        }
    }
    let from_head = match from {
        Some(_) => Some(git(&path, &["rev-parse", "HEAD"]).await?.trim().to_string()),
        None => None,
    };
    // What the source pointed at when it was taken: "moved since" compares with it.
    let source_sha = from;
    Ok((
        Workspace {
            mode: WorkspaceMode::Worktree,
            path: path.to_string_lossy().into_owned(),
            branch,
            from_head,
            source: source.filter(|_| from_checkout).map(str::to_owned),
            source_sha,
        },
        start,
    ))
}

/// How `prepare_worktree` got its worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// Made from the base now (or reset to it): an earlier try's commits are not this one's.
    Fresh,
    /// A worktree of ours that was there, with its work.
    Reused,
    /// A new worktree on a fix branch left from before: its commits are the agent's work.
    Resumed,
}

/// What `ready-<env>[-n]` is here.
enum Found {
    Worktree(PathBuf),
    Branch { local: bool, remote: bool },
    New,
}

/// A resumed fix branch takes the environment's base (`origin`): reset to it
/// when the branch holds nothing beyond it (merged), else a fast-forward or a
/// merge commit. A conflict leaves the worktree as it was (`MERGE_CONFLICT`).
async fn catch_up(wt: &Path, branch: &str, origin: &str, env_branch: &str) -> Res<()> {
    if is_ancestor(wt, "HEAD", origin).await? {
        return git(wt, &["reset", "--hard", "-q", origin])
            .await
            .map(|_| ());
    }
    let message = format!("Merge {env_branch} into {branch}");
    match merge_commit(wt, origin, &message).await? {
        Ok(()) => Ok(()),
        Err(files) => Err(CloudError::new(
            "MERGE_CONFLICT",
            format!(
                "{branch} holds your agent's earlier work, and it conflicts with {env_branch}: your agent can run `git merge {origin}` in the worktree {} and resolve them. Conflicting files: {}.",
                wt.display(),
                files.join(", ")
            ),
        )),
    }
}

/// A merge commit of `what` in `wt`. A conflict aborts it and answers the
/// conflicting files; a merge git refused to begin is git's error.
async fn merge_commit(wt: &Path, what: &str, message: &str) -> Res<Result<(), Vec<String>>> {
    let Err(e) = git(
        wt,
        &["merge", "--no-ff", "--no-edit", "-q", "-m", message, what],
    )
    .await
    else {
        return Ok(Ok(()));
    };
    let files = git(wt, &["diff", "--name-only", "--diff-filter=U"])
        .await
        .unwrap_or_default();
    let files: Vec<String> = files
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    // There was no merge on entry: one now is this call's own.
    if merging(wt).await {
        let _ = git(wt, &["merge", "--abort"]).await;
    }
    // Refused before it began (changes in the way): git's own words.
    if files.is_empty() {
        return Err(e);
    }
    Ok(Err(files))
}

/// A branch `use {source}` names: local, else `origin/<name>` from origin's refs.
async fn source_commit(top: &Path, source: &str) -> Res<String> {
    let mut refs = vec![format!("refs/heads/{source}")];
    if source.starts_with("origin/") {
        refs.push(format!("refs/remotes/{source}"));
    }
    for r in refs {
        if let Ok(sha) = git(
            top,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{r}^{{commit}}"),
            ],
        )
        .await
        {
            return Ok(sha.trim().to_string());
        }
    }
    Err(CloudError::invalid(format!(
        "{source} is not a branch here: name one of checkout.branches"
    )))
}

/// Local branches and origin's, newest commit first, at most 30; a local
/// branch hides its origin twin. `ahead` counts what `origin/<env_branch>`
/// lacks. The refs as they are: no fetch.
pub async fn branches(dir: &Path, env_branch: &str) -> Vec<BranchRef> {
    let Ok(out) = git(
        dir,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%1f%(objectname)%1f%(committerdate:iso-strict)",
            "refs/heads",
            "refs/remotes/origin",
        ],
    )
    .await
    else {
        return vec![];
    };
    let rows: Vec<(String, bool, &str, &str)> = out
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\x1f');
            let (r, sha, at) = (f.next()?, f.next()?, f.next()?);
            let (name, remote) = match r.strip_prefix("refs/heads/") {
                Some(n) => (n.to_string(), false),
                None => (r.strip_prefix("refs/remotes/")?.to_string(), true),
            };
            (name != "origin/HEAD").then_some((name, remote, sha, at))
        })
        .collect();
    let local: std::collections::HashSet<&str> =
        rows.iter().filter(|r| !r.1).map(|r| r.0.as_str()).collect();
    let base = format!("refs/remotes/origin/{env_branch}");
    let has_base = git(dir, &["rev-parse", "--verify", "--quiet", &base])
        .await
        .is_ok();
    let mut list = vec![];
    for (name, remote, sha, at) in rows
        .iter()
        .filter(|r| {
            !(r.1
                && r.0
                    .strip_prefix("origin/")
                    .is_some_and(|n| local.contains(n)))
        })
        .take(30)
    {
        let ahead = match has_base {
            true => git(dir, &["rev-list", "--count", &format!("{base}..{sha}")])
                .await
                .ok()
                .and_then(|n| n.trim().parse().ok()),
            false => None,
        };
        let short = match remote {
            true => name.strip_prefix("origin/").unwrap_or(name),
            false => name,
        };
        list.push(BranchRef {
            is_env_branch: short == env_branch,
            name: name.clone(),
            remote: *remote,
            sha: sha.to_string(),
            committed_at: at.to_string(),
            ahead,
        });
    }
    list
}

/// A reused worktree whose work was merged (a squash merge leaves its commits
/// off the base) starts again from the base, when it is clean.
pub fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

async fn refresh_merged_worktree(top: &Path, wt: &Path, branch: &str, origin: &str) -> bool {
    // Never the person's own checkout, even when it sits on this branch.
    if same_path(top, wt) {
        return false;
    }
    let clean = git(wt, &["status", "--porcelain"])
        .await
        .is_ok_and(|s| s.trim().is_empty());
    let ahead = git(wt, &["rev-list", "--count", &format!("{origin}..HEAD")])
        .await
        .ok()
        .and_then(|n| n.trim().parse::<u32>().ok())
        .unwrap_or(0);
    if !clean || ahead == 0 {
        return false;
    }
    let Ok(head) = git(wt, &["rev-parse", "HEAD"]).await else {
        return false;
    };
    let merged = tool::run(
        "gh",
        &[
            "pr",
            "list",
            "--head",
            branch,
            "--state",
            "merged",
            "--json",
            "number,headRefOid",
            "--limit",
            "20",
        ],
        Some(top),
        None,
        "GH_FAILED",
    )
    .await
    .ok()
    .filter(|o| o.ok())
    .and_then(|o| serde_json::from_str::<Value>(&o.stdout).ok())
    // Reset only when the merged pull request's head is exactly what this
    // worktree holds: local commits that were never pushed are never lost.
    .and_then(|v| {
        v.as_array().map(|a| {
            a.iter()
                .any(|p| p.get("headRefOid").and_then(Value::as_str) == Some(head.trim()))
        })
    })
    .unwrap_or(false);
    if merged {
        return git(wt, &["reset", "--hard", origin]).await.is_ok();
    }
    false
}

async fn is_ancestor(dir: &Path, a: &str, b: &str) -> Res<bool> {
    Ok(tool::run(
        "git",
        &["merge-base", "--is-ancestor", a, b],
        Some(dir),
        None,
        "TOOL_MISSING",
    )
    .await?
    .ok())
}

async fn checkout_branch(top: &Path) -> String {
    git(top, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .map(|b| b.trim().to_string())
        .unwrap_or_else(|_| "HEAD".into())
}

/// `from: checkout`: the worktree `wt` takes the person's `head` (the checkout's
/// HEAD, or the `source` branch, named `mine` in a merge) with what the environment has (`origin`), by fast-forward when it can, else by a merge
/// commit made in the worktree. The checkout is only read. A conflict leaves
/// the worktree as it was (`MERGE_CONFLICT`, the files in the message).
async fn take_checkout(
    wt: &Path,
    head: &str,
    mine: &str,
    origin: &str,
    env_branch: &str,
) -> Res<()> {
    // A merge someone started here and has not committed: never touched.
    if merging(wt).await {
        return Err(CloudError::new(
            "MERGE_IN_PROGRESS",
            format!(
                "Your agent started merging {mine} in {} and hasn't committed it; it can finish the merge (commit) and check again.",
                wt.display()
            ),
        ));
    }
    // A worktree that was there catches up with the environment when none of its own commits are in the way.
    let _ = git(wt, &["merge", "--ff-only", "-q", origin]).await;
    // Nothing of the person's that the environment lacks: the environment's head is the start.
    if is_ancestor(wt, head, origin).await?
        || git(wt, &["merge", "--ff-only", "-q", head]).await.is_ok()
    {
        return Ok(());
    }
    let message = format!("Merge {mine} into {env_branch} for the check");
    match merge_commit(wt, head, &message).await? {
        Ok(()) => Ok(()),
        Err(files) => Err(CloudError::new(
            "MERGE_CONFLICT",
            format!(
                "The commits of {mine} conflict with {env_branch}: your agent can merge {mine} in the worktree {} and resolve them. Conflicting files: {}.",
                wt.display(),
                files.join(", ")
            ),
        )),
    }
}

/// The worktree before the source is taken: its HEAD, its tracked changes as a stash
/// commit (`git stash create` moves nothing), and its untracked files.
struct Snapshot {
    head: String,
    branch: String,
    changes: Option<String>,
    /// The tracked changes could not be saved: a reset would lose them.
    unsaved: bool,
    untracked: std::collections::HashSet<String>,
}

async fn snapshot(wt: &Path) -> Res<Snapshot> {
    let head = git(wt, &["rev-parse", "HEAD"]).await?.trim().to_string();
    let branch = git(wt, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await?
        .trim()
        .to_string();
    let dirty = !git(wt, &["status", "--porcelain", "--untracked-files=no"])
        .await?
        .trim()
        .is_empty();
    let changes = match dirty {
        true => git(wt, &["stash", "create"])
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        false => None,
    };
    let untracked =
        nul_list(&git(wt, &["ls-files", "-z", "--others", "--exclude-standard"]).await?).collect();
    Ok(Snapshot {
        head,
        branch,
        unsaved: dirty && changes.is_none(),
        changes,
        untracked,
    })
}

/// Undo a take that failed half-way: the worktree's HEAD and tracked changes
/// as they were, and only the untracked files the take added go.
async fn restore(wt: &Path, before: &Snapshot) {
    // ponytail: without a saved copy of the agent's tracked changes (no git identity for `stash create`), the reset is skipped; a half-applied patch then stays.
    if !before.unsaved {
        let _ = git(wt, &["reset", "-q", "--hard", &before.head]).await;
        if let Some(changes) = &before.changes {
            let _ = git(wt, &["stash", "apply", "-q", changes]).await;
        }
    }
    let Ok(now) = git(wt, &["ls-files", "-z", "--others", "--exclude-standard"]).await else {
        return;
    };
    let added: Vec<String> = nul_list(&now)
        .filter(|f| !before.untracked.contains(f))
        .collect();
    if !added.is_empty() {
        let mut clean = vec!["--literal-pathspecs", "clean", "-fdq", "--"];
        clean.extend(added.iter().map(String::as_str));
        let _ = git(wt, &clean).await;
    }
}

/// Put the worktree back where `before` found it and hand `error` on. When it
/// cannot be put back, the error says so and names the worktree.
/// `only_if_moved`: leave it alone while HEAD and branch are where they were
/// (a refusal before anything changed, such as an agent's own merge in progress).
async fn put_back(
    wt: &Path,
    before: Option<&Snapshot>,
    only_if_moved: bool,
    error: CloudError,
) -> CloudError {
    let Some(before) = before else {
        return error;
    };
    let at = |wt: &Path| {
        let wt = wt.to_path_buf();
        async move {
            (
                git(&wt, &["rev-parse", "HEAD"]).await.ok(),
                git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]).await.ok(),
            )
        }
    };
    let (head, branch) = at(wt).await;
    let same = head.as_deref().map(str::trim) == Some(before.head.as_str())
        && branch.as_deref().map(str::trim) == Some(before.branch.as_str());
    if only_if_moved && same {
        return error;
    }
    restore(wt, before).await;
    let (head, branch) = at(wt).await;
    if head.as_deref().map(str::trim) == Some(before.head.as_str())
        && branch.as_deref().map(str::trim) == Some(before.branch.as_str())
    {
        return error;
    }
    CloudError::new(
        &error.code,
        format!(
            "{} The worktree {} could not be put back on {} ({}): look at it before using it.",
            error.message,
            wt.display(),
            before.branch,
            &before.head[..before.head.len().min(7)]
        ),
    )
}

/// A merge is in progress in `wt` (`MERGE_HEAD` is there).
async fn merging(wt: &Path) -> bool {
    git(wt, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .await
        .is_ok()
}

fn nul_list(out: &str) -> impl Iterator<Item = String> + '_ {
    out.split('\0').filter(|f| !f.is_empty()).map(str::to_owned)
}

/// Hide the run's own files in `info/exclude` (shared with the checkout).
async fn exclude_run_files(top: &Path) -> Res<()> {
    let rel = git(top, &["rev-parse", "--git-path", "info/exclude"]).await?;
    let file = top.join(rel.trim());
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    let mut add = String::new();
    for pattern in [
        ".iii/",
        "*.iii-cloud-run.yaml",
        "*.iii-cloud-run.lock",
        // Config files the run's compose writes next to the manifest (config/iii-cloud-run-*.yaml).
        "iii-cloud-run-*.yaml",
        ".env",
    ] {
        if !current.lines().any(|l| l == pattern) {
            add.push_str(pattern);
            add.push('\n');
        }
    }
    if add.is_empty() {
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| CloudError::new("GIT_FAILED", e.to_string()))?;
    }
    let sep = if current.is_empty() || current.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    std::fs::write(&file, format!("{current}{sep}{add}"))
        .map_err(|e| CloudError::new("GIT_FAILED", e.to_string()))
}
