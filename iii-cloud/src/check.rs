//! `iii-cloud::check` (spec §4): the cloud plan for a commit, the checks here,
//! one list of problems, and the readiness rule. The pure rules (issue to
//! problem, owner table, plan digest, ready, report) are public for the tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::cloud::Req;
use crate::ctx::{Ctx, new_id, now, short};
use crate::error::{CloudError, Res};
use crate::local::{LocalInput, Manifest, accepts_cloud_block, local_checks, parse_version};
use crate::tool::{self, git};
use crate::types::*;

/// `extra` tells apart problems of one code with no file (which variable, which runtime).
pub fn problem_id(
    source: Source,
    code: &str,
    file: Option<&str>,
    line: Option<u32>,
    extra: &str,
) -> String {
    let source = match source {
        Source::CloudPlan => "cloud_plan",
        Source::CheckHere => "check_here",
    };
    let mut hash = Sha1::new();
    hash.update(format!(
        "{source}\n{code}\n{}\n{}\n{extra}",
        file.unwrap_or(""),
        line.map(|l| l.to_string()).unwrap_or_default()
    ));
    hex::encode(hash.finalize())
}

/// Codes that always go to the person, whatever their `kind` (spec §4).
/// Pinned by a golden test; `skills/plan-codes.md` lists the same set.
pub const PERSON_CODES: [&str; 8] = [
    "RUNTIME_UNSUPPORTED",
    "LITERAL_CREDENTIAL",
    "SECRET_MISSING",
    "MULTIPLE_HTTP_GROUPS",
    "LOCAL_CONFIG_NOT_SET",
    "ENV_FILE_MISSING",
    "EMPTY_DEFAULT_VARIABLE",
    "WEB_*",
];

/// Who handles a plan issue. `fix_in_repo` is the agent's, `resolve_here` the
/// person's; a note without a kind goes to the person when it warns and to
/// nobody when it is only information.
pub fn owner_for(code: &str, kind: Option<&str>, severity: Severity) -> Owner {
    if PERSON_CODES.contains(&code) || code.starts_with("WEB_") {
        return Owner::Person;
    }
    match (kind, severity) {
        (Some("fix_in_repo"), _) => Owner::Agent,
        (Some("resolve_here"), _) => Owner::Person,
        (_, Severity::Info) => Owner::None,
        _ => Owner::Person,
    }
}

fn severity_of(issue: &Value) -> Severity {
    match issue.get("severity").and_then(Value::as_str) {
        Some("block") => Severity::Block,
        Some("info") => Severity::Info,
        _ => Severity::Warn,
    }
}

/// `issues[]` and `deploy_issues[]` of a plan as problems.
pub fn plan_problems(plan: &Value) -> Vec<Problem> {
    ["issues", "deploy_issues"]
        .iter()
        .flat_map(|k| plan.get(*k).and_then(Value::as_array).into_iter().flatten())
        .map(|issue| {
            let cloud = issue
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN")
                .to_string();
            let code = issue
                .pointer("/context/iii_code")
                .and_then(Value::as_str)
                .unwrap_or(&cloud)
                .to_string();
            let severity = severity_of(issue);
            let file = issue
                .pointer("/location/file")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let line = issue
                .pointer("/location/line")
                .and_then(Value::as_u64)
                .map(|l| l as u32);
            let excerpt = issue.get("excerpt").and_then(|e| {
                Some(Excerpt {
                    start_line: e.get("start_line")?.as_u64()? as u32,
                    lines: e
                        .get("lines")?
                        .as_array()?
                        .iter()
                        .filter_map(|l| l.as_str().map(str::to_owned))
                        .collect(),
                })
            });
            let detail = crate::problems::plan_detail(issue);
            let kind = issue.get("kind").and_then(Value::as_str);
            let mut problem = Problem {
                skipped: false,
                id: problem_id(
                    Source::CloudPlan,
                    &code,
                    file.as_deref(),
                    line,
                    detail.as_deref().unwrap_or(""),
                ),
                source: Source::CloudPlan,
                owner: owner_for(&code, kind, severity),
                title: crate::problems::title_for(&code),
                fix: crate::problems::plan_fix(&code, issue, plan),
                detail,
                code,
                cloud_code: Some(cloud),
                severity,
                file,
                line,
                excerpt,
            };
            if let Some(name) = crate::problems::issue_variable(issue) {
                crate::problems::database_value_owner(&mut problem, name);
            }
            problem
        })
        .collect()
}

/// Sorted keys, no whitespace.
pub fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical(&map[*k])
                    )
                })
                .collect();
            format!("{{{}}}", body.join(","))
        }
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

/// The plan keys the digest covers (and `differing_keys` names).
pub const DIGEST_KEYS: [&str; 8] = [
    "overrides",
    "settings",
    "workers",
    "groups",
    "resources",
    "variables",
    "issues",
    "deploy_issues",
];

