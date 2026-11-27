//! `deploy`, `watch`, the secret page call and the redaction, against wiremock,
//! real temp repositories and a fake `gh` on PATH.

mod support;

use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration;

use iii_cloud::check::plan_part;
use iii_cloud::deploy::deploy;
use iii_cloud::project::use_project;
use iii_cloud::secret::{ask, provide};
use iii_cloud::types::*;
use iii_cloud::watch::{poll_once, watch};
use serde_json::{Value, json};
use support::Harness;
use support::repo::*;
use wiremock::matchers::{header, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

static PATH_ONCE: Once = Once::new();

fn fake_gh_on_path() {
    PATH_ONCE.call_once(|| {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/bin");
        let path = format!("{dir}:{}", std::env::var("PATH").unwrap());
        // SAFETY: set once, before any test thread reads PATH concurrently with a write.
        unsafe { std::env::set_var("PATH", path) };
    });
}

const DEP_ID: &str = "33333333-3333-4333-8333-333333333333";
const NEW_ID: &str = "44444444-4444-4444-8444-444444444444";

fn plan(resources: bool) -> Value {
    json!({
        "environment": "production",
        "source": {"branch": "main", "manifest": "worker-compose.yaml", "manifest_sha256": "m", "lock_sha256": "l"},
        "settings": {"release": null},
        "workers": [], "groups": [], "variables": [], "issues": [], "deploy_issues": [],
        "resources": if resources { json!([{"kind": "database", "alias": "primary", "mode": "managed", "action": "create", "via": "database", "binding": "DATABASE_PRIMARY_URL", "postgres": {"tier": "shared"}}]) } else { json!([]) },
    })
}

/// The deployments list answers with the push deployment of whatever `main` is now.
struct PushList {
    origin: PathBuf,
    status: &'static str,
}

impl Respond for PushList {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let sha = git(&self.origin, &["rev-parse", "main"]);
        ResponseTemplate::new(200).set_body_json(json!({"deployments": [deployment(DEP_ID, &sha, self.status, "push")], "next_cursor": null}))
    }
}

fn deployment(id: &str, sha: &str, status: &str, kind: &str) -> Value {
    json!({"id": id, "number": 7, "environment_id": ENV, "type": kind, "status": status, "status_detail": null, "current": false,
        "commit": {"sha": sha, "branch": "main", "message": "m"}, "stages": [{"name": "workers", "status": "running", "detail": null, "started_at": null, "finished_at": null}],
        "error": null, "created_at": "2026-11-25T10:00:00Z", "started_at": null, "finished_at": null})
}

struct Setup {
    h: Harness,
    server: MockServer,
    repo: Repo,
    ws: PathBuf,
    check: Check,
    fakegh: PathBuf,
}

fn argv(s: &Setup) -> String {
    std::fs::read_to_string(s.fakegh.join("argv.log")).unwrap_or_default()
}

fn passed_run(commit: &str) -> Run {
    Run {
        id: "r".into(),
        check_id: "chk".into(),
        commit: commit.into(),
        engine: "0.27.0".into(),
        status: RunStatus::Passed,
        started_at: "t".into(),
        finished_at: None,
        stages: vec![],
        workers: WorkerCount { ready: 1, total: 1 },
        routes: RouteCount {
            count: 1,
            prefix: None,
        },
        healthcheck: None,
        not_covered: vec![],
        log_tail: vec![],
        error: None,
        reused: false,
        waiting_on: None,
    }
}

/// A connected context with a worktree, a ready check of the worktree's head, and a PR on it.
async fn setup(plan_value: Value) -> Setup {
    fake_gh_on_path();
    let server = MockServer::start().await;
    let repo = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    let p = use_project(
        &h.ctx,
        UseRequest {
            path: repo.checkout.to_string_lossy().into(),
            environment_id: Some(ENV.into()),
            mode: WorkspaceMode::Worktree,
            cloud_project_id: None,
            from: None,
            source: None,
        },
    )
    .await
    .unwrap();
    let ws = PathBuf::from(p.workspace.unwrap().path);
    let base = git(&repo.origin, &["rev-parse", "main"]);
    std::fs::write(ws.join("feature.txt"), "f\n").unwrap();
    // The names the tests ask for: `secret` only asks for what the compose file reads.
    let reads = [
        "DATABASE_URL",
        "ONLY_CLOUD",
        "SESSION_SECRET",
        "TOKEN",
        "TWO",
    ];
    let env: String = reads
        .iter()
        .map(|n| {
            // A default: the run here does not need it, so it never lands in a `.env`.
            let dflt = if *n == "ONLY_CLOUD" { ":-" } else { "" };
            format!("      {n}: ${{{n}{dflt}}}\n")
        })
        .collect();
    std::fs::write(
        ws.join("worker-compose.yaml"),
        format!("containers:\n  api:\n    worker: path://./workers/api\n    environment:\n{env}"),
    )
    .unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-m", "feature"]);
    let head = git(&ws, &["rev-parse", "HEAD"]);
    let (part, _) =
        plan_part(&json!({"can_deploy": true, "fingerprint": "fp-1", "plan": plan_value}));
    let check = Check {
        id: "chk".into(),
        environment_id: ENV.into(),
        commit: head.clone(),
        base: base.clone(),
        checked_at: "t".into(),
        duration_ms: 1,
        plan: part,
        here: HerePart {
            problems: 0,
            iii_version: Some("0.27.0".into()),
            accepts_cloud_block: true,
        },
        problems: vec![],
        run: Some(passed_run(&head)),
        ready: true,
        report: String::new(),
        manifest: None,
        handoff_id: None,
        commits: vec![],
    };
    h.ctx.store.update(|s| s.put_check(check.clone()));
    iii_cloud::deploy::save_plan(&h.ctx, "chk", &plan_value);
    let fakegh = repo.root.path().join("fakegh");
    std::fs::create_dir_all(&fakegh).unwrap();
    std::fs::write(fakegh.join("view.json"), json!({"number": 12, "url": "u", "title": "T", "state": "OPEN", "body": "", "headRefName": "iii-cloud/ready-production", "headRefOid": head, "baseRefName": "main", "mergeable": "MERGEABLE", "statusCheckRollup": []}).to_string()).unwrap();
    // The merge: a squash commit on origin's main whose parent is the base.
    std::fs::write(
        fakegh.join("merge.sh"),
        format!(
            "T=$(mktemp -d)\ngit clone -q '{o}' $T/c && cd $T/c && git fetch -q '{ws}' {head} && git checkout -q FETCH_HEAD -- . && git add . && git -c user.name=t -c user.email=t@e.t commit -qm squash && git push -q origin main\nOID=$(git rev-parse HEAD)\nprintf '{{\"number\":12,\"state\":\"MERGED\",\"headRefOid\":\"{head}\",\"baseRefName\":\"main\",\"mergeCommit\":{{\"oid\":\"%s\"}}}}' \"$OID\" > '{d}/view.json'\n",
            o = repo.origin.display(), ws = ws.display(), head = head, d = fakegh.display()
        ),
    ).unwrap();
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"can_deploy": true, "fingerprint": "fp-target", "plan": plan_value}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/logs$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"lines": [{"message": "newest"}, {"message": "oldest"}], "next_cursor": null}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/health$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "healthy", "engine": "ready", "workers": []})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/events$"))
        .respond_with(sse(&[]))
        .mount(&server)
        .await;
    Setup {
        h,
        server,
        repo,
        ws,
        check,
        fakegh,
    }
}

fn sse(messages: &[&str]) -> ResponseTemplate {
    let mut body = String::new();
    for m in messages {
        body.push_str(&format!(
            "data: {}\n\n",
            json!({"type": "log", "at": "t", "log": {"message": m}})
        ));
    }
    body.push_str("data: {\"type\":\"ping\",\"at\":\"t\"}\n\n");
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn dep_base() -> String {
    format!("/api/v1/teams/{TEAM}/environments/{ENV}/deployments")
}

async fn terminal_events(h: &Harness, id: &str) -> usize {
    tokio::time::sleep(Duration::from_millis(300)).await;
    h.fake
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.kind == EventKind::Deploy && e.id == id && e.terminal)
        .count()
}

#[tokio::test]
async fn merge_then_confirm_creates_one_pinned_deployment_and_follows_it_to_ready() {
    let s = setup(plan(true)).await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "needs_action",
        })
        .mount(&s.server)
        .await;
    // The real create: matched on the body flags; the key carries the target, read back below.
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .and(header("if-match", "fp-target"))
        .and(wiremock::matchers::body_partial_json(
            json!({"confirm_resources": true, "confirm_overrides": false}),
        ))
        .respond_with(|req: &Request| {
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            ResponseTemplate::new(201).set_body_json(deployment(
                NEW_ID,
                body["commit"].as_str().unwrap(),
                "in_progress",
                "manual",
            ))
        })
        .with_priority(1)
        .expect(1)
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{NEW_ID}", dep_base())))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(deployment(NEW_ID, "x", "ready", "manual"))
        })
        .mount(&s.server)
        .await;

    let d = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(d.id, NEW_ID);
    assert_eq!(
        (d.pr, d.approved_by_worker, d.checked_ready),
        (Some(12), true, true)
    );
    assert_eq!(d.check_id.as_deref(), Some("chk"));
    let log = argv(&s);
    assert!(
        log.contains(&format!(
            "pr merge 12 --repo ada/hello-iii --squash --match-head-commit {}",
            s.check.commit
        )),
        "{log}"
    );
    assert!(!log.contains("--admin") && !log.contains("--auto"));
    let posts: Vec<_> = s
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == dep_base())
        .collect();
    assert_eq!(posts.len(), 1);
    let target = git(&s.repo.origin, &["rev-parse", "main"]);
    assert_eq!(
        posts[0]
            .headers
            .get("idempotency-key")
            .unwrap()
            .to_str()
            .unwrap(),
        format!("iii-cloud:chk:{target}:0")
    );
    // The merge entry is gone once the deployment exists.
    assert!(s.h.ctx.store.read(|st| st.merges.is_empty()));
    assert_eq!(
        terminal_events(&s.h, NEW_ID).await,
        1,
        "one wake-up when it ends"
    );
    let stored =
        s.h.ctx
            .store
            .read(|st| st.deploys.iter().find(|x| x.id == NEW_ID).cloned().unwrap());
    assert_eq!(stored.outcome, Outcome::Ready);
    assert_eq!(
        stored.check_id.as_deref(),
        Some("chk"),
        "the follower keeps the check"
    );
    assert_eq!(
        stored.log_tail,
        vec!["oldest", "newest"],
        "newest first from the cloud, oldest first here"
    );
}

#[tokio::test]
async fn a_plan_that_needs_nothing_adopts_the_push_deployment_and_never_creates() {
    let s = setup(plan(false)).await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "queued",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(deployment(DEP_ID, "x", "ready", "push"))
        })
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&s.server)
        .await;
    let d = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(d.id, DEP_ID);
    assert!(!d.approved_by_worker, "it needed no confirmation");
    assert_eq!(terminal_events(&s.h, DEP_ID).await, 1);
}

#[tokio::test]
async fn a_changed_plan_after_the_merge_cancels_the_push_deployment_and_deploys_nothing() {
    let s = setup(plan(false)).await;
    // The plan of the merged commit differs from the one the check saw.
    s.server.reset().await;
    let mut changed = plan(false);
    changed["settings"] =
        json!({"release": {"worker": "api", "command": "x", "timeout": "1m", "source": "file"}});
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"can_deploy": true, "fingerprint": "fp2", "plan": changed})),
        )
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "queued",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{}/{DEP_ID}/cancel", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&s.server)
        .await;
    let err = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "PLAN_CHANGED");
    assert!(err.message.contains("settings"), "{}", err.message);
    // The merge is remembered: the next deploy resumes from the pin.
    assert_eq!(s.h.ctx.store.read(|st| st.merges.len()), 1);
}

/// Past Routes and TLS the cloud refuses the cancel (409): the merged commit is
/// deploying, and the caller and status say so instead of "check again".
#[tokio::test]
async fn a_refused_cancel_says_the_push_deployment_is_still_running() {
    let s = setup(plan(false)).await;
    s.server.reset().await;
    let mut changed = plan(false);
    changed["settings"] =
        json!({"release": {"worker": "api", "command": "x", "timeout": "1m", "source": "file"}});
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"can_deploy": true, "fingerprint": "fp2", "plan": changed})),
        )
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "in_progress",
        })
        .mount(&s.server)
        .await;
    let origin = s.repo.origin.clone();
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(move |_: &Request| {
            let sha = git(&origin, &["rev-parse", "main"]);
            ResponseTemplate::new(200).set_body_json(deployment(
                DEP_ID,
                &sha,
                "in_progress",
                "push",
            ))
        })
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{}/{DEP_ID}/cancel", dep_base())))
        .respond_with(
            ResponseTemplate::new(409).set_body_json(
                json!({"code": "CONFLICT", "message": "can't be canceled any more"}),
            ),
        )
        .expect(1)
        .mount(&s.server)
        .await;
    let err = deploy(&s.h.ctx, req("chk", Some(12), None))
        .await
        .unwrap_err();
    let merged = git(&s.repo.origin, &["rev-parse", "main"]);
    assert_eq!(err.code, "PUSH_STILL_RUNNING");
    assert!(
        err.message.contains(&merged[..7])
            && err
                .message
                .contains("was not cancelled (it is in progress)"),
        "{}",
        err.message
    );
    assert!(!err.message.contains("Check again"), "{}", err.message);
    let running = s.h.ctx.store.read(|st| {
        st.deploys
            .iter()
            .find(|d| d.id == DEP_ID)
            .map(|d| (d.outcome == Outcome::Running, d.commit.sha.clone()))
    });
    assert_eq!(running, Some((true, merged)));
}

/// A 404 to the cancel is a push that ended meanwhile, not one still running:
/// the list is read again and the caller gets the plain answer.
#[tokio::test]
async fn a_cancel_answered_404_reads_the_list_again_instead_of_saying_still_running() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let s = setup(plan(false)).await;
    s.server.reset().await;
    let mut changed = plan(false);
    changed["settings"] =
        json!({"release": {"worker": "api", "command": "x", "timeout": "1m", "source": "file"}});
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"can_deploy": true, "fingerprint": "fp2", "plan": changed})),
        )
        .mount(&s.server)
        .await;
    let ended = Arc::new(AtomicBool::new(false));
    let (origin, seen) = (s.repo.origin.clone(), ended.clone());
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(move |_: &Request| {
            let sha = git(&origin, &["rev-parse", "main"]);
            let status = if seen.load(Ordering::SeqCst) { "canceled" } else { "in_progress" };
            ResponseTemplate::new(200).set_body_json(
                json!({"deployments": [deployment(DEP_ID, &sha, status, "push")], "next_cursor": null}),
            )
        })
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{}/{DEP_ID}/cancel", dep_base())))
        .respond_with(move |_: &Request| {
            ended.store(true, Ordering::SeqCst);
            ResponseTemplate::new(404)
                .set_body_json(json!({"code": "NOT_FOUND", "message": "No such deployment."}))
        })
        .expect(1)
        .mount(&s.server)
        .await;
    let err = deploy(&s.h.ctx, req("chk", Some(12), None))
        .await
        .unwrap_err();
    assert_eq!(err.code, "PLAN_CHANGED", "{}", err.message);
}

#[tokio::test]
async fn a_second_deploy_after_the_merge_resumes_without_merging_again() {
    let s = setup(plan(false)).await;
    // First call: the merge happens, then the cloud's list fails.
    let failing = Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_json(json!({"code": "UNAVAILABLE", "message": "down"})),
        )
        .mount_as_scoped(&s.server)
        .await;
    let err = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "CLOUD_ERROR");
    drop(failing);
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "ready",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(deployment(DEP_ID, "x", "ready", "push"))
        })
        .mount(&s.server)
        .await;
    assert_eq!(
        s.h.ctx.store.read(|st| st.merges.len()),
        1,
        "saved right after the merge"
    );
    let merges_before = argv(&s).matches("pr merge").count();
    let d = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(d.id, DEP_ID);
    assert_eq!(
        argv(&s).matches("pr merge").count(),
        merges_before,
        "the merged pull request is not merged twice"
    );
}

