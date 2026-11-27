//! `iii-cloud::status` (spec §11): a store read. `refresh: true` first
//! re-reads the checkout, the pull request, the variables and the deployments.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::check::{local_iii_version, selected};
use crate::cloud::Req;
use crate::ctx::{Ctx, now};
use crate::error::Res;
use crate::error::WorkerError;
use crate::types::*;

/// A waiting request for a database URL the cloud provides is not needed: it
/// becomes `not_needed` (noted) unless the person chose an outside database.
fn settle_database_requests(ctx: &Ctx) {
    let changed: Vec<SecretRequest> = ctx.store.update(|s| {
        let mut out = vec![];
        for r in s.secrets.iter_mut() {
            if r.state == SecretState::Waiting
                && !r.external
                && crate::local::database_shaped(&r.name)
            {
                r.state = SecretState::Noted;
                r.mode = SecretMode::NotNeeded;
                r.resolved_at = Some(now());
                out.push(r.clone());
            }
        }
        out
    });
    for r in changed {
        ctx.emit(
            EventKind::Secret,
            &r.id,
            true,
            format!("{} is not needed: iii cloud provides the database", r.name),
            &r,
        );
    }
}

pub async fn status(ctx: &Arc<Ctx>, request: StatusRequest) -> Res<Status> {
    settle_database_requests(ctx);
    if request.refresh.unwrap_or(false) {
        refresh(ctx).await;
    }
    let cfg = ctx.config().await;
    let connection = ctx.connection(&cfg);
    let secrets = crate::handoff::env_values(ctx);
    let compose_files = compose_choices(ctx);
    Ok(ctx.store.read(|s| {
        let env = s.environment_id().map(str::to_owned);
        let mut open: Vec<SecretRequest> = vec![];
        let mut done: Vec<SecretRequest> = vec![];
        for r in s
            .secrets
            .iter()
            .filter(|r| env.as_deref() == Some(r.environment_id.as_str()))
        {
            if r.state == SecretState::Waiting {
                open.push(r.clone())
            } else {
                done.push(r.clone())
            }
        }
        done.sort_by(|a, b| b.resolved_at.cmp(&a.resolved_at));
        done.truncate(20);
        open.extend(done);
        Status {
            auto_fix: cfg.auto_fix,
            worker: WorkerInfo {
                version: env!("CARGO_PKG_VERSION").to_string(),
                iii_version: s
                    .iii_version
                    .clone()
                    .or_else(|| s.check.as_ref().and_then(|c| c.here.iii_version.clone())),
                cloud_engine: s.cloud_engine.clone(),
            },
            connection,
            project: s.project.clone(),
            candidate: s.candidate.clone(),
            skipped_values: env
                .as_deref()
                .and_then(|e| s.skipped_values.get(e))
                .map(|names| names.iter().cloned().collect())
                .unwrap_or_default(),
            check: s.check.clone(),
            checks: s.checks.iter().map(CheckSummary::from).collect(),
            recent_projects: s
                .recent_projects
                .iter()
                .filter(|r| {
                    !s.project.as_ref().is_some_and(|p| {
                        p.path == r.path
                            && p.cloud_project
                                .as_ref()
                                .is_some_and(|c| c.id == r.cloud_project_id)
                    }) && Path::new(&r.path).exists()
                })
                .cloned()
                .collect(),
            secrets: open,
            variables: s.variables.clone(),
            pr: s.pr.clone(),
            pr_error: s.pr_error.clone(),
            problems: [
                (s.project_deleted, "PROJECT_DELETED"),
                (s.environment_deleted, "ENVIRONMENT_DELETED"),
            ]
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, code)| WorkerError {
                code: code.into(),
                message: format!(
                    "This {} was removed in iii cloud; choose another.",
                    if code == "PROJECT_DELETED" {
                        "project"
                    } else {
                        "environment"
                    }
                ),
                status: None,
                fix: Some("Pick another with iii-cloud::project::use.".into()),
            })
            .collect(),
            // The selected environment's own deploys.
            handoff_brief: env
                .as_deref()
                .and_then(|e| s.handoffs.iter().find(|h| h.environment_id == e))
                .map(|h| crate::handoff::brief(s, h, &secrets)),
            agent: env.as_deref().and_then(|e| {
                s.handoffs.iter().find(|h| h.environment_id == e).map(|h| {
                    let mut a = h.agent.clone();
                    // Only what the newest check still has, and nothing once it ended.
                    a.working_on = if matches!(a.state, AgentPhase::Done | AgentPhase::Stopped) {
                        vec![]
                    } else {
                        crate::handoff::still_open(s, &h.environment_id, &a.working_on)
                    };
                    a
                })
            }),
            deploys: s
                .deploys
                .iter()
                .filter(|d| env.as_deref().is_none_or(|e| d.environment_id == e))
                .cloned()
                .map(|mut d| {
                    d.from_console = s.from_console(&d);
                    d
                })
                .collect(),
            running_deploys: s
                .deploys
                .iter()
                .filter(|d| d.is_moving())
                .map(|d| {
                    let o = s.deploy_origins.get(&d.id);
                    let mut deploy = d.clone();
                    deploy.from_console = s.from_console(d);
                    RunningDeploy {
                        deploy,
                        cloud_project_id: o.and_then(|o| o.cloud_project_id.clone()),
                        project_name: o.and_then(|o| o.project_name.clone()),
                        repository: o.map(|o| o.repository.clone()),
                        environment_name: o.map(|o| o.environment_name.clone()),
                        path: o.map(|o| o.path.clone()),
                    }
                })
                .collect(),
            skipped_problems: env
                .as_deref()
                .and_then(|e| s.skipped_problems.get(e))
                .map(|codes| codes.iter().cloned().collect())
                .unwrap_or_default(),
            override_choices: env
                .as_deref()
                .map(|e| crate::console::choices_of(s, e))
                .unwrap_or_default(),
            refreshed_at: s.refreshed_at.clone(),
            compose_files,
        }
    }))
}