/// SHA-256 of the plan without its commit (spec §8): `source.commit`,
/// `source.branch` and `environment` are left out.
pub fn plan_digest(plan: &Value) -> String {
    let mut subset = Map::new();
    for key in DIGEST_KEYS {
        subset.insert(
            key.to_string(),
            plan.get(key).cloned().unwrap_or(Value::Null),
        );
    }
    for key in ["manifest_sha256", "lock_sha256"] {
        subset.insert(
            key.to_string(),
            plan.pointer(&format!("/source/{key}"))
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    hex::encode(Sha256::digest(canonical(&Value::Object(subset)).as_bytes()))
}

/// The top-level keys of the digest input that differ between two plans.
pub fn differing_keys(a: &Value, b: &Value) -> Vec<String> {
    let mut keys = vec![];
    for key in DIGEST_KEYS {
        if a.get(key).unwrap_or(&Value::Null) != b.get(key).unwrap_or(&Value::Null) {
            keys.push(key.to_string());
        }
    }
    for key in ["manifest_sha256", "lock_sha256"] {
        let at = format!("/source/{key}");
        if a.pointer(&at) != b.pointer(&at) {
            keys.push(key.to_string());
        }
    }
    keys
}

/// Delete the saved plans of checks no longer kept.
pub fn prune_plans(ctx: &Ctx) {
    let kept: Vec<String> = ctx
        .store
        .read(|s| s.checks.iter().map(|c| c.id.clone()).collect());
    let Ok(read) = std::fs::read_dir(ctx.store.dir().join("plans")) else {
        return;
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !kept.iter().any(|id| name == format!("{id}.json")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub fn confirmations(plan: &Value) -> Confirmations {
    let resources = plan
        .get("resources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| r.get("action").and_then(Value::as_str) == Some("create"))
        .map(|r| ResourceConfirmation {
            kind: r
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            alias: r
                .get("alias")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            binding: r.get("binding").and_then(Value::as_str).map(str::to_owned),
            tier: r
                .pointer("/postgres/tier")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
        .collect();
    let overrides = plan
        .get("overrides")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|o| OverrideConfirmation {
            setting: o
                .get("setting")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            console: o.get("console").cloned().unwrap_or(Value::Null),
            file: o.get("file").cloned().unwrap_or(Value::Null),
        })
        .collect();
    Confirmations {
        resources,
        overrides,
    }
}

/// The plan part of a 200 answer.
pub fn plan_part(answer: &Value) -> (PlanPart, Vec<Problem>) {
    let plan = answer.get("plan").unwrap_or(&Value::Null);
    let problems = plan_problems(plan);
    let blocking = problems
        .iter()
        .filter(|p| p.severity == Severity::Block)
        .count() as u32;
    let warnings = problems
        .iter()
        .filter(|p| p.severity == Severity::Warn)
        .count() as u32;
    let can_deploy = answer
        .get("can_deploy")
        .and_then(Value::as_bool)
        .unwrap_or(blocking == 0);
    let part = PlanPart {
        status: if can_deploy {
            PlanStatus::Clean
        } else {
            PlanStatus::Blocked
        },
        fingerprint: answer
            .get("fingerprint")
            .and_then(Value::as_str)
            .map(str::to_owned),
        digest: Some(plan_digest(plan)),
        blocking,
        warnings,
        at_least: blocking > 0,
        confirmations: confirmations(plan),
        web_candidates: plan.pointer("/settings/web_candidates").cloned(),
        error: None,
        summary: plan
            .get("workers")
            .and_then(Value::as_array)
            .map(|workers| PlanSummary {
                workers: workers.len() as u32,
                groups: plan
                    .get("groups")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|g| g.get("name")?.as_str().map(str::to_owned))
                    .collect(),
            }),
    };
    (part, problems)
}

/// The plan problems the person skipped for now: `skipped`. By name
/// (`values::skip`), `EMPTY_DEFAULT_VARIABLE` turns to info (it never counts)
/// and `SECRET_MISSING` stays a block, since iii cloud needs a value to
/// deploy, even an empty one. By code (`problem::skip`), a warning turns to info.
pub fn mark_skipped(
    answer: &Value,
    part: &mut PlanPart,
    problems: &mut [Problem],
    skipped: &std::collections::BTreeSet<String>,
    codes: &std::collections::BTreeSet<String>,
) {
    for p in problems.iter_mut() {
        if codes.contains(&p.code) && p.severity == Severity::Warn {
            p.skipped = true;
            p.severity = Severity::Info;
        }
    }
    let plan = answer.get("plan").unwrap_or(&Value::Null);
    // `plan_problems` keeps the issues' order: one problem per issue.
    let issues = ["issues", "deploy_issues"]
        .iter()
        .flat_map(|k| plan.get(*k).and_then(Value::as_array).into_iter().flatten());
    for (p, issue) in problems.iter_mut().zip(issues) {
        if !crate::problems::issue_variable(issue).is_some_and(|n| skipped.contains(n)) {
            continue;
        }
        match p.code.as_str() {
            "EMPTY_DEFAULT_VARIABLE" => {
                p.skipped = true;
                p.severity = Severity::Info;
            }
            "SECRET_MISSING" => {
                p.skipped = true;
                p.detail = Some("iii cloud needs a value to deploy, even an empty one.".into());
            }
            _ => {}
        }
    }
    part.warnings = problems
        .iter()
        .filter(|p| p.severity == Severity::Warn)
        .count() as u32;
}

fn empty_plan(status: PlanStatus, error: Option<CloudError>) -> PlanPart {
    PlanPart {
        status,
        fingerprint: None,
        digest: None,
        blocking: 0,
        warnings: 0,
        at_least: false,
        confirmations: Confirmations {
            resources: vec![],
            overrides: vec![],
        },
        web_candidates: None,
        error: error.map(|e| e.to_worker_error()),
        summary: None,
    }
}

/// `ready = plan clean, no blocking problem here, and a passed run for this commit`.
pub fn is_ready(plan: &PlanPart, problems: &[Problem], run: Option<&Run>, commit: &str) -> bool {
    plan.status == PlanStatus::Clean
        && !problems
            .iter()
            .any(|p| p.source == Source::CheckHere && p.severity == Severity::Block)
        && run.is_some_and(|r| r.status == RunStatus::Passed && r.commit == commit)
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_lowercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The text the agent pastes into the pull request body (spec §4).
pub fn report(commit: &str, plan: &PlanPart, run: Option<&Run>) -> String {
    let sha = short(commit);
    let plan_line = match plan.status {
        PlanStatus::Clean => format!("Cloud plan for {sha}: no blocking issue."),
        PlanStatus::Blocked => format!(
            "Cloud plan for {sha}: {} blocking issue{}{}.",
            plan.blocking,
            if plan.blocking == 1 { "" } else { "s" },
            if plan.at_least { " at least" } else { "" }
        ),
        PlanStatus::NotPushed => format!(
            "Cloud plan for {sha}: not pushed to GitHub yet, so the cloud has no plan for it."
        ),
        PlanStatus::CommitNotPlanned => format!(
            "Cloud plan for {sha}: the cloud can't see this commit in the repository yet (push the branch)."
        ),
        PlanStatus::Unavailable => format!("Cloud plan for {sha}: unavailable."),
    };
    let run_line = match run {
        None => "Not run here yet.".to_string(),
        Some(r) if r.status == RunStatus::Running => "Running here like the cloud.".to_string(),
        Some(r) if r.status == RunStatus::Failed => match &r.error {
            Some(e) => format!(
                "Ran here like the cloud and failed at {:?}: {}.",
                e.stage, e.code
            )
            .replace("Install", "install"),
            None => "Ran here like the cloud and failed.".to_string(),
        },
        Some(r) => {
            let prefix = r
                .routes
                .prefix
                .as_ref()
                .map(|p| format!(" under {p}"))
                .unwrap_or_default();
            let health = r
                .healthcheck
                .as_ref()
                .and_then(|h| h.status)
                .map(|s| format!(", healthcheck {s}"))
                .unwrap_or_default();
            format!(
                "Ran here like the cloud: {} of {} workers, {} routes{prefix}{health}.",
                r.workers.ready, r.workers.total, r.routes.count
            )
        }
    };
    let covered = run
        .map(|r| r.not_covered.clone())
        .unwrap_or_else(default_not_covered);
    let covered: Vec<String> = covered.iter().map(|s| lower_first(s)).collect();
    format!(
        "## iii cloud readiness\n{plan_line}\n{run_line}\nNot covered: {}.\n",
        join_and(&covered)
    )
}

pub fn default_not_covered() -> Vec<String> {
    vec![
        "The cloud's build container".into(),
        "Production's secret values".into(),
        "Its machine size".into(),
        "Real traffic".into(),
    ]
}

/// Remote branch tips contain the commit: GitHub has it. Only the environment
/// branch is fetched by `fetch_remote`, so a commit on no tracking ref gets one
/// `git fetch origin` (every branch) before it counts as not pushed.
pub async fn pushed(ws: &std::path::Path, sha: &str) -> bool {
    let on_remote = || async {
        git(ws, &["branch", "-r", "--contains", sha])
            .await
            .is_ok_and(|o| !o.trim().is_empty())
    };
    if on_remote().await {
        return true;
    }
    let _ = tool::run("git", &["fetch", "origin"], Some(ws), None, "TOOL_MISSING").await;
    on_remote().await
}

/// The check's one GitHub round trip: the environment branch's tip into its
/// remote-tracking ref, and the commit too when it is not in the repository.
/// False when the branch could not be fetched: its remote head is then unknown.
pub async fn fetch_remote(ws: &std::path::Path, branch: &str, commit: &str) -> bool {
    let run = |args: Vec<String>| async move {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        tool::run("git", &refs, Some(ws), None, "TOOL_MISSING").await
    };
    let spec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    let here = run(vec![
        "cat-file".into(),
        "-e".into(),
        format!("{commit}^{{commit}}"),
    ])
    .await
    .is_ok_and(|o| o.ok());
    // One after the other: two git processes on a repository can fail on its lock.
    let fetched = run(vec!["fetch".into(), "origin".into(), spec]).await;
    if !here {
        let _ = run(vec!["fetch".into(), "origin".into(), commit.into()]).await;
    }
    fetched.is_ok_and(|o| o.ok())
}

/// The environment branch's head as of the last `fetch_remote`.
pub async fn fetched_head(ws: &std::path::Path, branch: &str) -> Res<String> {
    git(
        ws,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/remotes/origin/{branch}^{{commit}}"),
        ],
    )
    .await
    .map(|s| s.trim().to_string())
    .map_err(|_| CloudError::new("GIT_FAILED", format!("origin has no branch {branch}.")))
}

/// `base` is an ancestor of `commit`; only exit 0 counts. Local, no fetch.
pub async fn is_ancestor(ws: &std::path::Path, base: &str, commit: &str) -> bool {
    tool::run(
        "git",
        &["merge-base", "--is-ancestor", base, commit],
        Some(ws),
        None,
        "TOOL_MISSING",
    )
    .await
    .is_ok_and(|o| o.code == Some(0))
}

/// The commits of `base..commit` in the workspace, newest first, at most 50.
/// Empty when the base is unknown or either commit is not here.
pub async fn commits_between(ws: &std::path::Path, base: &str, commit: &str) -> Vec<CommitLine> {
    if base.is_empty() {
        return vec![];
    }
    let range = format!("{base}..{commit}");
    git(ws, &["log", "--format=%H%x1f%s", "-n", "50", &range])
        .await
        .map(|out| {
            out.lines()
                .filter_map(|l| {
                    let (sha, message) = l.split_once('\x1f')?;
                    Some(CommitLine {
                        sha: sha.to_string(),
                        message: message.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `iii-cloud::check::get`: a kept check as it was recorded; nothing runs again.
pub fn get(ctx: &Ctx, request: CheckGetRequest) -> Res<Check> {
    ctx.store
        .read(|s| s.find_check(&request.id).cloned())
        .ok_or_else(|| CloudError::invalid("no such check (only the latest 20 are kept)"))
}

/// `check {push: true}`: the checked commit goes to the workspace's branch on
/// origin, replacing an earlier try's (`--force-with-lease`). The commit, not
/// `HEAD`: a commit made meanwhile is not what this check plans. Only an
/// `iii-cloud/` fix branch no environment tracks: an environment's branch is
/// pushed by the person's deploy.
async fn push_fix_branch(ctx: &Ctx, sel: &Selected, commit: &str) -> Res<()> {
    let branch = git(&sel.workspace, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await?
        .trim()
        .to_string();
    let tracked = branch == sel.env.branch
        || ctx.store.read(|s| {
            s.project.as_ref().is_some_and(|p| {
                p.environments.iter().any(|e| e.branch == branch)
                    || p.cloud_projects
                        .iter()
                        .flat_map(|c| &c.environments)
                        .any(|e| e.branch == branch)
            })
        });
    if tracked {
        return Err(CloudError::new(
            "PUSH_REFUSED",
            format!("An environment deploys {branch}: only the person's deploy pushes it."),
        ));
    }
    if !branch.starts_with("iii-cloud/") {
        return Err(CloudError::new(
            "PUSH_REFUSED",
            format!("{branch} is not a fix branch: the check pushes only an iii-cloud/ branch."),
        ));
    }
    git(
        &sel.workspace,
        &[
            "push",
            "--force-with-lease",
            "origin",
            &format!("{commit}:refs/heads/{branch}"),
        ],
    )
    .await
    .map(|_| ())
}

/// The environment branch's head on the remote.
pub async fn remote_head(ws: &std::path::Path, branch: &str) -> Res<String> {
    remote_ref(ws, branch)
        .await?
        .ok_or_else(|| CloudError::new("GIT_FAILED", format!("origin has no branch {branch}.")))
}

/// A branch's head on the remote: `Ok(None)` when origin answered and has no
/// such branch, `Err` when it did not answer.
pub async fn remote_ref(ws: &std::path::Path, branch: &str) -> Res<Option<String>> {
    let out = git(
        ws,
        &["ls-remote", "origin", &format!("refs/heads/{branch}")],
    )
    .await?;
    Ok(out.split_whitespace().next().map(str::to_owned))
}

/// `base` is an ancestor of `commit`. The base is fetched first, and the
/// commit too when it is not here; only exit 0 counts.
pub async fn base_is_ancestor(
    ws: &std::path::Path,
    branch: &str,
    base: &str,
    commit: &str,
) -> bool {
    let run = |args: Vec<String>| async move {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        tool::run("git", &refs, Some(ws), None, "TOOL_MISSING").await
    };
    let _ = run(vec!["fetch".into(), "origin".into(), branch.into()]).await;
    if !run(vec![
        "cat-file".into(),
        "-e".into(),
        format!("{commit}^{{commit}}"),
    ])
    .await
    .is_ok_and(|o| o.ok())
    {
        let _ = run(vec!["fetch".into(), "origin".into(), commit.into()]).await;
    }
    run(vec![
        "merge-base".into(),
        "--is-ancestor".into(),
        base.into(),
        commit.into(),
    ])
    .await
    .is_ok_and(|o| o.code == Some(0))
}

pub async fn local_iii_version(iii_bin: &str) -> Option<String> {
    let out = tool::run(iii_bin, &["--version"], None, None, "TOOL_MISSING")
        .await
        .ok()?;
    parse_version(&out.stdout)
}

#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Selected {
    pub workspace: PathBuf,
    pub env: EnvironmentInfo,
    pub team_id: String,
}

/// The selected environment, its workspace and team.
pub fn selected(ctx: &Ctx) -> Res<Selected> {
    let cfg_conn = ctx.store.read(|s| s.project.clone());
    let project = cfg_conn.ok_or_else(CloudError::no_project)?;
    let (Some(ws), Some(env)) = (project.workspace.clone(), project.environment.clone()) else {
        return Err(CloudError::no_project());
    };
    let team_id = project
        .team
        .as_ref()
        .map(|t| t.id.clone())
        .or_else(|| {
            ctx.store.read(|s| {
                s.connection
                    .as_ref()
                    .and_then(|c| c.api_key.as_ref())
                    .map(|k| k.team_id.clone())
            })
        })
        .ok_or_else(CloudError::no_project)?;
    Ok(Selected {
        workspace: PathBuf::from(ws.path),
        env,
        team_id,
    })
}

/// The environment, worktree and team an agent tool acts on: the hand-off the
/// caller names (its own, whatever the page has selected now), else the global
/// selection. A hand-off whose worktree is on another branch is `WRONG_WORKTREE`.
pub async fn selected_for(ctx: &Ctx, handoff_id: Option<&str>) -> Res<Selected> {
    let Some(id) = handoff_id else {
        return selected(ctx);
    };
    let h = ctx
        .store
        .read(|s| s.handoffs.iter().find(|h| h.is(id)).cloned())
        .ok_or_else(|| CloudError::invalid("no such hand-off"))?;
    let (Some(env), false) = (h.env.clone(), h.workspace_path.is_empty()) else {
        // A hand-off made before it recorded its worktree: the global selection.
        return selected(ctx);
    };
    let workspace = PathBuf::from(&h.workspace_path);
    let head = git(&workspace, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .map(|b| b.trim().to_string());
    if !h.branch.is_empty() && head.as_deref().ok() != Some(h.branch.as_str()) {
        return Err(CloudError::new(
            "WRONG_WORKTREE",
            format!(
                "This hand-off works on {} in {}; the worktree is on {}.",
                h.branch,
                h.workspace_path,
                head.unwrap_or_else(|_| "an unknown branch".into())
            ),
        ));
    }
    Ok(Selected {
        workspace,
        env,
        team_id: h.team_id,
    })
}

/// The paths `git status` lists as changed in `dir`, every untracked file by
/// itself (`-z`; a rename or copy is not compared: `None`).
async fn changed_paths(dir: &Path) -> Option<Vec<String>> {
    let out = git(
        dir,
        &["status", "--porcelain", "-z", "--untracked-files=all"],
    )
    .await
    .ok()?;
    let mut paths = vec![];
    for entry in out.split('\0').filter(|e| !e.is_empty()) {
        if entry.len() < 4 || entry.starts_with(['R', 'C']) || entry[1..2].starts_with(['R', 'C']) {
            return None;
        }
        paths.push(entry[3..].to_string());
    }
    Some(paths)
}

/// Every uncommitted path of the worktree is uncommitted in the person's folder
/// too, with the same content: the files an older worker copied. Paths and
/// bytes only; anything else is not a copy.
async fn copies_of_the_folders_changes(ws: &Path, folder: &Path) -> Option<Vec<String>> {
    let mine = changed_paths(ws).await?;
    let theirs = changed_paths(folder).await?;
    if mine.is_empty() {
        return None;
    }
    for p in &mine {
        // A copy is a file both sides can read, with the same bytes: a deletion
        // on both sides, or a directory, is not.
        let same = match (std::fs::read(ws.join(p)), std::fs::read(folder.join(p))) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        if !theirs.contains(p) || !same {
            return None;
        }
    }
    Some(mine)
}

/// Commits of the worktree branch made before the worktree was started that are
/// not on the base: an earlier try's. Only for a worktree this worker made. The
/// commits it took from the person's checkout (`from_head`) are the base too.
async fn old_attempt_commits(ctx: &Ctx, sel: &Selected, commit: &str) -> Option<usize> {
    let (path, branch, from_head) = ctx.store.read(|s| {
        s.project
            .as_ref()
            .and_then(|p| p.workspace.as_ref())
            .map(|w| (w.path.clone(), w.branch.clone(), w.from_head.clone()))
    })?;
    if std::path::Path::new(&path) != sel.workspace {
        return None;
    }
    let started = ctx
        .store
        .read(|s| {
            s.worktree_started
                .get(&format!("{path}\n{branch}"))
                .cloned()
        })
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())?
        .timestamp();
    let range = match from_head {
        Some(head) => format!("{head}..{commit}"),
        None => format!("refs/remotes/origin/{}..{commit}", sel.env.branch),
    };
    let out = git(&sel.workspace, &["log", "--format=%ct", &range])
        .await
        .ok()?;
    let old = out
        .lines()
        .filter_map(|l| l.trim().parse::<i64>().ok())
        .filter(|ct| *ct < started)
        .count();
    (old > 0).then_some(old)
}

/// The cloud's 404 is about the commit (not planned yet), as opposed to an
/// environment or route that does not exist.
pub fn is_commit_404(body: &Value) -> bool {
    body.get("message")
        .and_then(Value::as_str)
        .is_some_and(|m| m.to_lowercase().contains("commit"))
}

/// The plan of a commit, with `manifest` in place of the environment's compose
/// file when given (the cloud stores nothing). The sha goes lowercase (the
/// cloud matches hex as written), and the plan must be of what was asked: its
/// `source.commit` starts with the commit, its `source.manifest` is the file.
pub async fn fetch_plan(
    ctx: &Ctx,
    sel: &Selected,
    commit: &str,
    manifest: Option<&str>,
) -> Res<(u16, Value)> {
    let asked = commit.to_lowercase();
    let mut req = Req::get(format!(
        "/teams/{}/environments/{}/plan",
        sel.team_id, sel.env.id
    ))
    .query("commit", asked.as_str());
    if let Some(m) = manifest {
        req = req.query("manifest", m);
    }
    let (status, body) = ctx.raw(&req).await?;
    if status == 200
        && let Some(got) = body.pointer("/plan/source/commit").and_then(Value::as_str)
    {
        let got = got.to_lowercase();
        if !(got.starts_with(&asked) || asked.starts_with(&got)) {
            return Err(CloudError::new(
                "CLOUD_ERROR",
                format!(
                    "The cloud planned {} instead of {}.",
                    short(&got),
                    short(&asked)
                ),
            ));
        }
    }
    // A cloud that ignores `manifest` plans the environment's file; a plan
    // that does not say which file it is is not taken as the asked one.
    if status == 200
        && let Some(want) = manifest
    {
        let got = body
            .pointer("/plan/source/manifest")
            .and_then(Value::as_str);
        if got != Some(want) {
            return Err(CloudError::new(
                "CLOUD_ERROR",
                format!(
                    "iii cloud planned {} instead of {want}.",
                    got.unwrap_or("the environment's file")
                ),
            ));
        }
    }
    Ok((status, body))
}

pub async fn check(ctx: &Arc<Ctx>, request: CheckRequest) -> Res<Check> {
    let started = Instant::now();
    // The harness drops a call at 300 s: every wait of this one ends by then,
    // counted from the start (the fetch and the plan read take from it too).
    let deadline = started + crate::run::call_wait(ctx);
    if request
        .manifest
        .as_deref()
        .is_some_and(|m| !crate::local::manifest_path(m))
    {
        return Err(CloudError::invalid(
            "manifest must be a worker-compose*.yaml of the repository, from its root, such as api/worker-compose.yaml",
        ));
    }
    let mut sel = selected_for(ctx, request.handoff_id.as_deref()).await?;
    // What this check is made for, read before anything slow: a `use` that lands meanwhile is not its source.
    let made_for = ctx.store.read(|s| s.checked_for_now(&sel.env.id));
    // Without a credential the checks on this machine still run; the plan is
    // unavailable and one sign-in problem says why.
    let signed_out = match ctx.credential().await {
        Ok(_) => None,
        Err(e)
            if matches!(
                e.code.as_str(),
                "NOT_CONNECTED" | "KEY_REJECTED" | "NEEDS_BROWSER_SIGN_IN"
            ) =>
        {
            Some(e)
        }
        Err(e) => return Err(e),
    };
    // An assisted environment's own branch that GitHub deleted (a head branch
    // removed after a merge) leaves nothing to compare with: the base is the
    // default branch, and the deploy creates the branch again.
    if !sel.env.production
        && sel.env.branch == crate::project::own_branch(&sel.env.name)
        && matches!(remote_ref(&sel.workspace, &sel.env.branch).await, Ok(None))
    {
        let stored = ctx
            .store
            .read(|s| s.project.as_ref().and_then(|p| p.default_branch.clone()));
        let default = match crate::project::live_default_branch(&sel.workspace).await {
            Ok(d) => Some(d),
            Err(_) => stored,
        };
        if let Some(d) = default {
            sel.env.branch = d;
        }
    }
    let cfg = ctx.config().await;
    let explicit = request.commit.clone();
    let commit = match explicit {
        Some(c) => {
            if !(7..=40).contains(&c.len()) || !c.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(CloudError::invalid("commit must be 7 to 40 hex characters"));
            }
            let c = c.to_lowercase();
            // The full sha when the commit is here: `ready` compares it with the
            // run's full HEAD. A commit only the cloud knows stays as given.
            git(
                &sel.workspace,
                &["rev-parse", "--verify", &format!("{c}^{{commit}}")],
            )
            .await
            .map(|full| full.trim().to_string())
            .unwrap_or(c)
        }
        None => git(&sel.workspace, &["rev-parse", "HEAD"])
            .await
            .map_err(|_| {
                CloudError::new(
                    "NO_COMMITS",
                    "The repository has no commits yet: commit first, then check.",
                )
            })?
            .trim()
            .to_string(),
    };
    if request.push.unwrap_or(false) {
        push_fix_branch(ctx, &sel, &commit).await?;
    }
    // Everything that waits on a network or a process starts together: the one
    // GitHub round trip (then the three answers from local refs), the
    // environment, the support matrix and the local engine's version.
    let git_part = async {
        let fetched = fetch_remote(&sel.workspace, &sel.env.branch, &commit).await;
        let is_pushed = request.commit.is_some() || pushed(&sel.workspace, &commit).await;
        // The gate does not fail open: a failed fetch leaves the head unknown.
        let head = if fetched {
            fetched_head(&sel.workspace, &sel.env.branch).await
        } else {
            Err(CloudError::new(
                "GIT_FAILED",
                format!("Could not fetch {} from origin.", sel.env.branch),
            ))
        };
        let ancestor = match &head {
            Ok(h) if !h.is_empty() => Some(is_ancestor(&sel.workspace, h, &commit).await),
            _ => None,
        };
        (is_pushed, head, ancestor)
    };
    let env_part = async {
        if signed_out.is_some() {
            Ok(Value::Null)
        } else {
            ctx.call(&Req::get(format!(
                "/teams/{}/environments/{}",
                sel.team_id, sel.env.id
            )))
            .await
        }
    };
    let ((is_pushed, base_head, ancestor), env_body, matrix, iii_version) = tokio::join!(
        git_part,
        env_part,
        support_matrix(ctx),
        local_iii_version(&cfg.iii_bin)
    );
    let env_body = env_body?;
    // The cloud's answer when there is one (a null manifest is none); the
    // stored record only when signed out.
    let env_manifest = if env_body.is_null() {
        sel.env.manifest.clone()
    } else {
        env_body
            .get("manifest")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    // The file: the one asked, else the newest check's of this environment
    // (its work stays on it, however the check was started); the environment's
    // own is no ask.
    let asked = request
        .manifest
        .clone()
        .or_else(|| {
            ctx.store
                .read(|s| s.work_file(&sel.env.id, env_manifest.as_deref()))
        })
        .filter(|m| env_manifest.as_deref() != Some(m.as_str()));
    // What this check is for: the file the cloud has now, not the cache.
    // Recorded whatever is selected: a hand-off's check of another environment still has its file.
    let mut made_for = made_for.unwrap_or(crate::store::CheckedFor {
        source: None,
        source_sha: None,
        workspace: false,
        compose: None,
    });
    made_for.compose = env_manifest.clone();
    let manifest_path = asked.clone().or(env_manifest);

    let mut plan_value: Option<Value> = None;
    let (plan, mut problems, plan_workers) = if !is_pushed {
        (empty_plan(PlanStatus::NotPushed, None), vec![], None)
    } else if let Some(e) = &signed_out {
        (
            empty_plan(PlanStatus::Unavailable, Some(e.clone())),
            vec![],
            None,
        )
    } else {
        match fetch_plan(ctx, &sel, &commit, asked.as_deref()).await {
            Ok((200, body)) => {
                let (mut part, mut problems) = plan_part(&body);
                let skipped = crate::values::skipped_of(ctx, &sel.env.id);
                let codes = crate::values::skipped_problems_of(ctx, &sel.env.id);
                mark_skipped(&body, &mut part, &mut problems, &skipped, &codes);
                plan_value = body.get("plan").cloned();
                let workers = body
                    .pointer("/plan/workers")
                    .and_then(Value::as_array)
                    .map(|ws| {
                        ws.iter()
                            .filter_map(|w| {
                                Some((
                                    w.get("name")?.as_str()?.to_string(),
                                    w.get("runtime").and_then(Value::as_str).map(str::to_owned),
                                ))
                            })
                            .collect::<Vec<_>>()
                    });
                (part, problems, workers)
            }
            // Only a 404 about the commit is "the cloud can't see it yet"; another
            // 404 (no such environment) is the cloud being unavailable for this.
            Ok((404, body)) if is_commit_404(&body) => {
                (empty_plan(PlanStatus::CommitNotPlanned, None), vec![], None)
            }
            Ok((status, body)) => (
                empty_plan(
                    PlanStatus::Unavailable,
                    Some(crate::cloud::cloud_error(status, &body)),
                ),
                vec![],
                None,
            ),
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "NOT_CONNECTED" | "KEY_REJECTED" | "NEEDS_BROWSER_SIGN_IN"
                ) =>
            {
                return Err(e);
            }
            Err(e) => (empty_plan(PlanStatus::Unavailable, Some(e)), vec![], None),
        }
    };

    let settings = env_body.get("settings").cloned().unwrap_or(Value::Null);
    let manifest = Manifest::load(&sel.workspace, manifest_path.as_deref());
    // The names the run here fills from the person's env files count as set
    // (`LOCAL_VARIABLE_UNSET` is what is left after the fill); their values are dropped here.
    let checkout = ctx
        .store
        .read(|s| s.project.as_ref().map(|p| PathBuf::from(&p.path)))
        .unwrap_or_else(|| sel.workspace.clone());
    // What iii cloud will resolve at deploy: the plan just fetched, so no round trip of its own.
    let cloud_set = plan_value
        .as_ref()
        .map(crate::values::cloud_set_in)
        .unwrap_or_default();
    let from_files = crate::values::here_values(
        &checkout,
        &sel.workspace,
        manifest_path.as_deref(),
        &sel.env,
        &cloud_set,
    );
    let skipped = crate::values::skipped_of(ctx, &sel.env.id);
    let filled: Vec<String> = from_files
        .iter()
        .map(|(name, _, _)| name.clone())
        // A name skipped for now runs here with an empty value.
        .chain(skipped.iter().cloned())
        .collect();
    let here_problems = local_checks(&LocalInput {
        workspace: &sel.workspace,
        manifest: manifest.as_ref(),
        filled: &filled,
        cloud_set: &cloud_set,
        settings: &settings,
        matrix: matrix.as_ref(),
        local_iii: iii_version.as_deref(),
        plan_status: plan.status,
        plan_workers: plan_workers.as_deref(),
    });
    let mut here_problems = here_problems;
    // Ready pins what gets merged: the base is in the commit's history, and
    // the worktree has nothing uncommitted (the run's own files are excluded).
    match &base_head {
        Ok(head) if !head.is_empty() => {
            if ancestor != Some(true) {
                here_problems.push(crate::local::here(
                    "BASE_NOT_IN_HISTORY",
                    format!("{} has commits this branch does not have", sel.env.branch),
                    Some(format!(
                        "Merge or rebase origin/{} into the branch, then check again.",
                        sel.env.branch
                    )),
                    Owner::Agent,
                    None,
                ));
            }
        }
        // The gate does not fail open: an unreadable base is a problem too.
        _ => here_problems.push(crate::local::here(
            "BASE_NOT_IN_HISTORY",
            format!("Could not read {} on origin", sel.env.branch),
            Some("The check pins the branch's head; origin did not answer.".into()),
            Owner::Agent,
            None,
        )),
    }
    let base_head = base_head.unwrap_or_default();
    if signed_out.is_some() {
        here_problems.push(crate::local::here(
            "SIGN_IN_REQUIRED",
            "Not signed in to iii cloud".into(),
            Some("Only the checks on this machine ran: the cloud's plan was not read.".into()),
            Owner::Person,
            None,
        ));
    }
    if request.commit.is_none()
        && let Some(count) = old_attempt_commits(ctx, &sel, &commit).await
    {
        let base = ctx.store.read(|s| {
            s.worktree_base
                .get(&sel.workspace.to_string_lossy().into_owned())
                .map(|(b, sha)| format!("{b} ({sha})"))
        });
        here_problems.push(crate::local::here(
            "OLD_ATTEMPT_COMMITS",
            "These commits come from an earlier try".into(),
            Some(format!(
                "{count} commit(s) on this branch were made before this worktree started and are not on the base. Start from the base: `git reset --hard {}` and redo the fixes.",
                base.as_deref().unwrap_or(&format!("origin/{}", sel.env.branch))
            )),
            Owner::Agent,
            None,
        ));
    }
    // A reused or resumed fix branch may hold what an older worker brought from the
    // person's folder: a commit of copied uncommitted files. A deploy would push it.
    if !base_head.is_empty()
        && let Ok(subjects) = git(
            &sel.workspace,
            &["log", "--format=%H %s", &format!("{base_head}..{commit}")],
        )
        .await
        && let Some((sha, subject)) = subjects
            .lines()
            .filter_map(|l| l.split_once(' '))
            .find(|(_, s)| s.starts_with("Uncommitted changes from "))
    {
        here_problems.push(crate::local::here(
            "BROUGHT_FILES_COMMIT",
            "A commit holds files copied from your folder".into(),
            Some(format!(
                "\"{subject}\" was made by an older version of this worker: it holds copies of uncommitted files, and a deploy would send them. Drop only that commit and keep the rest: `git rebase --onto {sha}^ {sha}`, then check again."
            )),
            Owner::Agent,
            None,
        ));
    }
    // As the run reads it: unreadable counts as dirty. What a local run left in the
    // worktree (rewritten seed files, data) is not the agent's: it counts again only
    // once its content is not what the run left.
    let split = crate::run::dirty_split(ctx, &sel.workspace).await;
    let own_changes = split.as_ref().map(|(agent, _)| agent.clone());
    let set_aside = split.map(|(_, aside)| aside).unwrap_or_default();
    let dirty = own_changes.as_ref().is_none_or(|p| !p.is_empty());
    let interrupted = ctx.store.read(|s| {
        s.checks
            .iter()
            .find(|c| c.environment_id == sel.env.id && c.run.is_some())
            .and_then(|c| c.run.as_ref())
            .and_then(|r| r.error.as_ref())
            .is_some_and(|e| e.code == "WORKER_RESTARTED")
    });
    let folder = ctx.store.read(|s| {
        s.project
            .as_ref()
            .map(|p| std::path::PathBuf::from(&p.path))
    });
    let copies = match &folder {
        Some(f) if request.commit.is_none() && dirty && *f != sel.workspace => {
            copies_of_the_folders_changes(&sel.workspace, f).await
        }
        _ => None,
    };
    if let Some(files) = copies {
        // The same files, with the same content, are uncommitted in the person's
        // folder: an older worker copied them here. Committing them would send them.
        let shown = files.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
        here_problems.push(crate::local::here(
            "BROUGHT_FILES",
            "The worktree holds copies of files from your folder".into(),
            Some(format!(
                "{shown}{} are uncommitted in your folder too, with the same content: an older version of this worker copied them here. Do not commit them: remove them from the copy (`git checkout -- <file>` for a tracked one, delete an untracked one), then check again.",
                if files.len() > 5 { " and more" } else { "" }
            )),
            Owner::Agent,
            None,
        ));
    } else if request.commit.is_none() && dirty {
        here_problems.push(crate::local::here(
            "UNCOMMITTED_CHANGES",
            "The worktree has changes that are not committed".into(),
            Some(format!(
                "{}{}",
                match &own_changes {
                    Some(p) => format!(
                        "{}{}. ",
                        p.iter().take(5).cloned().collect::<Vec<_>>().join(", "),
                        if p.len() > 5 { " and more" } else { "" }
                    ),
                    None => String::new(),
                },
                // A run cut off by a restart never set its files aside: some of these may be its.
                if interrupted {
                    "The local run was interrupted by a restart, so some of these may be what it wrote: restore those (`git checkout -- <file>`, or delete an untracked one) and commit yours (`git add <paths>`), then check and run again."
                } else {
                    "Commit your own changes (`git add <paths>`; the files the local run wrote are not yours: do not commit them), then check and run again."
                }
            )),
            Owner::Agent,
            None,
        ));
    }
    // What a local run left is not counted; one of those could still be the agent's own edit.
    if !set_aside.is_empty() {
        let mut info = crate::local::here(
            "RUN_LEFT_FILES",
            "The local run left files changed in the worktree".into(),
            Some(format!(
                "The local run left {} {} changed in the agent's copy; they do not count as uncommitted: {}{}. If one of them is your own edit, commit it.",
                set_aside.len(),
                if set_aside.len() == 1 {
                    "file"
                } else {
                    "files"
                },
                set_aside
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                if set_aside.len() > 5 { " and more" } else { "" }
            )),
            Owner::None,
            None,
        );
        info.severity = Severity::Info;
        here_problems.push(info);
    }
    let here = HerePart {
        problems: here_problems.len() as u32,
        accepts_cloud_block: accepts_cloud_block(iii_version.as_deref()),
        iii_version: iii_version.clone(),
    };
    problems.extend(here_problems);
    let base = base_head;
    let commits = commits_between(&sel.workspace, &base, &commit).await;
    let mut check = Check {
        id: new_id(),
        environment_id: sel.env.id.clone(),
        commit: commit.clone(),
        base,
        checked_at: now(),
        duration_ms: started.elapsed().as_millis() as u64,
        report: report(&commit, &plan, None),
        ready: false,
        plan,
        here,
        problems,
        run: None,
        manifest: manifest_path,
        // The full id, even when the agent gave the marker's 8 characters.
        handoff_id: request.handoff_id.as_deref().map(|id| {
            ctx.store.read(|s| {
                s.handoffs
                    .iter()
                    .find(|h| h.is(id))
                    .map_or_else(|| id.to_string(), |h| h.id.clone())
            })
        }),
        commits,
    };
    ctx.store.update(|s| {
        // The latest of the environment only when it is the selected one.
        let latest = s.environment_id() == Some(check.environment_id.as_str());
        s.put_check_for(check.clone(), latest);
        s.check_sources.insert(check.id.clone(), made_for);
    });
    if let Some(plan) = &plan_value {
        crate::deploy::save_plan(ctx, &check.id, plan);
    }
    prune_plans(ctx);
    // What the compose file needs, including what the plan flags with empty defaults.
    let rows = ctx.store.read(|s| s.variables.clone());
    let merged = crate::status::merged_variables(ctx, &sel, rows);
    crate::status::put_variables(ctx, &sel, merged);
    emit_check(ctx, &check);
    if request.run.unwrap_or(false) {
        // The run reads the saved plan's runtimes and this check's problems as it
        // starts, so it starts here, once both are stored, and not before.
        let key = run_key(
            &ctx.config().await.postgres_image,
            &sel.workspace,
            manifest.as_ref(),
            iii_version.as_deref(),
            &sel.env.id,
            &settings,
            &from_files,
            &cloud_set,
            &skipped,
        )
        .await;
        let head = git(&sel.workspace, &["rev-parse", "HEAD"])
            .await
            .map(|h| h.trim().to_string())
            .unwrap_or_default();
        let blocked = check
            .problems
            .iter()
            .any(|p| p.code == "LOCAL_ENGINE_VERSION");
        let reusable = match &key {
            Some(key) if !request.force.unwrap_or(false) && !dirty && !blocked => {
                passed_run(ctx, key, &head)
            }
            _ => None,
        };
        if let Some(old) = reusable {
            let id = check.id.clone();
            update_check(ctx, &id, |c| {
                c.run = Some(Run {
                    check_id: id.clone(),
                    reused: true,
                    ..old
                })
            });
        } else {
            // One deadline for the whole call (attach wait and the loop below), under
            // the harness's 300 s dispatch timeout. A dirty workspace never attaches.
            let run = crate::run::start_with(
                ctx,
                check.id.clone(),
                sel.clone(),
                key.clone().filter(|_| !dirty),
                deadline,
            )
            .await?;
            if let Some(key) = key {
                ctx.store.update(|s| {
                    s.run_keys.insert(run.id.clone(), key);
                    let held: Vec<String> = s
                        .checks
                        .iter()
                        .filter_map(|c| c.run.as_ref().map(|r| r.id.clone()))
                        .collect();
                    s.run_keys.retain(|id, _| held.contains(id));
                });
            }
        }
        // Back as soon as the run ended or says it waits on a worker (still running).
        loop {
            let current = ctx.store.read(|s| s.find_check(&check.id).cloned());
            if let Some(c) = current {
                check = c;
                if check
                    .run
                    .as_ref()
                    .is_none_or(|r| r.status != RunStatus::Running || r.waiting_on.is_some())
                    || Instant::now() >= deadline
                {
                    break;
                }
            }
            tokio::time::sleep(ctx.secs(1).min(Duration::from_millis(500))).await;
        }
    }
    // A check the person started on a hand-off's worktree: an agent that waits on a wake hears of it.
    if request.from_page && request.handoff_id.is_none() {
        crate::handoff::person_check_ended(ctx, &check, &sel.workspace);
    }
    Ok(check)
}

/// A hash of what a local run of the workspace would be made from: the compose
/// file's path and content, the engine, the environment's settings and the
/// names and values the run reads (the worktree `.env`, the env files it fills
/// from, the names skipped). Values are only hashed, never kept. None without a
/// readable compose file.
///
/// It also holds the local Postgres image. It does not cover the versions of
/// the tools the run calls (node, pnpm, docker), the files git ignores, or the
/// network (a package that changed upstream): a run made from the same key can
/// differ. `force: true` on the request runs again.
#[allow(clippy::too_many_arguments)] // each argument is one part of the key
pub async fn run_key(
    postgres_image: &str,
    ws: &std::path::Path,
    manifest: Option<&Manifest>,
    engine: Option<&str>,
    env_id: &str,
    settings: &Value,
    from_files: &[(String, String, String)],
    cloud_set: &[String],
    skipped: &std::collections::BTreeSet<String>,
) -> Option<String> {
    let m = manifest?;
    let head = git(ws, &["rev-parse", "HEAD"]).await.ok()?;
    let mut vars = crate::local::worktree_env(ws, m);
    vars.extend(from_files.iter().map(|(n, v, _)| (n.clone(), v.clone())));
    for name in skipped {
        if !vars.iter().any(|(n, _)| n == name) {
            vars.push((name.clone(), String::new()));
        }
    }
    vars.sort();
    // Which `${NAME:-}` names the cloud now fills: saving one there changes the key, a failed run is not served again.
    let applying = crate::values::applying_empty_defaults(m, cloud_set);
    let settings = canonical(settings);
    let mut hash = Sha256::new();
    let mut part = |bytes: &[u8]| {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    };
    for text in [
        ws.to_string_lossy().as_ref(),
        head.trim(),
        &m.file.to_string_lossy(),
        &m.text,
        engine.unwrap_or("unknown"),
        env_id,
        &settings,
        postgres_image,
    ] {
        part(text.as_bytes());
    }
    for (name, value) in &vars {
        part(name.as_bytes());
        part(value.as_bytes());
    }
    part(b"cloud-set");
    for name in &applying {
        part(name.as_bytes());
    }
    Some(hex::encode(hash.finalize()))
}

/// The passed run made from `key` on a clean `head`, from a check the store still keeps.
fn passed_run(ctx: &Ctx, key: &str, head: &str) -> Option<Run> {
    ctx.store.read(|s| {
        s.checks
            .iter()
            .filter_map(|c| c.run.as_ref())
            .find(|r| {
                r.status == RunStatus::Passed
                    && r.commit == head
                    && s.run_keys.get(&r.id).is_some_and(|k| k == key)
            })
            .cloned()
    })
}

pub fn emit_check(ctx: &Ctx, check: &Check) {
    let summary = if check.ready {
        format!("check {} is ready", short(&check.commit))
    } else {
        format!(
            "check {} is not ready: {} problems",
            short(&check.commit),
            check.problems.len()
        )
    };
    ctx.emit(EventKind::Check, &check.id, true, summary, check);
}

async fn support_matrix(ctx: &Ctx) -> Option<Value> {
    let cfg = ctx.config().await;
    let response = ctx
        .http
        .get(format!("{}/support-matrix", cfg.api_base()))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

/// Change a stored check (the run finished, say) and keep `ready` and the
/// report in step with it.
pub fn update_check(ctx: &Ctx, id: &str, f: impl FnOnce(&mut Check)) -> Option<Check> {
    let updated = ctx.store.update(|s| {
        let mut found = s.find_check(id).cloned()?;
        f(&mut found);
        found.ready = is_ready(
            &found.plan,
            &found.problems,
            found.run.as_ref(),
            &found.commit,
        );
        found.report = report(&found.commit, &found.plan, found.run.as_ref());
        // Only the latest check of the environment is `check`; an older one stays in the history.
        let is_latest = s.check.as_ref().is_some_and(|c| c.id == found.id);
        if let Some(slot) = s.checks.iter_mut().find(|c| c.id == found.id) {
            *slot = found.clone();
        }
        if is_latest {
            s.check = Some(found.clone());
        }
        Some(found)
    })?;
    emit_check(ctx, &updated);
    Some(updated)
}