#[tokio::test]
async fn a_waiting_deployment_is_confirmed_when_the_merge_was_made_on_github() {
    let s = setup(plan(true)).await;
    // The merge happened elsewhere: a squash commit on main.
    std::process::Command::new("sh")
        .arg(s.fakegh.join("merge.sh"))
        .status()
        .unwrap();
    let target = git(&s.repo.origin, &["rev-parse", "main"]);
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(deployment(
            DEP_ID,
            &target,
            "needs_action",
            "push",
        )))
        .up_to_n_times(1)
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "needs_action",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(deployment(NEW_ID, &target, "ready", "manual")),
        )
        .expect(1)
        .mount(&s.server)
        .await;
    let d = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: None,
            deployment_id: Some(DEP_ID.into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(d.id, NEW_ID);
    assert!(d.approved_by_worker && d.pr.is_none());
    assert!(!argv(&s).contains("pr merge"));
}

#[tokio::test]
async fn deploy_refuses_what_moved_or_is_not_ready() {
    let s = setup(plan(false)).await;
    // Not ready.
    s.h.ctx
        .store
        .update(|st| st.check.as_mut().unwrap().ready = false);
    let err = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "NOT_READY");
    s.h.ctx
        .store
        .update(|st| st.check.as_mut().unwrap().ready = true);
    let err = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "nope".into(),
            pr: None,
            deployment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "NOT_READY");
    // The pull request's head is not the checked commit.
    let view = std::fs::read_to_string(s.fakegh.join("view.json")).unwrap();
    std::fs::write(
        s.fakegh.join("view.json"),
        view.replace(&s.check.commit, "ffffffffffffffffffffffffffffffffffffffff"),
    )
    .unwrap();
    let err = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "PR_HEAD_CHANGED");
    std::fs::write(s.fakegh.join("view.json"), view).unwrap();
    // main moved since the check.
    let t = tempfile::tempdir().unwrap();
    git(
        t.path(),
        &["clone", "-q", s.repo.origin.to_str().unwrap(), "c"],
    );
    let c = t.path().join("c");
    std::fs::write(c.join("x"), "x").unwrap();
    git(&c, &["add", "."]);
    git(&c, &["commit", "-m", "moved"]);
    git(&c, &["push", "-q", "origin", "main"]);
    let err = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "MAIN_MOVED");
    assert!(!argv(&s).contains("pr merge"), "nothing was merged");
}

#[tokio::test]
async fn a_merge_that_gh_refuses_is_pr_not_mergeable() {
    let s = setup(plan(false)).await;
    std::fs::write(s.fakegh.join("merge.sh"), "echo 'Pull request is not mergeable: the base branch policy prohibits the merge.' >&2\nexit 1\n").unwrap();
    let err = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "PR_NOT_MERGEABLE");
    assert!(err.message.contains("base branch policy"));
}

#[tokio::test]
async fn a_forbidden_create_and_a_conflict_have_their_codes() {
    let s = setup(plan(true)).await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "needs_action",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("POST")).and(path(dep_base())).respond_with(ResponseTemplate::new(403).set_body_json(json!({"code": "FORBIDDEN", "message": "Approving resources in production takes Owner or Admin."}))).up_to_n_times(1).mount(&s.server).await;
    let err = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "FORBIDDEN");
    assert!(err.message.contains("Owner or Admin"));
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .respond_with(ResponseTemplate::new(409).set_body_json(
            json!({"code": "CONFLICT", "message": "The plan changed since it was reviewed."}),
        ))
        .up_to_n_times(1)
        .mount(&s.server)
        .await;
    assert_eq!(
        deploy(
            &s.h.ctx,
            DeployRequest {
                handoff_id: None,
                check_id: "chk".into(),
                pr: Some(12),
                deployment_id: None
            }
        )
        .await
        .unwrap_err()
        .code,
        "PLAN_CHANGED"
    );
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .respond_with(
            ResponseTemplate::new(422)
                .set_body_json(json!({"code": "IDEMPOTENCY_KEY_REUSED", "message": "reused"})),
        )
        .up_to_n_times(1)
        .mount(&s.server)
        .await;
    assert_eq!(
        deploy(
            &s.h.ctx,
            DeployRequest {
                handoff_id: None,
                check_id: "chk".into(),
                pr: Some(12),
                deployment_id: None
            }
        )
        .await
        .unwrap_err()
        .code,
        "IDEMPOTENCY_KEY_REUSED"
    );
}

#[tokio::test]
async fn a_retry_after_a_failed_deployment_is_a_new_idempotency_key() {
    let s = setup(plan(true)).await;
    // The list moves on with each create: the push deployment that was waiting is
    // replaced, and a fresh one waits for the next attempt.
    let creates = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (origin, seen) = (s.repo.origin.clone(), creates.clone());
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(move |_: &Request| {
            let sha = git(&origin, &["rev-parse", "main"]);
            let n = seen.load(std::sync::atomic::Ordering::SeqCst);
            let mut waiting = deployment(
                &format!("66666666-6666-4666-8666-66666666666{n}"),
                &sha,
                "needs_action",
                "push",
            );
            waiting["created_at"] = json!("2999-01-01T00:00:00Z");
            let mut old = deployment(
                "77777777-7777-4777-8777-777777777777",
                &sha,
                "replaced",
                "push",
            );
            old["created_at"] = json!("2999-01-01T00:00:00Z");
            ResponseTemplate::new(200)
                .set_body_json(json!({"deployments": [waiting, old], "next_cursor": null}))
        })
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .respond_with(move |req: &Request| {
            creates.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let body: Value = serde_json::from_slice(&req.body).unwrap();
            ResponseTemplate::new(201).set_body_json(deployment(
                NEW_ID,
                body["commit"].as_str().unwrap(),
                "failed",
                "manual",
            ))
        })
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{NEW_ID}", dep_base())))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(deployment(NEW_ID, "x", "failed", "manual"))
        })
        .mount(&s.server)
        .await;
    let first = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(first.status, "failed");
    deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap();
    let keys: Vec<String> = s
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == dep_base())
        .map(|r| {
            r.headers
                .get("idempotency-key")
                .unwrap()
                .to_str()
                .unwrap()
                .rsplit(':')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(keys, vec!["0", "1"]);
}

// ---------------------------------------------------------------- watch

#[tokio::test]
async fn a_ready_deploy_that_serves_no_route_is_a_failure_with_the_cloud_event_as_hint() {
    let s = setup(plan(false)).await;
    s.server.reset().await;
    let ready = deployment(DEP_ID, "abc1234", "ready", "push");
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"deployments": [ready.clone()]})),
        )
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(ready))
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/logs$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"lines": [], "next_cursor": null})),
        )
        .mount(&s.server)
        .await;
    Mock::given(method("GET")).and(path_regex(r"/health$")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "degraded", "engine": "ready", "workers": [{"group": "web", "worker": "api", "ready": true, "restarts": 0, "crashloop": false}]}))).mount(&s.server).await;
    Mock::given(method("GET")).and(path(format!("{}/{DEP_ID}/events", dep_base()))).respond_with(sse(&["Build ok", "The group web runs http, but no function of this release serves a route. Move to 0.26."])).mount(&s.server).await;
    let d = watch(
        &s.h.ctx,
        WatchRequest {
            handoff_id: None,
            deployment_id: Some(DEP_ID.into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(d.status, "ready");
    assert_eq!(d.outcome, Outcome::NoRoute);
    let e = d.error.unwrap();
    assert_eq!(e.code, "NO_ROUTE");
    assert!(e.hint.contains("serves a route"));
    assert!(
        d.log_tail.iter().any(|l| l.starts_with("health: degraded")),
        "{:?}",
        d.log_tail
    );
    assert_eq!(terminal_events(&s.h, DEP_ID).await, 1);
}

#[tokio::test]
async fn a_failed_deploy_brings_the_stage_the_code_and_the_last_lines() {
    let s = setup(plan(false)).await;
    s.server.reset().await;
    let mut failed = deployment(DEP_ID, "abc1234", "failed", "push");
    failed["error"] = json!({"stage": "workers", "worker": "api", "code": "WORKERS_NOT_READY", "hint": "OOMKilled", "action": null});
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(failed))
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/logs$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"lines": [{"message": "Cannot find package 'express'"}], "next_cursor": null}),
        ))
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/health$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "down", "engine": "ready", "workers": []})),
        )
        .mount(&s.server)
        .await;
    let d = watch(
        &s.h.ctx,
        WatchRequest {
            handoff_id: None,
            deployment_id: Some(DEP_ID.into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(d.outcome, Outcome::Failed);
    let e = d.error.unwrap();
    assert_eq!(
        (e.stage.as_str(), e.worker.as_deref(), e.code.as_str()),
        ("workers", Some("api"), "WORKERS_NOT_READY")
    );
    assert_eq!(d.log_tail[0], "Cannot find package 'express'");
    assert!(
        d.console_url.ends_with("/deployments/7")
            || d.console_url == s.h.ctx.config().await.cloud_url,
        "{}",
        d.console_url
    );
}

#[tokio::test]
async fn a_replaced_deployment_is_not_a_terminal_wake_up_and_needs_action_is() {
    let s = setup(plan(false)).await;
    s.server.reset().await;
    let waiting = deployment(DEP_ID, "abc1234", "needs_action", "push");
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(waiting))
        .up_to_n_times(1)
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(deployment(DEP_ID, "abc1234", "replaced", "push")),
        )
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/(logs|health)$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"lines": [], "status": "healthy", "engine": "ready", "workers": []}),
        ))
        .mount(&s.server)
        .await;
    watch(
        &s.h.ctx,
        WatchRequest {
            handoff_id: None,
            deployment_id: Some(DEP_ID.into()),
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let events: Vec<(String, bool)> =
        s.h.fake
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.kind == EventKind::Deploy)
            .map(|e| (e.summary.clone(), e.terminal))
            .collect();
    assert!(
        events[0].0.contains("needs your action") && events[0].1,
        "{events:?}"
    );
    assert!(
        events.iter().skip(1).all(|(_, t)| !t),
        "replaced wakes nobody: {events:?}"
    );
    assert_eq!(
        s.h.ctx.store.read(|st| st.deploys[0].outcome),
        Outcome::Replaced
    );
}

#[tokio::test]
async fn pushes_made_elsewhere_are_followed_when_created_after_the_selection() {
    let s = setup(plan(false)).await;
    s.server.reset().await;
    let mut old = deployment(
        "55555555-5555-4555-8555-555555555555",
        "old",
        "ready",
        "push",
    );
    old["created_at"] = json!("2000-01-01T00:00:00Z");
    let mut fresh = deployment(DEP_ID, "abc1234", "ready", "push");
    fresh["created_at"] = json!("2999-01-01T00:00:00Z");
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"deployments": [fresh, old]})),
        )
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/(logs|health|events)$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"lines": [], "status": "healthy", "engine": "ready", "workers": []}),
        ))
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/settings|/environments/[^/]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"settings": {}})))
        .mount(&s.server)
        .await;
    s.h.ctx.store.update(|st| {
        st.followed_since = Some("2026-11-25T00:00:00Z".into());
        st.put_check({
            let mut c = st.check.clone().unwrap();
            c.commit = "abc1234ffff".into();
            c
        });
    });
    poll_once(&s.h.ctx).await;
    let deploys = s.h.ctx.store.read(|st| st.deploys.clone());
    assert_eq!(deploys.len(), 1, "the old one was there before");
    assert_eq!(deploys[0].id, DEP_ID);
    assert!(deploys[0].checked_ready, "a ready check has this commit");
}

// -------------------------------------------------------------- secrets