/// The worktree's compose files for the page's app file picker. Like the
/// cloud's own picker, only a name iii cloud does not plan is not choosable.
fn compose_choices(ctx: &Ctx) -> Vec<ImportableManifest> {
    let Some(ws) = ctx.store.read(|s| {
        s.project
            .as_ref()
            .and_then(|p| p.workspace.as_ref())
            .map(|w| w.path.clone())
    }) else {
        return vec![];
    };
    crate::handoff::compose_files(Path::new(&ws))
        .into_iter()
        .map(|path| {
            let choosable = crate::local::manifest_path(&path);
            let (workers, has_cloud_block) = crate::local::compose_facts(Path::new(&ws), &path);
            ImportableManifest {
                choosable,
                issue: (!choosable).then(|| "NOT_WORKER_COMPOSE".to_string()),
                suggested_environment: None,
                workers,
                has_cloud_block,
                path,
            }
        })
        .collect()
}

/// Re-read the selected environment and keep the fresh values in the project.
pub async fn refresh_environment(
    ctx: &Arc<Ctx>,
    sel: &crate::check::Selected,
) -> Option<EnvironmentInfo> {
    let body = ctx
        .call(&Req::get(format!(
            "/teams/{}/environments/{}",
            sel.team_id, sel.env.id
        )))
        .await
        .ok()?;
    let fresh = crate::project::env_info(&body)?;
    let removed = crate::project::is_removed(&body);
    ctx.store.update(|s| {
        // The branch and name wherever the store copied them: a hand-off's brief
        // and `selected_for` read these, not the selection.
        for h in s.handoffs.iter_mut() {
            if h.env.as_ref().is_some_and(|e| e.id == fresh.id) {
                h.env = Some(fresh.clone());
            }
        }
        if let Some(p) = s.project.as_mut() {
            for e in p.environments.iter_mut().filter(|e| e.id == fresh.id) {
                e.name = fresh.name.clone();
                e.branch = fresh.branch.clone();
            }
            for e in p
                .cloud_projects
                .iter_mut()
                .flat_map(|c| c.environments.iter_mut())
                .filter(|e| e.id == fresh.id)
            {
                e.name = fresh.name.clone();
                e.branch = fresh.branch.clone();
                e.manifest = fresh.manifest.clone();
            }
        }
        // Only the selected environment's record: a secret request's may be another.
        if let Some(p) = s
            .project
            .as_mut()
            .filter(|p| p.environment.as_ref().is_some_and(|e| e.id == fresh.id))
        {
            s.environment_deleted = removed;
            p.environment = Some(fresh.clone());
        }
    });
    Some(fresh)
}

/// `store.variables`: the environment's variables as iii cloud lists them,
/// merged with what the plan and the compose file reference. A failed read
/// keeps the last list.
pub async fn refresh_variables(ctx: &Arc<Ctx>, sel: &crate::check::Selected) {
    let mut listed = None;
    if let Ok(body) = ctx
        .call(&Req::get(format!(
            "/teams/{}/environments/{}/variables",
            sel.team_id, sel.env.id
        )))
        .await
    {
        let rows: Vec<VariableRow> = body
            .get("variables")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| {
                let t = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
                Some(VariableRow {
                    name: t("name")?,
                    kind: t("type")?,
                    status: t("status")?,
                    required_by: v
                        .get("required_by")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default(),
                })
            })
            .collect();
        listed = Some(rows);
    }
    let rows = listed.unwrap_or_else(|| ctx.store.read(|s| s.variables.clone()));
    put_variables(ctx, sel, merged_variables(ctx, sel, rows));
}

