//! `iii-cloud::project::create` and `::create_environment`: the person's explicit
//! choice to make a new cloud project or environment. Nothing is created without
//! one of these calls, and both are page-only (denied to agents).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::cloud::{Req, cloud_error};
use crate::ctx::Ctx;
use crate::error::{CloudError, Res};
use crate::project::{resolve, use_project};
use crate::types::*;

/// `^[a-z0-9]+(?:-[a-z0-9]+)*$`, at most 30 characters (the cloud's environment name).
pub fn valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 30
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

fn lower_text(body: &Value) -> String {
    format!(
        "{} {}",
        body.get("code").and_then(Value::as_str).unwrap_or_default(),
        body.get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .to_lowercase()
}

/// The cloud's refusals of a create, as the codes the page shows.
/// `name_taken` is what a 409 means here (`ENV_NAME_TAKEN` or `PROJECT_NAME_TAKEN`).
pub fn create_error(status: u16, body: &Value, name_taken: &str) -> CloudError {
    let text = lower_text(body);
    let cloud = body
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let with = |code: &str, message: &str| {
        CloudError::new(
            code,
            if cloud.is_empty() {
                message.to_string()
            } else {
                format!("{message} ({cloud})")
            },
        )
        .with_status(status)
    };
    if text.contains("limit") || status == 402 {
        return with("PLAN_LIMIT", "The team's plan limit is reached.");
    }
    match status {
        403 => with("TEAM_FORBIDDEN", "Your role in this team can't do that."),
        409 => with(name_taken, "That name is already taken."),
        422 if text.contains("manifest") => with(
            "MANIFEST_NOT_FOUND",
            "The compose file is not in the repository on that branch.",
        ),
        422 if text.contains("branch")
            && (text.contains("not found")
                || text.contains("exist")
                || text.contains("unknown")) =>
        {
            with(
                "BRANCH_NOT_FOUND",
                "The repository does not have that branch.",
            )
        }
        422 if text.contains("branch") => {
            with("INVALID_BRANCH", "That is not a valid branch name.")
        }
        _ => cloud_error(status, body),
    }
}

/// An environment create's refusals.
pub fn environment_error(status: u16, body: &Value) -> CloudError {
    create_error(status, body, "ENV_NAME_TAKEN")
}

/// The key with this attempt's number: the same attempt replays the same
/// create, a retry after a refusal is a new one (like a deploy's).
fn keyed(ctx: &Ctx, key: &str) -> String {
    let attempt = ctx
        .store
        .read(|s| s.create_attempts.get(key).copied().unwrap_or(0));
    format!("{key}:{attempt}")
}

/// The key has had its answer, a refusal or a create that was made: the next create of the same thing gets
/// a new key. Only an answer that never arrived keeps the key, so a retry cannot make a second one.
fn spent(ctx: &Ctx, key: &str) {
    ctx.store
        .update(|s| *s.create_attempts.entry(key.to_string()).or_insert(0) += 1);
}

/// Whether what a create answered with is already deleted in iii cloud. iii cloud answers a key it has
/// seen with the answer it gave then, so a project or an environment made with that key and deleted since
/// comes back as if just made (this machine's count of the key is lost with its records).
async fn deleted(ctx: &Ctx, path: String) -> bool {
    match ctx.raw(&Req::get(path)).await {
        Ok((404, _)) => true,
        Ok((200, v)) => v.get("delete_after").is_some_and(|d| !d.is_null()),
        _ => false,
    }
}

/// `iii-cloud::project::create`: import the repository as a new cloud project.
pub async fn create_project(ctx: &Arc<Ctx>, request: ProjectCreateRequest) -> Res<CreateResponse> {
    if !valid_env_name(&request.environment_name) {
        return Err(CloudError::invalid(
            "environment_name must be lowercase letters, digits and hyphens, at most 30",
        ));
    }
    // The repository, as the folder says; the importable data, as the cloud says.
    let project = resolve(
        ctx,
        ResolveRequest {
            path: request.path.clone(),
            team_id: Some(request.team_id.clone()),
            cloud_project_id: None,
        },
    )
    .await?;
    let want = project.repository.to_lowercase();
    let listed = ctx
        .call(&Req::get(format!(
            "/teams/{}/github/repositories?full_name={want}",
            request.team_id
        )))
        .await?;
    let repo = listed
        .get("repositories")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|r| {
            r.get("full_name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.to_lowercase() == want)
        })
        .cloned()
        .ok_or_else(|| {
            CloudError::invalid("the team's GitHub App can't see this repository; install it first")
        })?;
    let repository_id = repo.get("id").and_then(Value::as_i64).ok_or_else(|| {
        CloudError::new(
            "CLOUD_ERROR",
            "iii cloud sent the repository without an id.",
        )
    })?;
    let choosable = repo
        .get("manifests")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|m| {
            m.get("path").and_then(Value::as_str) == Some(request.manifest.as_str())
                && m.get("choosable").and_then(Value::as_bool).unwrap_or(false)
        });
    if !choosable {
        return Err(CloudError::new(
            "MANIFEST_NOT_FOUND",
            "That compose file is not one of the repository's choosable manifests.",
        ));
    }
    let production = request.production.unwrap_or(true);
    let mut post = Req::post(
        format!("/teams/{}/projects", request.team_id),
        json!({
            "repository_id": repository_id,
            "manifest": request.manifest,
            "environment_name": request.environment_name,
            "production": production,
        }),
    );
    let key = format!(
        "iii-cloud:project:{}:{repository_id}:{}:{}",
        request.team_id, request.manifest, request.environment_name
    );
    post.idempotency_key = Some(keyed(ctx, &key));
    let (mut status, mut body) = ctx.raw(&post).await?;
    if (200..300).contains(&status)
        && let Some(id) = body.pointer("/project/id").and_then(Value::as_str)
        && deleted(ctx, format!("/teams/{}/projects/{id}", request.team_id)).await
    {
        // An earlier create's answer, replayed: one new key makes the new project.
        spent(ctx, &key);
        post.idempotency_key = Some(keyed(ctx, &key));
        (status, body) = ctx.raw(&post).await?;
    }
    spent(ctx, &key);
    if !(200..300).contains(&status) {
        return Err(create_error(status, &body, "PROJECT_NAME_TAKEN"));
    }
    let project_id = body
        .pointer("/project/id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let environment_id = body
        .pointer("/environment/id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if project_id.is_empty() || environment_id.is_empty() {
        return Err(CloudError::new(
            "CLOUD_ERROR",
            "iii cloud sent a project without its ids.",
        ));
    }
    // Select it: the worktree and the environment, with the cloud projects read fresh.
    // While another project is selected (adding one), it becomes the candidate instead.
    let selected = if adding(ctx) {
        resolve(
            ctx,
            ResolveRequest {
                path: request.path,
                team_id: None,
                cloud_project_id: Some(project_id.clone()),
            },
        )
        .await?
    } else {
        use_project(
            ctx,
            UseRequest {
                path: request.path,
                environment_id: Some(environment_id),
                mode: WorkspaceMode::Worktree,
                cloud_project_id: Some(project_id.clone()),
                from: None,
                source: None,
            },
        )
        .await?
    };
    Ok(CreateResponse {
        cloud_projects: selected.cloud_projects.clone(),
        project_id: Some(project_id),
        environment_id: selected.environment.as_ref().map(|e| e.id.clone()),
        project: selected,
    })
}

/// A project with a workspace is selected: `resolve` keeps a new folder as the candidate.
fn adding(ctx: &Ctx) -> bool {
    ctx.store
        .read(|s| s.project.as_ref().is_some_and(|p| p.workspace.is_some()))
}

/// `iii-cloud::project::create_environment`: a new environment in a cloud project.
pub async fn create_environment(
    ctx: &Arc<Ctx>,
    request: EnvironmentCreateRequest,
) -> Res<CreateResponse> {
    if !valid_env_name(&request.name) {
        return Err(CloudError::invalid(
            "name must be lowercase letters, digits and hyphens, at most 30",
        ));
    }
    // The project's team, and its environments, as the cloud has them now: the
    // selected project's, else the candidate's (adding a project; never selected here).
    let find = |p: &Project| {
        p.cloud_projects
            .iter()
            .find(|c| c.id == request.cloud_project_id)
            .cloned()
    };
    let (selected, candidate) = ctx.store.read(|s| (s.project.clone(), s.candidate.clone()));
    let mut found = selected
        .as_ref()
        .and_then(|p| find(p).map(|c| (p.clone(), c, false)))
        .or_else(|| {
            candidate
                .as_ref()
                .and_then(|p| find(p).map(|c| (p.clone(), c, true)))
        });
    if found.is_none() {
        crate::project::refresh_cloud_projects(ctx).await;
        found = ctx.store.read(|s| {
            s.project
                .as_ref()
                .and_then(|p| find(p).map(|c| (p.clone(), c, false)))
        });
    }
    if found.is_none() && selected.is_none() && candidate.is_none() {
        return Err(CloudError::no_project());
    }
    let (stored, cloud, of_candidate) =
        found.ok_or_else(|| CloudError::invalid("no such cloud project for this repository"))?;
    let branch = request
        .branch
        .clone()
        .or_else(|| stored.default_branch.clone())
        .ok_or_else(|| {
            CloudError::invalid("branch is required: the repository's default branch is not known")
        })?;
    let manifest = request
        .manifest
        .clone()
        .or_else(|| stored.environment.as_ref().and_then(|e| e.manifest.clone()))
        .unwrap_or_else(|| "worker-compose.yaml".to_string());
    // A branch the repository does not have is refused here, before anything is created.
    if request.branch.is_some() {
        let found = crate::tool::run(
            "git",
            &["ls-remote", "--heads", "origin", &branch],
            Some(std::path::Path::new(&stored.path)),
            None,
            "TOOL_MISSING",
        )
        .await?;
        if found.ok() && found.stdout.trim().is_empty() {
            return Err(CloudError::new(
                "BRANCH_NOT_FOUND",
                format!("{branch} is not on origin: push it first, or pick another branch."),
            ));
        }
    }
    // Secrets and settings carry over from production, unless the caller says otherwise.
    let start_empty = request.start_empty.unwrap_or(false);
    if start_empty && request.copy_from.is_some() {
        return Err(CloudError::invalid(
            "start_empty and copy_from exclude each other",
        ));
    }
    let copy_from = request.copy_from.clone().or_else(|| {
        cloud
            .environments
            .iter()
            .find(|e| e.production && !start_empty)
            .map(|e| e.id.clone())
    });
    let production = request.production.unwrap_or(false);
    let mut body = json!({ "name": request.name, "manifest": manifest, "branch": branch, "production": production });
    if let Some(from) = &copy_from {
        body["copy_from"] = json!(from);
    }
    let mut post = Req::post(
        format!(
            "/teams/{}/projects/{}/environments",
            cloud.team.id, cloud.id
        ),
        body,
    );
    let key = format!("iii-cloud:env:{}:{}", cloud.id, request.name);
    post.idempotency_key = Some(keyed(ctx, &key));
    let (mut status, mut answer) = ctx.raw(&post).await?;
    if (200..300).contains(&status)
        && let Some(id) = answer.get("id").and_then(Value::as_str)
        && deleted(ctx, format!("/teams/{}/environments/{id}", cloud.team.id)).await
    {
        // An earlier create's answer, replayed: one new key makes the new environment.
        spent(ctx, &key);
        post.idempotency_key = Some(keyed(ctx, &key));
        (status, answer) = ctx.raw(&post).await?;
    }
    spent(ctx, &key);
    if !(200..300).contains(&status) {
        return Err(environment_error(status, &answer));
    }
    let environment_id = answer
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if environment_id.is_empty() {
        return Err(CloudError::new(
            "CLOUD_ERROR",
            "iii cloud sent an environment without an id.",
        ));
    }
    if of_candidate {
        let project = resolve(
            ctx,
            ResolveRequest {
                path: stored.path,
                team_id: None,
                cloud_project_id: Some(cloud.id.clone()),
            },
        )
        .await?;
        return Ok(CreateResponse {
            cloud_projects: project.cloud_projects.clone(),
            project_id: Some(cloud.id),
            environment_id: Some(environment_id),
            project,
        });
    }
    let selected = use_project(
        ctx,
        UseRequest {
            path: stored.path,
            environment_id: Some(environment_id.clone()),
            mode: stored
                .workspace
                .as_ref()
                .map(|w| w.mode)
                .unwrap_or(WorkspaceMode::Worktree),
            cloud_project_id: Some(cloud.id.clone()),
            from: None,
            source: None,
        },
    )
    .await?;
    Ok(CreateResponse {
        cloud_projects: selected.cloud_projects.clone(),
        project_id: Some(cloud.id),
        environment_id: Some(environment_id),
        project: selected,
    })
}
