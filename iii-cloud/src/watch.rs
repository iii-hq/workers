//! `iii-cloud::watch` and the followers (spec §9): a deployment followed to a
//! terminal status, with the evidence when it fails; and the background poll
//! for pushes made elsewhere.

use std::sync::Arc;

use serde_json::Value;

use crate::check::selected;
use crate::cloud::Req;
use crate::ctx::{Ctx, now, short};
use crate::error::{CloudError, Res};
use crate::types::*;

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

/// `<cloud_url>/<team slug>/<project>/<environment>/deployments/<number>`; the
/// `$project` segment is the project's `endpoint_slug` (iii-cloud
/// `apps/web/src/projects/queries.ts:155`). Without the names, the cloud root.
pub fn console_url(
    cloud_url: &str,
    team: Option<&str>,
    project: Option<&str>,
    env: &str,
    number: u64,
) -> String {
    let base = cloud_url.trim_end_matches('/');
    match (team, project) {
        (Some(t), Some(p)) if !t.is_empty() && !p.is_empty() => {
            format!("{base}/{t}/{p}/{env}/deployments/{number}")
        }
        _ => base.to_string(),
    }
}

/// The release row of board 09, derived: the cloud runs the release inside
/// its `workers` stage.
pub fn release_row(
    has_release: bool,
    stages: &[DeployStage],
    error: Option<&DeployError>,
) -> Option<ReleaseRow> {
    if !has_release {
        return None;
    }
    let workers = stages
        .iter()
        .find(|st| st.name == "workers")
        .map(|st| st.status.as_str());
    let status = if error.is_some_and(|e| e.code == "RELEASE_FAILED") {
        ReleaseRowStatus::Failed
    } else {
        match workers {
            Some("passed") => ReleaseRowStatus::Passed,
            Some("running") => ReleaseRowStatus::Running,
            _ => ReleaseRowStatus::Pending,
        }
    };
    Some(ReleaseRow {
        status,
        duration_ms: None,
    })
}

pub fn outcome_of(status: &str) -> Outcome {
    match status {
        "ready" => Outcome::Ready,
        "failed" => Outcome::Failed,
        "canceled" => Outcome::Canceled,
        "replaced" => Outcome::Replaced,
        _ => Outcome::Running,
    }
}

/// Ends for good. `replaced` does not: the deployment that replaced it is the
/// one to follow.
pub fn is_terminal(status: &str) -> bool {
    matches!(status, "ready" | "failed" | "canceled")
}

/// A deployment of the cloud as the worker's record. `prev` keeps what the
/// worker added (log tail, outcome, pr, flags).
pub fn to_deploy(
    cloud_url: &str,
    names: (Option<&str>, Option<&str>, &str),
    d: &Value,
    prev: Option<&Deploy>,
    has_release: bool,
) -> Option<Deploy> {
    let status = s(d, "status")?;
    let number = d.get("number")?.as_u64()?;
    let stages: Vec<DeployStage> = d
        .get("stages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|st| DeployStage {
            name: s(st, "name").unwrap_or_default(),
            status: s(st, "status").unwrap_or_default(),
            detail: s(st, "detail"),
            started_at: s(st, "started_at"),
            finished_at: s(st, "finished_at"),
        })
        .collect();
    let error = d
        .get("error")
        .filter(|e| e.is_object())
        .map(|e| DeployError {
            stage: s(e, "stage").unwrap_or_default(),
            worker: s(e, "worker"),
            code: s(e, "code").unwrap_or_default(),
            hint: s(e, "hint").unwrap_or_default(),
            action: s(e, "action"),
        });
    // A no-route verdict the worker reached stays over the cloud's "ready".
    let keep_no_route = prev.is_some_and(|p| p.outcome == Outcome::NoRoute);
    let error = if keep_no_route {
        prev.and_then(|p| p.error.clone())
    } else {
        error
    };
    Some(Deploy {
        failure: prev.and_then(|p| p.failure.clone()),
        id: s(d, "id")?,
        number,
        environment_id: s(d, "environment_id").unwrap_or_default(),
        commit: DeployCommit {
            sha: d
                .pointer("/commit/sha")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message: d
                .pointer("/commit/message")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        kind: s(d, "type").unwrap_or_default(),
        status_detail: s(d, "status_detail"),
        release_row: release_row(has_release, &stages, error.as_ref()),
        outcome: if keep_no_route {
            Outcome::NoRoute
        } else {
            outcome_of(&status)
        },
        error,
        log_tail: prev.map(|p| p.log_tail.clone()).unwrap_or_default(),
        pr: prev.and_then(|p| p.pr),
        approved_by_worker: prev.is_some_and(|p| p.approved_by_worker),
        checked_ready: prev.is_some_and(|p| p.checked_ready),
        created_at: s(d, "created_at").unwrap_or_else(now),
        finished_at: s(d, "finished_at"),
        replaced_by: s(d, "replaced_by"),
        console_url: console_url(cloud_url, names.0, names.1, names.2, number),
        from_console: false,
        check_id: prev.and_then(|p| p.check_id.clone()),
        stages,
        status,
    })
}