#[tokio::test]
async fn a_secret_value_reaches_the_env_file_and_nothing_else() {
    let s = setup(plan(false)).await;
    let rec = ask(
        &s.h.ctx,
        SecretAsk {
            handoff_id: None,
            name: "SESSION_SECRET".into(),
            description: "cookie key".into(),
            mode: None,
            external: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(rec.state, SecretState::Waiting);
    assert_eq!(
        ask(
            &s.h.ctx,
            SecretAsk {
                handoff_id: None,
                name: "SESSION_SECRET".into(),
                description: "x".into(),
                mode: None,
                external: None
            }
        )
        .await
        .unwrap()
        .id,
        rec.id,
        "the open request is returned"
    );
    assert_eq!(
        ask(
            &s.h.ctx,
            SecretAsk {
                handoff_id: None,
                name: "III_X".into(),
                description: "x".into(),
                mode: None,
                external: None
            }
        )
        .await
        .unwrap_err()
        .code,
        "NOT_A_SECRET"
    );
    let note = ask(
        &s.h.ctx,
        SecretAsk {
            handoff_id: None,
            name: "OTHER".into(),
            description: "x".into(),
            mode: Some(SecretMode::NotNeeded),
            external: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(note.state, SecretState::Noted);

    let value = "s3cr3t-value-zzzz";
    // A key whose permission does not cover variables: the cloud refuses the route.
    Mock::given(method("PUT"))
        .and(path_regex(r"/variables/SESSION_SECRET$"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"message": "This API key can't call this route."})),
        )
        .mount(&s.server)
        .await;
    let refused = provide(
        &s.h.ctx,
        ProvideRequest {
            request_id: rec.id.clone(),
            value: Some(value.into()),
            leave_empty: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (refused.state, refused.cloud, refused.local),
        (
            SecretState::Waiting,
            Some(CloudOutcome::Skipped),
            Some(LocalOutcome::Set)
        ),
        "never Set when the cloud did not take it"
    );
    assert_eq!(refused.error.unwrap().code, "NEEDS_BROWSER_SIGN_IN");
    assert!(refused.resolved_at.is_none());
    // The key may write variables: the value is PUT and the request is Set.
    Mock::given(method("PUT"))
        .and(path_regex(r"/variables/SESSION_SECRET$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .with_priority(1)
        .mount(&s.server)
        .await;
    let done = provide(
        &s.h.ctx,
        ProvideRequest {
            request_id: rec.id.clone(),
            value: Some(value.into()),
            leave_empty: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (done.state, done.cloud),
        (SecretState::Set, Some(CloudOutcome::Set))
    );
    let env = s.ws.join(".env");
    assert_eq!(
        std::fs::read_to_string(&env).unwrap(),
        format!("SESSION_SECRET={value}\n")
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&env).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // The value is in no record, event, or file of the worker's.
    let state = std::fs::read_to_string(s.h.ctx.store.dir().join("state.json")).unwrap();
    let events = serde_json::to_string(&*s.h.fake.events.lock().unwrap()).unwrap();
    let status = serde_json::to_string(
        &iii_cloud::status::status(&s.h.ctx, StatusRequest::default())
            .await
            .unwrap(),
    )
    .unwrap();
    for text in [
        &state,
        &events,
        &status,
        &serde_json::to_string(&done).unwrap(),
    ] {
        assert!(!text.contains(value));
    }
    assert!(s.server.received_requests().await.unwrap().iter().all(|r| {
        r
            .url
            .path()
            .ends_with("keys/self")
            // The value goes to the cloud's variable and nowhere else.
            || r.url.path().ends_with("/variables/SESSION_SECRET")
            || !String::from_utf8_lossy(&r.body).contains(value)
    }));
    // A resolved request can't be answered again; a value is one line.
    assert_eq!(
        provide(
            &s.h.ctx,
            ProvideRequest {
                request_id: rec.id,
                value: Some("x".into()),
                leave_empty: None
            }
        )
        .await
        .unwrap_err()
        .code,
        "INVALID_REQUEST"
    );
    let again = ask(
        &s.h.ctx,
        SecretAsk {
            handoff_id: None,
            name: "TWO".into(),
            description: "x".into(),
            mode: None,
            external: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        provide(
            &s.h.ctx,
            ProvideRequest {
                request_id: again.id.clone(),
                value: Some("a\nb".into()),
                leave_empty: None
            }
        )
        .await
        .unwrap_err()
        .code,
        "INVALID_REQUEST"
    );
    let empty = provide(
        &s.h.ctx,
        ProvideRequest {
            request_id: again.id,
            value: None,
            leave_empty: Some(true),
        },
    )
    .await
    .unwrap();
    assert_eq!(empty.state, SecretState::LeftEmpty);
    // The page-only call carries no value in what the bad-input error says.
    let bad = iii_cloud::functions::parse_request::<ProvideRequest>(
        json!({"request_id": "r", "leave_empty": "hunter2-hunter2"}),
    )
    .err()
    .unwrap();
    assert!(!bad.message.contains("hunter2"));
}

#[tokio::test]
async fn a_tracked_env_file_is_never_written_and_a_cloud_set_value_stays_out_of_it() {
    let s = setup(plan(false)).await;
    // The manifest dir's .env is tracked in git.
    std::fs::write(s.ws.join(".env"), "TRACKED=1\n").unwrap();
    git(&s.ws, &["add", "-f", ".env"]);
    git(&s.ws, &["commit", "-m", "env"]);
    Mock::given(method("PUT"))
        .and(path_regex(r"/variables/SESSION_SECRET$"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"message": "This API key can't call this route."})),
        )
        .mount(&s.server)
        .await;
    let rec = ask(
        &s.h.ctx,
        SecretAsk {
            handoff_id: None,
            name: "SESSION_SECRET".into(),
            description: "x".into(),
            mode: None,
            external: None,
        },
    )
    .await
    .unwrap();
    let done = provide(
        &s.h.ctx,
        ProvideRequest {
            request_id: rec.id,
            value: Some("another-secret-value".into()),
            leave_empty: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(done.local, Some(LocalOutcome::Skipped));
    // The key can't write variables and the file is tracked: saved nowhere, so not "set".
    assert_eq!(done.state, SecretState::Waiting);
    assert_eq!(done.error.unwrap().code, "NEEDS_BROWSER_SIGN_IN");
    assert_eq!(
        std::fs::read_to_string(s.ws.join(".env")).unwrap(),
        "TRACKED=1\n"
    );
}

#[tokio::test]
async fn a_reused_worktree_whose_pull_request_was_merged_starts_again_from_the_base() {
    let s = setup(plan(false)).await;
    // The worktree has a commit that is not on main (a squash merge leaves it so).
    let ahead = git(&s.ws, &["rev-parse", "HEAD"]);
    let main = git(&s.repo.origin, &["rev-parse", "main"]);
    assert_ne!(ahead, main);
    let again = || UseRequest {
        path: s.repo.checkout.to_string_lossy().into(),
        environment_id: Some(ENV.into()),
        mode: WorkspaceMode::Worktree,
        cloud_project_id: None,
        from: None,
        source: None,
    };
    // Not merged: the work stays.
    std::fs::write(s.fakegh.join("list.json"), "[]").unwrap();
    use_project(&s.h.ctx, again()).await.unwrap();
    assert_eq!(git(&s.ws, &["rev-parse", "HEAD"]), ahead);
    // Merged and clean: it starts again from origin/main.
    // A merged pull request whose head is another commit: local work stays.
    std::fs::write(
        s.fakegh.join("list.json"),
        "[{\"number\": 12, \"headRefOid\": \"ffffffffffffffffffffffffffffffffffffffff\"}]",
    )
    .unwrap();
    use_project(&s.h.ctx, again()).await.unwrap();
    assert_eq!(
        git(&s.ws, &["rev-parse", "HEAD"]),
        ahead,
        "unpushed local commits are never lost"
    );
    std::fs::write(
        s.fakegh.join("list.json"),
        format!("[{{\"number\": 12, \"headRefOid\": \"{ahead}\"}}]"),
    )
    .unwrap();
    use_project(&s.h.ctx, again()).await.unwrap();
    assert_eq!(git(&s.ws, &["rev-parse", "HEAD"]), main);
    // Dirty: never reset.
    std::fs::write(s.ws.join("feature2.txt"), "x").unwrap();
    git(&s.ws, &["add", "."]);
    git(&s.ws, &["commit", "-m", "more"]);
    std::fs::write(s.ws.join("dirty.txt"), "x").unwrap();
    let head = git(&s.ws, &["rev-parse", "HEAD"]);
    use_project(&s.h.ctx, again()).await.unwrap();
    assert_eq!(git(&s.ws, &["rev-parse", "HEAD"]), head);
}

#[tokio::test]
async fn the_persons_own_checkout_on_the_worktree_branch_gets_a_new_name_and_is_never_reset() {
    let s = setup(plan(false)).await;
    // Remove the worktree and put the branch on the primary checkout, with a commit.
    git(
        &s.repo.checkout,
        &["worktree", "remove", "--force", s.ws.to_str().unwrap()],
    );
    git(
        &s.repo.checkout,
        &["checkout", "iii-cloud/ready-production"],
    );
    let head = git(&s.repo.checkout, &["rev-parse", "HEAD"]);
    std::fs::write(
        s.fakegh.join("list.json"),
        format!("[{{\"number\": 1, \"headRefOid\": \"{head}\"}}]"),
    )
    .unwrap();
    let p = use_project(
        &s.h.ctx,
        UseRequest {
            path: s.repo.checkout.to_string_lossy().into(),
            environment_id: Some(ENV.into()),
            mode: WorkspaceMode::Worktree,
            cloud_project_id: None,
            from: None,
            source: None,
        },
    )
    .await
    .unwrap();
    let w = p.workspace.unwrap();
    assert_eq!(w.branch, "iii-cloud/ready-production-2");
    assert!(!iii_cloud::project::same_path(
        std::path::Path::new(&w.path),
        &s.repo.checkout
    ));
    assert_eq!(git(&s.repo.checkout, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&s.repo.checkout, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "iii-cloud/ready-production"
    );
}

#[tokio::test]
async fn deploy_rechecks_that_the_base_is_in_the_checked_commits_history() {
    let s = setup(plan(false)).await;
    s.h.ctx.store.update(|st| {
        st.check.as_mut().unwrap().base = "0123456789012345678901234567890123456789".into()
    });
    let err = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "NOT_READY");
    assert!(!argv(&s).contains("pr merge"));
}

/// The value goes to the team and environment whose request asked for it, not
/// to the one the page has selected when the person saves it.
#[tokio::test]
async fn a_secret_goes_to_its_requests_environment_not_the_one_selected_now() {
    let s = setup(plan(false)).await;
    let rec = ask(
        &s.h.ctx,
        SecretAsk {
            handoff_id: None,
            name: "SESSION_SECRET".into(),
            description: "x".into(),
            mode: None,
            external: None,
        },
    )
    .await
    .unwrap();
    const OTHER_TEAM: &str = "55555555-5555-4555-8555-555555555555";
    const OTHER_ENV: &str = "66666666-6666-4666-8666-666666666666";
    // The person switches the page to another team's environment before saving.
    s.h.ctx.store.update(|st| {
        let p = st.project.as_mut().unwrap();
        p.team = Some(TeamRef {
            id: OTHER_TEAM.into(),
            name: "o".into(),
            slug: "o".into(),
        });
        p.environment.as_mut().unwrap().id = OTHER_ENV.into();
    });
    Mock::given(method("PUT"))
        .and(path(format!(
            "/api/v1/teams/{TEAM}/environments/{ENV}/variables/SESSION_SECRET"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .with_priority(1)
        .expect(1)
        .mount(&s.server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(r"/variables/"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&s.server)
        .await;
    let done = provide(
        &s.h.ctx,
        ProvideRequest {
            request_id: rec.id,
            value: Some("v4lue-zzzz".into()),
            leave_empty: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (done.state, done.cloud, done.environment_id.as_str()),
        (SecretState::Set, Some(CloudOutcome::Set), ENV)
    );
    // Reading the request's environment does not overwrite the selected one.
    let selected = s.h.ctx.store.read(|st| {
        st.project
            .as_ref()
            .and_then(|p| p.environment.as_ref())
            .map(|e| e.id.clone())
    });
    assert_eq!(selected.as_deref(), Some(OTHER_ENV));
}

#[tokio::test]
async fn a_cloud_refusal_of_a_secret_carries_a_fixed_text_not_the_clouds() {
    let s = setup(plan(false)).await;
    // A browser session instead of the key, so the page's value is sent to the cloud.
    s.h.fake
        .secrets
        .lock()
        .unwrap()
        .insert("IIICLOUD_SESSION".into(), "iiia_tok".into());
    let mut conn = Connection::signed_out(&s.h.ctx.config().await.cloud_url);
    conn.status = ConnStatus::Connected;
    conn.method = Some(Method::Browser);
    s.h.ctx.set_connection(conn);
    *s.h.ctx.cred.lock().unwrap() = None;
    s.h.ctx.store.update(|st| {
        st.project.as_mut().unwrap().team = Some(TeamRef {
            id: TEAM.into(),
            name: "t".into(),
            slug: "t".into(),
        })
    });
    Mock::given(method("PUT")).and(path_regex(r"/variables/")).respond_with(ResponseTemplate::new(422).set_body_json(json!({"code": "VALIDATION_FAILED", "message": "the value hunter2-zzzz is not allowed", "fix": "use another hunter2-zzzz"}))).mount(&s.server).await;
    let rec = ask(
        &s.h.ctx,
        SecretAsk {
            handoff_id: None,
            name: "TOKEN".into(),
            description: "x".into(),
            mode: None,
            external: None,
        },
    )
    .await
    .unwrap();
    let done = provide(
        &s.h.ctx,
        ProvideRequest {
            request_id: rec.id,
            value: Some("hunter2-zzzz".into()),
            leave_empty: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(done.state, SecretState::Failed);
    let e = done.error.unwrap();
    assert_eq!(
        (e.code.as_str(), e.message.as_str(), e.status, e.fix.clone()),
        (
            "CLOUD_ERROR",
            "the cloud refused the value",
            Some(422),
            None
        )
    );
    let all = format!(
        "{}{}",
        std::fs::read_to_string(s.h.ctx.store.dir().join("state.json")).unwrap(),
        serde_json::to_string(&*s.h.fake.events.lock().unwrap()).unwrap()
    );
    assert!(!all.contains("hunter2-zzzz"));
}

#[tokio::test]
async fn a_replaced_deployment_hands_over_to_its_successor() {
    let s = setup(plan(false)).await;
    s.server.reset().await;
    let mut replaced = deployment(DEP_ID, "abc1234", "replaced", "push");
    replaced["replaced_by"] = json!(NEW_ID);
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(replaced))
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{NEW_ID}", dep_base())))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(deployment(NEW_ID, "abc1234", "ready", "manual")),
        )
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/(logs|health|events)$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"lines": [], "status": "healthy", "engine": "ready", "workers": []}),
        ))
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/environments/[^/]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"settings": {}})))
        .mount(&s.server)
        .await;
    watch(
        &s.h.ctx,
        WatchRequest {
            handoff_id: None,
            deployment_id: Some(DEP_ID.into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        terminal_events(&s.h, NEW_ID).await,
        1,
        "the successor is followed to its end"
    );
    let (old, new) = s.h.ctx.store.read(|st| {
        (
            st.deploys.iter().find(|d| d.id == DEP_ID).cloned().unwrap(),
            st.deploys.iter().find(|d| d.id == NEW_ID).cloned().unwrap(),
        )
    });
    assert_eq!(
        (old.outcome, new.outcome),
        (Outcome::Replaced, Outcome::Ready)
    );
    // It was replaced by a deployment this worker did not create: that ends it for the agent.
    assert_eq!(terminal_events(&s.h, DEP_ID).await, 1);
    assert_eq!(old.replaced_by.as_deref(), Some(NEW_ID));
}

fn req(check: &str, pr: Option<u64>, deployment_id: Option<&str>) -> DeployRequest {
    DeployRequest {
        handoff_id: None,
        check_id: check.into(),
        pr,
        deployment_id: deployment_id.map(Into::into),
    }
}

#[tokio::test]
async fn another_409_keeps_the_clouds_code_and_is_not_plan_changed() {
    let s = setup(plan(true)).await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "needs_action",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .respond_with(ResponseTemplate::new(409).set_body_json(
            json!({"code": "QUEUE_FULL", "message": "Too many deployments are waiting."}),
        ))
        .mount(&s.server)
        .await;
    let err = deploy(&s.h.ctx, req("chk", Some(12), None))
        .await
        .unwrap_err();
    assert_eq!((err.code.as_str(), err.status), ("CLOUD_ERROR", Some(409)));
    assert!(err.message.contains("QUEUE_FULL"));
}

#[tokio::test]
async fn a_pull_request_merged_on_github_with_another_head_is_not_deployed() {
    let s = setup(plan(false)).await;
    // Merged on GitHub (no merge of ours saved) after a newer push: the head is not what was checked.
    std::process::Command::new("sh")
        .arg(s.fakegh.join("merge.sh"))
        .status()
        .unwrap();
    let view = std::fs::read_to_string(s.fakegh.join("view.json"))
        .unwrap()
        .replace(&s.check.commit, "ffffffffffffffffffffffffffffffffffffffff");
    std::fs::write(s.fakegh.join("view.json"), view).unwrap();
    let err = deploy(&s.h.ctx, req("chk", Some(12), None))
        .await
        .unwrap_err();
    assert_eq!(err.code, "PR_HEAD_CHANGED");
}

#[tokio::test]
async fn a_named_deployment_is_confirmed_only_when_it_waits_and_is_the_checked_commit() {
    let s = setup(plan(true)).await;
    std::process::Command::new("sh")
        .arg(s.fakegh.join("merge.sh"))
        .status()
        .unwrap();
    let target = git(&s.repo.origin, &["rev-parse", "main"]);
    // Not waiting: refused.
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(deployment(
            DEP_ID,
            &target,
            "in_progress",
            "push",
        )))
        .up_to_n_times(1)
        .mount(&s.server)
        .await;
    assert_eq!(
        deploy(&s.h.ctx, req("chk", None, Some(DEP_ID)))
            .await
            .unwrap_err()
            .code,
        "NOT_READY"
    );
    // Waiting, but its commit is another one (its tree is not the checked tree): refused.
    let t = tempfile::tempdir().unwrap();
    git(
        t.path(),
        &["clone", "-q", s.repo.origin.to_str().unwrap(), "c"],
    );
    let c = t.path().join("c");
    std::fs::write(c.join("other.txt"), "x").unwrap();
    git(&c, &["add", "."]);
    git(&c, &["commit", "-m", "other"]);
    git(&c, &["push", "-q", "origin", "main"]);
    let other = git(&s.repo.origin, &["rev-parse", "main"]);
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(deployment(
            DEP_ID,
            &other,
            "needs_action",
            "push",
        )))
        .mount(&s.server)
        .await;
    let err = deploy(&s.h.ctx, req("chk", None, Some(DEP_ID)))
        .await
        .unwrap_err();
    assert_eq!(err.code, "MAIN_MOVED");
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&s.server)
        .await;
}

#[tokio::test]
async fn a_push_deployment_from_before_the_merge_is_not_adopted() {
    let s = setup(plan(false)).await;
    let origin = s.repo.origin.clone();
    // The list has a stale deployment of the same sha, created long before the merge.
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(move |_: &Request| {
            let sha = git(&origin, &["rev-parse", "main"]);
            let mut stale = deployment(DEP_ID, &sha, "ready", "push");
            stale["created_at"] = json!("2000-01-01T00:00:00Z");
            ResponseTemplate::new(200)
                .set_body_json(json!({"deployments": [stale], "next_cursor": null}))
        })
        .mount(&s.server)
        .await;
    std::fs::write(
        s.fakegh.join("merge.sh"),
        std::fs::read_to_string(s.fakegh.join("merge.sh"))
            .unwrap()
            .replace(
                "\"mergeCommit\"",
                "\"mergedAt\":\"2026-11-25T12:00:00Z\",\"mergeCommit\"",
            ),
    )
    .unwrap();
    let err = deploy(&s.h.ctx, req("chk", Some(12), None))
        .await
        .unwrap_err();
    assert_eq!(err.code, "CLOUD_ERROR");
    assert!(err.message.contains("no deployment"), "{}", err.message);
}

#[tokio::test]
async fn a_changed_plan_waits_for_the_push_deployment_to_cancel_it_but_only_after_a_merge() {
    let s = setup(plan(false)).await;
    s.server.reset().await;
    let mut changed = plan(false);
    changed["settings"] =
        json!({"release": {"worker": "a", "command": "x", "timeout": "1m", "source": "file"}});
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"can_deploy": true, "fingerprint": "fp2", "plan": changed})),
        )
        .mount(&s.server)
        .await;
    // The push deployment shows up on the third look only.
    let (origin, looks) = (
        s.repo.origin.clone(),
        std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    );
    Mock::given(method("GET")).and(path(dep_base())).respond_with(move |_: &Request| {
        let sha = git(&origin, &["rev-parse", "main"]);
        if looks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
            return ResponseTemplate::new(200).set_body_json(json!({"deployments": [], "next_cursor": null}));
        }
        ResponseTemplate::new(200).set_body_json(json!({"deployments": [deployment(DEP_ID, &sha, "queued", "push")], "next_cursor": null}))
    }).mount(&s.server).await;
    Mock::given(method("POST"))
        .and(path(format!("{}/{DEP_ID}/cancel", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&s.server)
        .await;
    assert_eq!(
        deploy(&s.h.ctx, req("chk", Some(12), None))
            .await
            .unwrap_err()
            .code,
        "PLAN_CHANGED"
    );
}

#[tokio::test]
async fn without_a_merge_a_changed_plan_cancels_nothing() {
    let s = setup(plan(false)).await;
    // The checked commit is on the environment branch: only then can it be deployed as it is.
    let main = git(&s.repo.origin, &["rev-parse", "main"]);
    s.h.ctx
        .store
        .update(|st| st.check.as_mut().unwrap().commit = main);
    s.server.reset().await;
    let mut changed = plan(false);
    changed["settings"] =
        json!({"release": {"worker": "a", "command": "x", "timeout": "1m", "source": "file"}});
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"can_deploy": true, "fingerprint": "fp2", "plan": changed})),
        )
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"/cancel$"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&s.server)
        .await;
    assert_eq!(
        deploy(&s.h.ctx, req("chk", None, None))
            .await
            .unwrap_err()
            .code,
        "PLAN_CHANGED"
    );
}