/// `store.variables` is the selected environment's: rows of another one are dropped.
pub fn put_variables(ctx: &Ctx, sel: &crate::check::Selected, rows: Vec<VariableRow>) {
    ctx.store.update(|s| {
        if s.environment_id() == Some(sel.env.id.as_str()) {
            s.variables = rows;
        }
    });
}

/// The variables the cloud lists, plus every one the plan or the manifest
/// references that it does not: a name with an empty default is
/// `empty_default` ("optional, empty by default"), one with no default `missing`.
pub fn merged_variables(
    ctx: &Ctx,
    sel: &crate::check::Selected,
    mut rows: Vec<VariableRow>,
) -> Vec<VariableRow> {
    // Only `sel`'s latest check: never another environment's plan or file.
    let check = ctx.store.read(|s| {
        s.check
            .as_ref()
            .filter(|c| c.environment_id == sel.env.id)
            .map(|c| (c.id.clone(), c.manifest.clone()))
    });
    let plan = check
        .as_ref()
        .map(|(id, _)| id.clone())
        .and_then(|id| std::fs::read_to_string(crate::deploy::plan_file(ctx, &id)).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    for v in plan
        .as_ref()
        .and_then(|p| p.get("variables"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = v.get("name").and_then(Value::as_str) else {
            continue;
        };
        if rows.iter().any(|r| r.name == name) {
            continue;
        }
        rows.push(VariableRow {
            name: name.to_string(),
            kind: v
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("variable")
                .to_string(),
            status: v
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("missing")
                .to_string(),
            required_by: v
                .get("required_by")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    // The latest check's file (another one than the environment's, maybe).
    let manifest_rel = check
        .and_then(|(_, manifest)| manifest)
        .or(sel.env.manifest.clone());
    if let Some(m) = crate::local::Manifest::load(&sel.workspace, manifest_rel.as_deref()) {
        let uses = m.variable_uses().0;
        for u in &uses {
            if rows.iter().any(|r| r.name == u.name) {
                continue;
            }
            let status = match u.default.as_deref() {
                None => "missing",
                Some("") => "empty_default",
                // A real default is a value already: nothing for the person to give.
                Some(_) => continue,
            };
            let mut required_by: Vec<String> = vec![];
            for c in uses.iter().filter(|x| x.name == u.name) {
                if !required_by.contains(&c.container) {
                    required_by.push(c.container.clone());
                }
            }
            rows.push(VariableRow {
                name: u.name.clone(),
                kind: "variable".into(),
                status: status.into(),
                required_by,
            });
        }
    }
    rows
}

/// `served..<tip>` in the checkout; `None` when the served commit is not here
/// and can't be fetched.
async fn not_served(checkout: &Path, served: &str, tip: &str) -> Option<Vec<CommitLine>> {
    let have = || async {
        crate::tool::git(
            checkout,
            &["cat-file", "-e", &format!("{served}^{{commit}}")],
        )
        .await
        .is_ok()
    };
    if !have().await {
        let _ = crate::tool::git(checkout, &["fetch", "origin", served]).await;
        if !have().await {
            return None;
        }
    }
    Some(crate::check::commits_between(checkout, served, tip).await)
}

async fn refresh(ctx: &Arc<Ctx>) {
    let cfg = ctx.config().await;
    let iii = local_iii_version(&cfg.iii_bin).await;
    let engine = match ctx
        .http
        .get(format!("{}/support-matrix", cfg.api_base()))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r
            .json::<Value>()
            .await
            .ok()
            .and_then(|v| v.get("engine")?.as_str().map(str::to_owned)),
        _ => None,
    };
    ctx.store.update(|s| {
        s.iii_version = iii;
        s.cloud_engine = engine;
    });
    let Ok(sel) = selected(ctx) else {
        ctx.store.update(|s| s.refreshed_at = Some(now()));
        return;
    };
    // The environment as the cloud has it now (its manifest may have changed).
    refresh_environment(ctx, &sel).await;
    // The checkout.
    if let Some(project) = ctx.store.read(|s| s.project.clone())
        && let Ok(mut facts) = crate::project::checkout_facts(Path::new(&project.path)).await
    {
        facts.branches = crate::project::branches(Path::new(&project.path), &sel.env.branch).await;
        ctx.store.update(|s| {
            if let Some(p) = s.project.as_mut() {
                p.checkout = facts;
            }
        });
    }
    // The pull request: only production deploys by one. A stale one of an
    // assisted environment must not show.
    if !sel.env.production {
        ctx.store.update(|s| {
            s.pr = None;
            s.pr_error = None;
        });
    } else if let Some(project) = ctx.store.read(|s| s.project.clone())
        && let Some(ws) = project.workspace.as_ref()
    {
        let base = ctx.store.read(|s| s.check.as_ref().map(|c| c.base.clone()));
        let started = ctx.store.read(|s| {
            s.worktree_started
                .get(&format!("{}\n{}", ws.path, ws.branch))
                .cloned()
        });
        let result = crate::deploy::read_pr(
            Path::new(&ws.path),
            &project.repository,
            &ws.branch,
            &sel.env.branch,
            base.as_deref(),
            started.as_deref(),
        )
        .await;
        ctx.store.update(|s| match result {
            Ok(pr) => {
                s.pr = pr;
                s.pr_error = None;
            }
            // A gh failure keeps the last good answer and says what went wrong.
            Err(e) => s.pr_error = Some(e.to_worker_error()),
        });
    }
    // The variables.
    refresh_variables(ctx, &sel).await;
    // Every cloud project of the repository, fresh (a project made in the console shows).
    crate::project::refresh_cloud_projects(ctx).await;
    // Each environment's branch head on origin, so the page sees a branch that moved.
    let mut branches: Vec<String> = ctx.store.read(|s| {
        s.project
            .iter()
            .flat_map(|p| &p.cloud_projects)
            .flat_map(|c| &c.environments)
            .map(|e| e.branch.clone())
            .collect()
    });
    branches.sort();
    branches.dedup();
    let mut heads = std::collections::BTreeMap::new();
    for branch in branches {
        let head = crate::check::remote_head(&sel.workspace, &branch)
            .await
            .ok();
        heads.insert(branch, head);
    }
    ctx.store.update(|s| {
        for e in s
            .project
            .iter_mut()
            .flat_map(|p| &mut p.cloud_projects)
            .flat_map(|c| &mut c.environments)
        {
            e.head = heads.get(&e.branch).cloned().flatten();
        }
    });
    // When any environment of the cloud project in use last deployed: the page
    // knows it is live even when the selected environment never deployed. A
    // failed read keeps the last value.
    let in_use = ctx.store.read(|s| {
        s.project
            .as_ref()
            .and_then(|p| p.cloud_project.as_ref())
            .map(|c| c.id.clone())
    });
    if let Some(id) = in_use
        && let Some(at) = crate::project::last_deployment_at(ctx, &sel.team_id, &id).await
    {
        ctx.store.update(|s| {
            if let Some(p) = s.project.as_mut() {
                if let Some(c) = p.cloud_project.as_mut().filter(|c| c.id == id) {
                    c.last_deployment_at = Some(at.clone());
                }
                for c in p.cloud_projects.iter_mut().filter(|c| c.id == id) {
                    c.last_deployment_at = Some(at.clone());
                }
            }
        });
    }
    // The environments as the cloud has them now (one made later in the console
    // shows), and each one's recent deployments, console deploys included.
    if let Some(project) = ctx.store.read(|s| s.project.clone())
        && let (Some(team), Some(cp)) = (project.team.as_ref(), project.cloud_project.as_ref())
        && let Ok(infos) = crate::project::read_environments(ctx, &team.id, &cp.id).await
    {
        ctx.store.update(|s| {
            if let Some(p) = s.project.as_mut() {
                p.environments = infos
                    .iter()
                    .map(|e| EnvironmentBrief {
                        id: e.id.clone(),
                        name: e.name.clone(),
                        production: e.production,
                        branch: e.branch.clone(),
                    })
                    .collect();
            }
        });
        for env in &infos {
            crate::watch::refresh_recent(ctx, &team.id, &env.id, &env.name).await;
        }
    } else {
        crate::watch::refresh_recent(ctx, &sel.team_id, &sel.env.id, &sel.env.name).await;
    }
    // The deployments.
    crate::watch::poll_once(ctx).await;
    // The checkout's commits the selected environment does not serve yet.
    let served = ctx.store.read(|s| {
        let env = s.environment_id()?;
        s.deploys
            .iter()
            .find(|d| d.environment_id == env && d.outcome == Outcome::Ready)
            .map(|d| d.commit.sha.clone())
    });
    if let Some(project) = ctx.store.read(|s| s.project.clone()) {
        // The branch the worktree took (`use {source}`), else the checkout's HEAD.
        let tip = project
            .workspace
            .as_ref()
            .and_then(|w| w.source.as_deref())
            .unwrap_or("HEAD");
        let not_served = match served {
            Some(sha) => not_served(Path::new(&project.path), &sha, tip).await,
            None => None,
        };
        ctx.store.update(|s| {
            if let Some(p) = s.project.as_mut().filter(|p| p.path == project.path) {
                p.checkout.not_served = not_served;
            }
        });
    }
    ctx.store.update(|s| s.refreshed_at = Some(now()));
}