fn names(ctx: &Ctx) -> (Option<String>, Option<String>, String) {
    ctx.store.read(|st| {
        let p = st.project.as_ref();
        (
            p.and_then(|p| p.team.as_ref()).map(|t| t.slug.clone()),
            p.and_then(|p| p.cloud_project.as_ref())
                .map(|c| c.slug.clone()),
            p.and_then(|p| p.environment.as_ref())
                .map(|e| e.name.clone())
                .unwrap_or_default(),
        )
    })
}

/// A ready check of the same environment has the deployment's commit.
pub fn checked_ready(ctx: &Ctx, env_id: &str, sha: &str) -> bool {
    ctx.store.read(|st| {
        st.check.iter().chain(st.checks.iter()).any(|c| {
            c.ready
                && c.environment_id == env_id
                && !sha.is_empty()
                && (c.commit.starts_with(sha) || sha.starts_with(&c.commit))
        })
    })
}

async fn has_release(ctx: &Ctx, team: &str, env: &str) -> bool {
    ctx.call(&Req::get(format!("/teams/{team}/environments/{env}")))
        .await
        .ok()
        .is_some_and(|b| !b.pointer("/settings/release").is_none_or(Value::is_null))
}

/// The selected project's facts for one of its environments.
fn origin_of(
    st: &crate::store::State,
    env_id: &str,
    team_id: Option<String>,
) -> Option<DeployOrigin> {
    let p = st.project.as_ref()?;
    let environment_name = p
        .environment
        .iter()
        .map(|e| (e.id.as_str(), e.name.as_str()))
        .chain(
            p.environments
                .iter()
                .map(|e| (e.id.as_str(), e.name.as_str())),
        )
        .find(|(id, _)| *id == env_id)?
        .1
        .to_string();
    Some(DeployOrigin {
        team_id: team_id.or_else(|| p.team.as_ref().map(|t| t.id.clone()))?,
        team_slug: p.team.as_ref().map(|t| t.slug.clone()),
        cloud_project_id: p.cloud_project.as_ref().map(|c| c.id.clone()),
        project_slug: p.cloud_project.as_ref().map(|c| c.slug.clone()),
        project_name: p.cloud_project.as_ref().map(|c| c.name.clone()),
        repository: p.repository.clone(),
        environment_name,
        path: p.path.clone(),
    })
}

/// The console's names for a deploy's URL: where it was made from, else the selection.
fn names_for(ctx: &Ctx, id: &str) -> (Option<String>, Option<String>, String) {
    ctx.store
        .read(|st| {
            st.deploy_origins.get(id).map(|o| {
                (
                    o.team_slug.clone(),
                    o.project_slug.clone(),
                    o.environment_name.clone(),
                )
            })
        })
        .unwrap_or_else(|| names(ctx))
}