#[tokio::test]
async fn a_follower_stops_on_a_404_and_frees_its_key() {
    let s = setup(plan(false)).await;
    s.server.reset().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/environments/[^/]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"settings": {}})))
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(json!({"code": "NOT_FOUND", "message": "gone"})),
        )
        .mount(&s.server)
        .await;
    iii_cloud::watch::follow(&s.h.ctx, DEP_ID.into());
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !s.h.ctx
            .busy
            .lock()
            .unwrap()
            .contains(&format!("deploy:{DEP_ID}"))
    );
}

#[tokio::test]
async fn a_follower_asks_for_the_deployments_own_environment() {
    let s = setup(plan(false)).await;
    s.server.reset().await;
    let other_env = "99999999-9999-4999-8999-999999999999";
    let mut d = deployment(DEP_ID, "abc1234", "ready", "push");
    d["environment_id"] = json!(other_env);
    let stored =
        iii_cloud::watch::to_deploy("http://x", (None, None, "other"), &d, None, false).unwrap();
    s.h.ctx.store.update(|st| st.put_deploy(stored));
    Mock::given(method("GET"))
        .and(path_regex(r"/environments/[^/]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"settings": {}})))
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/teams/{TEAM}/environments/{other_env}/deployments/{DEP_ID}"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(d))
        .expect(1..)
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/(logs|health)$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"lines": [], "status": "healthy", "engine": "ready", "workers": []}),
        ))
        .mount(&s.server)
        .await;
    iii_cloud::watch::follow(&s.h.ctx, DEP_ID.into());
    tokio::time::sleep(Duration::from_millis(500)).await;
}

#[tokio::test]
async fn a_run_records_the_workspace_head_and_dirty_state_and_ready_needs_it_equal() {
    let s = setup(plan(false)).await;
    // Dirty: the run's commit is `<head>+dirty`, never equal to the checked commit.
    std::fs::write(s.ws.join("dirty.txt"), "x").unwrap();
    iii_cloud::run::start(&s.h.ctx, "chk".into()).await.unwrap();
    let run =
        s.h.ctx
            .store
            .read(|st| st.check.as_ref().unwrap().run.clone().unwrap());
    assert_eq!(run.commit, format!("{}+dirty", s.check.commit));
    assert!(!s.h.ctx.store.read(|st| st.check.as_ref().unwrap().ready));
}

#[tokio::test]
async fn saved_plans_are_pruned_with_their_checks_and_problem_ids_tell_apart_file_less_problems() {
    let s = setup(plan(false)).await;
    iii_cloud::deploy::save_plan(&s.h.ctx, "orphan", &plan(false));
    iii_cloud::check::prune_plans(&s.h.ctx);
    assert!(!s.h.ctx.store.dir().join("plans/orphan.json").exists());
    assert!(
        s.h.ctx.store.dir().join("plans/chk.json").exists(),
        "the kept check's plan stays"
    );
}

#[tokio::test]
async fn the_manifest_dir_is_refreshed_and_the_env_file_follows_it() {
    let s = setup(plan(false)).await;
    // The environment was selected with an old manifest; the cloud has a new one now.
    s.h.ctx.store.update(|st| {
        st.project
            .as_mut()
            .unwrap()
            .environment
            .as_mut()
            .unwrap()
            .manifest = Some("old/worker-compose.yaml".into())
    });
    s.server.reset().await;
    Mock::given(method("GET")).and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": ENV, "project_id": "p", "name": "production", "production": true, "branch": "main", "manifest": "sub/worker-compose.yaml", "url": null, "settings": {}}))).mount(&s.server).await;
    Mock::given(method("GET"))
        .and(path_regex(r"/(deployments|variables)$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"deployments": [], "variables": []})),
        )
        .mount(&s.server)
        .await;
    // A status refresh keeps the fresh value.
    iii_cloud::status::status(
        &s.h.ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await
    .unwrap();
    let manifest = s.h.ctx.store.read(|st| {
        st.project
            .as_ref()
            .unwrap()
            .environment
            .as_ref()
            .unwrap()
            .manifest
            .clone()
    });
    assert_eq!(manifest.as_deref(), Some("sub/worker-compose.yaml"));
    // And provide reads it fresh too.
    s.h.ctx.store.update(|st| {
        st.project
            .as_mut()
            .unwrap()
            .environment
            .as_mut()
            .unwrap()
            .manifest = Some("old/worker-compose.yaml".into())
    });
    let rec = ask(
        &s.h.ctx,
        SecretAsk {
            handoff_id: None,
            name: "TOKEN".into(),
            description: "x".into(),
            mode: None,
            external: None,
        },
    )
    .await
    .unwrap();
    provide(
        &s.h.ctx,
        ProvideRequest {
            request_id: rec.id,
            value: Some("some-secret-value".into()),
            leave_empty: None,
        },
    )
    .await
    .unwrap();
    assert!(s.ws.join("sub/.env").exists());
    assert!(!s.ws.join("old/.env").exists());
}

#[tokio::test]
async fn a_database_url_is_never_asked_from_the_person_unless_it_is_outside_the_cloud() {
    let s = setup(plan(false)).await;
    for name in ["DATABASE_URL", "POSTGRES_PRISMA_URL", "APP_DATABASE_URL"] {
        let err = ask(
            &s.h.ctx,
            SecretAsk {
                handoff_id: None,
                name: name.into(),
                description: "x".into(),
                mode: None,
                external: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "DATABASE_FROM_CLOUD", "{name}");
        assert!(
            err.message.contains("package://database")
                && err.message.contains("DATABASE_PRIMARY_URL")
        );
    }
    let ok = ask(
        &s.h.ctx,
        SecretAsk {
            handoff_id: None,
            name: "DATABASE_URL".into(),
            description: "an outside Postgres".into(),
            mode: None,
            external: Some(true),
        },
    )
    .await
    .unwrap();
    assert_eq!((ok.state, ok.external), (SecretState::Waiting, true));
    // A request that was already waiting (asked before this rule) settles itself on the next status.
    let old = SecretRequest {
        id: "old".into(),
        environment_id: ENV.into(),
        name: "DATABASE_URL".into(),
        description: "x".into(),
        mode: SecretMode::Ask,
        state: SecretState::Waiting,
        cloud: None,
        local: None,
        asked_at: "t".into(),
        resolved_at: None,
        error: None,
        external: false,
        target: None,
    };
    s.h.ctx.store.update(|st| st.secrets.push(old));
    let status = iii_cloud::status::status(&s.h.ctx, StatusRequest::default())
        .await
        .unwrap();
    let find = |id: &str| status.secrets.iter().find(|r| r.id == id).cloned().unwrap();
    assert_eq!(
        (find("old").state, find("old").mode),
        (SecretState::Noted, SecretMode::NotNeeded)
    );
    assert_eq!(
        find(&ok.id).state,
        SecretState::Waiting,
        "the person's explicit choice stays"
    );
    assert!(s.h.fake.kinds().contains(&"secret:true".to_string()));
}

#[tokio::test]
async fn a_commit_off_the_environment_branch_is_refused_at_once_with_merge_first() {
    let s = setup(plan(false)).await;
    // No pull request and no deployment: the checked commit is the worktree's, not on main.
    s.server.reset().await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"deployments": []})))
        .mount(&s.server)
        .await;
    let started = std::time::Instant::now();
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "MERGE_FIRST");
    assert!(
        err.message.starts_with(
            "Merge the pull request first: Deploy… merges it and deploys the merged commit."
        ),
        "{}",
        err.message
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "no polling for a push that never comes"
    );
}

fn set_pr(s: &Setup, changes: Value) {
    let mut v: Value =
        serde_json::from_str(&std::fs::read_to_string(s.fakegh.join("view.json")).unwrap())
            .unwrap();
    for (k, val) in changes.as_object().unwrap() {
        v[k] = val.clone();
    }
    std::fs::write(s.fakegh.join("view.json"), v.to_string()).unwrap();
}

async fn current_pr(s: &Setup) -> Option<PullRequest> {
    iii_cloud::status::status(
        &s.h.ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await
    .unwrap()
    .pr
}

#[tokio::test]
async fn only_the_pull_request_of_this_try_counts() {
    let s = setup(plan(false)).await;
    let head = s.check.commit.clone();
    // This try's: open, from the fix branch, head = the worktree's HEAD.
    assert_eq!(current_pr(&s).await.unwrap().head_sha, head);
    // An earlier try's closed pull request is nothing, whatever its head.
    set_pr(&s, json!({"state": "CLOSED"}));
    assert!(
        current_pr(&s).await.is_none(),
        "a closed unmerged pull request is never the current one"
    );
    // A merged one is history: it shows in the deploys, not as the current PR.
    set_pr(&s, json!({"state": "MERGED"}));
    assert!(current_pr(&s).await.is_none());
    // Open, but its head is not in this worktree's history (the earlier try's 86c49e1).
    set_pr(
        &s,
        json!({"state": "OPEN", "headRefOid": "86c49e1000000000000000000000000000000aa"}),
    );
    assert!(current_pr(&s).await.is_none());
    // Open, with an older commit of this worktree as its head (pushed, then more work on top).
    std::fs::write(s.ws.join("more.txt"), "m\n").unwrap();
    git(&s.ws, &["add", "."]);
    git(&s.ws, &["commit", "-m", "more"]);
    set_pr(&s, json!({"headRefOid": head}));
    assert_eq!(current_pr(&s).await.unwrap().head_sha, head);
    // Another head branch is not the fix branch's.
    set_pr(&s, json!({"headRefName": "someone-else"}));
    assert!(current_pr(&s).await.is_none());
    // Created before this worktree began: an earlier try's, even with a matching head.
    set_pr(
        &s,
        json!({"headRefName": "iii-cloud/ready-production", "createdAt": "2000-01-01T00:00:00Z"}),
    );
    assert!(current_pr(&s).await.is_none());
    set_pr(&s, json!({"createdAt": "2999-01-01T00:00:00Z"}));
    assert!(current_pr(&s).await.is_some());
}

#[tokio::test]
async fn the_secrets_tab_lists_every_variable_the_plan_or_the_manifest_references() {
    let s = setup(plan(false)).await;
    std::fs::write(
        s.ws.join("worker-compose.yaml"),
        "containers:\n  api:\n    worker: path://./workers/api\n    environment:\n      OPENAI_API_KEY: ${OPENAI_API_KEY:-}\n      SUPABASE_URL: ${SUPABASE_URL:-}\n      DB_PASS: ${DB_PASS}\n      REGION: ${REGION:-eu}\n",
    )
    .unwrap();
    // The plan flags one with an empty default; the cloud's own list has another.
    let mut p = plan(false);
    p["variables"] = json!([{"name": "OPENAI_API_KEY", "type": "secret", "required_by": ["api"], "applies": "all", "status": "empty_default"}]);
    iii_cloud::deploy::save_plan(&s.h.ctx, "chk", &p);
    s.h.ctx.store.update(|st| {
        st.variables = vec![VariableRow {
            name: "SESSION_SECRET".into(),
            kind: "secret".into(),
            status: "set".into(),
            required_by: vec!["api".into()],
        }]
    });
    let status = iii_cloud::status::status(
        &s.h.ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await
    .unwrap();
    let row = |n: &str| status.variables.iter().find(|v| v.name == n).cloned();
    assert_eq!(
        row("OPENAI_API_KEY").map(|v| (v.kind, v.status)),
        Some(("secret".into(), "empty_default".into())),
        "from the plan"
    );
    assert_eq!(
        row("SUPABASE_URL").map(|v| (v.status, v.required_by)),
        Some(("empty_default".into(), vec!["api".into()])),
        "from the manifest"
    );
    assert_eq!(row("DB_PASS").map(|v| v.status), Some("missing".into()));
    assert!(row("REGION").is_none(), "a real default is a value already");
    assert!(
        row("SESSION_SECRET").is_some(),
        "what the cloud lists stays"
    );
    assert_eq!(
        status
            .variables
            .iter()
            .filter(|v| v.name == "OPENAI_API_KEY")
            .count(),
        1,
        "no duplicates"
    );
}

async fn failed_release(s: &Setup, lines: &[&str], settings_release: Option<&str>) {
    s.server.reset().await;
    let mut failed = deployment(DEP_ID, "86c49e1c", "failed", "manual");
    failed["error"] = json!({"stage": "workers", "worker": "api", "code": "RELEASE_FAILED", "hint": "The release command exited with 1.", "action": null});
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(failed))
        .mount(&s.server)
        .await;
    let release = settings_release.map(|c| json!({"worker": "api", "command": c}));
    Mock::given(method("GET")).and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": ENV, "name": "production", "branch": "main", "production": true, "manifest": "worker-compose.yaml", "settings": {"release": release}}))).mount(&s.server).await;
    // Newest first, as the cloud sends them; only the release source is asked for.
    let newest_first: Vec<Value> = lines.iter().rev().map(|l| json!({"message": l})).collect();
    Mock::given(method("GET"))
        .and(path_regex(r"/logs$"))
        .and(wiremock::matchers::query_param("source", "release"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"lines": newest_first, "next_cursor": null})),
        )
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/logs$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"lines": [{"message": "from the wrong source"}], "next_cursor": null}),
        ))
        .with_priority(9)
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/health$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "healthy", "engine": "ready", "workers": []})),
        )
        .mount(&s.server)
        .await;
}

