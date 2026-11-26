//! The pull request read (spec §7) and `iii-cloud::deploy` (spec §8): merge,
//! pin to the plan the person saw, create the deployment with the
//! confirmations, and follow it.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::check::{Selected, differing_keys, fetch_plan, plan_digest, remote_head};
use crate::cloud::{Req, cloud_error};
use crate::ctx::Ctx;
use crate::error::{CloudError, Res, cut};
use crate::tool::{self, gh};
use crate::types::*;
use crate::watch::{finish, follow, is_terminal, record, to_deploy};

const PR_FIELDS: &str = "number,url,title,state,body,headRefName,headRefOid,baseRefName,mergeable,statusCheckRollup,createdAt";

/// The rollup of a pull request's checks.
pub fn rollup(checks: &Value) -> PrChecks {
    let Some(items) = checks.as_array().filter(|a| !a.is_empty()) else {
        return PrChecks::None;
    };
    let text = |c: &Value, k: &str| {
        c.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let failing = items.iter().any(|c| {
        matches!(
            text(c, "conclusion").as_str(),
            "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED"
        ) || matches!(text(c, "state").as_str(), "FAILURE" | "ERROR")
    });
    if failing {
        return PrChecks::Failing;
    }
    let pending = items.iter().any(|c| {
        matches!(
            text(c, "status").as_str(),
            "IN_PROGRESS" | "QUEUED" | "PENDING" | "WAITING"
        ) || text(c, "state") == "PENDING"
    });
    if pending {
        PrChecks::Pending
    } else {
        PrChecks::Passing
    }
}

/// `gh pr view` of the workspace branch. No pull request is `None`.
/// Only the pull request of this try counts: open, from the fix branch, whose
/// head is the worktree's HEAD or a commit of its history, made after the
/// worktree began. A closed one an earlier try left is nothing; a merged one is
/// history (it shows in the deploys).
pub async fn read_pr(
    ws: &Path,
    repo: &str,
    branch: &str,
    env_branch: &str,
    check_base: Option<&str>,
    worktree_started: Option<&str>,
) -> Res<Option<PullRequest>> {
    let out = tool::run(
        "gh",
        &["pr", "view", branch, "--repo", repo, "--json", PR_FIELDS],
        Some(ws),
        None,
        "GH_FAILED",
    )
    .await?;
    if !out.ok() {
        if out.stderr.contains("no pull requests found") {
            return Ok(None);
        }
        return Err(CloudError::new("GH_FAILED", cut(out.stderr.trim(), 2048)));
    }
    let v: Value = serde_json::from_str(&out.stdout)
        .map_err(|_| CloudError::new("GH_FAILED", "gh printed no JSON."))?;
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    if s("state") != "OPEN" || s("headRefName") != branch {
        return Ok(None);
    }
    let head_sha = s("headRefOid");
    let in_history = tool::run(
        "git",
        &["merge-base", "--is-ancestor", &head_sha, "HEAD"],
        Some(ws),
        None,
        "TOOL_MISSING",
    )
    .await?
    .ok();
    if !in_history {
        return Ok(None);
    }
    let (made, began) = (
        parse_time(v.get("createdAt").and_then(Value::as_str)),
        parse_time(worktree_started),
    );
    if let (Some(made), Some(began)) = (made, began)
        && made < began
    {
        return Ok(None);
    }
    let main_moved = match (check_base, remote_head(ws, env_branch).await) {
        (Some(base), Ok(head)) if !base.is_empty() => head != base,
        _ => false,
    };
    Ok(Some(PullRequest {
        number: v.get("number").and_then(Value::as_u64).unwrap_or(0),
        url: s("url"),
        title: s("title"),
        state: match s("state").as_str() {
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            _ => PrState::Open,
        },
        head: s("headRefName"),
        head_sha: s("headRefOid"),
        base: s("baseRefName"),
        body: s("body"),
        checks: rollup(v.get("statusCheckRollup").unwrap_or(&Value::Null)),
        mergeable: v
            .get("mergeable")
            .and_then(Value::as_str)
            .map(str::to_owned),
        main_moved,
    }))
}

fn same_commit(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

pub fn plan_file(ctx: &Ctx, check_id: &str) -> std::path::PathBuf {
    ctx.store
        .dir()
        .join("plans")
        .join(format!("{check_id}.json"))
}

/// Keep the plan a check saw, to name what changed if it changes.
pub fn save_plan(ctx: &Ctx, check_id: &str, plan: &Value) {
    let path = plan_file(ctx, check_id);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, plan.to_string());
}

pub type Since = Option<chrono::DateTime<chrono::Utc>>;

fn parse_time(text: Option<&str>) -> Since {
    text.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// The push's deployment for `target`, from the list: not a replaced one (its
/// successor is the one), and not one made before the merge (`since`).
fn find_push(list: &Value, target: &str, since: Since) -> Option<Value> {
    list.get("deployments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|d| d.get("status").and_then(Value::as_str) != Some("replaced"))
        .filter(|d| {
            let created = parse_time(d.get("created_at").and_then(Value::as_str));
            match (since, created) {
                (Some(since), Some(created)) => created >= since,
                _ => true,
            }
        })
        .find(|d| {
            d.pointer("/commit/sha")
                .and_then(Value::as_str)
                .is_some_and(|s| same_commit(s, target))
        })
        .cloned()
}

/// After a landed merge or push whose pin failed: wait for the push deployment
/// (the same 120 s as the adoption) and cancel it while it is queued, in
/// progress or waiting for action, so "nothing is deployed" holds. `None`:
/// nothing carries the commit (canceled, failed, or no deployment showed up).
/// `Some`: a deployment still carries it, the new file included: one already
/// `ready`, or one whose cancel the cloud refused (409, once Routes and TLS has
/// begun). Any other answer (a 404 because it ended meanwhile, a network error)
/// reads the list again; at the deadline the one last seen is returned.
async fn cancel_push(ctx: &Ctx, base: &str, target: &str, since: Since) -> Option<Value> {
    let deadline = Instant::now() + ctx.secs(120);
    let mut running = None;
    loop {
        if let Ok(list) = ctx.call(&Req::get(base)).await
            && let Some(d) = find_push(&list, target, since)
        {
            match (
                d.get("status").and_then(Value::as_str),
                d.get("id").and_then(Value::as_str),
            ) {
                (Some("failed" | "canceled"), _) => return None,
                (Some("queued" | "in_progress" | "needs_action"), Some(id)) => {
                    match ctx
                        .raw(&Req::post(format!("{base}/{id}/cancel"), json!({})))
                        .await
                    {
                        Ok((200..=299, _)) => return None,
                        Ok((409, _)) => return Some(d),
                        _ => running = Some(d),
                    }
                }
                _ => return Some(d),
            }
        }
        if Instant::now() >= deadline {
            return running;
        }
        tokio::time::sleep(ctx.secs(5)).await;
    }
}

/// With a merge the cloud makes a push deployment: when the target plan needs
/// no confirmation, adopt it; when it does, wait until it is `needs_action`.
/// `Ok(None)` means: create the confirmed deployment.
/// The environment's deployments, following `next_cursor` (at most 5 pages)
/// until the commit's own push shows up: a busy environment can have more than
/// one page of newer ones.
pub async fn list_pages(ctx: &Ctx, base: &str, target: &str, since: Since) -> Res<Value> {
    let mut all: Vec<Value> = vec![];
    let mut cursor: Option<String> = None;
    for _ in 0..5 {
        let mut req = Req::get(base);
        if let Some(c) = &cursor {
            req = req.query("cursor", c.as_str());
        }
        let page = ctx.call(&req).await?;
        all.extend(
            page.get("deployments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        cursor = page
            .get("next_cursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let merged = json!({ "deployments": all });
        if cursor.is_none() || find_push(&merged, target, since).is_some() {
            return Ok(merged);
        }
    }
    Ok(json!({ "deployments": all }))
}

pub async fn push_deployment(
    ctx: &Ctx,
    base: &str,
    target: &str,
    need_confirm: bool,
    since: Since,
) -> Res<Option<Value>> {
    let deadline = Instant::now() + ctx.secs(120);
    loop {
        let list = list_pages(ctx, base, target, since).await?;
        if let Some(d) = find_push(&list, target, since) {
            let status = d.get("status").and_then(Value::as_str).unwrap_or_default();
            // One that already failed is history, not this deploy: start a new one.
            if matches!(status, "failed" | "canceled" | "gone") {
                return Ok(None);
            }
            // One waiting for a person was planned earlier (its validation may have
            // failed, its plan may differ from the check's): never approved here by
            // adoption. A new deployment carries the check's own fingerprint.
            if status == "needs_action" {
                return Ok(None);
            }
            if !need_confirm {
                return Ok(Some(d));
            }
            if status == "needs_action" {
                return Ok(None);
            }
            if is_terminal(status) {
                return Ok(Some(d));
            }
        }
        if Instant::now() >= deadline {
            return Err(CloudError::new(
                "CLOUD_ERROR",
                format!(
                    "iii cloud has no deployment of {} waiting for you after 120 s. Call deploy again, or redeploy it from the console.",
                    crate::ctx::short(target)
                ),
            ));
        }
        tokio::time::sleep(ctx.secs(5)).await;
    }
}

/// What landed must be what was checked: the merge commit's first parent is
/// the base the check saw (a rebase merge only needs the base in its history),
/// and its tree is the checked commit's tree.
async fn verify_merge(
    ws: &Path,
    branch: &str,
    target: &str,
    base: &str,
    commit: &str,
    rebase: bool,
) -> Res<()> {
    crate::tool::git(ws, &["fetch", "origin", branch]).await?;
    let ok = if rebase {
        tool::run(
            "git",
            &["merge-base", "--is-ancestor", base, target],
            Some(ws),
            None,
            "TOOL_MISSING",
        )
        .await?
        .ok()
    } else {
        crate::tool::git(ws, &["rev-parse", &format!("{target}^1")])
            .await
            .map(|p| p.trim() == base)
            .unwrap_or(false)
    };
    if !ok {
        return Err(CloudError::new(
            "MAIN_MOVED",
            format!("Something else landed on {branch} before the merge. Check again."),
        ));
    }
    let tree = |c: String| async move {
        crate::tool::git(ws, &["rev-parse", &format!("{c}^{{tree}}")])
            .await
            .map(|t| t.trim().to_string())
    };
    match (
        tree(target.to_string()).await,
        tree(commit.to_string()).await,
    ) {
        (Ok(a), Ok(b)) if a == b => Ok(()),
        _ => Err(CloudError::new(
            "MAIN_MOVED",
            "The merged commit is not the commit that was checked. Check again.",
        )),
    }
}

/// The environment's compose file as iii cloud has it now.
async fn env_manifest(ctx: &Ctx, sel: &Selected) -> Res<Option<String>> {
    let body = ctx
        .call(&Req::get(format!(
            "/teams/{}/environments/{}",
            sel.team_id, sel.env.id
        )))
        .await?;
    Ok(body
        .get("manifest")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

/// Switch the environment to `manifest` (`environments.update`) and keep the
/// fresh environment record.
async fn set_manifest(ctx: &Arc<Ctx>, sel: &Selected, manifest: &str) -> Res<()> {
    let (status, body) = ctx
        .raw(&Req::patch(
            format!("/teams/{}/environments/{}", sel.team_id, sel.env.id),
            json!({ "manifest": manifest }),
        ))
        .await?;
    if !(200..=299).contains(&status) {
        let mut e = cloud_error(status, &body);
        if status == 403 {
            e.code = "FORBIDDEN".into();
        }
        e.message = format!(
            "{} could not switch to {manifest}: {}",
            sel.env.name, e.message
        );
        return Err(e);
    }
    crate::status::refresh_environment(ctx, sel).await;
    Ok(())
}

/// The deploy failed after the environment switched to `to` and before any
/// deployment carries it: `from` goes back, best effort, and the error says so.
async fn switch_back(
    ctx: &Arc<Ctx>,
    sel: &Selected,
    from: Option<&str>,
    to: &str,
    mut e: CloudError,
) -> CloudError {
    let env = &sel.env.name;
    let note = match from {
        Some(from) => match set_manifest(ctx, sel, from).await {
            Ok(()) => format!("{env} is back on {from}."),
            Err(_) => format!(
                "{env} still runs {to}: putting {from} back failed, so change it back in iii cloud."
            ),
        },
        None => format!("{env} runs {to} now: it had no compose file set before."),
    };
    e.message = format!("{} {note}", e.message);
    e
}

/// The confirmed deployment of `target`, pinned to the fingerprint of the
/// plan that was seen.
async fn create_deployment(
    ctx: &Ctx,
    base: &str,
    check_id: &str,
    target: &str,
    confirmations: &Confirmations,
    fingerprint: String,
) -> Res<Value> {
    let attempt = ctx.store.read(|s| {
        s.deploys
            .iter()
            .filter(|d| {
                same_commit(&d.commit.sha, target)
                    && matches!(d.status.as_str(), "failed" | "canceled")
            })
            .count()
    });
    let mut post = Req::post(
        base,
        json!({
            "commit": target,
            "confirm_resources": !confirmations.resources.is_empty(),
            "confirm_overrides": !confirmations.overrides.is_empty(),
        }),
    );
    post.if_match = Some(fingerprint);
    // A retry after a failure is a new key; the same attempt replays the same deployment.
    post.idempotency_key = Some(format!("iii-cloud:{check_id}:{target}:{attempt}"));
    let (status, body) = ctx.raw(&post).await?;
    match status {
        200..=299 => Ok(body),
        // Only the cloud's own "reviewed plan changed" is PLAN_CHANGED
        // (iii-cloud `crates/deploy/src/api.rs:555`); other 409s keep their code.
        409 if body
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|m| m.contains("plan changed since it was reviewed")) =>
        {
            Err(CloudError::new(
                "PLAN_CHANGED",
                "The plan changed between the read and the deploy. Check again.",
            )
            .with_status(409))
        }
        403 => {
            let mut e = cloud_error(status, &body);
            e.code = "FORBIDDEN".into();
            Err(e)
        }
        _ => Err(cloud_error(status, &body)),
    }
}

pub async fn deploy(ctx: &Arc<Ctx>, request: DeployRequest) -> Res<Deploy> {
    let sel = crate::check::selected_for(ctx, request.handoff_id.as_deref()).await?;
    let cfg = ctx.config().await;
    ctx.credential().await?;
    // 1. The check.
    let check = ctx.store.read(|s| s.find_check(&request.check_id).cloned());
    let check = check
        .filter(|c| c.environment_id == sel.env.id && c.ready)
        .ok_or_else(|| {
            CloudError::new(
                "NOT_READY",
                "The check does not exist, is of another environment, or is not ready.",
            )
        })?;
    // The pin holds at deploy time too: the base is still in the checked commit's history.
    if !check.base.is_empty()
        && !crate::check::base_is_ancestor(
            &sel.workspace,
            &sel.env.branch,
            &check.base,
            &check.commit,
        )
        .await
    {
        return Err(CloudError::new(
            "NOT_READY",
            "The checked commit does not contain the base it was checked against. Check again.",
        ));
    }
    let repo = ctx
        .store
        .read(|s| s.project.as_ref().map(|p| p.repository.clone()))
        .ok_or_else(CloudError::no_project)?;
    let base = format!(
        "/teams/{}/environments/{}/deployments",
        sel.team_id, sel.env.id
    );
    let rebase = cfg.merge_method == crate::config::MergeMethod::Rebase;
    // The file. A check of another compose file than the environment's runs
    // now: the person's deploy is the confirmation, so the environment
    // switches here (never in `check`), before anything makes the deployment.
    let current = match &check.manifest {
        Some(_) => env_manifest(ctx, &sel).await?,
        None => None,
    };
    let switch = check
        .manifest
        .clone()
        .filter(|m| current.as_deref() != Some(m.as_str()));
    let mut switched = false;

    // 2. The target: the merge of the pull request, a waiting deployment, or the checked commit.
    // `landed`: the merge or the push made it the branch's head, so the cloud starts a push deployment.
    let mut landed = false;
    let mut since: Since = None;
    let mut waiting: Option<String> = None;
    let target = match (request.pr, &request.deployment_id) {
        (Some(number), _) => {
            let n = number.to_string();
            let view = gh(
                &sel.workspace,
                &[
                    "pr",
                    "view",
                    &n,
                    "--repo",
                    &repo,
                    "--json",
                    "state,headRefOid,baseRefName,mergeable,mergeCommit,mergedAt",
                ],
            )
            .await
            .map_err(|e| CloudError::new("GH_FAILED", e.message))?;
            let v: Value = serde_json::from_str(&view)
                .map_err(|_| CloudError::new("GH_FAILED", "gh printed no JSON."))?;
            let state = v.get("state").and_then(Value::as_str).unwrap_or_default();
            let head = v
                .get("headRefOid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match state {
                // Resume after a merge (ours, or one done on GitHub): from the pin.
                "MERGED" => {
                    let saved = ctx.store.read(|s| {
                        s.merges
                            .iter()
                            .find(|m| m.check_id == check.id && m.pr == number)
                            .cloned()
                    });
                    // A pull request merged on GitHub must be the commit that was checked.
                    if saved.is_none() && !same_commit(&head, &check.commit) {
                        return Err(CloudError::new(
                            "PR_HEAD_CHANGED",
                            "The pull request was merged with a head that is not the commit that was checked.",
                        ));
                    }
                    let target = saved
                        .as_ref()
                        .map(|m| m.target.clone())
                        .or_else(|| {
                            v.pointer("/mergeCommit/oid")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .ok_or_else(|| {
                            CloudError::new(
                                "PR_NOT_MERGEABLE",
                                "The pull request is merged but GitHub reports no merge commit.",
                            )
                        })?;
                    verify_merge(
                        &sel.workspace,
                        &sel.env.branch,
                        &target,
                        &check.base,
                        &check.commit,
                        rebase,
                    )
                    .await?;
                    since = parse_time(
                        saved
                            .as_ref()
                            .and_then(|m| m.merged_at.as_deref())
                            .or(v.get("mergedAt").and_then(Value::as_str)),
                    );
                    landed = true;
                    target
                }
                "OPEN" => {
                    if !same_commit(&head, &check.commit) {
                        return Err(CloudError::new(
                            "PR_HEAD_CHANGED",
                            "The pull request's head is not the commit that was checked.",
                        ));
                    }
                    if v.get("baseRefName").and_then(Value::as_str) != Some(sel.env.branch.as_str())
                    {
                        return Err(CloudError::new(
                            "PR_NOT_MERGEABLE",
                            format!("The pull request does not target {}.", sel.env.branch),
                        ));
                    }
                    if remote_head(&sel.workspace, &sel.env.branch).await? != check.base {
                        return Err(CloudError::new(
                            "MAIN_MOVED",
                            format!("{} moved since the check. Check again.", sel.env.branch),
                        ));
                    }
                    // Before the merge: the push it starts plans and deploys the new file.
                    if let Some(to) = &switch {
                        set_manifest(ctx, &sel, to).await?;
                        switched = true;
                    }
                    let refused = match tool::run(
                        "gh",
                        &[
                            "pr",
                            "merge",
                            &n,
                            "--repo",
                            &repo,
                            cfg.merge_method.flag(),
                            "--match-head-commit",
                            &head,
                        ],
                        Some(&sel.workspace),
                        None,
                        "GH_FAILED",
                    )
                    .await
                    {
                        Ok(out) if out.ok() => None,
                        Ok(out) => Some(CloudError::new(
                            "PR_NOT_MERGEABLE",
                            cut(out.stderr.trim(), 2048),
                        )),
                        Err(e) => Some(e),
                    };
                    if let Some(e) = refused {
                        return Err(match &switch {
                            Some(to) => switch_back(ctx, &sel, current.as_deref(), to, e).await,
                            None => e,
                        });
                    }
                    let (target, merged_at) = merge_commit(&sel.workspace, &repo, &n).await?;
                    landed = true;
                    since = parse_time(merged_at.as_deref());
                    // Saved right after the merge: a second deploy resumes from the pin.
                    let pending = crate::store::PendingMerge {
                        check_id: check.id.clone(),
                        pr: number,
                        target: target.clone(),
                        merged_at,
                    };
                    ctx.store.update(|s| {
                        s.merges
                            .retain(|m| !(m.check_id == pending.check_id && m.pr == pending.pr));
                        s.merges.push(pending);
                    });
                    verify_merge(
                        &sel.workspace,
                        &sel.env.branch,
                        &target,
                        &check.base,
                        &check.commit,
                        rebase,
                    )
                    .await?;
                    target
                }
                _ => {
                    return Err(CloudError::new(
                        "PR_NOT_MERGEABLE",
                        "The pull request is closed.",
                    ));
                }
            }
        }
        (None, Some(id)) => {
            uuid::Uuid::parse_str(id)
                .map_err(|_| CloudError::invalid("deployment_id must be a UUID"))?;
            let d = ctx.call(&Req::get(format!("{base}/{id}"))).await?;
            if d.get("status").and_then(Value::as_str) != Some("needs_action") {
                return Err(CloudError::new(
                    "NOT_READY",
                    "The deployment is not waiting for action.",
                ));
            }
            let target = d
                .pointer("/commit/sha")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    CloudError::new(
                        "CLOUD_ERROR",
                        "iii cloud sent a deployment without a commit.",
                    )
                })?;
            // A merge made on GitHub: what landed must be what was checked.
            verify_merge(
                &sel.workspace,
                &sel.env.branch,
                &target,
                &check.base,
                &check.commit,
                rebase,
            )
            .await?;
            waiting = Some(id.clone());
            target
        }
        (None, None) => {
            // Deploy takes commits of the environment's branch only: the plan of any
            // commit is readable, the deploy is not (the cloud answers 422).
            let on_branch = {
                let _ =
                    crate::tool::git(&sel.workspace, &["fetch", "origin", &sel.env.branch]).await;
                tool::run(
                    "git",
                    &[
                        "merge-base",
                        "--is-ancestor",
                        &check.commit,
                        &format!("origin/{}", sel.env.branch),
                    ],
                    Some(&sel.workspace),
                    None,
                    "TOOL_MISSING",
                )
                .await?
                .ok()
            };
            let default = ctx
                .store
                .read(|s| s.project.as_ref().and_then(|p| p.default_branch.clone()));
            if !on_branch
                && !crate::project::tracks_other_branch(default.as_deref(), &sel.env.branch)
            {
                return Err(CloudError::new(
                    "MERGE_FIRST",
                    "Merge the pull request first: Deploy… merges it and deploys the merged commit.",
                ));
            }
            if !on_branch {
                // An environment on its own branch has no pull request: this deploy
                // pushes the checked commit to it, on top of the base it was checked on.
                if remote_head(&sel.workspace, &sel.env.branch).await? != check.base {
                    return Err(CloudError::new(
                        "MAIN_MOVED",
                        format!("{} moved since the check. Check again.", sel.env.branch),
                    ));
                }
                // Before the push: the push deployment it starts plans the new file.
                if let Some(to) = &switch {
                    set_manifest(ctx, &sel, to).await?;
                    switched = true;
                }
                if let Err(mut e) = push_env_branch(&sel, &check).await {
                    return Err(match &switch {
                        // It may have landed: its push deployment would carry the new file.
                        Some(to) if e.code == "PUSH_UNKNOWN" => {
                            e.message = format!("{} {} stays on {to}.", e.message, sel.env.name);
                            e
                        }
                        Some(to) => switch_back(ctx, &sel, current.as_deref(), to, e).await,
                        None => e,
                    });
                }
                // No `since`: the commit was on no deployment of this environment
                // (it was not on the branch), and this machine's clock is not the cloud's.
                landed = true;
            }
            check.commit.clone()
        }
    };

    // 4. Pin: the plan of the target must be the plan that was seen, with the checked file.
    let (status, body) = fetch_plan(ctx, &sel, &target, switch.as_deref()).await?;
    if status != 200 {
        return Err(cloud_error(status, &body));
    }
    let plan = body.get("plan").cloned().unwrap_or(Value::Null);
    if Some(plan_digest(&plan)) != check.plan.digest {
        if landed && let Some(running) = cancel_push(ctx, &base, &target, since).await {
            // Not "check again": the merged commit is deploying. Status shows it, followed.
            if let Some(id) = running.get("id").and_then(Value::as_str) {
                ctx.store.update(|s| {
                    if !s.worker_deploys.iter().any(|d| d == id) {
                        s.worker_deploys.push(id.to_string());
                    }
                });
            }
            crate::watch::refresh_recent(ctx, &sel.team_id, &sel.env.id, &sel.env.name).await;
            // That push carries the new file: the switch stays.
            let kept = match (&switch, switched) {
                (Some(to), true) => format!(" {} now uses {to}.", sel.env.name),
                _ => String::new(),
            };
            return Err(CloudError::new(
                "PUSH_STILL_RUNNING",
                format!(
                    "The plan changed since the check, and the deployment of {} was not cancelled (it is {}). Follow it in Deploys.{kept}",
                    crate::ctx::short(&target),
                    running
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("running")
                        .replace('_', " ")
                ),
            ));
        }
        let keys = std::fs::read_to_string(plan_file(ctx, &check.id))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|old| differing_keys(&old, &plan))
            .unwrap_or_default();
        let e = CloudError::new(
            "PLAN_CHANGED",
            format!(
                "The plan changed since the check{}. Check again.",
                if keys.is_empty() {
                    String::new()
                } else {
                    format!(": {}", keys.join(", "))
                }
            ),
        );
        // The push was canceled (or never showed): nothing carries the new file.
        return Err(match &switch {
            Some(to) if switched => switch_back(ctx, &sel, current.as_deref(), to, e).await,
            _ => e,
        });
    }
    let fingerprint = body
        .get("fingerprint")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let confirmations = crate::check::confirmations(&plan);
    let need_confirm = !confirmations.resources.is_empty() || !confirmations.overrides.is_empty();

    // A switch not made before a merge is made now, right before the deployment that
    // carries it. The push of this commit, or the named waiting deployment, was
    // made with the old file: never adopted; the new deployment replaces a
    // waiting one.
    let late = switch.is_some() && !switched;
    if let (true, Some(to)) = (late, &switch) {
        set_manifest(ctx, &sel, to).await?;
    }

    // 5. The push's deployment: adopt it, or confirm it with a new one. A named
    // waiting deployment is confirmed; nothing else is searched.
    let adopted = if waiting.is_some() || late {
        None
    } else {
        push_deployment(ctx, &base, &target, need_confirm, since).await?
    };
    let (created, approved) = match adopted {
        Some(adopted) => (adopted, false),
        None => {
            match create_deployment(ctx, &base, &check.id, &target, &confirmations, fingerprint)
                .await
            {
                Ok(body) => (body, true),
                Err(mut e) => {
                    // A refusal the cloud answered (a 4xx) made nothing: the old file goes
                    // back. With no answer (a transport error has no status) or a 5xx (a
                    // gateway that timed out, say) the deployment may exist.
                    let answered = e.status.is_some_and(|s| (400..500).contains(&s))
                        || matches!(e.code.as_str(), "NOT_CONNECTED" | "KEY_REJECTED");
                    return Err(match &switch {
                        Some(to) if late && answered => {
                            switch_back(ctx, &sel, current.as_deref(), to, e).await
                        }
                        Some(to) if late => {
                            e.message = format!(
                                "{} The answer was lost: {} stays on {to}; see Deploys.",
                                e.message, sel.env.name
                            );
                            e
                        }
                        _ => e,
                    });
                }
            }
        }
    };

    // 7. Record and follow.
    let release = !plan.pointer("/settings/release").is_none_or(Value::is_null);
    let (team, project) = ctx.store.read(|s| {
        let p = s.project.as_ref();
        (
            p.and_then(|p| p.team.as_ref()).map(|t| t.slug.clone()),
            p.and_then(|p| p.cloud_project.as_ref())
                .map(|c| c.slug.clone()),
        )
    });
    let mut record_ = to_deploy(
        &cfg.cloud_url,
        (team.as_deref(), project.as_deref(), &sel.env.name),
        &created,
        None,
        release,
    )
    .ok_or_else(|| {
        CloudError::new(
            "CLOUD_ERROR",
            "iii cloud sent a deployment this worker can't read.",
        )
    })?;
    record_.pr = request.pr;
    record_.approved_by_worker = approved;
    record_.checked_ready = true;
    record_.check_id = Some(check.id.clone());
    if record_.environment_id.is_empty() {
        record_.environment_id = sel.env.id.clone();
    }
    if is_terminal(&record_.status) {
        record_ = finish(ctx, &sel.team_id, &sel.env.id, record_).await;
    }
    // This worker made it (or adopted the push of its own merge): not a console deploy.
    ctx.store.update(|s| {
        if !s.worker_deploys.contains(&record_.id) {
            s.worker_deploys.push(record_.id.clone());
        }
    });
    record(ctx, &record_);
    ctx.store
        .update(|s| s.merges.retain(|m| m.check_id != check.id));
    if !is_terminal(&record_.status) {
        follow(ctx, record_.id.clone());
    }
    Ok(record_)
}

/// Push the checked commit to the environment's own branch, never forced. A
/// refusal is `MAIN_MOVED` when the branch moved meanwhile, else `PUSH_REFUSED`;
/// `PUSH_UNKNOWN` when origin can't be read after a failed push either.
async fn push_env_branch(sel: &Selected, check: &Check) -> Res<()> {
    let refspec = format!("{}:refs/heads/{}", check.commit, sel.env.branch);
    let out = tool::run(
        "git",
        &["push", "origin", &refspec],
        Some(&sel.workspace),
        None,
        "TOOL_MISSING",
    )
    .await?;
    if out.ok() {
        return Ok(());
    }
    match remote_head(&sel.workspace, &sel.env.branch).await {
        // The answer was lost, but the push landed.
        Ok(head) if head == check.commit => Ok(()),
        Ok(head) if head != check.base => Err(CloudError::new(
            "MAIN_MOVED",
            format!("{} moved since the check. Check again.", sel.env.branch),
        )),
        Err(_) => Err(CloudError::new(
            "PUSH_UNKNOWN",
            cut(
                &format!(
                    "Could not tell whether {} reached {}: the push failed and origin did not answer after. See Deploys before deploying again. Git said: {}",
                    crate::ctx::short(&check.commit),
                    sel.env.branch,
                    out.stderr.trim()
                ),
                2048,
            ),
        )),
        Ok(_) => Err(CloudError::new(
            "PUSH_REFUSED",
            cut(
                &format!(
                    "GitHub refused the push to {}: {}",
                    sel.env.branch,
                    out.stderr.trim()
                ),
                2048,
            ),
        )),
    }
}

/// The merge commit and the time of the merge, read until they appear (30 s).
async fn merge_commit(ws: &Path, repo: &str, number: &str) -> Res<(String, Option<String>)> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let out = gh(
            ws,
            &[
                "pr",
                "view",
                number,
                "--repo",
                repo,
                "--json",
                "mergeCommit,mergedAt",
            ],
        )
        .await?;
        let v: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
        if let Some(oid) = v.pointer("/mergeCommit/oid").and_then(Value::as_str) {
            return Ok((
                oid.to_string(),
                v.get("mergedAt").and_then(Value::as_str).map(str::to_owned),
            ));
        }
        if Instant::now() >= deadline {
            return Err(CloudError::new(
                "GH_FAILED",
                "GitHub did not report the merge commit in 30 s.",
            ));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