/// Store a deployment record and emit it. The wake-ups: a deployment that
/// ends (`terminal: true`), and one that starts waiting for action (once).
/// `replaced` is not terminal: the confirmed deployment that replaced it is
/// the one to follow.
pub fn record(ctx: &Ctx, deploy: &Deploy) {
    let selected_team = crate::check::selected(ctx)
        .ok()
        .filter(|sel| sel.env.id == deploy.environment_id)
        .map(|sel| sel.team_id);
    let prev_status = ctx.store.update(|st| {
        // Where it was made from, the first time it is seen with its project selected.
        if !st.deploy_origins.contains_key(&deploy.id)
            && let Some(origin) = origin_of(st, &deploy.environment_id, selected_team.clone())
        {
            st.deploy_origins.insert(deploy.id.clone(), origin);
        }
        let prev = st
            .deploys
            .iter()
            .find(|d| d.id == deploy.id)
            .map(|d| d.status.clone());
        st.put_deploy(deploy.clone());
        prev
    });
    let wakes = deploy.status == "needs_action" && prev_status.as_deref() != Some("needs_action");
    // A deployment replaced by one this worker did not make ends for us: wake
    // the agent. One replaced by the confirmed deployment we created does not.
    let replaced_by_other = deploy.outcome == Outcome::Replaced
        && deploy.replaced_by.as_deref().is_some_and(|succ| {
            !ctx.store.read(|st| {
                st.deploys
                    .iter()
                    .any(|d| d.id == succ && d.approved_by_worker)
            })
        });
    let terminal = wakes
        || replaced_by_other
        || !matches!(deploy.outcome, Outcome::Running | Outcome::Replaced);
    let summary = match (&deploy.outcome, &deploy.error) {
        (Outcome::Running, _) => {
            format!("deploy {} is {}", short(&deploy.commit.sha), deploy.status)
        }
        (Outcome::Ready, _) => format!("deploy {} is ready", short(&deploy.commit.sha)),
        (_, Some(e)) => format!(
            "deploy {} failed at {}: {}",
            short(&deploy.commit.sha),
            e.stage,
            e.code
        ),
        (o, None) => format!("deploy {} {:?}", short(&deploy.commit.sha), o).to_lowercase(),
    };
    let summary = if wakes {
        format!("deploy {} needs your action", short(&deploy.commit.sha))
    } else {
        summary
    };
    // The event carries what the status read would say about where it came from
    // (the stored record is compared with the cloud's, so it keeps its own value).
    let mut event = deploy.clone();
    event.from_console = ctx.store.read(|s| s.from_console(deploy));
    ctx.emit(EventKind::Deploy, &deploy.id, terminal, summary, &event);
}