#[tokio::test]
async fn a_failed_release_says_what_failed_with_the_release_logs_redacted() {
    let s = setup(plan(false)).await;
    std::fs::write(s.ws.join(".env"), "API_TOKEN=tok-abcdefgh-1234\n").unwrap();
    let mut lines: Vec<String> = (0..60).map(|i| format!("line {i}")).collect();
    lines.push(
        "using postgres://app_login:hunter2pw@db.internal:5432/app and tok-abcdefgh-1234".into(),
    );
    lines.push("x".repeat(400));
    lines.push("ELIFECYCLE Command exited 1".into());
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    failed_release(&s, &refs, Some("pnpm --filter api db:migrate")).await;
    let d = watch(
        &s.h.ctx,
        WatchRequest {
            handoff_id: None,
            deployment_id: Some(DEP_ID.into()),
        },
    )
    .await
    .unwrap();
    let f = d.failure.expect("a failed deploy carries its failure");
    assert_eq!(
        (f.code.as_str(), f.stage.as_str(), f.worker.as_deref()),
        ("RELEASE_FAILED", "workers", Some("api"))
    );
    assert_eq!(
        f.title,
        "The release command failed: `pnpm --filter api db:migrate`"
    );
    assert_eq!(
        f.release_command.as_deref(),
        Some("pnpm --filter api db:migrate")
    );
    assert_eq!(f.log_tail.len(), 40, "the last ~40 lines");
    assert_eq!(
        f.log_tail.last().unwrap(),
        "ELIFECYCLE Command exited 1",
        "oldest first, the end last"
    );
    assert!(
        f.log_tail.iter().all(|l| l.chars().count() <= 303),
        "each cut to 300"
    );
    let all = f.log_tail.join("\n");
    assert!(
        !all.contains("hunter2pw")
            && !all.contains("tok-abcdefgh")
            && !all.contains("wrong source"),
        "{all}"
    );
    assert!(all.contains("postgres://app_login:[redacted]@db.internal"));
    // It exited 1 and printed nothing that says why.
    assert!(f.silent);
    assert_eq!(
        f.hint,
        "the command exited 1 without printing an error; the tool may hide it, so run it locally the same way"
    );
}

#[tokio::test]
async fn a_failure_with_error_text_is_not_silent_and_keeps_the_clouds_hint() {
    let s = setup(plan(false)).await;
    failed_release(
        &s,
        &[
            "error: permission denied for schema public",
            "ELIFECYCLE Command exited 1",
        ],
        None,
    )
    .await;
    let d = watch(
        &s.h.ctx,
        WatchRequest {
            handoff_id: None,
            deployment_id: Some(DEP_ID.into()),
        },
    )
    .await
    .unwrap();
    let f = d.failure.unwrap();
    assert!(!f.silent);
    assert_eq!(f.hint, "The release command exited with 1.");
    assert!(f.title.starts_with("The release command failed"));
}

#[tokio::test]
async fn the_deploy_fix_brief_carries_the_failure_the_log_and_the_release_command() {
    let s = setup(plan(false)).await;
    failed_release(
        &s,
        &["> drizzle-kit migrate", "ELIFECYCLE Command exited 1"],
        Some("pnpm migrate"),
    )
    .await;
    watch(
        &s.h.ctx,
        WatchRequest {
            handoff_id: None,
            deployment_id: Some(DEP_ID.into()),
        },
    )
    .await
    .unwrap();
    let out = iii_cloud::handoff::create(
        &s.h.ctx,
        HandoffRequest {
            check_id: Some("chk".into()),
            kind: HandoffKind::DeployFix,
            deployment_id: Some(DEP_ID.into()),
            text: None,
            candidate: None,
        },
    )
    .await
    .unwrap();
    assert!(!out.id.is_empty());
    let brief = iii_cloud::status::status(&s.h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .handoff_brief
        .unwrap();
    let d = brief.deploy.unwrap();
    assert_eq!(
        d.title.as_deref(),
        Some("The release command failed: `pnpm migrate`")
    );
    assert_eq!(
        (d.release_command.as_deref(), d.silent),
        (Some("pnpm migrate"), true)
    );
    assert_eq!(
        d.log_tail,
        vec!["> drizzle-kit migrate", "ELIFECYCLE Command exited 1"]
    );
    assert!(
        d.hint
            .unwrap()
            .contains("exited 1 without printing an error")
    );
}

/// The environment is not production and deploys `main`, which is not the
/// repository's default branch (`trunk`): an assisted deploy that pushes `main`.
async fn direct(s: &Setup) {
    set_default_branch(s, "trunk");
    not_production(s).await;
}

/// The repository's default branch on origin (what an assisted deploy reads live).
fn set_default_branch(s: &Setup, name: &str) {
    git(&s.repo.origin, &["branch", "-f", name, "main"]);
    git(
        &s.repo.origin,
        &["symbolic-ref", "HEAD", &format!("refs/heads/{name}")],
    );
}

/// The environment is not production, in the store and in iii cloud's answer
/// (a later `env_file` answers the same, with its own file).
async fn not_production(s: &Setup) {
    s.h.ctx.store.update(|st| {
        st.project
            .as_mut()
            .unwrap()
            .environment
            .as_mut()
            .unwrap()
            .production = false
    });
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": ENV, "project_id": "p", "name": "production", "production": false, "branch": "main", "manifest": "worker-compose.yaml", "url": null, "settings": {}})))
        .with_priority(2)
        .mount(&s.server)
        .await;
    // The project's environments, live: the selected one and the store's others.
    let ctx = s.h.ctx.clone();
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/projects/p/environments")))
        .respond_with(move |_: &Request| {
            let (me, others) = ctx.store.read(|st| {
                let p = st.project.as_ref().unwrap();
                (p.environment.clone().unwrap(), p.environments.clone())
            });
            let mut all = vec![json!({"id": me.id, "name": me.name, "production": false, "branch": me.branch, "manifest": me.manifest, "url": null})];
            all.extend(others.iter().filter(|e| e.id != me.id).map(|e| json!({"id": e.id, "name": e.name, "production": e.production, "branch": e.branch, "manifest": null, "url": null})));
            ResponseTemplate::new(200).set_body_json(json!({"environments": all}))
        })
        .mount(&s.server)
        .await;
    // The deployments list as it is before anything happens (an assisted deploy reads it first).
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"deployments": [], "next_cursor": null})),
        )
        .with_priority(9)
        .mount(&s.server)
        .await;
}

async fn push_list(s: &Setup, status: &'static str) {
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status,
        })
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(deployment(DEP_ID, "x", "ready", "push"))
        })
        .mount(&s.server)
        .await;
}

#[tokio::test]
async fn an_assisted_environment_on_its_own_branch_is_pushed_by_the_persons_deploy_and_its_push_deployment_adopted()
 {
    let s = setup(plan(false)).await;
    direct(&s).await;
    // No pull request is read for it, whatever gh would say.
    assert!(current_pr(&s).await.is_none());
    // The brief tells the agent to push its fix branch only, never the environment's.
    iii_cloud::handoff::create(
        &s.h.ctx,
        HandoffRequest {
            check_id: Some("chk".into()),
            kind: HandoffKind::Ready,
            deployment_id: None,
            text: None,
            candidate: None,
        },
    )
    .await
    .unwrap();
    let brief = iii_cloud::status::status(&s.h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .handoff_brief
        .unwrap();
    assert!(brief.pushes_directly);
    assert!(
        brief.next_step.contains("push only your fix branch")
            && brief
                .next_step
                .contains("Never push a branch an environment deploys from")
            && brief
                .next_step
                .contains("production gets an assisted deploy")
            && brief.next_step.contains("production's own branch")
            && !brief.next_step.contains("main"),
        "{}",
        brief.next_step
    );
    assert!(
        brief.next_step.contains(
            "Your fix branch is iii-cloud/ready-production: push it with `git push --force-with-lease -u origin HEAD`"
        ),
        "{}",
        brief.next_step
    );
    // A pull request hand-off on this environment opens none either.
    s.h.ctx.store.update(|st| {
        st.handoffs
            .iter_mut()
            .for_each(|h| h.agent.state = AgentPhase::Done)
    });
    iii_cloud::handoff::create(
        &s.h.ctx,
        HandoffRequest {
            check_id: Some("chk".into()),
            kind: HandoffKind::PullRequest,
            deployment_id: None,
            text: None,
            candidate: None,
        },
    )
    .await
    .unwrap();
    let brief = iii_cloud::status::status(&s.h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .handoff_brief
        .unwrap();
    assert_eq!(brief.kind, HandoffKind::PullRequest);
    assert!(
        !brief.next_step.contains("gh pr create") && brief.next_step.contains("No pull request"),
        "{}",
        brief.next_step
    );
    push_list(&s, "queued").await;
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!((d.id.as_str(), d.approved_by_worker), (DEP_ID, false));
    assert_eq!(
        git(&s.repo.origin, &["rev-parse", "main"]),
        s.check.commit,
        "the deploy pushed the checked commit"
    );
    assert!(!argv(&s).contains("pr merge"));
}

#[tokio::test]
async fn a_direct_push_switches_the_file_first_and_the_confirmed_deployment_keeps_it() {
    let s = setup(next_plan(true)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    direct(&s).await;
    push_list(&s, "needs_action").await;
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!((d.id.as_str(), d.approved_by_worker), (NEW_ID, true));
    assert_eq!(
        f.log(),
        vec![format!("switch to {NEXT}"), format!("create with {NEXT}")],
        "the switch comes before the push"
    );
    assert_eq!(*f.manifest.lock().unwrap(), NEXT);
    assert_eq!(git(&s.repo.origin, &["rev-parse", "main"]), s.check.commit);
}

#[tokio::test]
async fn a_direct_push_that_is_refused_or_finds_main_moved_puts_the_old_file_back() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    direct(&s).await;
    // A commit on origin that main will jump to.
    let t = tempfile::tempdir().unwrap();
    git(
        t.path(),
        &["clone", "-q", s.repo.origin.to_str().unwrap(), "c"],
    );
    let c = t.path().join("c");
    std::fs::write(c.join("other.txt"), "o").unwrap();
    git(&c, &["add", "."]);
    git(&c, &["commit", "-m", "other"]);
    git(&c, &["push", "-q", "origin", "HEAD:side"]);
    let side = git(&s.repo.origin, &["rev-parse", "side"]);
    let hook = s.repo.origin.join("hooks/pre-receive");
    let set_hook = |body: &str| {
        std::fs::write(&hook, format!("#!/bin/sh\n{body}\nexit 1\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    // GitHub refuses the push (a protected branch, say): nothing landed.
    set_hook("echo 'protected branch' >&2");
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "PUSH_REFUSED");
    assert!(
        err.message.contains("protected branch")
            && err
                .message
                .ends_with("production is back on worker-compose.yaml."),
        "{}",
        err.message
    );
    // main moves while the push is on its way: not fast-forward.
    set_hook(&format!(
        "env -u GIT_QUARANTINE_PATH -u GIT_OBJECT_DIRECTORY -u GIT_ALTERNATE_OBJECT_DIRECTORIES git update-ref refs/heads/main {side}"
    ));
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "MAIN_MOVED", "{}", err.message);
    assert_eq!(git(&s.repo.origin, &["rev-parse", "main"]), side);
    // Already moved: refused before anything switches or pushes.
    std::fs::remove_file(&hook).unwrap();
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "MAIN_MOVED");
    assert_eq!(
        f.log(),
        vec![
            format!("switch to {NEXT}"),
            "switch to worker-compose.yaml".to_string(),
            format!("switch to {NEXT}"),
            "switch to worker-compose.yaml with the commit on main".to_string(),
        ]
    );
    assert_eq!(*f.manifest.lock().unwrap(), "worker-compose.yaml");
    assert_eq!(git(&s.repo.origin, &["rev-parse", "main"]), side);
}

/// A direct push of the other file whose plan then changed, with the push's
/// deployment in `status` and a cancel the cloud accepts `cancels` times.
async fn changed_after_push(
    status: &'static str,
    cancels: u64,
) -> (Setup, EnvFile, CloudErrorLike) {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    direct(&s).await;
    let mut changed = next_plan(false);
    changed["settings"] =
        json!({"release": {"worker": "a", "command": "x", "timeout": "1m", "source": "file"}});
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"can_deploy": true, "fingerprint": "fp2", "plan": changed})),
        )
        .with_priority(1)
        .mount(&s.server)
        .await;
    push_list(&s, status).await;
    Mock::given(method("POST"))
        .and(path(format!("{}/{DEP_ID}/cancel", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(cancels)
        .mount(&s.server)
        .await;
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    (s, f, (err.code, err.message))
}

type CloudErrorLike = (String, String);

#[tokio::test]
async fn a_changed_plan_after_the_direct_push_cancels_its_deployment_and_puts_the_old_file_back() {
    // Queued, or waiting for the person: canceled, so nothing carries the new file.
    for status in ["queued", "needs_action"] {
        let (_s, f, (code, message)) = changed_after_push(status, 1).await;
        assert_eq!(code, "PLAN_CHANGED", "{status}");
        assert!(
            message.ends_with("production is back on worker-compose.yaml."),
            "{status}: {message}"
        );
        assert_eq!(*f.manifest.lock().unwrap(), "worker-compose.yaml");
    }
}

#[tokio::test]
async fn a_push_deployment_already_ready_carries_the_new_file_and_the_switch_stays() {
    let (_s, f, (code, message)) = changed_after_push("ready", 0).await;
    assert_eq!(code, "PUSH_STILL_RUNNING");
    assert!(
        message.contains("was not cancelled (it is ready)")
            && message.ends_with(&format!("production now uses {NEXT}.")),
        "{message}"
    );
    assert_eq!(*f.manifest.lock().unwrap(), NEXT);
}

#[tokio::test]
async fn a_5xx_from_the_create_may_have_made_the_deployment_so_the_switch_stays() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 503).await;
    pushed_directly(&s).await;
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert!(
        err.message
            .contains(&format!("The answer was lost: production stays on {NEXT}")),
        "{}",
        err.message
    );
    assert_eq!(
        f.log(),
        vec![
            format!("switch to {NEXT} with the commit on main"),
            format!("create with {NEXT}")
        ],
        "never switched back"
    );
    assert_eq!(*f.manifest.lock().unwrap(), NEXT);
}

#[tokio::test]
async fn a_push_whose_outcome_is_unknown_keeps_the_switch_and_says_so() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    direct(&s).await;
    // The push fails, and origin can't be read after it either.
    let config = git(&s.repo.checkout, &["rev-parse", "--git-path", "config"]);
    let config = s.repo.checkout.join(config);
    std::fs::write(
        s.repo.origin.join("hooks/pre-receive"),
        format!(
            "#!/bin/sh\ngit config --file '{c}' --unset url.{o}.insteadOf\ngit config --file '{c}' url./nonexistent/origin.git.insteadOf https://github.com/ada/hello-iii.git\nexit 1\n",
            c = config.display(),
            o = s.repo.origin.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            s.repo.origin.join("hooks/pre-receive"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "PUSH_UNKNOWN", "{}", err.message);
    assert!(
        err.message.starts_with(&format!(
            "Could not tell whether {} reached main",
            &s.check.commit[..7]
        )) && err.message.contains("See Deploys")
            && err
                .message
                .ends_with(&format!("production stays on {NEXT}.")),
        "{}",
        err.message
    );
    assert_eq!(
        f.log(),
        vec![format!("switch to {NEXT}")],
        "never switched back"
    );
}

#[tokio::test]
async fn a_deploy_this_worker_made_is_not_from_the_console() {
    let s = setup(plan(false)).await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "queued",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(deployment(DEP_ID, "x", "ready", "push"))
        })
        .mount(&s.server)
        .await;
    deploy(&s.h.ctx, req("chk", Some(12), None)).await.unwrap();
    let status = iii_cloud::status::status(&s.h.ctx, StatusRequest::default())
        .await
        .unwrap();
    let d = status.deploys.iter().find(|d| d.id == DEP_ID).unwrap();
    assert!(!d.from_console);
}

async fn refreshed(s: &Setup) -> Status {
    iii_cloud::status::status(
        &s.h.ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_gh_error_keeps_the_last_good_pull_request_and_says_what_went_wrong() {
    let s = setup(plan(false)).await;
    let good = std::fs::read_to_string(s.fakegh.join("view.json")).unwrap();
    let first = refreshed(&s).await;
    assert!(first.pr.is_some() && first.pr_error.is_none());
    // gh fails (prints no JSON): the last good one stays, with the error beside it.
    std::fs::write(s.fakegh.join("view.json"), "not json").unwrap();
    let broken = refreshed(&s).await;
    assert_eq!(
        broken.pr.as_ref().map(|p| p.number),
        first.pr.map(|p| p.number)
    );
    assert_eq!(broken.pr_error.unwrap().code, "GH_FAILED");
    // gh answers again: the error clears.
    std::fs::write(s.fakegh.join("view.json"), good).unwrap();
    let healed = refreshed(&s).await;
    assert!(healed.pr.is_some() && healed.pr_error.is_none());
}

#[tokio::test]
async fn the_deployment_list_is_followed_by_cursor_until_the_push_shows_up() {
    let s = setup(plan(false)).await;
    let base = dep_base();
    let target = s.check.commit.clone();
    Mock::given(method("GET"))
        .and(path(base.clone()))
        .and(wiremock::matchers::query_param("cursor", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deployments": [deployment(DEP_ID, &target, "ready", "push")], "next_cursor": null
        })))
        .with_priority(1)
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(base.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deployments": [deployment(NEW_ID, "ffffffffffffffffffffffffffffffffffffffff", "ready", "push")],
            "next_cursor": "page2"
        })))
        .mount(&s.server)
        .await;
    let list = iii_cloud::deploy::list_pages(
        &s.h.ctx,
        base.trim_start_matches("/api/v1"),
        &target,
        None,
        &[],
    )
    .await
    .unwrap();
    let ids: Vec<&str> = list["deployments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["id"].as_str())
        .collect();
    assert_eq!(ids, vec![NEW_ID, DEP_ID]);
}

#[tokio::test]
async fn following_a_deployment_the_cloud_lost_marks_it_gone_and_a_refusal_stops_the_loop() {
    let s = setup(plan(false)).await;
    let running = watch_record(&s, DEP_ID, "running");
    s.h.ctx.store.update(|st| st.put_deploy(running));
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(json!({"code": "NOT_FOUND", "message": "gone"})),
        )
        .mount(&s.server)
        .await;
    iii_cloud::watch::follow(&s.h.ctx, DEP_ID.into());
    let mut gone = None;
    for _ in 0..200 {
        gone =
            s.h.ctx
                .store
                .read(|st| st.deploys.iter().find(|d| d.id == DEP_ID).cloned())
                .filter(|d| d.status == "gone");
        if gone.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let gone = gone.expect("the record says it is gone");
    assert_eq!(gone.outcome, Outcome::Canceled);
    assert!(gone.status_detail.unwrap().contains("no longer has"));
    // A 403 ends the loop (the key is freed) and changes nothing.
    let other = watch_record(&s, NEW_ID, "running");
    s.h.ctx.store.update(|st| st.put_deploy(other));
    Mock::given(method("GET"))
        .and(path(format!("{}/{NEW_ID}", dep_base())))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"code": "FORBIDDEN", "message": "no"})),
        )
        .mount(&s.server)
        .await;
    iii_cloud::watch::follow(&s.h.ctx, NEW_ID.into());
    for _ in 0..200 {
        if !s
            .h
            .ctx
            .busy
            .lock()
            .unwrap()
            .contains(&format!("deploy:{NEW_ID}"))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        !s.h.ctx
            .busy
            .lock()
            .unwrap()
            .contains(&format!("deploy:{NEW_ID}"))
    );
    let after =
        s.h.ctx
            .store
            .read(|st| st.deploys.iter().find(|d| d.id == NEW_ID).cloned())
            .unwrap();
    assert_eq!(after.outcome, Outcome::Running);
}

fn watch_record(s: &Setup, id: &str, status: &str) -> Deploy {
    iii_cloud::watch::to_deploy(
        "http://x",
        (None, None, "production"),
        &deployment(id, &s.check.commit, status, "push"),
        None,
        false,
    )
    .unwrap()
}

#[tokio::test]
async fn a_provided_value_is_redacted_even_when_no_env_file_holds_it() {
    let s = setup(plan(false)).await;
    Mock::given(method("PUT"))
        .and(path_regex(r"/variables/ONLY_CLOUD$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&s.server)
        .await;
    let rec = ask(
        &s.h.ctx,
        SecretAsk {
            handoff_id: None,
            name: "ONLY_CLOUD".into(),
            description: "x".into(),
            mode: None,
            external: None,
        },
    )
    .await
    .unwrap();
    let value = "in-memory-only-value-9";
    let done = provide(
        &s.h.ctx,
        ProvideRequest {
            request_id: rec.id,
            value: Some(value.into()),
            leave_empty: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(done.cloud, Some(CloudOutcome::Set));
    assert!(
        !s.ws.join(".env").exists()
            || !std::fs::read_to_string(s.ws.join(".env"))
                .unwrap()
                .contains(value)
    );
    assert!(iii_cloud::handoff::env_values(&s.h.ctx).contains(&value.to_string()));
    assert_eq!(
        iii_cloud::run::redact(
            &format!("token {value} end"),
            &iii_cloud::handoff::env_values(&s.h.ctx)
        ),
        "token [redacted] end"
    );
}

#[tokio::test]
async fn a_failed_or_canceled_deployment_of_the_commit_is_never_adopted() {
    let s = setup(plan(false)).await;
    let base = dep_base();
    let base = base.trim_start_matches("/api/v1");
    let target = s.check.commit.clone();
    for (status, adopted) in [
        ("failed", false),
        ("canceled", false),
        ("gone", false),
        ("running", true),
        ("ready", true),
    ] {
        s.server.reset().await;
        Mock::given(method("GET"))
            .and(path(dep_base()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "deployments": [deployment(DEP_ID, &target, status, "manual")], "next_cursor": null
            })))
            .mount(&s.server)
            .await;
        // The credential survives a reset of the mocks only through the memory cache.
        let got = iii_cloud::deploy::push_deployment(&s.h.ctx, base, &target, false, None, &[])
            .await
            .unwrap();
        assert_eq!(got.is_some(), adopted, "{status}");
    }
}

const NEXT: &str = "worker-compose-next.yaml";

/// The plan of the other compose file.
fn next_plan(resources: bool) -> Value {
    let mut p = plan(resources);
    p["source"]["manifest"] = json!(NEXT);
    p
}

/// The ready check was of the other compose file.
fn check_of_next(s: &Setup) {
    s.h.ctx.store.update(|st| {
        st.check.as_mut().unwrap().manifest = Some(NEXT.into());
        st.checks[0].manifest = Some(NEXT.into());
    });
}

/// The environment's compose file as the fake cloud holds it, and what
/// happened to it and to deployments, in order.
struct EnvFile {
    manifest: std::sync::Arc<std::sync::Mutex<String>>,
    /// The branch the environment follows in the fake cloud.
    branch: std::sync::Arc<std::sync::Mutex<String>>,
    log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl EnvFile {
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

/// `production` as the context's store has it: `direct` flips it, and the cloud follows.
async fn env_file(s: &Setup, start: &str, create_status: u16) -> EnvFile {
    let f = EnvFile {
        manifest: std::sync::Arc::new(std::sync::Mutex::new(start.into())),
        branch: std::sync::Arc::new(std::sync::Mutex::new("main".into())),
        log: Default::default(),
    };
    let env_path = format!("/api/v1/teams/{TEAM}/environments/{ENV}");
    let production = {
        let ctx = s.h.ctx.clone();
        move || {
            ctx.store.read(|st| {
                st.project
                    .as_ref()
                    .and_then(|p| p.environment.as_ref())
                    .is_none_or(|e| e.production)
            })
        }
    };
    let body = std::sync::Arc::new(
        move |m: &str, b: &str| json!({"id": ENV, "project_id": "p", "name": "production", "production": production(), "branch": b, "manifest": m, "url": null, "settings": {}}),
    );
    let (m, br, get_body) = (f.manifest.clone(), f.branch.clone(), body.clone());
    Mock::given(method("GET"))
        .and(path(env_path.clone()))
        .respond_with(move |_: &Request| {
            ResponseTemplate::new(200)
                .set_body_json(get_body(&m.lock().unwrap(), &br.lock().unwrap()))
        })
        .with_priority(1)
        .mount(&s.server)
        .await;
    let (m, br, log, origin, base) = (
        f.manifest.clone(),
        f.branch.clone(),
        f.log.clone(),
        s.repo.origin.clone(),
        s.check.base.clone(),
    );
    Mock::given(method("PATCH"))
        .and(path(env_path))
        .respond_with(move |r: &Request| {
            let sent: Value = serde_json::from_slice(&r.body).unwrap();
            let merged = git(&origin, &["rev-parse", "main"]) != base;
            let mut what = vec![];
            if let Some(b) = sent["branch"].as_str() {
                // The branch must be on origin before the environment follows it.
                let there = std::process::Command::new("git")
                    .args(["rev-parse", "--verify", "-q", &format!("refs/heads/{b}")])
                    .current_dir(&origin)
                    .output()
                    .unwrap()
                    .status
                    .success();
                what.push(format!(
                    "move to {b}{}",
                    if there { "" } else { " (not on origin)" }
                ));
                *br.lock().unwrap() = b.to_string();
            }
            if let Some(to) = sent["manifest"].as_str() {
                what.push(format!(
                    "switch to {to}{}",
                    if merged && sent["branch"].is_null() {
                        " with the commit on main"
                    } else {
                        ""
                    }
                ));
                *m.lock().unwrap() = to.to_string();
            }
            log.lock().unwrap().push(what.join(" and "));
            ResponseTemplate::new(200).set_body_json(body(&m.lock().unwrap(), &br.lock().unwrap()))
        })
        .mount(&s.server)
        .await;
    let (m, log) = (f.manifest.clone(), f.log.clone());
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .and(header("if-match", "fp-target"))
        .respond_with(move |r: &Request| {
            let sent: Value = serde_json::from_slice(&r.body).unwrap();
            log.lock()
                .unwrap()
                .push(format!("create with {}", m.lock().unwrap()));
            match create_status {
                201 => ResponseTemplate::new(201).set_body_json(deployment(
                    NEW_ID,
                    sent["commit"].as_str().unwrap(),
                    "queued",
                    "manual",
                )),
                // A redirect to itself: the client gives up with no answer (a transport error).
                307 => ResponseTemplate::new(307).insert_header("location", dep_base().as_str()),
                other => ResponseTemplate::new(other).set_body_json(
                    json!({"code": "FORBIDDEN", "message": "Approving needs Owner or Admin."}),
                ),
            }
        })
        .with_priority(1)
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{NEW_ID}", dep_base())))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(deployment(NEW_ID, "x", "ready", "manual"))
        })
        .mount(&s.server)
        .await;
    f
}

fn local_env_manifest(s: &Setup) -> Option<String> {
    s.h.ctx.store.read(|st| {
        st.project
            .as_ref()
            .and_then(|p| p.environment.as_ref())
            .and_then(|e| e.manifest.clone())
    })
}

fn planned_files(requests: &[Request]) -> Vec<Option<String>> {
    requests
        .iter()
        .filter(|r| r.url.path().ends_with("/plan"))
        .map(|r| {
            r.url
                .query_pairs()
                .find(|(k, _)| k == "manifest")
                .map(|(_, v)| v.into_owned())
        })
        .collect()
}

#[tokio::test]
async fn deploying_a_check_of_another_file_switches_the_environment_before_the_merge() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    // The push the merge starts plans with the new file: it is the deploy.
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "queued",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(deployment(DEP_ID, "x", "ready", "push"))
        })
        .mount(&s.server)
        .await;
    let d = deploy(&s.h.ctx, req("chk", Some(12), None)).await.unwrap();
    assert_eq!(d.id, DEP_ID);
    assert_eq!(
        f.log(),
        vec![format!("switch to {NEXT}")],
        "before the merge, once"
    );
    let requests = s.server.received_requests().await.unwrap();
    assert_eq!(
        planned_files(&requests),
        vec![Some(NEXT.to_string())],
        "pinned with the checked file"
    );
    assert_eq!(local_env_manifest(&s).as_deref(), Some(NEXT));
}

/// The environment deploys main directly and the checked commit is on it: its
/// push deployment was made with the old file.
async fn pushed_directly(s: &Setup) {
    git(&s.ws, &["push", "-q", "origin", "HEAD:main"]);
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "queued",
        })
        .mount(&s.server)
        .await;
}

#[tokio::test]
async fn a_push_made_with_the_old_file_is_not_adopted_and_the_switch_comes_right_before_the_create()
{
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    pushed_directly(&s).await;
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!((d.id.as_str(), d.approved_by_worker), (NEW_ID, true));
    assert_eq!(
        f.log(),
        vec![
            format!("switch to {NEXT} with the commit on main"),
            format!("create with {NEXT}")
        ]
    );
    assert_eq!(local_env_manifest(&s).as_deref(), Some(NEXT));
}

#[tokio::test]
async fn a_create_that_fails_after_the_switch_puts_the_old_file_back_and_says_so() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 403).await;
    pushed_directly(&s).await;
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "FORBIDDEN");
    assert!(
        err.message
            .ends_with("production is back on worker-compose.yaml."),
        "{}",
        err.message
    );
    assert_eq!(
        f.log(),
        vec![
            format!("switch to {NEXT} with the commit on main"),
            format!("create with {NEXT}"),
            "switch to worker-compose.yaml with the commit on main".to_string()
        ]
    );
    assert_eq!(*f.manifest.lock().unwrap(), "worker-compose.yaml");
    assert_eq!(
        local_env_manifest(&s).as_deref(),
        Some("worker-compose.yaml")
    );
}

#[tokio::test]
async fn a_refused_merge_after_the_switch_puts_the_old_file_back() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    std::fs::write(
        s.fakegh.join("merge.sh"),
        "echo 'Pull request is not mergeable.' >&2\nexit 1\n",
    )
    .unwrap();
    let err = deploy(&s.h.ctx, req("chk", Some(12), None))
        .await
        .unwrap_err();
    assert_eq!(err.code, "PR_NOT_MERGEABLE");
    assert!(
        err.message
            .ends_with("production is back on worker-compose.yaml."),
        "{}",
        err.message
    );
    assert_eq!(
        f.log(),
        vec![
            format!("switch to {NEXT}"),
            "switch to worker-compose.yaml".to_string()
        ]
    );
}

#[tokio::test]
async fn a_check_of_the_environments_own_file_switches_nothing() {
    let s = setup(plan(false)).await;
    s.h.ctx.store.update(|st| {
        st.check.as_mut().unwrap().manifest = Some("worker-compose.yaml".into());
        st.checks[0].manifest = Some("worker-compose.yaml".into());
    });
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "queued",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(deployment(DEP_ID, "x", "ready", "push"))
        })
        .mount(&s.server)
        .await;
    let d = deploy(&s.h.ctx, req("chk", Some(12), None)).await.unwrap();
    assert_eq!(d.id, DEP_ID);
    assert!(f.log().is_empty(), "{:?}", f.log());
    let requests = s.server.received_requests().await.unwrap();
    assert_eq!(planned_files(&requests), vec![None]);
}