/// The last 20 log lines of a deployment, oldest first.
async fn log_tail(ctx: &Ctx, team: &str, env: &str, id: &str) -> Vec<String> {
    let req = Req::get(format!("/teams/{team}/environments/{env}/logs")).query("deployment_id", id);
    let body = ctx.call(&req).await.unwrap_or(Value::Null);
    let mut lines: Vec<String> = body
        .get("lines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(20)
        .filter_map(|l| s(l, "message"))
        .collect();
    lines.reverse();
    lines
}

/// The cloud's line for a group that runs http and serves no route. It is a
/// deployment event, not a log line (iii-cloud `control-plane/src/workers.rs:114,798`).
pub const NO_ROUTE_MARK: &str = "but no function of this release serves a route";

/// The no-route line among a deployment's events, if any.
pub fn no_route_in(events: &[Value]) -> Option<String> {
    events.iter().find_map(|e| {
        let message = e.pointer("/log/message").and_then(Value::as_str)?;
        message.contains(NO_ROUTE_MARK).then(|| message.to_string())
    })
}

async fn no_route_line(ctx: &Ctx, team: &str, env: &str, id: &str) -> Option<String> {
    let events = ctx
        .sse_events(
            &format!("/teams/{team}/environments/{env}/deployments/{id}/events"),
            ctx.secs(5),
        )
        .await
        .ok()?;
    no_route_in(&events)
}

/// The environment's health, as evidence lines: its status and each worker
/// that is not ready or restarts.
pub fn health_lines(health: &Value) -> Vec<String> {
    let mut lines = vec![format!(
        "health: {} (engine {})",
        health
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        health
            .get("engine")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    )];
    for w in health
        .get("workers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let ready = w.get("ready").and_then(Value::as_bool).unwrap_or(true);
        let restarts = w.get("restarts").and_then(Value::as_u64).unwrap_or(0);
        let crashloop = w.get("crashloop").and_then(Value::as_bool) == Some(true);
        if !ready || restarts > 0 || crashloop {
            lines.push(format!(
                "health: {}/{} {}, {restarts} restarts{}",
                w.get("group").and_then(Value::as_str).unwrap_or("?"),
                w.get("worker").and_then(Value::as_str).unwrap_or("?"),
                if ready { "ready" } else { "not ready" },
                if crashloop { ", crashloop" } else { "" }
            ));
        }
    }
    lines
}

/// Make the terminal evidence: the log tail, and the no-route verdict.
pub async fn finish(ctx: &Ctx, team: &str, env: &str, mut deploy: Deploy) -> Deploy {
    deploy.log_tail = log_tail(ctx, team, env, &deploy.id).await;
    if deploy.status == "ready"
        && let Some(line) = no_route_line(ctx, team, env, &deploy.id).await
    {
        deploy.outcome = Outcome::NoRoute;
        deploy.error = Some(DeployError {
            stage: "routes_tls".into(),
            worker: None,
            code: "NO_ROUTE".into(),
            hint: line,
            action: None,
        });
    }
    if deploy.status != "ready" || deploy.outcome == Outcome::NoRoute {
        let path = format!("/teams/{team}/environments/{env}/health");
        if let Ok(health) = ctx.call(&Req::get(path)).await {
            deploy.log_tail.extend(health_lines(&health));
        }
    }
    if matches!(deploy.outcome, Outcome::Failed | Outcome::NoRoute) && deploy.error.is_some() {
        deploy.failure = failure_of(ctx, team, env, &deploy).await;
    }
    deploy
}

/// The log source of the stage that failed: the release's own log for a release
/// failure, the build's for a build, and so on. `None`: every source.
pub fn failure_source(code: &str, stage: &str) -> Option<&'static str> {
    match (code, stage) {
        ("RELEASE_FAILED", _) => Some("release"),
        ("BUILD_FAILED", _) | (_, "build") => Some("build"),
        (_, "resources") => Some("provisioning"),
        (_, "workers") => Some("worker"),
        _ => None,
    }
}

/// A plain title per code. A release failure names its command.
pub fn failure_title(code: &str, release_command: Option<&str>) -> String {
    match code {
        "RELEASE_FAILED" => match release_command {
            Some(c) => format!("The release command failed: `{c}`"),
            None => "The release command failed".to_string(),
        },
        "BUILD_FAILED" => "The build failed".into(),
        "WORKERS_NOT_READY" => "The workers did not become ready".into(),
        "HEALTHCHECK_FAILED" => "The health check failed".into(),
        "NO_ROUTE" => "The http group serves no route".into(),
        other => crate::problems::title_for(other),
    }
}

/// `Some(exit code)` when the lines show an exit status but no error text: the
/// command died without saying why.
pub fn silent_exit(lines: &[String]) -> Option<i32> {
    const SAYS: [&str; 10] = [
        "error",
        "err!",
        "exception",
        "fatal",
        "cannot",
        "could not",
        "failed",
        "denied",
        "traceback",
        "panic",
    ];
    let exit_code = |l: &str| -> Option<i32> {
        let low = l.to_lowercase();
        let at = low.find("exit")?;
        let digits: String = low[at..]
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok().filter(|_| low[at..].len() < 80)
    };
    let (exits, others): (Vec<&String>, Vec<&String>) =
        lines.iter().partition(|l| exit_code(l).is_some());
    let code = exits.last().and_then(|l| exit_code(l))?;
    let speaks = others.iter().any(|l| {
        let low = l.to_lowercase();
        SAYS.iter().any(|w| low.contains(w))
    });
    (!speaks && code != 0).then_some(code)
}