#[tokio::test]
async fn a_changed_plan_after_the_merge_whose_push_was_canceled_puts_the_old_file_back() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    // The merged commit plans differently (with the new file all the same).
    let mut changed = next_plan(false);
    changed["settings"] =
        json!({"release": {"worker": "api", "command": "x", "timeout": "1m", "source": "file"}});
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"can_deploy": true, "fingerprint": "fp2", "plan": changed})),
        )
        .with_priority(1)
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "queued",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{}/{DEP_ID}/cancel", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&s.server)
        .await;
    let err = deploy(&s.h.ctx, req("chk", Some(12), None))
        .await
        .unwrap_err();
    assert_eq!(err.code, "PLAN_CHANGED");
    assert!(
        err.message
            .ends_with("production is back on worker-compose.yaml."),
        "{}",
        err.message
    );
    assert_eq!(
        f.log(),
        vec![
            format!("switch to {NEXT}"),
            "switch to worker-compose.yaml with the commit on main".to_string()
        ]
    );
}

#[tokio::test]
async fn a_push_still_running_after_a_changed_plan_keeps_the_switch_and_says_so() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    let mut changed = next_plan(false);
    changed["settings"] =
        json!({"release": {"worker": "api", "command": "x", "timeout": "1m", "source": "file"}});
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"can_deploy": true, "fingerprint": "fp2", "plan": changed})),
        )
        .with_priority(1)
        .mount(&s.server)
        .await;
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(PushList {
            origin: s.repo.origin.clone(),
            status: "in_progress",
        })
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{}/{DEP_ID}/cancel", dep_base())))
        .respond_with(
            ResponseTemplate::new(409).set_body_json(
                json!({"code": "CONFLICT", "message": "can't be canceled any more"}),
            ),
        )
        .mount(&s.server)
        .await;
    let err = deploy(&s.h.ctx, req("chk", Some(12), None))
        .await
        .unwrap_err();
    assert_eq!(err.code, "PUSH_STILL_RUNNING");
    assert!(
        err.message
            .ends_with(&format!("production now uses {NEXT}.")),
        "{}",
        err.message
    );
    assert_eq!(f.log(), vec![format!("switch to {NEXT}")]);
}

#[tokio::test]
async fn a_create_whose_answer_is_lost_keeps_the_switch() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 307).await;
    pushed_directly(&s).await;
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.status), ("CLOUD_ERROR", None));
    assert!(
        err.message.ends_with(&format!(
            "The answer was lost: production stays on {NEXT}; see Deploys."
        )),
        "{}",
        err.message
    );
    let log = f.log();
    assert_eq!(log[0], format!("switch to {NEXT} with the commit on main"));
    assert!(
        log[1..].iter().all(|l| l.starts_with("create with")),
        "no switch back: {log:?}"
    );
    assert_eq!(*f.manifest.lock().unwrap(), NEXT);
}

async fn deploy_chk(s: &Setup) -> Result<Deploy, iii_cloud::error::CloudError> {
    deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
}

async fn reuse(s: &Setup) -> Result<Project, iii_cloud::error::CloudError> {
    use_project(
        &s.h.ctx,
        UseRequest {
            path: s.repo.checkout.to_string_lossy().into(),
            environment_id: Some(ENV.into()),
            mode: WorkspaceMode::Worktree,
            cloud_project_id: None,
            from: None,
            source: None,
        },
    )
    .await
}

/// A check is of the source and compose file the workspace had when it was made.
#[tokio::test]
async fn deploy_refuses_a_check_whose_source_or_compose_file_was_replaced() {
    let s = setup(plan(false)).await;
    // A check as `check` makes it: it names the compose file it was made for.
    s.h.ctx.store.update(|st| {
        for c in st.check.iter_mut().chain(st.checks.iter_mut()) {
            c.manifest = Some("worker-compose.yaml".into());
        }
    });
    // Selecting the environment again changes nothing: the check is still deployable.
    reuse(&s).await.unwrap();
    // It goes past the guard (this cloud has no deployments route, so the call ends there).
    let before = deploy_chk(&s).await.unwrap_err();
    assert_eq!(before.message, "iii cloud answered 404");
    // The environment's compose file is another one now (a refused check leaves C1 the latest).
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": ENV, "project_id": "p", "name": "production", "production": true, "branch": "main", "manifest": "other/worker-compose.yaml", "url": null, "settings": {}})))
        .with_priority(1)
        .mount(&s.server)
        .await;
    reuse(&s).await.unwrap();
    assert_eq!(s.h.ctx.store.read(|st| st.check.clone().unwrap().id), "chk");
    let err = deploy_chk(&s).await.unwrap_err();
    assert_eq!(err.code, "SOURCE_CHANGED");
    assert!(
        err.message
            .contains("production's source or compose file changed")
    );
    // A check that asked for another file on purpose is not refused for the environment's.
    s.h.ctx.store.update(|st| {
        for c in st.check.iter_mut().chain(st.checks.iter_mut()) {
            c.manifest = Some("other/asked.yaml".into());
        }
    });
    assert_ne!(
        deploy_chk(&s).await.err().map(|e| e.code),
        Some("SOURCE_CHANGED".to_string())
    );
}

#[tokio::test]
async fn deploy_refuses_a_check_made_for_another_source() {
    let s = setup(plan(false)).await;
    // `use` with a source is never for production, so the workspace's source is set as `use` would.
    s.h.ctx.store.update(|st| {
        st.project
            .as_mut()
            .unwrap()
            .workspace
            .as_mut()
            .unwrap()
            .source = Some("feature-x".into())
    });
    let err = deploy(
        &s.h.ctx,
        DeployRequest {
            handoff_id: None,
            check_id: "chk".into(),
            pr: Some(12),
            deployment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "SOURCE_CHANGED");
    // Another branch of the checkout with no name given: only its commit differs.
    s.h.ctx.store.update(|st| {
        let w = st.project.as_mut().unwrap().workspace.as_mut().unwrap();
        w.source = None;
        w.source_sha = Some("0123456789012345678901234567890123456789".into());
    });
    assert_eq!(deploy_chk(&s).await.unwrap_err().code, "SOURCE_CHANGED");
}

/// The event carries what the status read says about where a deployment came from.
#[tokio::test]
async fn a_deploy_event_says_whether_it_came_from_the_console_like_the_status_does() {
    let s = setup(plan(false)).await;
    let mine = "11111111-aaaa-4aaa-8aaa-111111111111";
    let theirs = "22222222-bbbb-4bbb-8bbb-222222222222";
    s.h.ctx
        .store
        .update(|st| st.worker_deploys.push(mine.into()));
    for id in [mine, theirs] {
        let mut d = iii_cloud::watch::to_deploy(
            "http://x",
            (None, None, "production"),
            &deployment(id, "abc1234", "needs_action", "manual"),
            None,
            false,
        )
        .unwrap();
        d.from_console = !d.from_console; // whatever the record said, the event follows the rule
        iii_cloud::watch::record(&s.h.ctx, &d);
    }
    let events = s.h.fake.events.lock().unwrap().clone();
    let of = |id: &str| {
        events
            .iter()
            .rev()
            .find(|e| e.kind == EventKind::Deploy && e.id == id)
            .map(|e| e.record["from_console"].clone())
    };
    assert_eq!(of(mine), Some(json!(false)));
    assert_eq!(of(theirs), Some(json!(true)));
    let st = iii_cloud::status::status(&s.h.ctx, StatusRequest::default())
        .await
        .unwrap();
    for d in st.deploys {
        assert_eq!(d.from_console, d.id != mine, "{}", d.id);
    }
}

/// The compose file is compared with the cloud's own, not the cache's: a change made in
/// the console before the next refresh is not undone by the deploy.
#[tokio::test]
async fn deploy_refuses_when_the_cloud_has_another_compose_file_than_the_check_before_any_refresh()
{
    let s = setup(plan(false)).await;
    s.h.ctx.store.update(|st| {
        for c in st.check.iter_mut().chain(st.checks.iter_mut()) {
            c.manifest = Some("worker-compose.yaml".into());
        }
    });
    // The console changed the file; the worker's cache still says the old one.
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": ENV, "project_id": "p", "name": "production", "production": true, "branch": "main", "manifest": "console/z.yaml", "url": null, "settings": {}})))
        .with_priority(1)
        .mount(&s.server)
        .await;
    assert_eq!(
        s.h.ctx.store.read(|st| st
            .project
            .as_ref()
            .and_then(|p| p.environment.as_ref())
            .and_then(|e| e.manifest.clone())),
        Some("worker-compose.yaml".to_string())
    );
    let err = deploy_chk(&s).await.unwrap_err();
    assert_eq!(err.code, "SOURCE_CHANGED");
    let requests = s.server.received_requests().await.unwrap();
    assert!(
        !requests.iter().any(|r| r.method.as_str() == "PATCH"),
        "the environment was switched back"
    );
}

/// A teammate's push deployment is not "from the console"; a manual one made there is.
#[tokio::test]
async fn only_a_manual_deployment_this_worker_did_not_make_is_from_the_console() {
    let s = setup(plan(false)).await;
    let push = "33333333-cccc-4ccc-8ccc-333333333333";
    let manual = "44444444-dddd-4ddd-8ddd-444444444444";
    for (id, kind) in [(push, "push"), (manual, "manual")] {
        let d = iii_cloud::watch::to_deploy(
            "http://x",
            (None, None, "production"),
            &deployment(id, "abc1234", "needs_action", kind),
            None,
            false,
        )
        .unwrap();
        iii_cloud::watch::record(&s.h.ctx, &d);
    }
    let events = s.h.fake.events.lock().unwrap().clone();
    let of = |id: &str| {
        events
            .iter()
            .rev()
            .find(|e| e.kind == EventKind::Deploy && e.id == id)
            .map(|e| e.record["from_console"].clone())
    };
    assert_eq!(of(push), Some(json!(false)));
    assert_eq!(of(manual), Some(json!(true)));
    let st = iii_cloud::status::status(&s.h.ctx, StatusRequest::default())
        .await
        .unwrap();
    for d in st.deploys {
        assert_eq!(d.from_console, d.id == manual, "{}", d.id);
    }
}

/// A check made while another environment was selected has its compose file on record
/// and no source: the console's change is still seen, the source half does not apply.
#[tokio::test]
async fn a_check_made_while_another_environment_was_selected_is_guarded_by_its_compose_file() {
    let s = setup(plan(false)).await;
    s.h.ctx.store.update(|st| {
        for c in st.check.iter_mut().chain(st.checks.iter_mut()) {
            c.manifest = Some("worker-compose.yaml".into());
        }
        st.check_sources.insert(
            "chk".into(),
            iii_cloud::store::CheckedFor {
                source: None,
                source_sha: None,
                workspace: false,
                compose: Some("worker-compose.yaml".into()),
            },
        );
        // A source on the workspace now is nothing the check was recorded for.
        st.project
            .as_mut()
            .unwrap()
            .workspace
            .as_mut()
            .unwrap()
            .source = Some("feat-x".into());
    });
    // The cloud's file is the check's: it goes past the guard.
    let ok = deploy_chk(&s).await.unwrap_err();
    assert_ne!(ok.code, "SOURCE_CHANGED");
    // The console changes the file: refused, and the environment is not switched back.
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": ENV, "project_id": "p", "name": "production", "production": true, "branch": "main", "manifest": "console/y.yaml", "url": null, "settings": {}})))
        .with_priority(1)
        .mount(&s.server)
        .await;
    assert_eq!(deploy_chk(&s).await.unwrap_err().code, "SOURCE_CHANGED");
    let requests = s.server.received_requests().await.unwrap();
    assert!(!requests.iter().any(|r| r.method.as_str() == "PATCH"));
}

#[tokio::test]
async fn a_commit_already_on_the_branch_is_deployed_after_a_short_wait_when_no_push_deployment_comes()
 {
    let s = setup(plan(false)).await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    direct(&s).await;
    git(&s.ws, &["push", "-q", "origin", "HEAD:main"]);
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"deployments": [], "next_cursor": null})),
        )
        .mount(&s.server)
        .await;
    let started = std::time::Instant::now();
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!((d.id.as_str(), d.approved_by_worker), (NEW_ID, true));
    assert_eq!(f.log(), vec!["create with worker-compose.yaml"]);
    // 120 ticks of 10 ms is the old wait; the grace is 10 ticks.
    assert!(started.elapsed() < Duration::from_millis(900));
    assert_eq!(git(&s.repo.origin, &["rev-parse", "main"]), s.check.commit);
}

// ---------------------------------------------------- the first assisted deploy

const OWN: &str = "iii-cloud/production";

/// A first assisted deploy: not production, on the default branch, signed in with the browser.
async fn first_assisted(s: &Setup) {
    not_production(s).await;
    s.h.ctx.store.update(|st| {
        st.connection.as_mut().unwrap().method = Some(Method::Browser);
    });
}

/// The deployments list. The checked commit's push deployment shows only once
/// the environment follows `OWN` and the branch is on origin (what the real
/// webhook needs), so a deploy that adopts it has made it happen first.
async fn list_of_commit(s: &Setup, f: &EnvFile, status: Option<&'static str>) {
    let sha = s.check.commit.clone();
    let (origin, branch) = (s.repo.origin.clone(), f.branch.clone());
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(move |_: &Request| {
            let follows = *branch.lock().unwrap() == OWN
                && std::process::Command::new("git")
                    .args(["rev-parse", "--verify", "-q", &format!("refs/heads/{OWN}")])
                    .current_dir(&origin)
                    .output()
                    .unwrap()
                    .status
                    .success();
            let all: Vec<Value> = status
                .filter(|_| follows)
                .map(|st| deployment(DEP_ID, &sha, st, "push"))
                .into_iter()
                .collect();
            ResponseTemplate::new(200)
                .set_body_json(json!({"deployments": all, "next_cursor": null}))
        })
        .mount(&s.server)
        .await;
    one_deployment(s).await;
}

async fn one_deployment(s: &Setup) {
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(|_: &Request| {
            ResponseTemplate::new(200).set_body_json(deployment(DEP_ID, "x", "ready", "push"))
        })
        .mount(&s.server)
        .await;
}

async fn requests_of(s: &Setup, verb: &str) -> Vec<Request> {
    s.server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| {
            r.method.as_str() == verb && r.url.path().ends_with(&format!("environments/{ENV}"))
        })
        .collect()
}

fn origin_branch(s: &Setup, branch: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{branch}"),
        ])
        .current_dir(&s.repo.origin)
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[tokio::test]
async fn the_first_assisted_deploy_pushes_the_commit_to_its_own_branch_moves_the_environment_and_adopts_the_push()
 {
    let s = setup(plan(false)).await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    list_of_commit(&s, &f, Some("queued")).await;
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!((d.id.as_str(), d.approved_by_worker), (DEP_ID, false));
    assert_eq!(
        origin_branch(&s, OWN).as_deref(),
        Some(s.check.commit.as_str())
    );
    assert_eq!(
        git(&s.repo.origin, &["rev-parse", "main"]),
        s.check.base,
        "the default branch is never pushed"
    );
    // One write, made after the branch exists on origin; nothing else.
    assert_eq!(f.log(), vec![format!("move to {OWN}")]);
    let patches = requests_of(&s, "PATCH").await;
    assert_eq!(patches.len(), 1);
    assert_eq!(
        serde_json::from_slice::<Value>(&patches[0].body).unwrap(),
        json!({"branch": OWN})
    );
    // The page's record follows.
    let env =
        s.h.ctx
            .store
            .read(|st| st.project.as_ref().unwrap().environment.clone().unwrap());
    assert_eq!(env.branch, OWN);
}

#[tokio::test]
async fn with_no_push_deployment_the_first_assisted_deploy_creates_one_after_a_short_wait() {
    let s = setup(plan(false)).await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    list_of_commit(&s, &f, None).await;
    let started = std::time::Instant::now();
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!((d.id.as_str(), d.approved_by_worker), (NEW_ID, true));
    // The old wait was 120 ticks of 10 ms; the grace is 10.
    assert!(started.elapsed() < Duration::from_millis(900));
    let posts = requests_of_path(&s, "POST", &dep_base()).await;
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].headers.get("if-match").unwrap().to_str().unwrap(),
        "fp-target"
    );
    assert_eq!(
        f.log(),
        vec![
            format!("move to {OWN}"),
            "create with worker-compose.yaml".to_string()
        ]
    );
}

async fn requests_of_path(s: &Setup, verb: &str, p: &str) -> Vec<Request> {
    s.server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == p)
        .collect()
}

#[tokio::test]
async fn a_first_assisted_deploy_of_another_compose_file_moves_the_branch_and_the_file_in_one_write()
 {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    list_of_commit(&s, &f, Some("needs_action")).await;
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!(d.id, NEW_ID);
    assert_eq!(
        f.log(),
        vec![
            format!("move to {OWN} and switch to {NEXT}"),
            format!("create with {NEXT}")
        ],
        "no manifest write of its own before the move"
    );
    assert_eq!(requests_of(&s, "PATCH").await.len(), 1);
}

#[tokio::test]
async fn a_branch_of_that_name_with_other_commits_is_never_touched() {
    let s = setup(plan(false)).await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    let t = tempfile::tempdir().unwrap();
    git(
        t.path(),
        &["clone", "-q", s.repo.origin.to_str().unwrap(), "c"],
    );
    let c = t.path().join("c");
    std::fs::write(c.join("other.txt"), "o").unwrap();
    git(&c, &["add", "."]);
    git(&c, &["commit", "-m", "other"]);
    git(&c, &["push", "-q", "origin", &format!("HEAD:{OWN}")]);
    let theirs = git(&s.repo.origin, &["rev-parse", OWN]);
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "OWN_BRANCH_TAKEN", "{}", err.message);
    assert!(err.message.len() < 200 && err.message.ends_with("Nothing changed."));
    assert_eq!(git(&s.repo.origin, &["rev-parse", OWN]), theirs);
    assert!(f.log().is_empty() && requests_of(&s, "PATCH").await.is_empty());
}

fn refuse_pushes(s: &Setup) {
    use std::os::unix::fs::PermissionsExt;
    let hook = s.repo.origin.join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'protected branch' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn a_refused_push_moves_nothing() {
    let s = setup(plan(false)).await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    refuse_pushes(&s);
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "PUSH_REFUSED", "{}", err.message);
    assert!(origin_branch(&s, OWN).is_none());
    assert!(f.log().is_empty() && requests_of(&s, "PATCH").await.is_empty());
}

#[tokio::test]
async fn a_refused_move_is_retried_by_the_second_click_without_pushing_again() {
    let s = setup(plan(false)).await;
    Mock::given(method("PATCH"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"code": "FORBIDDEN", "message": "no"})),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&s.server)
        .await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    list_of_commit(&s, &f, Some("queued")).await;
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "BRANCH_NOT_MOVED", "{}", err.message);
    assert!(err.message.contains("answered 403") && err.message.contains("Deploy again"));
    // The branch is on origin, no environment follows it, nothing deployed.
    assert_eq!(
        origin_branch(&s, OWN).as_deref(),
        Some(s.check.commit.as_str())
    );
    assert!(f.log().is_empty());
    // Hand the second push a hook that refuses: it must not be asked to push.
    refuse_pushes(&s);
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!(d.id, DEP_ID);
    assert_eq!(requests_of(&s, "PATCH").await.len(), 2);
    assert_eq!(f.log(), vec![format!("move to {OWN}")]);
}

#[tokio::test]
async fn a_refused_create_leaves_the_environment_on_its_branch_and_the_second_click_goes_straight_to_it()
 {
    let s = setup(plan(false)).await;
    Mock::given(method("POST"))
        .and(path(dep_base()))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({"code": "UNPROCESSABLE", "message": "commit: not a commit of the environment's branch."})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&s.server)
        .await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    list_of_commit(&s, &f, None).await;
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.status, Some(422));
    assert!(
        err.message.ends_with(&format!(
            "production now follows {OWN}; main was not pushed."
        )),
        "{}",
        err.message
    );
    refuse_pushes(&s);
    let started = std::time::Instant::now();
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!(d.id, NEW_ID);
    assert!(started.elapsed() < Duration::from_millis(900));
    assert_eq!(
        f.log(),
        vec![
            format!("move to {OWN}"),
            "create with worker-compose.yaml".to_string()
        ]
    );
}

#[tokio::test]
async fn a_default_branch_that_moved_since_the_check_pushes_nothing() {
    let s = setup(plan(false)).await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    let t = tempfile::tempdir().unwrap();
    git(
        t.path(),
        &["clone", "-q", s.repo.origin.to_str().unwrap(), "c"],
    );
    let c = t.path().join("c");
    std::fs::write(c.join("other.txt"), "o").unwrap();
    git(&c, &["add", "."]);
    git(&c, &["commit", "-m", "other"]);
    git(&c, &["push", "-q", "origin", "HEAD:main"]);
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "MAIN_MOVED");
    assert!(origin_branch(&s, OWN).is_none() && f.log().is_empty());
}

#[tokio::test]
async fn an_api_key_cannot_move_an_environment_so_nothing_is_pushed() {
    let s = setup(plan(false)).await;
    not_production(&s).await;
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "NEEDS_BROWSER_SIGN_IN");
    assert!(origin_branch(&s, OWN).is_none());
}

#[tokio::test]
async fn production_never_pushes_whatever_the_default_branch_is() {
    let s = setup(plan(false)).await;
    s.h.ctx
        .store
        .update(|st| st.project.as_mut().unwrap().default_branch = Some("trunk".into()));
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "MERGE_FIRST");
    assert_eq!(git(&s.repo.origin, &["rev-parse", "main"]), s.check.base);
}

#[tokio::test]
async fn a_branch_another_environment_follows_gets_its_own_branch_for_this_one() {
    let s = setup(plan(false)).await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    // `main` is not the default here, but production (another environment) follows it.
    s.h.ctx.store.update(|st| {
        let p = st.project.as_mut().unwrap();
        p.default_branch = Some("trunk".into());
        p.environments = vec![EnvironmentBrief {
            id: "other".into(),
            name: "live".into(),
            production: true,
            branch: "main".into(),
        }];
    });
    list_of_commit(&s, &f, Some("queued")).await;
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!(d.id, DEP_ID);
    assert_eq!(f.log(), vec![format!("move to {OWN}")]);
    assert_eq!(git(&s.repo.origin, &["rev-parse", "main"]), s.check.base);
}

#[tokio::test]
async fn only_production_reads_a_pull_request_and_a_stale_one_is_cleared() {
    let s = setup(plan(false)).await;
    assert!(
        current_pr(&s).await.is_some(),
        "production on main reads it"
    );
    // The same environment, not production, still on the default branch.
    not_production(&s).await;
    assert!(current_pr(&s).await.is_none());
    assert!(
        s.h.ctx
            .store
            .read(|st| st.pr.is_none() && st.pr_error.is_none())
    );
}

/// The list always holds one deployment of the checked commit, made before this deploy began.
async fn list_with_older(s: &Setup, status: &'static str) {
    let sha = s.check.commit.clone();
    Mock::given(method("GET"))
        .and(path(dep_base()))
        .respond_with(move |_: &Request| {
            ResponseTemplate::new(200).set_body_json(
                json!({"deployments": [deployment(DEP_ID, &sha, status, "push")], "next_cursor": null}),
            )
        })
        .with_priority(1)
        .mount(&s.server)
        .await;
    one_deployment(s).await;
}

#[tokio::test]
async fn an_older_deployment_of_the_commit_is_never_taken_for_this_deploys_own() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    list_with_older(&s, "ready").await;
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!(
        (d.id.as_str(), d.approved_by_worker),
        (NEW_ID, true),
        "a new deployment carries the switch"
    );
    assert_eq!(
        f.log(),
        vec![
            format!("move to {OWN} and switch to {NEXT}"),
            format!("create with {NEXT}")
        ]
    );
}

#[tokio::test]
async fn an_older_running_deployment_of_the_commit_is_not_cancelled_when_the_plan_changed() {
    let s = setup(next_plan(false)).await;
    check_of_next(&s);
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    let mut changed = next_plan(false);
    changed["settings"] =
        json!({"release": {"worker": "a", "command": "x", "timeout": "1m", "source": "file"}});
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"can_deploy": true, "fingerprint": "fp2", "plan": changed})),
        )
        .with_priority(1)
        .mount(&s.server)
        .await;
    list_with_older(&s, "in_progress").await;
    Mock::given(method("POST"))
        .and(path(format!("{}/{DEP_ID}/cancel", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(&s.server)
        .await;
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "PLAN_CHANGED", "{}", err.message);
    assert!(
        err.message.contains(&format!("now follows {OWN}")),
        "{}",
        err.message
    );
    assert!(f.log().iter().any(|l| l == "switch to worker-compose.yaml"));
}

#[tokio::test]
async fn a_branch_another_environment_follows_is_never_pushed_even_as_the_own_branch() {
    let s = setup(plan(false)).await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    // `qa` (once `test`) still follows the branch this environment would take as its own.
    s.h.ctx.store.update(|st| {
        st.project.as_mut().unwrap().environments = vec![EnvironmentBrief {
            id: "qa".into(),
            name: "qa".into(),
            production: false,
            branch: OWN.into(),
        }];
    });
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert_eq!(err.code, "OWN_BRANCH_TAKEN");
    assert!(
        err.message.contains("followed by qa") && err.message.ends_with("Nothing changed."),
        "{}",
        err.message
    );
    assert!(origin_branch(&s, OWN).is_none() && f.log().is_empty());
}

#[tokio::test]
async fn an_older_checked_commit_on_the_own_branch_is_fast_forwarded() {
    let s = setup(plan(false)).await;
    let f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    // An earlier attempt pushed the base (an ancestor of the checked commit).
    git(&s.repo.origin, &["branch", OWN, &s.check.base]);
    list_of_commit(&s, &f, Some("queued")).await;
    let d = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap();
    assert_eq!(d.id, DEP_ID);
    assert_eq!(
        origin_branch(&s, OWN).as_deref(),
        Some(s.check.commit.as_str())
    );
    assert_eq!(f.log(), vec![format!("move to {OWN}")]);
}

#[tokio::test]
async fn confirming_a_held_push_deployment_of_the_checked_commit_is_not_a_merge() {
    let s = setup(plan(false)).await;
    // The check has two commits on its base: a merge check would refuse it (MAIN_MOVED).
    std::fs::write(s.ws.join("second.txt"), "2\n").unwrap();
    git(&s.ws, &["add", "."]);
    git(&s.ws, &["commit", "-m", "second"]);
    let head = git(&s.ws, &["rev-parse", "HEAD"]);
    s.h.ctx.store.update(|st| {
        for c in st.checks.iter_mut().chain(st.check.iter_mut()) {
            c.commit = head.clone();
            c.run.as_mut().unwrap().commit = head.clone();
        }
    });
    let _f = env_file(&s, "worker-compose.yaml", 201).await;
    not_production(&s).await;
    let held = deployment(DEP_ID, &head, "needs_action", "push");
    Mock::given(method("GET"))
        .and(path(format!("{}/{DEP_ID}", dep_base())))
        .respond_with(ResponseTemplate::new(200).set_body_json(held))
        .with_priority(1)
        .mount(&s.server)
        .await;
    let d = deploy(&s.h.ctx, req("chk", None, Some(DEP_ID)))
        .await
        .unwrap();
    assert_eq!((d.id.as_str(), d.approved_by_worker), (NEW_ID, true));
}

#[tokio::test]
async fn a_pull_request_is_refused_for_an_environment_that_is_not_production() {
    let s = setup(plan(false)).await;
    not_production(&s).await;
    let err = deploy(&s.h.ctx, req("chk", Some(12), None))
        .await
        .unwrap_err();
    assert_eq!(err.code, "NO_PULL_REQUEST");
    assert!(!argv(&s).contains("pr merge") && !argv(&s).contains("pr view"));
    assert_eq!(git(&s.repo.origin, &["rev-parse", "main"]), s.check.base);
}

#[tokio::test]
async fn the_push_decision_reads_the_environments_and_the_default_branch_live() {
    // An environment made in the console after the folder was read follows `main` too.
    let s = setup(plan(false)).await;
    let _f = env_file(&s, "worker-compose.yaml", 201).await;
    set_default_branch(&s, "trunk");
    first_assisted(&s).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/projects/p/environments")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": [
            {"id": ENV, "name": "production", "production": false, "branch": "main", "manifest": null, "url": null},
            {"id": "preview-id", "name": "preview", "production": false, "branch": "main", "manifest": null, "url": null}
        ]})))
        .with_priority(1)
        .mount(&s.server)
        .await;
    let before = git(&s.repo.origin, &["rev-parse", "main"]);
    let _ = deploy(&s.h.ctx, req("chk", None, None)).await;
    assert_eq!(
        git(&s.repo.origin, &["rev-parse", "main"]),
        before,
        "the shared branch is not pushed"
    );
    assert_eq!(
        origin_branch(&s, OWN).as_deref(),
        Some(s.check.commit.as_str())
    );

    // A default branch renamed after the folder was read: the store still says `trunk`.
    let s = setup(plan(false)).await;
    let _f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    s.h.ctx
        .store
        .update(|st| st.project.as_mut().unwrap().default_branch = Some("trunk".into()));
    let before = git(&s.repo.origin, &["rev-parse", "main"]);
    let _ = deploy(&s.h.ctx, req("chk", None, None)).await;
    assert_eq!(
        git(&s.repo.origin, &["rev-parse", "main"]),
        before,
        "main is the default now"
    );
    assert_eq!(
        origin_branch(&s, OWN).as_deref(),
        Some(s.check.commit.as_str())
    );
}

#[tokio::test]
async fn when_the_environments_cannot_be_read_nothing_is_pushed() {
    let s = setup(plan(false)).await;
    let _f = env_file(&s, "worker-compose.yaml", 201).await;
    first_assisted(&s).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/teams/{TEAM}/projects/p/environments"
        )))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&s.server)
        .await;
    let err = deploy(&s.h.ctx, req("chk", None, None)).await.unwrap_err();
    assert!(
        err.message.contains("environments") && err.message.ends_with("Try again."),
        "{}",
        err.message
    );
    assert!(origin_branch(&s, OWN).is_none());
    assert_eq!(git(&s.repo.origin, &["rev-parse", "main"]), s.check.base);
}

#[tokio::test]
async fn a_checks_file_is_kept_only_while_the_environment_still_has_the_file_it_had_then() {
    let s = setup(plan(false)).await;
    let check = |id: &str, env: &str, file: Option<&str>| Check {
        id: id.into(),
        environment_id: env.into(),
        manifest: file.map(str::to_owned),
        ..s.check.clone()
    };
    let at = |compose: &str| iii_cloud::store::CheckedFor {
        source: None,
        source_sha: None,
        workspace: true,
        compose: Some(compose.into()),
    };
    let mut st = iii_cloud::store::State::default();
    // Newest first, two environments interleaved: A's work is on w.yaml (its environment had y.yaml
    // then), B's on b.yaml, and A's older check named nothing.
    st.checks = vec![
        check("b1", "B", Some("b.yaml")),
        check("a2", "A", Some("w.yaml")),
        check("b0", "B", None),
        check("a1", "A", None),
    ];
    st.check_sources.insert("b1".into(), at("main-b.yaml"));
    st.check_sources.insert("a2".into(), at("y.yaml"));
    // Unchanged in the console: the work stays on the check's file, each environment on its own.
    assert_eq!(st.work_file("A", Some("y.yaml")).as_deref(), Some("w.yaml"));
    assert_eq!(
        st.work_file("B", Some("main-b.yaml")).as_deref(),
        Some("b.yaml")
    );
    // Not known now: the check's file stays (nothing says it changed).
    assert_eq!(st.work_file("A", None).as_deref(), Some("w.yaml"));
    // The person set another file in the console: the check must not undo it.
    assert_eq!(st.work_file("A", Some("x.yaml")), None);
    // B's change does not touch A.
    assert_eq!(st.work_file("A", Some("y.yaml")).as_deref(), Some("w.yaml"));
    assert_eq!(st.work_file("B", Some("other.yaml")), None);
    assert_eq!(st.work_file("C", Some("y.yaml")), None);
}