/// The release command: the environment's setting, else the manifest's.
async fn release_command(ctx: &Ctx, team: &str, env: &str) -> Option<String> {
    let body = ctx
        .call(&Req::get(format!("/teams/{team}/environments/{env}")))
        .await
        .ok();
    if let Some(c) = body
        .as_ref()
        .and_then(|b| b.pointer("/settings/release/command"))
        .and_then(Value::as_str)
    {
        return Some(c.to_string());
    }
    // The selected workspace's file, only when it is this deployment's environment.
    let sel = selected(ctx).ok().filter(|sel| sel.env.id == env)?;
    let manifest = crate::local::Manifest::load(&sel.workspace, sel.env.manifest.as_deref())?;
    manifest.cloud_release().map(|r| r.command)
}

/// The failing stage's last lines, oldest first, cut and redacted.
async fn stage_log(
    ctx: &Ctx,
    team: &str,
    env: &str,
    id: &str,
    source: Option<&str>,
) -> Vec<String> {
    let mut req =
        Req::get(format!("/teams/{team}/environments/{env}/logs")).query("deployment_id", id);
    if let Some(source) = source {
        req = req.query("source", source);
    }
    let body = ctx.call(&req).await.unwrap_or(Value::Null);
    let secrets = crate::handoff::env_values(ctx);
    let mut lines: Vec<String> = body
        .get("lines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(40)
        .filter_map(|l| s(l, "message"))
        .map(|l| crate::run::redact(&crate::run::redact_urls(&l), &secrets))
        .map(|l| crate::error::cut(&l, 300))
        .collect();
    lines.reverse();
    lines
}

/// What failed, in plain words, with the failing stage's log.
pub async fn failure_of(
    ctx: &Ctx,
    team: &str,
    env: &str,
    deploy: &Deploy,
) -> Option<DeployFailure> {
    let e = deploy.error.as_ref()?;
    let release = if e.code == "RELEASE_FAILED" {
        release_command(ctx, team, env).await
    } else {
        None
    };
    let log_tail = stage_log(
        ctx,
        team,
        env,
        &deploy.id,
        failure_source(&e.code, &e.stage),
    )
    .await;
    let silent = silent_exit(&log_tail);
    let hint = match silent {
        Some(code) => format!(
            "the command exited {code} without printing an error; the tool may hide it, so run it locally the same way"
        ),
        None => e.hint.clone(),
    };
    Some(DeployFailure {
        code: e.code.clone(),
        stage: e.stage.clone(),
        worker: e.worker.clone(),
        title: failure_title(&e.code, release.as_deref()),
        hint,
        log_tail,
        silent: silent.is_some(),
        release_command: release,
    })
}

/// Follow one deployment until it ends. At most one follower per deployment.
pub fn follow(ctx: &Arc<Ctx>, id: String) {
    let key = format!("deploy:{id}");
    if !ctx.busy.lock().unwrap().insert(key.clone()) {
        return;
    }
    let ctx = ctx.clone();
    tokio::spawn(async move {
        follow_loop(&ctx, &id).await;
        ctx.busy.lock().unwrap().remove(&key);
    });
}

async fn follow_loop(ctx: &Arc<Ctx>, id: &str) {
    // The deployment's own team and environment, not whichever is selected now:
    // it is followed after a switch, and with nothing selected.
    let (env_id, team) = ctx.store.read(|st| {
        (
            st.deploys
                .iter()
                .find(|d| d.id == id)
                .map(|d| d.environment_id.clone())
                .filter(|e| !e.is_empty()),
            st.deploy_origins.get(id).map(|o| o.team_id.clone()),
        )
    });
    let sel = selected(ctx).ok();
    let Some(env_id) = env_id.or_else(|| sel.as_ref().map(|s| s.env.id.clone())) else {
        return;
    };
    let Some(team_id) = team.or_else(|| sel.map(|s| s.team_id)) else {
        return;
    };
    let release = has_release(ctx, &team_id, &env_id).await;
    loop {
        let cfg = ctx.config().await;
        let req = Req::get(format!(
            "/teams/{team_id}/environments/{env_id}/deployments/{id}"
        ));
        match ctx.call(&req).await {
            Ok(body) => {
                let prev = ctx
                    .store
                    .read(|st| st.deploys.iter().find(|d| d.id == id).cloned());
                let n = names_for(ctx, id);
                let Some(mut next) = to_deploy(
                    &cfg.cloud_url,
                    (n.0.as_deref(), n.1.as_deref(), &n.2),
                    &body,
                    prev.as_ref(),
                    release,
                ) else {
                    return;
                };
                next.checked_ready |= checked_ready(ctx, &next.environment_id, &next.commit.sha);
                if next.status == "replaced" {
                    // Hand over to the successor, and stop following this one.
                    if prev.as_ref() != Some(&next) {
                        record(ctx, &next);
                    }
                    if let Some(successor) = body.get("replaced_by").and_then(Value::as_str) {
                        follow(ctx, successor.to_string());
                    }
                    return;
                }
                if is_terminal(&next.status) {
                    let done = finish(ctx, &team_id, &env_id, next).await;
                    record(ctx, &done);
                    return;
                }
                if prev.as_ref() != Some(&next) {
                    record(ctx, &next);
                }
            }
            // No credential, or no right to read it: following cannot succeed.
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "NOT_CONNECTED" | "KEY_REJECTED" | "NEEDS_BROWSER_SIGN_IN"
                ) || matches!(e.status, Some(401 | 403)) =>
            {
                return;
            }
            // The deployment is gone: say so in the record, so nothing waits for it.
            Err(e) if e.status == Some(404) => {
                let gone = ctx
                    .store
                    .read(|st| st.deploys.iter().find(|d| d.id == id).cloned());
                if let Some(mut d) = gone.filter(|d| d.outcome == Outcome::Running) {
                    d.status = "gone".into();
                    d.outcome = Outcome::Canceled;
                    d.status_detail = Some("iii cloud no longer has this deployment.".into());
                    record(ctx, &d);
                }
                return;
            }
            Err(e) => tracing::warn!(error = %e, "deployment poll failed"),
        }
        tokio::time::sleep(ctx.secs(5)).await;
    }
}

/// List an environment's recent deployments (at most 5) and record the ones
/// not seen yet, console deploys included, with their failure when they failed.
pub async fn refresh_recent(ctx: &Arc<Ctx>, team_id: &str, env_id: &str, env_name: &str) {
    let Ok(list) = ctx
        .call(&Req::get(format!(
            "/teams/{team_id}/environments/{env_id}/deployments"
        )))
        .await
    else {
        return;
    };
    let cfg = ctx.config().await;
    let (team, project) = ctx.store.read(|st| {
        let p = st.project.as_ref();
        (
            p.and_then(|p| p.team.as_ref()).map(|t| t.slug.clone()),
            p.and_then(|p| p.cloud_project.as_ref())
                .map(|c| c.slug.clone()),
        )
    });
    for d in list
        .get("deployments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(5)
    {
        let id = s(d, "id").unwrap_or_default();
        let prev = ctx
            .store
            .read(|st| st.deploys.iter().find(|x| x.id == id).cloned());
        // One that already ended is as it will be: leave it.
        if prev.as_ref().is_some_and(|p| p.outcome != Outcome::Running) {
            continue;
        }
        let Some(mut deploy) = to_deploy(
            &cfg.cloud_url,
            (team.as_deref(), project.as_deref(), env_name),
            d,
            prev.as_ref(),
            false,
        ) else {
            continue;
        };
        if deploy.environment_id.is_empty() {
            deploy.environment_id = env_id.to_string();
        }
        deploy.checked_ready |= checked_ready(ctx, &deploy.environment_id, &deploy.commit.sha);
        if is_terminal(&deploy.status) {
            deploy = finish(ctx, team_id, env_id, deploy).await;
        }
        if prev.as_ref() != Some(&deploy) {
            record(ctx, &deploy);
        }
        if !is_terminal(&deploy.status) && deploy.status != "replaced" {
            follow(ctx, deploy.id.clone());
        }
    }
}

/// `iii-cloud::watch`.
pub async fn watch(ctx: &Arc<Ctx>, request: WatchRequest) -> Res<Deploy> {
    let sel = crate::check::selected_for(ctx, request.handoff_id.as_deref()).await?;
    let cfg = ctx.config().await;
    let base = format!(
        "/teams/{}/environments/{}/deployments",
        sel.team_id, sel.env.id
    );
    let id = match request.deployment_id {
        Some(id) => {
            uuid::Uuid::parse_str(&id)
                .map_err(|_| CloudError::invalid("deployment_id must be a UUID"))?;
            id
        }
        None => {
            let list = ctx.call(&Req::get(&base)).await?;
            list.pointer("/deployments/0/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    CloudError::new("CLOUD_ERROR", "This environment has no deployment yet.")
                })?
        }
    };
    let body = ctx.call(&Req::get(format!("{base}/{id}"))).await?;
    let prev = ctx
        .store
        .read(|st| st.deploys.iter().find(|d| d.id == id).cloned());
    let release = has_release(ctx, &sel.team_id, &sel.env.id).await;
    let n = names(ctx);
    let mut deploy = to_deploy(
        &cfg.cloud_url,
        (n.0.as_deref(), n.1.as_deref(), &n.2),
        &body,
        prev.as_ref(),
        release,
    )
    .ok_or_else(|| {
        CloudError::new(
            "CLOUD_ERROR",
            "iii cloud sent a deployment this worker can't read.",
        )
    })?;
    deploy.checked_ready |= checked_ready(ctx, &deploy.environment_id, &deploy.commit.sha);
    if is_terminal(&deploy.status) && prev.as_ref().is_none_or(|p| p.outcome == Outcome::Running) {
        deploy = finish(ctx, &sel.team_id, &sel.env.id, deploy).await;
    }
    if prev.as_ref() != Some(&deploy) {
        record(ctx, &deploy);
    }
    if !is_terminal(&deploy.status) {
        follow(ctx, deploy.id.clone());
    }
    Ok(deploy)
}

/// Pushes made elsewhere: while connected with a selected environment, poll
/// the deployments every 30 s and follow every one created after the
/// environment was selected. Also resumes the non-terminal deploys of the
/// store after a restart.
pub fn ensure_poller(ctx: &Arc<Ctx>) {
    if !ctx.busy.lock().unwrap().insert("poller".into()) {
        return;
    }
    let ctx2 = ctx.clone();
    let task = tokio::spawn(async move {
        loop {
            poll_once(&ctx2).await;
            tokio::time::sleep(ctx2.secs(30)).await;
        }
    });
    // Kept, so a changed cloud URL (which signs out) stops it.
    *ctx.deploys_task.lock().unwrap() = Some(task);
}

pub async fn poll_once(ctx: &Arc<Ctx>) {
    // Every known deploy still moving is followed, of any project and with
    // nothing selected: a switch or a restart never drops one.
    let (since, known) = ctx
        .store
        .read(|st| (st.followed_since.clone(), st.deploys.clone()));
    for d in known.iter().filter(|d| d.is_moving()) {
        follow(ctx, d.id.clone());
    }
    let Ok(sel) = selected(ctx) else { return };
    let list = ctx
        .call(&Req::get(format!(
            "/teams/{}/environments/{}/deployments",
            sel.team_id, sel.env.id
        )))
        .await;
    let Ok(list) = list else { return };
    let cfg = ctx.config().await;
    let release = has_release(ctx, &sel.team_id, &sel.env.id).await;
    for d in list
        .get("deployments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let created = s(d, "created_at").unwrap_or_default();
        let id = s(d, "id").unwrap_or_default();
        if since
            .as_deref()
            .is_some_and(|since| created.as_str() <= since)
            || known.iter().any(|k| k.id == id)
        {
            continue;
        }
        let n = names(ctx);
        if let Some(mut deploy) = to_deploy(
            &cfg.cloud_url,
            (n.0.as_deref(), n.1.as_deref(), &n.2),
            d,
            None,
            release,
        ) {
            deploy.checked_ready = checked_ready(ctx, &deploy.environment_id, &deploy.commit.sha);
            if is_terminal(&deploy.status) {
                deploy = finish(ctx, &sel.team_id, &sel.env.id, deploy).await;
            }
            record(ctx, &deploy);
            if !is_terminal(&deploy.status) {
                follow(ctx, deploy.id.clone());
            }
        }
    }
}
